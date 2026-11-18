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

/// Java: StarlarkRuleClassFunctionsTest.testStructAccessingUnknownField (starlark-rust patch
/// 0013 routes the message through StructImpl.getErrorMessageForUnknownField).
#[test]
fn struct_accessing_unknown_field() {
    assert_eq!(
        exec_err("x = struct(a = 1, b = 2)\ny = x.c"),
        Err("'struct' value has no field or method 'c'\nAvailable attributes: a, b".into())
    );
}

/// Java: StarlarkRuleClassFunctionsTest.testGetattrNoAttr
#[test]
fn getattr_no_attr() {
    assert_eq!(
        exec_err("s = struct(a='val')\ngetattr(s, 'b')"),
        Err("'struct' value has no field or method 'b'\nAvailable attributes: a".into())
    );
}

/// Java: StarlarkRuleClassFunctionsTest.testStructsInDicts
#[test]
fn structs_in_dicts() {
    let m = common::exec(
        "d = {struct(a = 1): 'aa', struct(b = 2): 'bb'}\n\
         r1 = d[struct(a = 1)]\n\
         r2 = d[struct(b = 2)]\n\
         r3 = str([d[k] for k in d])\n",
    )
    .unwrap();
    assert_eq!(common::global_repr(&m, "r1"), r#""aa""#);
    assert_eq!(common::global_repr(&m, "r2"), r#""bb""#);
    assert_eq!(common::global_repr(&m, "r3"), r#""[\"aa\", \"bb\"]""#);

    let err = exec_err("{struct(a = []): 'foo'}").unwrap_err();
    assert!(err.contains("unhashable type: 'struct'"), "{err}");
}

// Java: net/starlark/java/eval/StarlarkValue.java:104 StarlarkValue.checkHashable ("even though a
// list may not be used as a dict key (even if frozen), a struct containing a list is hashable"):
// StarlarkInfo is hashable iff it is immutable, and a frozen list or dict is immutable. Java then
// hashes the fields with `Object.hashCode` (lists by elements in order, dicts order-independent).
// This is the case of rules_cc's `depset([linker_input])`, whose list fields are frozen.
#[test]
fn struct_with_frozen_list_and_dict_fields_is_hashable() {
    let a = common::exec(
        "L12 = struct(a = [1, 2])\n\
         L12B = struct(a = [1, 2])\n\
         L21 = struct(a = [2, 1])\n\
         DXY = struct(d = {'x': 1, 'y': 2})\n\
         DYX = struct(d = {'y': 2, 'x': 1})\n",
    )
    .unwrap();
    let mut loads = blaze_starlark::loader::LoadMap::new();
    loads.insert("//pkg:a.bzl", a);
    let (m, _) = common::exec_with(
        "load('//pkg:a.bzl', 'L12', 'L12B', 'L21', 'DXY', 'DYX')\n\
         r1 = len(dict([(L12, 1), (L12B, 2)]))\n\
         r2 = len(dict([(L12, 1), (L21, 2)]))\n\
         r3 = len(dict([(DXY, 1), (DYX, 2)]))\n",
        blaze_starlark::StarlarkSemantics::default(),
        loads,
    );
    let m = m.unwrap();
    assert_eq!(common::global_repr(&m, "r1"), "1");
    assert_eq!(common::global_repr(&m, "r2"), "2");
    assert_eq!(common::global_repr(&m, "r3"), "1");
}
