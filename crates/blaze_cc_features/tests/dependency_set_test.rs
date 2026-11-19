//! Port of `src/test/java/com/google/devtools/build/lib/util/DependencySetTest.java`.
//!
//! PORT: Java uses Scratch's in-memory file system with root "/". Blaze has no in-memory file
//! system, so the `.d` files live in a temporary directory on the real one. Parsing tests keep root
//! "/"; the two write tests use the temporary directory as root, so the expected text has that
//! directory in front of the absolute dependency paths.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_cc_features::dependency_set::DependencySet;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};

struct Scratch {
    dir: tempfile::TempDir,
    fs: FileSystemRef,
}

impl Scratch {
    fn new() -> Scratch {
        Scratch {
            dir: tempfile::tempdir().unwrap(),
            fs: Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256)),
        }
    }

    fn path(&self, p: &str) -> Path {
        Path::create_from_str(p, &self.fs)
    }

    /// Java: `scratch.file(name, lines...)`: each line followed by '\n'. `name` is placed in the
    /// temporary directory.
    fn file(&self, name: &str, lines: &[&str]) -> Path {
        let mut content = String::new();
        for line in lines {
            content.push_str(line);
            content.push('\n');
        }
        self.file_bytes(name, content.as_bytes())
    }

    fn file_bytes(&self, name: &str, content: &[u8]) -> Path {
        let real = format!("{}{}", self.dir.path().display(), name);
        std::fs::create_dir_all(std::path::Path::new(&real).parent().unwrap()).unwrap();
        std::fs::write(&real, content).unwrap();
        self.path(&real)
    }

    fn tmp(&self) -> String {
        self.dir.path().display().to_string()
    }
}

fn new_dependency_set(scratch: &Scratch) -> DependencySet {
    DependencySet::new(scratch.path("/"))
}

fn set_of(paths: &[&Path]) -> HashSet<String> {
    paths
        .iter()
        .map(|p| p.get_path_string().to_string())
        .collect()
}

fn deps(depset: &DependencySet) -> HashSet<String> {
    depset
        .get_dependencies()
        .iter()
        .map(|p| p.get_path_string().to_string())
        .collect()
}

fn read(scratch: &Scratch, dotd: &Path) -> DependencySet {
    let mut depset = new_dependency_set(scratch);
    depset.read(dotd).unwrap();
    depset
}

#[test]
fn dot_d_parser_simple() {
    let s = Scratch::new();
    let file1 = s.path("/usr/local/blah/blah/genhello/hello.cc");
    let file2 = s.path("/usr/local/blah/blah/genhello/hello.h");
    let filename = "hello.o";
    let dotd = s.file(
        "/tmp/foo.d",
        &[
            &format!("{filename}: \\"),
            &format!(" {} \\", file1.get_path_string()),
            &format!(" {} ", file2.get_path_string()),
        ],
    );
    let depset = read(&s, &dotd);
    assert_eq!(deps(&depset), set_of(&[&file1, &file2]));
    assert_eq!(depset.get_output_file_name(), Some(filename));
}

#[test]
fn dot_d_parser_simple_crlf() {
    let s = Scratch::new();
    let file1 = s.path("/usr/local/blah/blah/genhello/hello.cc");
    let file2 = s.path("/usr/local/blah/blah/genhello/hello.h");
    let filename = "hello.o";
    let dotd = s.file(
        "/tmp/foo.d",
        &[
            &format!("{filename}: \\\r"),
            &format!(" {} \\\r", file1.get_path_string()),
            &format!(" {} ", file2.get_path_string()),
        ],
    );
    let depset = read(&s, &dotd);
    assert_eq!(deps(&depset), set_of(&[&file1, &file2]));
    assert_eq!(depset.get_output_file_name(), Some(filename));
}

#[test]
fn dot_d_parser_simple_cr() {
    let s = Scratch::new();
    let file1 = s.path("/usr/local/blah/blah/genhello/hello.cc");
    let file2 = s.path("/usr/local/blah/blah/genhello/hello.h");
    let filename = "hello.o";
    let dotd = s.file(
        "/tmp/foo.d",
        &[&format!(
            "{filename}: \\\r {} \\\r {} ",
            file1.get_path_string(),
            file2.get_path_string()
        )],
    );
    let depset = read(&s, &dotd);
    assert_eq!(deps(&depset), set_of(&[&file1, &file2]));
    assert_eq!(depset.get_output_file_name(), Some(filename));
}

