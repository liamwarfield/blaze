//! Port of src/test/java/com/google/devtools/build/lib/collect/nestedset/DepsetTest.java
//! (Starlark-level tests; Java-API-only tests are adapted to the Rust API or ignored).

mod common;

use blaze_collect::Order;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::depset::Depset;
use blaze_starlark::loader::LoadMap;
use common::eval_expr;
use common::exec;
use common::exec_err;
use common::global_repr;

fn elem_type(src: &str, name: &str) -> String {
    let m = exec(src).unwrap();
    let v = m.get(name).unwrap();
    Depset::from_value(v.value())
        .unwrap()
        .element_type()
        .to_owned()
}

fn to_list(expr: &str) -> String {
    eval_expr(&format!("({expr}).to_list()")).unwrap()
}

#[test]
fn test_constructor() {
    let m = exec("s = depset(order='default')").unwrap();
    assert_eq!(m.get("s").unwrap().value().get_type(), "depset");
}

#[test]
fn test_tuples() {
    let src = "\
s_one = depset([('1', '2'), ('3', '4')])
s_two = depset(direct = [('1', '2'), ('3', '4'), ('5', '6')])
s_three = depset(transitive = [s_one, s_two])
s_four = depset(direct = [('1', '3')], transitive = [s_one, s_two])
s_five = depset(direct = [('1', '3', '5')], transitive = [s_one, s_two])
s_six = depset(transitive = [s_one, s_five])
s_seven = depset(direct = [('1', '3')], transitive = [s_one, s_five])
s_eight = depset(direct = [(1, 3)], transitive = [s_one, s_two])
four = s_four.to_list()
five = s_five.to_list()
eight = s_eight.to_list()
";
    for n in ["s_one", "s_two", "s_three", "s_eight"] {
        assert_eq!(elem_type(src, n), "tuple");
    }
    let m = exec(src).unwrap();
    // containsExactly (any order) in Java; the order below is Java's default order.
    assert_eq!(
        global_repr(&m, "four"),
        r#"[("1", "2"), ("3", "4"), ("5", "6"), ("1", "3")]"#
    );
    assert_eq!(
        global_repr(&m, "five"),
        r#"[("1", "2"), ("3", "4"), ("5", "6"), ("1", "3", "5")]"#
    );
    assert_eq!(
        global_repr(&m, "eight"),
        r#"[("1", "2"), ("3", "4"), ("5", "6"), (1, 3)]"#
    );
}

