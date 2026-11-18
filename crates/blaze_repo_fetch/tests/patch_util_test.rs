//! Port of `PatchUtilTest` (src/test/java/com/google/devtools/build/lib/bazel/repository/decompressor).
//!
//! PORT: Java runs on an `InMemoryFileSystem` rooted at `/root`; Rust uses a temporary directory
//! on the real file system, so `/root` in expected messages is replaced by its path.

use blaze_repo_fetch::decompressor::patch_util::{self, PatchError};
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};

struct Scratch {
    _tmp: tempfile::TempDir,
    root: Path,
}

impl Scratch {
    fn new() -> Scratch {
        let tmp = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let base = std::fs::canonicalize(tmp.path()).unwrap();
        let root = Path::create_from_str(base.to_str().unwrap(), &fs).get_child("root");
        root.create_directory_and_parents().unwrap();
        Scratch { _tmp: tmp, root }
    }

    /// `scratch.file("/root/<name>", lines...)`: each line followed by "\n".
    fn file(&self, name: &str, lines: &[&str]) -> Path {
        let file = self.root.get_relative_str(name);
        assert!(
            !file.exists(),
            "Could not create scratch file (file exists) {}",
            file
        );
        let mut content = String::new();
        for l in lines {
            content.push_str(l);
            content.push('\n');
        }
        fs_utils::write_content(&file, content.as_bytes()).unwrap();
        file
    }

    /// Replaces the Java test's `/root` with the real root path.
    fn msg(&self, s: &str) -> String {
        s.replace("/root", self.root.get_path_string())
    }
}

fn read_lines(p: &Path) -> Vec<String> {
    fs_utils::read_lines(p).unwrap()
}

fn apply_err(patch_file: &Path, strip: i32, root: &Path) -> String {
    match patch_util::apply(patch_file, strip, root) {
        Err(PatchError::Patch(e)) => e.message,
        Err(PatchError::Io(e)) => panic!("expected PatchFailedException, got IOException: {}", e),
        Ok(()) => panic!("expected PatchFailedException"),
    }
}

fn assert_contains(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "expected message to contain:\n{}\nbut was:\n{}",
        needle,
        haystack
    );
}

#[test]
fn test_add_file() {
    let s = Scratch::new();
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/newfile b/newfile",
            "new file mode 100544",
            "index 0000000..f742c88",
            "--- /dev/null",
            "+++ b/newfile",
            "@@ -0,0 +1,2 @@",
            "+I'm a new file",
            "+hello, world",
            "-- ",
            "2.21.0.windows.1",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    let new_file = s.root.get_relative_str("newfile");
    assert_eq!(
        read_lines(&new_file),
        vec!["I'm a new file", "hello, world"]
    );
    // Make sure file permission is set as specified.
    assert!(new_file.is_readable().unwrap());
    assert!(!new_file.is_writable().unwrap());
    assert!(new_file.is_executable().unwrap());
}

#[test]
fn test_add_one_line_file() {
    let s = Scratch::new();
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/newfile b/newfile",
            "new file mode 100644",
            "index 0000000..f742c88",
            "--- /dev/null",
            "+++ b/newfile",
            "@@ -0,0 +1 @@", // diff will produce such chunk header for one line file.
            "+hello, world",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    let new_file = s.root.get_relative_str("newfile");
    assert_eq!(read_lines(&new_file), vec!["hello, world"]);
}

