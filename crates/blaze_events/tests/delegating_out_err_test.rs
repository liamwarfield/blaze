//! Port of `src/test/java/com/google/devtools/build/lib/util/io/DelegatingOutErrTest.java`.

use blaze_events::RecordingOutErr;
use blaze_events::io::DelegatingOutErr;

fn join_lines(lines: &[&str]) -> String {
    lines.join("\n")
}

#[test]
fn test_new_delegate_is_like_dev_null() {
    let delegate = DelegatingOutErr::new();
    delegate.as_out_err().print_out("Hello, world.\n");
    delegate.as_out_err().print_err("Feel free to ignore me.\n");
    delegate.as_out_err().close().unwrap();
}

#[test]
fn test_subscribe_and_unsubscribe_sink() {
    let delegate = DelegatingOutErr::new();
    let out = delegate.as_out_err();
    out.print_out("Nobody will listen to this.\n");
    let sink = RecordingOutErr::new();
    delegate.add_sink(sink.out_err());
    out.print_out_ln("Hello, sink.");
    delegate.remove_sink(sink.out_err());
    out.print_out_ln("... and alone again ...");
    delegate.add_sink(sink.out_err());
    out.print_out_ln("How are things?");
    assert_eq!(sink.out_as_latin1(), "Hello, sink.\nHow are things?\n");
    out.close().unwrap();
}

#[test]
fn test_subscribe_multiple_sinks() {
    let delegate = DelegatingOutErr::new();
    let out = delegate.as_out_err();
    let left = RecordingOutErr::new();
    let right = RecordingOutErr::new();
    delegate.add_sink(left.out_err());
    out.print_out_ln("left only");
    delegate.add_sink(right.out_err());
    out.print_out_ln("both");
    delegate.remove_sink(left.out_err());
    out.print_out_ln("right only");
    delegate.remove_sink(right.out_err());
    out.print_out_ln("silence");
    delegate.add_sink(left.out_err());
    delegate.add_sink(right.out_err());
    out.print_out_ln("left and right");
    assert_eq!(
        left.out_as_latin1(),
        join_lines(&["left only", "both", "left and right", ""])
    );
    assert_eq!(
        right.out_as_latin1(),
        join_lines(&["both", "right only", "left and right", ""])
    );
    out.close().unwrap();
}