#[test]
fn test_get_set() {
    assert_eq!(to_list("depset(['a', 'b'])"), r#"["a", "b"]"#);
}

#[test]
fn test_get_set_direct() {
    assert_eq!(to_list("depset(direct = ['a', 'b'])"), r#"["a", "b"]"#);
}

#[test]
fn test_to_list() {
    assert_eq!(to_list("depset(['a', 'b'])"), r#"["a", "b"]"#);
}

#[test]
fn test_to_list_direct() {
    assert_eq!(to_list("depset(direct = ['a', 'b'])"), r#"["a", "b"]"#);
}

#[test]
fn test_order() {
    let m = exec("s = depset(['a', 'b'], order='postorder')").unwrap();
    let s = m.get("s").unwrap();
    assert_eq!(
        Depset::from_value(s.value()).unwrap().order(),
        Order::COMPILE_ORDER
    );
}

#[test]
fn test_order_direct() {
    let m = exec("s = depset(direct = ['a', 'b'], order='postorder')").unwrap();
    let s = m.get("s").unwrap();
    assert_eq!(
        Depset::from_value(s.value()).unwrap().order(),
        Order::COMPILE_ORDER
    );
}

#[test]
fn test_bad_order() {
    assert_eq!(
        exec_err("depset(['a'], order='non_existing')"),
        Err("Invalid order: non_existing".into())
    );
}

#[test]
fn test_bad_order_direct() {
    assert_eq!(
        exec_err("depset(direct = ['a'], order='non_existing')"),
        Err("Invalid order: non_existing".into())
    );
}

#[test]
fn test_empty_generic_type() {
    assert_eq!(elem_type("s = depset()", "s"), "empty");
}

#[test]
fn test_homogeneous_generic_type() {
    assert_eq!(
        elem_type("s = depset(direct = ['a', 'b', 'c'])", "s"),
        "string"
    );
}

#[test]
fn test_homogeneous_generic_type_direct() {
    assert_eq!(
        elem_type("s = depset(direct = ['a', 'b', 'c'], transitive = [])", "s"),
        "string"
    );
}

#[test]
fn test_homogeneous_generic_type_transitive() {
    assert_eq!(
        elem_type(
            "s = depset(direct = ['a', 'b', 'c'], transitive = [depset(['x'])])",
            "s"
        ),
        "string"
    );
}

#[test]
fn test_transitive_incompatible_order() {
    assert_eq!(
        exec_err(
            "depset(direct = ['a', 'b'], order='postorder',\n       transitive = [depset(['c', 'd'], order='topological')])"
        ),
        Err("Order 'postorder' is incompatible with order 'topological'".into())
    );
}

#[test]
fn test_bad_generic_type() {
    assert_eq!(
        exec_err("depset(['a', 5])"),
        Err("cannot add an item of type 'int' to a depset of 'string'".into())
    );
}

#[test]
fn test_bad_generic_type_direct() {
    assert_eq!(
        exec_err("depset(direct = ['a', 5])"),
        Err("cannot add an item of type 'int' to a depset of 'string'".into())
    );
}

#[test]
fn test_bad_generic_type_transitive() {
    assert_eq!(
        exec_err("depset(['a', 'b'], transitive=[depset([1])])"),
        Err("cannot add an item of type 'int' to a depset of 'string'".into())
    );
}

#[test]
fn test_too_many_positionals() {
    // Java: "depset() accepts no more than 2 positional arguments but got 3"; text differs.
    assert!(exec_err("depset([], 'default', [])").is_err());
}

#[test]
fn test_transitive_order_direct() {
    assert_eq!(
        to_list("depset(direct=[], transitive=[depset(['a', 'b', 'c'])])"),
        r#"["a", "b", "c"]"#
    );
    assert_eq!(
        to_list("depset(direct=['a'], transitive = [depset(['b', 'c'])])"),
        r#"["b", "c", "a"]"#
    );
    assert_eq!(
        to_list("depset(direct=['a', 'b'], transitive = [depset(['c'])])"),
        r#"["c", "a", "b"]"#
    );
    assert_eq!(
        to_list("depset(direct=['a', 'b', 'c'], transitive = [depset([])])"),
        r#"["a", "b", "c"]"#
    );
}

#[test]
fn test_incompatible_union() {
    // Java: "unsupported binary operation: depset + list" (text differs).
    assert!(exec_err("depset([]) + ['a']").is_err());
    assert!(exec_err("depset([]) | ['a']").is_err());
}

#[test]
fn test_to_string() {
    assert_eq!(
        eval_expr("str(depset([3, 4, 5], transitive = [depset([2, 4, 6])]))"),
        Ok(r#""depset([2, 4, 6, 3, 5])""#.into())
    );
}

#[test]
fn test_to_string_with_order() {
    assert_eq!(
        eval_expr(
            "str(depset([3, 4, 5], transitive = [depset([2, 4, 6])], order = 'topological'))"
        ),
        Ok(r#""depset([3, 5, 6, 4, 2], order = \"topological\")""#.into())
    );
}

#[test]
fn test_to_list_for_starlark() {
    assert_eq!(
        to_list("depset([3, 4, 5], transitive = [depset([2, 4, 6])])"),
        "[2, 4, 6, 3, 5]"
    );
}

#[test]
fn test_depset_is_not_iterable() {
    for src in [
        "list(depset(['a', 'b']))",
        "max(depset([1, 2, 3]))",
        "1 in depset([1, 2, 3])",
        "sorted(depset(['a', 'b']))",
        "tuple(depset(['a', 'b']))",
        "[x for x in depset()]",
        "len(depset(['a']))",
    ] {
        assert!(exec_err(&format!("x = {src}")).is_err(), "{src}");
    }
}

#[test]
fn test_order_compatibility() {
    let orders = ["default", "postorder", "topological", "preorder"];
    for first in orders {
        for second in orders {
            let src = format!(
                "s1 = depset(['1', '11'], order = '{first}')\n\
                 s2 = depset(['2', '22'], order = '{second}')\n\
                 x = depset(transitive = [s1, s2], order = '{first}')\n"
            );
            let compatible = exec(&src).is_ok();
            let want = first == "default" || second == "default" || first == second;
            assert_eq!(compatible, want, "{first} {second}");
        }
    }
}

#[test]
fn test_mutable_depset_elements_desired_behavior() {
    for (src, want) in [
        (
            "depset([[1,2,3]])",
            "depset elements must not be mutable values",
        ),
        (
            "depset(direct=[[1,2,3]])",
            "depset elements must not be mutable values",
        ),
        (
            "depset([struct(a=[])])",
            "depset elements must not be mutable values",
        ),
        (
            "depset(direct=[struct(a=[])])",
            "depset elements must not be mutable values",
        ),
        ("depset([{}])", "depset elements must not be mutable values"),
        (
            "depset(direct=[{}])",
            "depset elements must not be mutable values",
        ),
        (
            "depset([struct(a={})])",
            "depset elements must not be mutable values",
        ),
        (
            "depset(direct=[struct(a={})])",
            "depset elements must not be mutable values",
        ),
    ] {
        assert_eq!(exec_err(&format!("x = {src}")), Err(want.into()), "{src}");
    }
}

#[test]
fn test_frozen_list_element() {
    // Java: a frozen list is immutable but still "depsets cannot contain items of type 'list'";
    // a tuple holding it is accepted.
    let mut loads = LoadMap::new();
    let lib = exec("x = []").unwrap();
    loads.insert(":lib.bzl", lib);
    let (r, _) = common::exec_with(
        // Java also accepts depset([(x,)]); starlark-rust lists are never hashable, so that
        // fails here with "Value of type `list` is not hashable" (known difference).
        "load(':lib.bzl', 'x')\nbad = depset([x])\n",
        StarlarkSemantics::default(),
        loads,
    );
    assert_eq!(
        r.unwrap_err().message,
        "depsets cannot contain items of type 'list'"
    );
}

#[test]
fn test_constructor_depth_limit() {
    let setup = "def create_depset(depth):\n  x = depset([0])\n  for i in range(1, depth):\n    x = depset([i], transitive = [x])\n";
    assert!(exec(&format!("{setup}r = create_depset(3000)\n")).is_ok());
    assert_eq!(
        exec_err(&format!("{setup}r = create_depset(4000)\n")),
        Err("depset depth 3501 exceeds limit (3500)".into())
    );
    let sem = StarlarkSemantics {
        nested_set_depth_limit: 100,
        ..Default::default()
    };
    let (r, _) = common::exec_with(
        &format!("{setup}r = create_depset(99)\n"),
        sem.clone(),
        LoadMap::new(),
    );
    assert!(r.is_ok());
    let (r, _) = common::exec_with(
        &format!("{setup}r = create_depset(1000)\n"),
        sem,
        LoadMap::new(),
    );
    assert_eq!(
        r.unwrap_err().message,
        "depset depth 101 exceeds limit (100)"
    );
}

#[test]
fn test_set_comparison() {
    assert_eq!(
        exec_err("x = depset([1, 2]) < depset([3, 4])"),
        Err("unsupported comparison: depset <=> depset".into())
    );
}

#[test]
fn test_depset_direct_invalid_type() {
    assert_eq!(
        exec_err("depset(direct='hello')"),
        Err(
            "in call to depset(), parameter 'direct' got value of type 'string', want 'sequence or NoneType'"
                .into()
        )
    );
}

#[test]
fn test_empty_depset_interned_per_order() {
    let m = exec(
        "stable1 = depset()\nstable2 = depset()\npreorder1 = depset(order = 'preorder')\npreorder2 = depset(order = 'preorder')\n",
    )
    .unwrap();
    let same = |a: &str, b: &str| m.get(a).unwrap().value().ptr_eq(m.get(b).unwrap().value());
    assert!(same("stable1", "stable2"));
    assert!(same("preorder1", "preorder2"));
    assert!(!same("stable1", "preorder1"));
}

#[test]
fn test_single_non_empty_transitive_and_no_directs_unwrapped() {
    let m = exec(
        "inner = depset([1, 2, 3])\nouter = depset(transitive = [depset(), inner, depset()])\n",
    )
    .unwrap();
    let inner = m.get("inner").unwrap();
    let outer = m.get("outer").unwrap();
    assert!(inner.value().ptr_eq(outer.value()));
}

#[test]
fn test_single_non_empty_transitive_and_matching_direct_unwrapped() {
    let m = exec(
        "inner = depset([1])\nouter = depset([1], transitive = [depset(), inner, depset()])\n",
    )
    .unwrap();
    let inner = m.get("inner").unwrap();
    let outer = m.get("outer").unwrap();
    assert!(inner.value().ptr_eq(outer.value()));
}

#[test]
fn depset_survives_freezing_and_converts_to_nested_set() {
    // Not in Java: the Rust conversion to blaze_collect::NestedSet keeps to_list order.
    let m = exec(
        "a = depset(['x', 'y'])\nb = depset(['z'], transitive = [a], order = 'topological')\nc = depset(['w', 'x'], transitive = [a, depset(['q'])], order = 'preorder')\n",
    )
    .unwrap();
    for name in ["a", "b", "c"] {
        let v = m.get(name).unwrap();
        let d = Depset::from_value(v.value()).unwrap();
        let starlark: Vec<String> = d.to_list().unwrap().iter().map(|v| v.to_str()).collect();
        let ns = d
            .to_nested_set(&mut |v| Ok(v.unpack_str().unwrap().to_owned()))
            .unwrap();
        assert_eq!(*ns.to_list(), starlark, "{name}");
    }
}

// W4-analysis: to_nested_set_with_memo keeps Java's NestedSet identity across calls.
// Java: Depset.getSet returns the wrapped NestedSet; NestedSetBuilder.build returns a lone
// transitive set of the same order (NestedSetBuilder.java build), and sub-sets share their array.
#[test]
fn to_nested_set_with_memo_keeps_node_identity_across_calls() {
    use blaze_collect::NestedSet;
    use blaze_starlark::depset::DepsetNodeMemo;
    use std::collections::HashMap;

    struct Memo(HashMap<(usize, bool), NestedSet<String>>);
    impl DepsetNodeMemo<String> for Memo {
        fn get(&mut self, node: usize, frozen: bool) -> Option<NestedSet<String>> {
            self.0.get(&(node, frozen)).cloned()
        }
        fn put(&mut self, node: usize, frozen: bool, set: NestedSet<String>) -> NestedSet<String> {
            self.0.entry((node, frozen)).or_insert(set).clone()
        }
    }

    let m = exec(
        "\
a = depset(['x', 'y'])
b = depset(['z'], transitive = [a])
c = depset(['w'], transitive = [a])
d = depset(transitive = [a])
",
    )
    .unwrap();
    let mut memo = Memo(HashMap::new());
    let convert = |name: &str, memo: &mut Memo| -> NestedSet<String> {
        let v = m.get(name).unwrap();
        let value = v.value();
        Depset::from_value(value)
            .unwrap()
            .to_nested_set_with_memo(
                value.unpack_frozen().is_some(),
                &mut |e| Ok(e.unpack_str().unwrap().to_owned()),
                memo,
            )
            .unwrap()
    };
    let b1 = convert("b", &mut memo);
    let b2 = convert("b", &mut memo);
    assert_eq!(b1.node_id(), b2.node_id());
    let a = convert("a", &mut memo);
    let c = convert("c", &mut memo);
    let d = convert("d", &mut memo);
    // The same order and elements as the plain conversion.
    let plain = |name: &str| -> Vec<String> {
        let v = m.get(name).unwrap();
        Depset::from_value(v.value())
            .unwrap()
            .to_nested_set(&mut |e| Ok(e.unpack_str().unwrap().to_owned()))
            .unwrap()
            .to_list()
            .to_vec()
    };
    assert_eq!(b1.to_list().to_vec(), plain("b"));
    assert_eq!(c.to_list().to_vec(), plain("c"));
    assert_eq!(d.to_list().to_vec(), plain("d"));
    // `a`'s node is the one inside `b` and `c` (one sub-array, shared), and `d` is `a`'s set.
    let sub = |s: &NestedSet<String>| -> Vec<usize> {
        match s.children() {
            blaze_collect::nestedset::NestedSetChildren::Array(entries) => entries
                .into_iter()
                .filter_map(|e| match e {
                    blaze_collect::nestedset::NestedSetChild::Set(x) => Some(x.node_id()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    };
    assert_eq!(sub(&b1), vec![a.node_id()]);
    assert_eq!(sub(&c), vec![a.node_id()]);
    assert_eq!(d.node_id(), a.node_id());
}