#[test]
fn test_delete_file() {
    let s = Scratch::new();
    let old_file = s.file("oldfile", &["I'm an old file", "bye, world"]);
    let patch_file = s.file(
        "patchfile",
        &[
            "--- a/oldfile",
            "+++ /dev/null",
            "@@ -1,2 +0,0 @@",
            "-I'm an old file",
            "-bye, world",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    assert!(!old_file.exists());
}

#[test]
fn test_delete_one_line_file() {
    let s = Scratch::new();
    let old_file = s.file("oldfile", &["bye, world"]);
    let patch_file = s.file(
        "patchfile",
        &[
            "--- a/oldfile",
            "+++ /dev/null",
            "@@ -1 +0,0 @@", // diff will produce such chunk header for one line file.
            "-bye, world",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    assert!(!old_file.exists());
}

#[test]
fn test_delete_all_content_but_not_file() {
    // If newfile is not /dev/null, we don't delete the file even it's empty after patching,
    // this is the behavior of patch command line tool.
    let s = Scratch::new();
    let old_file = s.file("oldfile", &["I'm an old file", "bye, world"]);
    let patch_file = s.file(
        "patchfile",
        &[
            "--- a/oldfile",
            "+++ b/oldfile",
            "@@ -1,2 +0,0 @@",
            "-I'm an old file",
            "-bye, world",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    assert!(old_file.exists());
    assert!(read_lines(&old_file).is_empty());
}

#[test]
fn test_apply_to_old_file() {
    // If both oldfile and newfile exist, we should patch the old file.
    let s = Scratch::new();
    let old_file = s.file("oldfile", &["line one"]);
    let new_file = s.file("newfile", &["line one"]);
    let patch_file = s.file(
        "patchfile",
        &[
            "--- oldfile",
            "+++ newfile",
            "@@ -1,1 +1,2 @@",
            " line one",
            "+line two",
        ],
    );
    patch_util::apply(&patch_file, 0, &s.root).unwrap();
    assert_eq!(read_lines(&old_file), vec!["line one", "line two"]);
    // new file should not change
    assert_eq!(read_lines(&new_file), vec!["line one"]);
}

#[test]
fn test_apply_to_new_file() {
    // If only newfile exists, we should patch the new file.
    let s = Scratch::new();
    let new_file = s.file("newfile", &["line one"]);
    new_file.set_readable(true).unwrap();
    new_file.set_writable(true).unwrap();
    new_file.set_executable(true).unwrap();
    let patch_file = s.file(
        "patchfile",
        &[
            "--- oldfile",
            "+++ newfile",
            "@@ -1,1 +1,2 @@",
            " line one",
            "+line two",
        ],
    );
    patch_util::apply(&patch_file, 0, &s.root).unwrap();
    assert_eq!(read_lines(&new_file), vec!["line one", "line two"]);
    // Make sure file permission is preserved.
    assert!(new_file.is_readable().unwrap());
    assert!(new_file.is_writable().unwrap());
    assert!(new_file.is_executable().unwrap());
}

#[test]
fn test_change_file_permission() {
    let s = Scratch::new();
    let my_file = s.file("test.sh", &["line one"]);
    my_file.set_readable(true).unwrap();
    my_file.set_writable(true).unwrap();
    my_file.set_executable(false).unwrap();
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/test.sh b/test.sh",
            "old mode 100644",
            "new mode 100755",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    assert_eq!(read_lines(&my_file), vec!["line one"]);
    assert!(my_file.is_readable().unwrap());
    assert!(my_file.is_writable().unwrap());
    assert!(my_file.is_executable().unwrap());
}

#[test]
fn test_git_format_patching() {
    let s = Scratch::new();
    let foo = s.file(
        "foo.cc",
        &[
            "#include <stdio.h>",
            "",
            "void main(){",
            "  printf(\"Hello foo\");",
            "}",
        ],
    );
    let bar = s.file("bar.cc", &["void lib(){", "  printf(\"Hello bar\");", "}"]);
    let patch_file = s.file(
        "patchfile",
        &[
            "From d205551eab3350afdb380f90ef83442ffcc0e22b Mon Sep 17 00:00:00 2001",
            "From: Yun Peng <pcloudy@google.com>",
            "Date: Thu, 6 Jun 2019 11:34:08 +0200",
            "Subject: [PATCH] 2",
            "",
            "---",
            " bar.cc | 2 +-",
            " foo.cc | 1 +",
            " 2 files changed, 2 insertions(+), 1 deletion(-)",
            "",
            "diff --git a/bar.cc b/bar.cc",
            "index e77137b..36dc9ab 100644",
            "--- a/bar.cc",
            "+++ b/bar.cc",
            "@@ -1,3 +1,3 @@",
            " void lib(){",
            "-  printf(\"Hello bar\");",
            "+  printf(\"Hello patch\");",
            " }",
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            "--- a/foo.cc",
            "+++ b/foo.cc",
            "@@ -2,4 +2,5 @@",
            " ",
            " void main(){",
            "   printf(\"Hello foo\");",
            "+  printf(\"Hello from patch\");",
            " }",
            "-- ",
            "2.21.0.windows.1",
            "",
            "",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    let new_foo = vec![
        "#include <stdio.h>",
        "",
        "void main(){",
        "  printf(\"Hello foo\");",
        "  printf(\"Hello from patch\");",
        "}",
    ];
    let new_bar = vec!["void lib(){", "  printf(\"Hello patch\");", "}"];
    assert_eq!(read_lines(&foo), new_foo);
    assert_eq!(read_lines(&bar), new_bar);
}

#[test]
fn test_git_format_renaming() {
    let s = Scratch::new();
    let foo = s.file(
        "foo.cc",
        &[
            "#include <stdio.h>",
            "",
            "void main(){",
            "  printf(\"Hello foo\");",
            "}",
        ],
    );
    let bar = s.file("bar.cc", &["void lib(){", "  printf(\"Hello bar\");", "}"]);
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/bar.cc b/bar.cpp",
            "similarity index 61%",
            "rename from bar.cc",
            "rename to bar.cpp",
            "index e77137b..9e35ee4 100644",
            "--- a/bar.cc",
            "+++ b/bar.cpp",
            "@@ -1,3 +1,4 @@",
            " void lib(){",
            "   printf(\"Hello bar\");",
            "+  printf(\"Hello cpp\");",
            " }",
            "diff --git a/foo.cc b/foo.cpp",
            "similarity index 100%",
            "rename from foo.cc",
            "rename to foo.cpp",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    let new_foo = vec![
        "#include <stdio.h>",
        "",
        "void main(){",
        "  printf(\"Hello foo\");",
        "}",
    ];
    let new_bar = vec![
        "void lib(){",
        "  printf(\"Hello bar\");",
        "  printf(\"Hello cpp\");",
        "}",
    ];
    let foo_cpp = s.root.get_relative_str("foo.cpp");
    let bar_cpp = s.root.get_relative_str("bar.cpp");
    assert!(!foo.exists());
    assert!(!bar.exists());
    assert_eq!(read_lines(&foo_cpp), new_foo);
    assert_eq!(read_lines(&bar_cpp), new_bar);
}

#[test]
fn test_match_with_offset() {
    let s = Scratch::new();
    let foo = s.file(
        "foo.cc",
        &[
            "#include <stdio.h>",
            "",
            "void main(){",
            "  printf(\"Hello foo\");",
            "}",
        ],
    );
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            "--- a/foo.cc",
            "+++ b/foo.cc",
            "@@ -6,4 +6,5 @@", // Should match with offset -4, original is "@@ -2,4 +2,5 @@"
            " ",
            " void main(){",
            "   printf(\"Hello foo\");",
            "+  printf(\"Hello from patch\");",
            " }",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    let new_foo = vec![
        "#include <stdio.h>",
        "",
        "void main(){",
        "  printf(\"Hello foo\");",
        "  printf(\"Hello from patch\");",
        "}",
    ];
    assert_eq!(read_lines(&foo), new_foo);
}

#[test]
// regression test for https://github.com/bazelbuild/bazel/issues/17897#issuecomment-1749389613
fn test_multiple_chunks() {
    let s = Scratch::new();
    let foo = s.file(
        "foo",
        &[
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
            "17", "18",
        ],
    );
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo b/foo",
            "index c20ab12..b83bdb1 100644",
            "--- a/foo",
            "+++ b/foo",
            "@@ -1,12 +1,5 @@",
            " 1",
            " 2",
            "-3",
            "-4",
            "-5",
            "-6",
            "-7",
            "-8",
            "-9",
            " 10",
            " 11",
            " 12",
            "@@ -15,4 +8,7 @@",
            " 15",
            " 16",
            " 17",
            "+a",
            "+b",
            "+c",
            " 18",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    let new_foo = vec![
        "1", "2", "10", "11", "12", "13", "14", "15", "16", "17", "a", "b", "c", "18",
    ];
    assert_eq!(read_lines(&foo), new_foo);
}

#[test]
fn test_multiple_chunks_with_different_offset() {
    let s = Scratch::new();
    let foo = s.file(
        "foo",
        &[
            "1", "3", "4", "5", "6", "7", "8", "9", "10", "11", "13", "14",
        ],
    );
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo b/foo",
            "index c20ab12..b83bdb1 100644",
            "--- a/foo",
            "+++ b/foo",
            "@@ -3,4 +3,5 @@", // Should match with offset -2, original is "@@ -1,4 +1,5 @@"
            " 1",
            "+2",
            " 3",
            " 4",
            " 5",
            "@@ -4,5 +5,6 @@", // Should match with offset 4, original is "@@ -8,4 +9,5 @@"
            " 9",
            " 10",
            " 11",
            "+12",
            " 13",
            " 14",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    let new_foo = vec![
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14",
    ];
    assert_eq!(read_lines(&foo), new_foo);
}

#[test]
fn test_failed_to_get_file_name() {
    let s = Scratch::new();
    s.file(
        "foo.cc",
        &[
            "#include <stdio.h>",
            "",
            "void main(){",
            "  printf(\"Hello foo\");",
            "}",
        ],
    );
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            "--- a/foo.cc",
            "+++ b/foo.cc",
            "@@ -2,4 +2,5 @@",
            " ",
            " void main(){",
            "   printf(\"Hello foo\");",
            "+  printf(\"Hello from patch\");",
            " }",
        ],
    );
    let expected = apply_err(&patch_file, 2, &s.root); // strip=2 is wrong
    assert_contains(
        &expected,
        "Cannot determine file name with strip = 2 at line 3:\n--- a/foo.cc",
    );
}

#[test]
fn test_patch_file_not_found() {
    let s = Scratch::new();
    let expected = apply_err(&s.root.get_relative_str("patchfile"), 1, &s.root);
    assert_contains(&expected, &s.msg("Cannot find patch file: /root/patchfile"));
}

#[test]
fn test_cannot_find_file_to_patch() {
    let s = Scratch::new();
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            "--- a/foo.cc",
            "+++ /dev/null",
            "@@ -2,4 +2,5 @@",
            " ",
            " void main(){",
            "   printf(\"Hello foo\");",
            "+  printf(\"Hello from patch\");",
            " }",
        ],
    );
    let expected = apply_err(&patch_file, 1, &s.root);
    assert_contains(
        &expected,
        "Cannot find file to patch (near line 3), old file name (foo.cc) doesn't exist, \
         new file name is not specified.",
    );
}

#[test]
fn test_cannot_rename_file() {
    let s = Scratch::new();
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/bar.cc b/bar.cpp",
            "similarity index 61%",
            "rename from bar.cc",
            "rename to bar.cpp",
            "index e77137b..9e35ee4 100644",
            "--- a/bar.cc",
            "+++ b/bar.cpp",
            "@@ -1,3 +1,4 @@",
            " void lib(){",
            "   printf(\"Hello bar\");",
            "+  printf(\"Hello cpp\");",
            " }",
            "diff --git a/foo.cc b/foo.cpp",
            "similarity index 100%",
            "rename from foo.cc",
            "rename to foo.cpp",
        ],
    );

    let expected = apply_err(&patch_file, 1, &s.root);
    assert_contains(
        &expected,
        "Cannot rename file (near line 6), old file name (bar.cc) doesn't exist.",
    );

    s.file("bar.cc", &["void lib(){", "  printf(\"Hello bar\");", "}"]);
    s.file("foo.cc", &[]);
    s.file("foo.cpp", &[]);

    let expected = apply_err(&patch_file, 1, &s.root);
    assert_contains(
        &expected,
        "Cannot rename file (near line 17), new file name (foo.cpp) already exists.",
    );
}

#[test]
fn test_patch_outside_of_repository() {
    let s = Scratch::new();
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            "--- a/../other_root/foo.cc",
            "+++ b/../other_root/foo.cc",
            "@@ -2,4 +2,5 @@",
            " ",
            " void main(){",
            "   printf(\"Hello foo\");",
            "+  printf(\"Hello from patch\");",
            " }",
        ],
    );
    let expected = apply_err(&patch_file, 1, &s.root);
    assert_contains(
        &expected,
        &s.msg(
            "Cannot patch file outside of external repository (/root), \
             file path = \"../other_root/foo.cc\" at line 3",
        ),
    );
}

