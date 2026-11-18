mod common;
use blaze_options::{OptionsClass, OptionsParser};
use common::test_options::TestOptions;

#[test]
fn smoke() {
    let mut p = OptionsParser::builder()
        .options_classes(&[TestOptions::class()])
        .build()
        .unwrap();
    p.parse(&[
        "--test_string=x",
        "res",
        "--test_expansion",
        "--test_multiple_string",
        "a",
        "--test_multiple_string=b",
        "--test_implicit_requirement=q",
    ])
    .unwrap();
    let o = p.get_options::<TestOptions>().unwrap();
    assert_eq!(o.test_string, "x");
    assert_eq!(o.expanded_c, 42);
    assert!(!o.expanded_a);
    assert_eq!(o.test_multiple_string, vec!["a", "b"]);
    assert_eq!(o.implicit_requirement_a, "implicit requirement, required");
    assert_eq!(p.get_residue(), vec!["res"]);
    let d = TestOptions::default();
    assert_eq!(d.expanded_c, 12);
    assert_eq!(d.test_string_null_by_default, None);
    let e = p.parse(&["--test_strin=y"]).unwrap_err();
    assert_eq!(
        e.get_message(),
        "Unrecognized option: --test_strin=y (did you mean '--test_string'?)"
    );
    println!("{:?}", p.canonicalize());
}
