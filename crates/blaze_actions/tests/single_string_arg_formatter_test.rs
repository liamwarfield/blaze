//! Port of `src/test/java/com/google/devtools/build/lib/actions/SingleStringArgFormatterTest.java`.

use std::panic::{AssertUnwindSafe, catch_unwind};

use blaze_actions::actions::single_string_arg_formatter::{format, formatted_length, is_valid};

fn panics_with_illegal_argument(f: impl FnOnce()) -> bool {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(()) => false,
        Err(e) => {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            msg.starts_with("IllegalArgumentException")
        }
    }
}

#[test]
fn invalid() {
    for format_str in [
        "hello",
        "hello %%s",
        "hello %s %s",
        "%s hello %s",
        "hello %",
        "hello %f",
        "hello %s %f",
        "hello %s %",
    ] {
        assert!(!is_valid(format_str), "{format_str}");
        assert!(
            panics_with_illegal_argument(|| {
                format(format_str, "world");
            }),
            "{format_str}"
        );
        assert!(
            panics_with_illegal_argument(|| {
                formatted_length(format_str);
            }),
            "{format_str}"
        );
    }
}

#[test]
fn valid() {
    for (format_str, expected) in [
        ("hello %s", "hello world"),
        ("%s hello", "world hello"),
        ("hello %s, hello", "hello world, hello"),
        ("hello %%s %s", "hello %s world"),
    ] {
        assert!(is_valid(format_str), "{format_str}");
        assert_eq!(format(format_str, "world"), expected);
        assert_eq!(
            formatted_length(format_str),
            (expected.len() - "world".len()) as i32
        );
    }
}