#[test]
fn dot_d_parser_leading_crlf() {
    let s = Scratch::new();
    let file1 = s.path("/usr/local/blah/blah/genhello/hello.cc");
    let file2 = s.path("/usr/local/blah/blah/genhello/hello.h");
    let filename = "hello.o";
    let dotd = s.file(
        "/tmp/foo.d",
        &[&format!(
            "\r\n{filename}: \\\r\n {} \\\r\n {} ",
            file1.get_path_string(),
            file2.get_path_string()
        )],
    );
    let depset = read(&s, &dotd);
    assert_eq!(deps(&depset), set_of(&[&file1, &file2]));
    assert_eq!(depset.get_output_file_name(), Some(filename));
}

#[test]
fn dot_d_parser_odd_formatting() {
    let s = Scratch::new();
    let file1 = s.path("/usr/local/blah/blah/genhello/hello.cc");
    let file2 = s.path("/usr/local/blah/blah/genhello/hello.h");
    let file3 = s.path("/usr/local/blah/blah/genhello/other.h");
    let file4 = s.path("/usr/local/blah/blah/genhello/onemore.h");
    let filename = "hello.o";
    let dotd = s.file(
        "/tmp/foo.d",
        &[
            &format!("{filename}: {} \\", file1.get_path_string()),
            &format!(" {}\\", file2.get_path_string()),
            &format!(" {} {}", file3.get_path_string(), file4.get_path_string()),
        ],
    );
    let depset = read(&s, &dotd);
    assert_eq!(deps(&depset), set_of(&[&file1, &file2, &file3, &file4]));
    assert_eq!(depset.get_output_file_name(), Some(filename));
}

#[test]
fn dot_d_parser_relative_filenames() {
    let s = Scratch::new();
    let root = s.path("/");
    let file1 = root.get_relative_str("hello.cc");
    let file2 = root.get_relative_str("hello.h");
    let filename = "hello.o";
    let dotd = s.file(
        "/tmp/foo.d",
        &[
            &format!("{filename}: \\"),
            &format!(" {} \\", file1.relative_to(&root).get_path_string()),
            &format!(" {} ", file2.relative_to(&root).get_path_string()),
        ],
    );
    let depset = read(&s, &dotd);
    assert_eq!(deps(&depset), set_of(&[&file1, &file2]));
    assert_eq!(depset.get_output_file_name(), Some(filename));
}