#[test]
fn test_chunk_does_not_match() {
    let s = Scratch::new();
    s.file(
        "foo.cc",
        &[
            "line1", "line2", "line3", "line4", "line5", "line6", "line7", "line8",
        ],
    );
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            "--- a/foo.cc",
            "+++ b/foo.cc",
            "@@ -1,8 +1,9 @@",
            " line1",
            " line2",
            " line3",
            " WRONG",      // Should be "line4", in the middle so fuzz can't help
            " ALSO WRONG", // Should be "line5"
            " line6",
            "+inserted",
            " line7",
            " line8",
        ],
    );
    let expected = apply_err(&patch_file, 1, &s.root);
    assert_contains(
        &expected,
        &s.msg(
            "in patch applied to /root/foo.cc: could not apply patch due to \
             CONTENT_DOES_NOT_MATCH_TARGET",
        ),
    );
}

#[test]
fn test_unexpected_context_line() {
    let s = Scratch::new();
    s.file(
        "foo.cc",
        &[
            "#include <stdio.h>",
            "",
            "void main(){",
            "  printf(\"Hello foo\");",
            "}",
        ],
    );
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            "--- a/foo.cc",
            "+++ b/foo.cc",
            "@@ -2,4 +2,5 @@",
            " ",
            " void main(){",
            "   printf(\"Hello foo\");",
            "+  printf(\"Hello from patch\");",
            // Adding this line will cause the chunk body not matching the header
            // "@@ -2,4 +2,5 @@"
            "+",
            " }",
        ],
    );
    let expected = apply_err(&patch_file, 1, &s.root);
    assert_contains(
        &expected,
        "Wrong chunk detected near line 11:  }, does not expect a context line here.",
    );
}

