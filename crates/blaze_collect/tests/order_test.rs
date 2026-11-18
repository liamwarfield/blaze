//! Port of `src/test/java/com/google/devtools/build/lib/collect/nestedset/OrderTest.java`.

use blaze_collect::Order;

#[test]
fn test_parsing() {
    for current in Order::VALUES {
        assert_eq!(Order::parse(current.get_starlark_name()).unwrap(), current);
    }
}

#[test]
fn test_for_errors() {
    // PORT: Java also passes `null` ("Invalid order: null"); Rust's `&str` cannot be null.
    cause_error("");
    cause_error("lol");
    cause_error("naive");
    cause_error("naivelink");
}

fn cause_error(invalid_name: &str) {
    let ex = Order::parse(invalid_name).expect_err("expected IllegalArgumentException");
    assert!(ex.message.starts_with("Invalid order"));
    assert_eq!(ex.message, format!("Invalid order: {invalid_name}"));
}
