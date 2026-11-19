//! `FileOutErr.childOutErr()` and `FileOutErr.dump(from, to)` (not covered by Java's
//! `FileOutErrTest`; expected values follow the Java code at
//! `com/google/devtools/build/lib/util/io/FileOutErr.java:218` and `:232`).

use blaze_exec::util::FileOutErr;
use blaze_vfs::{DigestHashFunction, FileSystemExt, UnixFileSystem};

fn out_err(dir: &tempfile::TempDir, name: &str) -> FileOutErr {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = fs.get_path(dir.path().to_str().unwrap());
    FileOutErr::new(
        root.get_child(&format!("{name}.out")),
        root.get_child(&format!("{name}.err")),
    )
}

#[test]
fn child_out_err_numbers_the_children_of_each_parent_from_zero() {
    let dir = tempfile::tempdir().unwrap();
    let a = out_err(&dir, "a");
    let b = out_err(&dir, "b");

    let a0 = a.child_out_err();
    let a1 = a.clone().child_out_err();
    let b0 = b.child_out_err();

    assert_eq!(a0.get_output_path().get_base_name(), "a.out-0");
    assert_eq!(a0.get_error_path().get_base_name(), "a.err-0");
    // A clone is the same parent: its children continue the numbering.
    assert_eq!(a1.get_output_path().get_base_name(), "a.out-1");
    assert_eq!(b0.get_output_path().get_base_name(), "b.out-0");
    assert_eq!(
        a0.get_output_path().get_parent_directory(),
        a.get_output_path().get_parent_directory()
    );
    // A child counts its own children.
    assert_eq!(
        a0.child_out_err().get_output_path().get_base_name(),
        "a.out-0-0"
    );
}

#[test]
fn dump_copies_the_recorded_output_and_creates_nothing_when_empty() {
    let dir = tempfile::tempdir().unwrap();
    let from = out_err(&dir, "from");
    let to = out_err(&dir, "to");

    FileOutErr::dump(&from, &to);
    assert!(!to.get_output_path().exists());
    assert!(!to.get_error_path().exists());

    from.write_out(b"out\n").unwrap();
    from.write_err(b"err\n").unwrap();
    to.write_out(b"before\n").unwrap();
    FileOutErr::dump(&from, &to);
    assert_eq!(to.out_as_bytes(), b"before\nout\n");
    assert_eq!(to.err_as_bytes(), b"err\n");
}