#[test]
fn test_match_with_fuzz() {
    let s = Scratch::new();
    let foo = s.file(
        "foo.cc",
        &[
            "#include <stdio.h>",
            "",
            "void main(){",
            "  printf(\"Hello foo\");",
            "}",
        ],
    );
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            "--- a/foo.cc",
            "+++ b/foo.cc",
            "@@ -2,4 +2,5 @@",
            " ",
            " void main(){",
            "   printf(\"Hello foo\");",
            "+  printf(\"Hello from patch\");",
            " WRONG CONTEXT LINE", // Last context line doesn't match, but fuzz can drop it
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    let new_foo = vec![
        "#include <stdio.h>",
        "",
        "void main(){",
        "  printf(\"Hello foo\");",
        "  printf(\"Hello from patch\");",
        "}",
    ];
    assert_eq!(read_lines(&foo), new_foo);
}

#[test]
fn test_missing_context_line() {
    let s = Scratch::new();
    s.file(
        "foo.cc",
        &[
            "#include <stdio.h>",
            "",
            "void main(){",
            "  printf(\"Hello foo\");",
            "}",
        ],
    );
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            "--- a/foo.cc",
            "+++ b/foo.cc",
            "@@ -2,4 +2,5 @@",
            " ",
            " void main(){",
            "   printf(\"Hello foo\");",
            "+  printf(\"Hello from patch\");",
        ],
    );
    let expected = apply_err(&patch_file, 1, &s.root);
    assert_contains(&expected, "Expecting more chunk line at line 10");
}

