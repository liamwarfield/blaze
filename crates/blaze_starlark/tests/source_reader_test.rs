//! `EvalException.setSourceReaderSupplier`: the source line of each traceback frame comes from
//! the installed reader (Bazel's reads the file at the frame's path from the file system, so a
//! frame whose file is not on disk, such as `/virtual_builtins_bzl/...` of `@_builtins`, prints
//! no line). The reader is process-wide, so this binary holds a single test.

mod common;

use std::sync::Arc;

use blaze_starlark::error::{Location, set_source_reader_supplier};
use common::exec;

const SRC: &str = "def foo():\n  bar(1)\ndef bar(x):\n  fail('boom')\nfoo()";

#[test]
fn source_line_comes_from_the_installed_reader() {
    // Without a reader, the lines come from the evaluated source.
    let e = exec(SRC).unwrap_err();
    assert_eq!(
        e.message_with_stack(),
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 5, column 4, in <toplevel>\n\
\t\tfoo()\n\
\tFile \"test.bzl\", line 2, column 6, in foo\n\
\t\tbar(1)\n\
\tFile \"test.bzl\", line 4, column 7, in bar\n\
\t\tfail('boom')\n\
Error in fail: boom"
    );

    // A reader that cannot read the file (Bazel's reader for a path that is not on disk):
    // no source lines (EvalException.formatCallStack: `if (line != null)`).
    set_source_reader_supplier(Arc::new(|_loc: &Location| None));
    let e = exec(SRC).unwrap_err();
    assert_eq!(
        e.message_with_stack(),
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 5, column 4, in <toplevel>\n\
\tFile \"test.bzl\", line 2, column 6, in foo\n\
\tFile \"test.bzl\", line 4, column 7, in bar\n\
Error in fail: boom"
    );

    // The reader's line is printed trimmed (`line.trim()`), whatever the evaluated source says.
    set_source_reader_supplier(Arc::new(|loc: &Location| {
        Some(format!("  \tline {} of {}  ", loc.line, loc.file))
    }));
    let e = exec(SRC).unwrap_err();
    assert_eq!(
        e.message_with_stack(),
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 5, column 4, in <toplevel>\n\
\t\tline 5 of test.bzl\n\
\tFile \"test.bzl\", line 2, column 6, in foo\n\
\t\tline 2 of test.bzl\n\
\tFile \"test.bzl\", line 4, column 7, in bar\n\
\t\tline 4 of test.bzl\n\
Error in fail: boom"
    );
}
