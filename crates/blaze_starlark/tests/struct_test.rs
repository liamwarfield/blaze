//! `struct` and the StarlarkInfo base. Ports of
//! src/test/java/com/google/devtools/build/lib/packages/StarlarkInfoTest.java (equivalence,
//! concat) adapted to `struct`, plus repr/json behaviour from StructImpl.

mod common;

use common::eval_expr;
use common::exec_err;

#[test]
fn repr_sorts_fields() {
    assert_eq!(
        eval_expr("str(struct(b = 2, a = 'x'))"),
        Ok(r#""struct(a = \"x\", b = 2)""#.into())
    );
    assert_eq!(eval_expr("type(struct())"), Ok(r#""struct""#.into()));
    assert_eq!(eval_expr("type(struct)"), Ok(r#""Provider""#.into()));
    assert_eq!(
        eval_expr("dir(struct(b = 1, a = 2))"),
        Ok(r#"["a", "b"]"#.into())
    );
}

#[test]
fn equivalence() {
    assert_eq!(
        eval_expr("struct(f1 = 4, f2 = 5) == struct(f2 = 5, f1 = 4)"),
        Ok("True".into())
    );
    assert_eq!(
        eval_expr("struct(f1 = 4, f2 = 5) == struct(f1 = 4, f2 = 6)"),
        Ok("False".into())
    );
    assert_eq!(
        eval_expr("struct(f1 = 4, f2 = 5) == struct(f1 = 4)"),
        Ok("False".into())
    );
    assert_eq!(
        eval_expr("len(dict([(struct(a = 1), 1), (struct(a = 1), 2)]))"),
        Ok("1".into())
    );
}

#[test]
fn concat_with_overlapping_fields_fails() {
    assert_eq!(
        exec_err("x = struct(f1 = 4, f2 = 5) + struct(f1 = 4)"),
        Err("cannot add struct instances with common field 'f1'".into())
    );
}

#[test]
fn concat_with_different_fields() {
    assert_eq!(
        eval_expr("struct(f1 = 4) + struct(f2 = 5)"),
        Ok("struct(f1 = 4, f2 = 5)".into())
    );
}

#[test]
fn fields_and_getattr() {
    assert_eq!(eval_expr("struct(a = 1).a"), Ok("1".into()));
    assert_eq!(eval_expr("hasattr(struct(a = 1), 'b')"), Ok("False".into()));
    assert_eq!(eval_expr("getattr(struct(a = 1), 'b', 3)"), Ok("3".into()));
    assert!(exec_err("x = struct(a = 1).b").is_err());
    assert!(exec_err("x = struct(1)").is_err());
}