#[test]
fn dot_d_parser_escape_dollar() {
    let s = Scratch::new();
    let dotd = s.file(
        "/tmp/foo.d",
        &[
            "hello.o: \\",
            " /usr/local/blah/$$blah/$$hello.cc \\",
            " /usr/local/blah/blah/hel$$$$lo.h \\",
            " /usr/local/blah/$$blah/hello.h",
        ],
    );
    let expected: HashSet<String> = [
        "/usr/local/blah/$blah/$hello.cc",
        "/usr/local/blah/blah/hel$$lo.h",
        "/usr/local/blah/$blah/hello.h",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(deps(&read(&s, &dotd)), expected);
}

#[test]
fn dot_d_parser_empty_file() {
    let s = Scratch::new();
    let dotd = s.file("/tmp/empty.d", &[]);
    let depset = read(&s, &dotd);
    let headers = depset.get_dependencies();
    assert!(
        headers.is_empty(),
        "Not empty: {} {:?}",
        headers.len(),
        headers
    );
    assert_eq!(depset.get_output_file_name(), None);
}

#[test]
fn dot_d_parser_multiple_targets() {
    let s = Scratch::new();
    let file1 = s.path("/usr/local/blah/blah/genhello/hello.cc");
    let file2 = s.path("/usr/local/blah/blah/genhello/hello.h");
    let dotd = s.file(
        "/tmp/foo.d",
        &[
            "hello.o: \\",
            &format!(" {}", file1.get_path_string()),
            "hello2.o: \\",
            &format!(" {}", file2.get_path_string()),
        ],
    );
    assert_eq!(deps(&read(&s, &dotd)), set_of(&[&file1, &file2]));
}

/// Regression test: if gcc fails to execute remotely, and we retry locally, then the behavior of
/// gcc's DEPENDENCIES_OUTPUT option is to append, not overwrite, the .d file. As a result, during
/// retry, a second stanza is written to the file.
///
/// We handle this by merging all of the stanzas.
#[test]
fn dot_d_parser_duplicate_stanza() {
    let s = Scratch::new();
    let file1 = s.path("/usr/local/blah/blah/genhello/hello.cc");
    let file2 = s.path("/usr/local/blah/blah/genhello/hello.h");
    let file3 = s.path("/usr/local/blah/blah/genhello/other.h");
    let dotd = s.file(
        "/tmp/foo.d",
        &[
            "hello.o: \\",
            &format!(" {} \\", file1.get_path_string()),
            &format!(" {} ", file2.get_path_string()),
            "hello.o: \\",
            &format!(" {} \\", file1.get_path_string()),
            &format!(" {} ", file3.get_path_string()),
        ],
    );
    let depset = read(&s, &dotd);
    let mut actual: Vec<&str> = depset
        .get_dependencies()
        .iter()
        .map(|p| p.get_path_string())
        .collect();
    actual.sort_unstable();
    let mut expected = vec![
        file1.get_path_string(),
        file1.get_path_string(),
        file2.get_path_string(),
        file3.get_path_string(),
    ];
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn dot_d_parser_error_on_no_trailing_newline() {
    let s = Scratch::new();
    let file1 = s.path("/usr/local/blah/blah/genhello/hello.cc");
    let dotd = s.file_bytes(
        "/tmp/foo.d",
        format!("hello.o: \\\n {}", file1.get_path_string()).as_bytes(),
    );
    let mut depset = new_dependency_set(&s);
    let e = depset.read(&dotd).unwrap_err();
    assert!(
        e.to_string().contains("File does not end in a newline"),
        "{e}"
    );
}

/// Test compatibility with --config=nvcc, which writes an extra space before the colon.
#[test]
fn dot_d_parser_space_before_colon() {
    let s = Scratch::new();
    let file1 = s.path("/usr/local/blah/blah/genhello/hello.cc");
    let file2 = s.path("/usr/local/blah/blah/genhello/hello.h");
    let filename = "hello.o";
    let dotd = s.file(
        "/tmp/foo.d",
        &[
            &format!("{filename} : \\"),
            &format!(" {} \\", file1.get_path_string()),
            &format!(" {} ", file2.get_path_string()),
        ],
    );
    let depset = read(&s, &dotd);
    assert_eq!(deps(&depset), set_of(&[&file1, &file2]));
    assert_eq!(depset.get_output_file_name(), Some(filename));
}

/// Bug-for-bug compatibility with --config=msvc, which writes malformed .d files.
#[test]
fn dot_d_parser_missing_backslash() {
    let s = Scratch::new();
    let file1 = s.path("/usr/local/blah/blah/genhello/hello.cc");
    let file2 = s.path("/usr/local/blah/blah/genhello/hello.h");
    let filename = "hello.o";
    let dotd = s.file(
        "/tmp/foo.d",
        &[
            &format!("{filename}: "),
            &format!(" {} \\", file1.get_path_string()),
            &format!(" {} ", file2.get_path_string()),
        ],
    );
    let depset = read(&s, &dotd);
    assert!(depset.get_dependencies().is_empty());
}

#[test]
fn write_set() {
    let s = Scratch::new();
    let t = s.tmp();
    let root = s.path(&t);
    let file1 = s.path(&format!("{t}/usr/local/blah/blah/genhello/hello.cc"));
    let file2 = s.path(&format!("{t}/usr/local/blah/blah/genhello/hello.h"));
    let file3 = s.path(&format!("{t}/usr/local/blah/blah/genhello/other.h"));
    let filename = format!("{t}/usr/local/blah/blah/genhello/hello.o");

    let mut dep_set1 = DependencySet::new(root);
    dep_set1.add_dependencies(&[file1, file2, file3]);
    dep_set1.set_output_file_name(&filename);

    let outfile = s.path(&filename);
    let dotd = s.path(&format!("{t}/usr/local/blah/blah/genhello/hello.d"));
    dotd.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    dep_set1.write(&outfile, ".d").unwrap();

    let dotd_contents = std::fs::read_to_string(dotd.get_path_string()).unwrap();
    let expected = format!(
        "usr/local/blah/blah/genhello/hello.o:  \\\n  {t}/usr/local/blah/blah/genhello/hello.cc \
         \\\n  {t}/usr/local/blah/blah/genhello/hello.h \\\n  \
         {t}/usr/local/blah/blah/genhello/other.h\n"
    );
    assert_eq!(dotd_contents, expected);
    assert_eq!(dep_set1.get_output_file_name(), Some(filename.as_str()));
}

#[test]
fn write_read_set() {
    let s = Scratch::new();
    let t = s.tmp();
    let filename = format!("{t}/usr/local/blah/blah/genhello/hello.d");
    let file1 = s.path(&format!("{t}/usr/local/blah/blah/genhello/hello.cc"));
    let file2 = s.path(&format!("{t}/usr/local/blah/blah/genhello/hello.h"));
    let file3 = s.path(&format!("{t}/usr/local/blah/blah/genhello/other.h"));
    let mut dep_set1 = DependencySet::new(s.path(&t));
    dep_set1.add_dependencies(&[file1, file2, file3]);
    dep_set1.set_output_file_name(&filename);

    let dotd = s.path(&filename);
    dotd.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    dep_set1.write(&dotd, ".d").unwrap();

    let mut dep_set2 = DependencySet::new(s.path(&t));
    dep_set2.read(&dotd).unwrap();
    assert_eq!(dep_set2, dep_set1);
    // due to how pic.d files are written, absolute paths are changed into relatives
    assert_eq!(
        format!("{t}/{}", dep_set2.get_output_file_name().unwrap()),
        dep_set1.get_output_file_name().unwrap()
    );
}

// ---------------------------------------------------------------------------------------------
// Not in the Java test: paths that are not ASCII (review 2026-11-18-actions S2-1).
// Java: `new String(content, 0, w, ISO_8859_1)` keeps every byte of a path.

/// A header whose name is UTF-8 text: the path is the internal string of its bytes.
#[test]
fn dot_d_parser_utf8_filename() {
    use blaze_util::string_encoding::{latin1_encode, unicode_to_internal};
    let scratch = Scratch::new();
    let content = "hello.o: \\\n /usr/include/café.h \\\n /usr/include/🐷.h\n";
    let mut set = new_dependency_set(&scratch);
    set.process(content.as_bytes().to_vec()).unwrap();
    let got: Vec<&str> = set
        .get_dependencies()
        .iter()
        .map(|p| p.get_path_string())
        .collect();
    let want: Vec<String> = ["/usr/include/café.h", "/usr/include/🐷.h"]
        .iter()
        .map(|s| unicode_to_internal(s).into_owned())
        .collect();
    assert_eq!(got, want);
    // The path's bytes are the bytes of the .d file.
    assert_eq!(&*latin1_encode(got[0]), "/usr/include/café.h".as_bytes());
}

/// A header whose name is not UTF-8 (Latin-1 `caf\xe9.h`, and a byte ff): its bytes are kept.
#[test]
fn dot_d_parser_non_utf8_filename() {
    use blaze_util::string_encoding::latin1_encode;
    let scratch = Scratch::new();
    let mut set = new_dependency_set(&scratch);
    set.process(b"caf\xe9.o: \\\n /usr/include/caf\xe9.h \\\n /usr/include/\xff.h\n".to_vec())
        .unwrap();
    assert_eq!(set.get_output_file_name(), Some("caf\u{e9}.o"));
    let dep = set.get_dependencies()[0].get_path_string().to_string();
    assert_eq!(dep, "/usr/include/caf\u{e9}.h");
    assert_eq!(&*latin1_encode(&dep), b"/usr/include/caf\xe9.h");
    let dep = set.get_dependencies()[1].get_path_string().to_string();
    assert_eq!(&*latin1_encode(&dep), b"/usr/include/\xff.h");
}
