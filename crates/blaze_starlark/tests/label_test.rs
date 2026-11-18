//! The Label value and `Label()` (cmdline/Label.java Starlark methods,
//! StarlarkRuleClassFunctions.label).

mod common;

use common::eval_expr;
use common::exec_err;

#[test]
fn constructor_and_attributes() {
    assert_eq!(eval_expr("Label('//a/b:c').name"), Ok(r#""c""#.into()));
    assert_eq!(eval_expr("Label('//a/b:c').package"), Ok(r#""a/b""#.into()));
    assert_eq!(eval_expr("Label(':c').package"), Ok(r#""pkg""#.into()));
    assert_eq!(
        eval_expr("Label('@foo//x').repo_name"),
        Ok(r#""foo+""#.into())
    );
    assert_eq!(
        eval_expr("Label('@foo//x').workspace_root"),
        Ok(r#""external/foo+""#.into())
    );
    assert_eq!(
        eval_expr("Label('//a:b').same_package_label('d')"),
        Ok(r#"Label("//a:d")"#.into())
    );
    assert_eq!(
        eval_expr("Label('//a:b').relative(':e')"),
        Ok(r#"Label("//a:e")"#.into())
    );
    assert_eq!(
        eval_expr("Label(Label('//a:b'))"),
        Ok(r#"Label("//a:b")"#.into())
    );
}

#[test]
fn repr_str_equality() {
    assert_eq!(
        eval_expr("repr(Label('//a:b'))"),
        Ok(r#""Label(\"//a:b\")""#.into())
    );
    assert_eq!(eval_expr("str(Label('//a:b'))"), Ok(r#""@@//a:b""#.into()));
    assert_eq!(
        eval_expr("Label('//a:b') == Label('//a:b')"),
        Ok("True".into())
    );
    assert_eq!(eval_expr("type(Label('//a:b'))"), Ok(r#""Label""#.into()));
    assert_eq!(
        eval_expr("sorted([Label('//b:a'), Label('//a:b')])"),
        Ok(r#"[Label("//a:b"), Label("//b:a")]"#.into())
    );
    assert_eq!(
        eval_expr("len(dict([(Label('//a:b'), 1), (Label('//a:b'), 2)]))"),
        Ok("1".into())
    );
}

#[test]
fn invalid_label() {
    let e = exec_err("x = Label('//a:b:c')").unwrap_err();
    assert!(e.starts_with("invalid label in Label(): "), "{e}");
}

/// starlark-rust patch 0011: `%s` and `"{}".format` use `str`, as Bazel does.
#[test]
fn patch_0011_percent_s_uses_str() {
    assert_eq!(eval_expr("'%s' % Label('//a:b')"), Ok(r#""@@//a:b""#.into()));
    assert_eq!(eval_expr("'x=%s' % Label('//a:b')"), Ok(r#""x=@@//a:b""#.into()));
    assert_eq!(eval_expr("'{}'.format(Label('//a:b'))"), Ok(r#""@@//a:b""#.into()));
    assert_eq!(eval_expr("'%r' % Label('//a:b')"), Ok(r#""Label(\"//a:b\")""#.into()));
}
