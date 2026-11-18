//! Port of `lib/runtime/QueryRuntimeHelper.java`: where the output of query, cquery and aquery
//! goes (stdout, or the file of `--output_file`).
//!
//! PORT: the Java interface lives in `lib/runtime`; `blaze_runtime` sits above the query crates
//! that use it, so it is ported next to the query output types.

use std::cell::RefCell;
use std::rc::Rc;

use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_vfs::Path;

use super::output_formatter::OutputStream;

/// The exception of a [`create`] failure.
// Java: com/google/devtools/build/lib/runtime/QueryRuntimeHelper.java:128 QueryRuntimeHelper.QueryRuntimeHelperException
#[derive(Clone, Debug)]
pub struct QueryRuntimeHelperException {
    message: String,
    detailed_code: failure_details::query::Code,
}

impl QueryRuntimeHelperException {
    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/runtime/QueryRuntimeHelper.java:142 QueryRuntimeHelperException.getFailureDetail
    pub fn get_failure_detail(&self) -> FailureDetail {
        FailureDetail {
            message: self.message.clone(),
            category: Some(Category::Query(failure_details::Query {
                code: self.detailed_code as i32,
            })),
        }
    }
}

impl std::fmt::Display for QueryRuntimeHelperException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for QueryRuntimeHelperException {}

/// `StdoutQueryRuntimeHelperFactory.create(env, options)` followed by
/// `getOutputStreamForQueryOutput()`: `stdout` (`env.getReporter().getOutErr().getOutputStream()`)
/// when `--output_file` is empty, else the file `working_directory/<output_file>`.
///
/// PORT: Java closes the file in `QueryRuntimeHelper.close()` ("Could not close query output
/// file"); here the file closes when the last handle of the stream is dropped, and a close error
/// cannot be observed.
// Java: com/google/devtools/build/lib/runtime/QueryRuntimeHelper.java:57 StdoutQueryRuntimeHelperFactory.create
pub fn create(
    stdout: &OutputStream,
    working_directory: &Path,
    output_file: &str,
) -> Result<OutputStream, QueryRuntimeHelperException> {
    if output_file.is_empty() {
        return Ok(stdout.clone());
    }
    // Java: com/google/devtools/build/lib/runtime/QueryRuntimeHelper.java:99 FileQueryRuntimeHelper.create
    let path = working_directory.get_relative_str(output_file);
    match std::fs::File::create(path.get_path_string()) {
        Ok(file) => Ok(Rc::new(RefCell::new(file))),
        Err(_) => Err(QueryRuntimeHelperException {
            message: format!(
                "Could not open query output file {}",
                path.get_path_string()
            ),
            detailed_code: failure_details::query::Code::QueryOutputWriteFailure,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_vfs::{DigestHashFunction, FileSystemExt, UnixFileSystem};

    fn stdout() -> OutputStream {
        Rc::new(RefCell::new(Vec::<u8>::new()))
    }

    // QueryRuntimeHelper has no JUnit test at the pin; these check the factory's two branches.
    #[test]
    fn empty_output_file_is_stdout() {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let out = stdout();
        let got = create(&out, &fs.get_path("/tmp"), "").unwrap();
        assert!(Rc::ptr_eq(&out, &got));
    }

    #[test]
    fn output_file_is_relative_to_the_working_directory() {
        let dir = std::env::temp_dir().join(format!("blaze-qrh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let wd = fs.get_path(dir.to_str().unwrap());
        let out = create(&stdout(), &wd, "out.txt").unwrap();
        std::io::Write::write_all(&mut *out.borrow_mut(), b"//x:y\n").unwrap();
        drop(out);
        assert_eq!(std::fs::read(dir.join("out.txt")).unwrap(), b"//x:y\n");
        let Err(err) = create(&stdout(), &wd, "missing-dir/out.txt") else {
            panic!("expected QueryRuntimeHelperException");
        };
        assert_eq!(
            err.get_message(),
            format!(
                "Could not open query output file {}/missing-dir/out.txt",
                dir.display()
            )
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