#[test]
fn test_missing_chunk_header() {
    let s = Scratch::new();
    s.file(
        "foo.cc",
        &[
            "#include <stdio.h>",
            "",
            "void main(){",
            "  printf(\"Hello foo\");",
            "}",
        ],
    );
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            "--- a/foo.cc",
            "+++ b/foo.cc",
            // Missing @@ -l,s +l,s @@ line
            " ",
            " void main(){",
            "   printf(\"Hello foo\");",
            "+  printf(\"Hello from patch\");",
            " }",
        ],
    );
    let expected = apply_err(&patch_file, 1, &s.root);
    assert_contains(
        &expected,
        "Looks like a unified diff at line 3, but no patch chunk was found.",
    );
}

#[test]
fn test_missing_prelude_lines() {
    let s = Scratch::new();
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.cc b/foo.cc",
            "index f3008f9..ec4aaa0 100644",
            // Missing "--- a/foo.cc",
            // Missing "+++ b/foo.cc",
            "@@ -2,4 +2,5 @@",
            " ",
            " void main(){",
            "   printf(\"Hello foo\");",
            "+  printf(\"Hello from patch\");",
            " }",
        ],
    );
    let expected = apply_err(&patch_file, 1, &s.root);
    assert_contains(
        &expected,
        "The patch content must start with ---/+++ prelude lines at line 3",
    );
}

#[test]
fn test_apply_patch_with_no_newline_at_end_of_file_add_newline() {
    // Regression test for https://github.com/bazelbuild/bazel/issues/17376
    let s = Scratch::new();
    let foo = s.file(
        "FormatTokenSource.h",
        &[
            "class IndexedTokenSource : public FormatTokenSource {",
            "",
            "#undef DEBUG_TYPE",
            "",
            "#endif",
        ],
    );
    let patch_file = s.file(
        "patchfile",
        &[
            "--- FormatTokenSource.h",
            "+++ FormatTokenSource.h",
            "@@ -1,5 +1,5 @@",
            "-class IndexedTokenSource : public FormatTokenSource {",
            "+class LLVM_GSL_POINTER IndexedTokenSource : public FormatTokenSource {",
            " ",
            " #undef DEBUG_TYPE",
            " ",
            "-#endif",
            "\\ No newline at end of file",
            "+#endif",
        ],
    );
    patch_util::apply(&patch_file, 0, &s.root).unwrap();
    let expected = vec![
        "class LLVM_GSL_POINTER IndexedTokenSource : public FormatTokenSource {",
        "",
        "#undef DEBUG_TYPE",
        "",
        "#endif",
    ];
    assert_eq!(read_lines(&foo), expected);
}

#[test]
fn test_apply_patch_with_no_newline_at_end_of_file_still_no_newline() {
    let s = Scratch::new();
    let foo = s.file("foo.txt", &["line1", "line2", "#endif"]);
    let patch_file = s.file(
        "patchfile",
        &[
            "--- a/foo.txt",
            "+++ b/foo.txt",
            "@@ -1,3 +1,3 @@",
            " line1",
            " line2",
            "-#endif",
            "\\ No newline at end of file",
            "+#endif_modified",
            "\\ No newline at end of file",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    assert_eq!(read_lines(&foo), vec!["line1", "line2", "#endif_modified"]);
}

#[test]
fn test_apply_patch_with_no_newline_at_end_of_file_context_line() {
    let s = Scratch::new();
    let foo = s.file("foo.txt", &["old", "tail"]);
    let patch_file = s.file(
        "patchfile",
        &[
            "--- a/foo.txt",
            "+++ b/foo.txt",
            "@@ -1,2 +1,2 @@",
            "-old",
            "+new",
            " tail",
            "\\ No newline at end of file",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    assert_eq!(read_lines(&foo), vec!["new", "tail"]);
}

#[test]
fn test_apply_patch_with_no_newline_at_end_of_file_add_file() {
    let s = Scratch::new();
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/newfile b/newfile",
            "new file mode 100644",
            "--- /dev/null",
            "+++ b/newfile",
            "@@ -0,0 +1 @@",
            "+hello, world",
            "\\ No newline at end of file",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    let new_file = s.root.get_relative_str("newfile");
    assert_eq!(read_lines(&new_file), vec!["hello, world"]);
}

#[test]
fn test_apply_patch_with_no_newline_at_end_of_file_delete_file() {
    let s = Scratch::new();
    let old_file = s.file("oldfile", &["bye, world"]);
    let patch_file = s.file(
        "patchfile",
        &[
            "--- a/oldfile",
            "+++ /dev/null",
            "@@ -1 +0,0 @@",
            "-bye, world",
            "\\ No newline at end of file",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    assert!(!old_file.exists());
}

#[test]
fn test_apply_patch_with_no_newline_at_end_of_file_multiple_files() {
    let s = Scratch::new();
    let foo = s.file("foo.txt", &["foo_old"]);
    let bar = s.file("bar.txt", &["bar_old"]);
    let patch_file = s.file(
        "patchfile",
        &[
            "diff --git a/foo.txt b/foo.txt",
            "--- a/foo.txt",
            "+++ b/foo.txt",
            "@@ -1 +1 @@",
            "-foo_old",
            "\\ No newline at end of file",
            "+foo_new",
            "diff --git a/bar.txt b/bar.txt",
            "--- a/bar.txt",
            "+++ b/bar.txt",
            "@@ -1 +1 @@",
            "-bar_old",
            "+bar_new",
            "\\ No newline at end of file",
        ],
    );
    patch_util::apply(&patch_file, 1, &s.root).unwrap();
    assert_eq!(read_lines(&foo), vec!["foo_new"]);
    assert_eq!(read_lines(&bar), vec!["bar_new"]);
}
