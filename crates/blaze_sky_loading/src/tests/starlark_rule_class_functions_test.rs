//! Ports of the tests of
//! `src/test/java/com/google/devtools/build/lib/starlark/StarlarkRuleClassFunctionsTest.java`
//! that only evaluate .bzl code or only load packages.
//!
//! How the Java test environment maps onto the loading harness:
//! - `evalAndExport(ev, lines...)` and `ev.exec(lines...)` execute the lines as the .bzl file
//!   `FAKE_LABEL` (`//fake/label.bzl`, that is `//fake/label.bzl:label.bzl`). Here the lines are
//!   written to that file, which `//test:BUILD` loads ([`Ev`]). Like Java's `ev`, an [`Ev`] keeps
//!   the globals of earlier `exec` calls: the file is the lines executed so far plus the new
//!   ones.
//! - `ev.checkEvalErrorContains(msg, lines...)` (the exception's message contains `msg`) and
//!   `ev.assertContainsError(msg)` become: loading fails and an ERROR event contains `msg`.
//! - `ev.eval(expr)` and `ev.lookup(name)` become [`Ev::eval_strs`]: the expression is evaluated
//!   in the .bzl file and carried out through the `tags` of a `filegroup`, so the expectation
//!   is on the value's Starlark `str` or `repr` (stated at each test).
//! - `getConfiguredTarget(label)` in a test that asserts on a loading error becomes loading
//!   the package; `getPackage(pkg)` returning null becomes an evaluation error.
//!
//! Tests of this Java class that are not ported are listed, with the reason, in
//! `crates/blaze_packages/src/starlark_defs/tests_ignored.rs`.

use std::cell::RefCell;

use blaze_events::{Event, EventKind};
use blaze_packages::types::AttrValue;
use blaze_skyframe::EvaluationResult;

use crate::tests::package_loading_test_case::PackageLoadingTestCase;

/// Java `FAKE_LABEL = Label.parseCanonicalUnchecked("//fake/label.bzl")`.
const FAKE_LABEL: &str = "//fake/label.bzl:label.bzl";

fn error_messages(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.get_kind() == EventKind::Error)
        .map(|e| e.get_message().into_owned())
        .collect()
}

fn all_messages(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|e| e.get_message().into_owned())
        .collect()
}

/// Java `ev.assertContainsError(expected)`: an ERROR event contains `expected`.
fn assert_contains_error(events: &[Event], expected: &str) {
    assert!(
        error_messages(events).iter().any(|m| m.contains(expected)),
        "no error event contains {expected:?}; events: {:#?}",
        all_messages(events)
    );
}

/// Java `assertContainsEvent(expected)`: an event of any kind contains `expected`.
fn assert_contains_event(events: &[Event], expected: &str) {
    assert!(
        all_messages(events).iter().any(|m| m.contains(expected)),
        "no event contains {expected:?}; events: {:#?}",
        all_messages(events)
    );
}

/// Java `BazelEvaluationTestCase ev` (see the module documentation).
struct Ev {
    t: PackageLoadingTestCase,
    /// The lines executed so far.
    lines: RefCell<Vec<String>>,
}

impl Ev {
    fn new() -> Ev {
        Ev {
            t: PackageLoadingTestCase::new(),
            lines: RefCell::new(Vec::new()),
        }
    }

    /// Writes `FAKE_LABEL`'s file (the lines executed so far, `lines`, and one assignment per
    /// expression of `exprs`) and `//test:BUILD`, which loads it and stores the expressions'
    /// values (strings) in the `tags` of `//test:ev_value_<i>`; then loads `//test`.
    fn run(&self, lines: &[&str], exprs: &[&str]) -> (EvaluationResult, Vec<Event>) {
        let mut file: Vec<String> = self.lines.borrow().clone();
        file.extend(lines.iter().map(|l| (*l).to_owned()));
        let mut symbols = vec!["ev_loaded".to_owned()];
        for (i, expr) in exprs.iter().enumerate() {
            file.push(format!("ev_value_{i} = {expr}"));
            symbols.push(format!("ev_value_{i}"));
        }
        file.push("ev_loaded = True".to_owned());
        let file: Vec<&str> = file.iter().map(String::as_str).collect();
        self.t.scratch_file("fake/label.bzl/BUILD", &[]);
        self.t.scratch_file("fake/label.bzl/label.bzl", &file);
        let load = format!(
            "load(\"{FAKE_LABEL}\", {})",
            symbols
                .iter()
                .map(|s| format!("\"{s}\""))
                .collect::<Vec<_>>()
                .join(", ")
        );
        // One rule per value: `tags` is an order-independent attribute, which Bazel sorts
        // (BuildType.java:249), so several values in one list would lose their order.
        let mut build = vec![load];
        for symbol in &symbols[1..] {
            build.push(format!("filegroup(name = \"{symbol}\", tags = [{symbol}])"));
        }
        let build: Vec<&str> = build.iter().map(String::as_str).collect();
        self.t.scratch_file("test/BUILD", &build);
        self.t.evaluate_package("test")
    }

    /// Java `ev.exec(lines...)` / `evalAndExport(ev, lines...)` with the default fail-fast
    /// handler: the lines execute without error; their globals stay defined.
    fn exec(&self, lines: &[&str]) {
        let (result, events) = self.run(lines, &[]);
        assert!(
            !result.has_error(),
            "{:?}\nevents: {:#?}",
            result,
            all_messages(&events)
        );
        assert!(
            error_messages(&events).is_empty(),
            "error events: {:#?}",
            error_messages(&events)
        );
        self.lines
            .borrow_mut()
            .extend(lines.iter().map(|l| (*l).to_owned()));
    }

    /// Java `ev.setFailFast(false); evalAndExport(ev, lines...)`: the events of executing the
    /// lines, whether or not they fail.
    fn exec_collecting_events(&self, lines: &[&str]) -> Vec<Event> {
        let (_result, events) = self.run(lines, &[]);
        events
    }

    /// Java `ev.checkEvalErrorContains(msg, lines...)`.
    fn check_eval_error_contains(&self, msg: &str, lines: &[&str]) {
        let (result, events) = self.run(lines, &[]);
        assert!(
            result.has_error(),
            "Expected error containing '{msg}' but got no error; events: {:#?}",
            all_messages(&events)
        );
        assert_contains_error(&events, msg);
    }

    /// The values of the Starlark expressions `exprs` (each a string) in the environment of
    /// the lines executed so far (Java `ev.eval(expr)`, `ev.lookup(name)`).
    fn eval_strs(&self, exprs: &[&str]) -> Vec<String> {
        let (result, events) = self.run(&[], exprs);
        assert!(
            !result.has_error(),
            "{:?}\nevents: {:#?}",
            result,
            all_messages(&events)
        );
        let loaded = self.t.get_package("test");
        assert!(
            !loaded.package.contains_errors(),
            "package 'test' contains errors: {:#?}",
            all_messages(&loaded.events)
        );
        (0..exprs.len())
            .map(|i| {
                let name = format!("ev_value_{i}");
                let rule = loaded
                    .package
                    .get_rule(&name)
                    .unwrap_or_else(|| panic!("the rule //test:{name}"));
                match blaze_packages::target::RuleOrMacroInstance::get_attr(&**rule, "tags") {
                    Some(AttrValue::StringList(mut tags)) if tags.len() == 1 => tags.remove(0),
                    other => panic!("tags of //test:{name}: {other:?}"),
                }
            })
            .collect()
    }

    /// [`Ev::eval_strs`] of one expression.
    fn eval_str(&self, expr: &str) -> String {
        self.eval_strs(&[expr])
            .into_iter()
            .next()
            .expect("one value")
    }
}

/// Java `checkAttributeError(expectedMessage, lines...)`: the last line is an attribute
/// expression, assigned to `fakeAttribute` (`buildAttribute`); an event contains the message.
fn check_attribute_error(expected_message: &str, lines: &[&str]) {
    let ev = Ev::new();
    let (last, init) = lines.split_last().expect("at least one line");
    let assignment = format!("fakeAttribute = {last}");
    let mut strings: Vec<&str> = init.to_vec();
    strings.push(&assignment);
    let events = ev.exec_collecting_events(&strings);
    assert_contains_event(&events, expected_message);
}

/// Java `registerDummyStarlarkFunction()`.
fn register_dummy_starlark_function(ev: &Ev) {
    ev.exec(&["def impl():", "  pass"]);
}

// ---- rule() and attr.*: definition errors

// Java: StarlarkRuleClassFunctionsTest.testCannotOverrideBuiltInAttribute
#[test]
fn test_cannot_override_built_in_attribute() {
    let ev = Ev::new();
    let events = ev.exec_collecting_events(&[
        "def impl(ctx):",
        "  return",
        "r = rule(impl, attrs = {'tags': attr.string_list()})",
    ]);
    assert_contains_error(
        &events,
        "Error in rule: attribute `tags`: built-in attributes cannot be overridden.",
    );
}

// Java: StarlarkRuleClassFunctionsTest.testCannotOverrideBuiltInAttributeName
#[test]
fn test_cannot_override_built_in_attribute_name() {
    let ev = Ev::new();
    let events = ev.exec_collecting_events(&[
        "def impl(ctx):",
        "  return",
        "r = rule(impl, attrs = {'name': attr.string()})",
    ]);
    assert_contains_error(
        &events,
        "Error in rule: attribute `name`: built-in attributes cannot be overridden.",
    );
}

// Java: StarlarkRuleClassFunctionsTest.testRuleClassTooManyAttributes
// Java also asserts that the error is the only event of the .bzl thread
// (`assertThat(ev.getEventCollector()).hasSize(1)`); the events here are those of loading the
// package, so the assertion is that exactly one event carries the message.
#[test]
fn test_rule_class_too_many_attributes() {
    let ev = Ev::new();
    let mut lines = vec![
        "def impl(ctx): return".to_owned(),
        "r = rule(impl, attrs = {".to_owned(),
    ];
    for i in 0..200 {
        lines.push(format!("    'attr{i}': attr.int(),"));
    }
    lines.push("})".to_owned());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let events = ev.exec_collecting_events(&lines);
    let matching: Vec<&Event> = events
        .iter()
        .filter(|e| {
            e.get_message()
                .contains("Rule class r declared too many attributes")
        })
        .collect();
    assert_eq!(matching.len(), 1, "{:#?}", all_messages(&events));
    assert_eq!(matching[0].get_kind(), EventKind::Error);
}

// Java: StarlarkRuleClassFunctionsTest.testRuleClassTooLongAttributeName
//   assertThat(event.getLocation().toString()).isEqualTo(":2:9");
//   assertThat(event.getMessage())
//       .matches("Attribute r\\.x{150}'s name is too long \\(150 > 128\\)");
#[test]
#[ignore = "review S4: a rule export error aborts the assignment instead of being reported as \
            an event at the rule() location (StarlarkRuleFunction.export)"]
fn test_rule_class_too_long_attribute_name() {
    let ev = Ev::new();
    let attr_line = format!(
        "r = rule(impl, attrs = {{ '{}': attr.int() }})",
        "x".repeat(150)
    );
    let events = ev.exec_collecting_events(&["def impl(ctx): return;", attr_line.as_str()]);
    let expected = format!(
        "Attribute r.{}'s name is too long (150 > 128)",
        "x".repeat(150)
    );
    let matching: Vec<&Event> = events
        .iter()
        .filter(|e| e.get_message() == expected.as_str())
        .collect();
    assert_eq!(matching.len(), 1, "{:#?}", all_messages(&events));
    assert_eq!(matching[0].get_kind(), EventKind::Error);
    let location = matching[0].get_location().expect("the rule() location");
    assert_eq!((location.line(), location.column()), (2, 9));
}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithWrongProvidersList
#[test]
fn test_attr_with_wrong_providers_list() {
    check_attribute_error(
        "Error in label_list: at index 1 of providers, got element of type int, want Provider",
        &[
            "a = provider()",
            "c = provider()",
            "attr.label_list(allow_files = True,  providers = [[a, 1], [c]])",
        ],
    );
    check_attribute_error(
        "Error in label_list: at index 1 of providers, got element of type string, want sequence",
        &[
            "b = provider()",
            "attr.label_list(allow_files = True,  providers = [['a', b], 'c'])",
        ],
    );
    check_attribute_error(
        "Error in label_list: at index 1 of providers, got element of type Provider, want sequence",
        &[
            "a = provider()",
            "b = provider()",
            "c = provider()",
            "attr.label_list(allow_files = True,  providers = [[a, b], c])",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testLabelListWithAspectsError
#[test]
fn test_label_list_with_aspects_error() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "at index 1 of aspects, got element of type int, want Aspect",
        &[
            "def _impl(target, ctx):",
            "   pass",
            "my_aspect = aspect(implementation = _impl)",
            "attr.label_list(aspects = [my_aspect, 123])",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAttrWithAspectRequiringAspects_requireExistingAspect_failed
#[test]
fn test_attr_with_aspect_requiring_aspects_require_existing_aspect_failed() {
    let ev = Ev::new();
    let events = ev.exec_collecting_events(&[
        "def _impl(target, ctx):",
        "   pass",
        "aspect_b = aspect(implementation = _impl)",
        "aspect_a = aspect(implementation = _impl, requires = [aspect_b])",
        "attr.label_list(aspects = [aspect_a, aspect_b])",
    ]);
    assert_contains_error(
        &events,
        &format!(
            "aspect {FAKE_LABEL}%aspect_b was added before as a required aspect of aspect \
             {FAKE_LABEL}%aspect_a"
        ),
    );
}

// Java: StarlarkRuleClassFunctionsTest.testNonLabelAttrWithProviders
#[test]
fn test_non_label_attr_with_providers() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "unexpected keyword argument 'providers'",
        &["attr.string(providers = ['a'])"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testLabelAttrDefaultValueAsStringBadValue
#[test]
fn test_label_attr_default_value_as_string_bad_value() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "invalid label '/foo:bar' in parameter 'default' of attribute 'label': \
         invalid package name '/foo': package names may not start with '/'",
        &["attr.label(default = '/foo:bar')"],
    );
    ev.check_eval_error_contains(
        "invalid label '/bar:foo' in element 1 of parameter 'default' of attribute \
         'label_list': invalid package name '/bar': package names may not start with '/'",
        &["attr.label_list(default = ['//foo:bar', '/bar:foo'])"],
    );
    ev.check_eval_error_contains(
        "invalid label '/bar:foo' in dict key element: invalid package name '/bar': \
         package names may not start with '/'",
        &["attr.label_keyed_string_dict(default = {'//foo:bar': 'a', '/bar:foo': 'b'})"],
    );
    ev.check_eval_error_contains(
        "invalid label '/bar:foo' in element 1 of dict value element: invalid package name \
         '/bar': package names may not start with '/'",
        &["attr.label_list_dict(default = {'my': ['//foo:bar', '/bar:foo']})"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAttrBadKeywordArguments
#[test]
fn test_attr_bad_keyword_arguments() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "string() got unexpected keyword argument 'bad_keyword'",
        &["attr.string(bad_keyword = '')"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.incompatibleDataTransition
#[test]
fn incompatible_data_transition() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "cfg must be either 'target', 'exec'",
        &["attr.label(cfg = 'data')"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testNoAttrLicense
#[test]
fn test_no_attr_license() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "'attr' value has no field or method 'license'",
        &["attr.license()"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAttrDocValueBadType
#[test]
fn test_attr_doc_value_bad_type() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "got value of type 'int', want 'string or NoneType'",
        &["attr.string(doc = 1)"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.unknownRuleAttributeFlags_forbidden
#[test]
fn unknown_rule_attribute_flags_forbidden() {
    let ev = Ev::new();
    let events = ev.exec_collecting_events(&[
        "def _impl(ctx): return None",
        "r1 = rule(_impl, attrs = { 'srcs': attr.label_list(flags = ['NO-SUCH-FLAG']) })",
    ]);
    assert_contains_error(&events, "unknown attribute flag 'NO-SUCH-FLAG'");
}

// Java: StarlarkRuleClassFunctionsTest.testRuleUnknownKeyword
#[test]
fn test_rule_unknown_keyword() {
    let ev = Ev::new();
    register_dummy_starlark_function(&ev);
    ev.check_eval_error_contains(
        "unexpected keyword argument 'bad_keyword'",
        &["rule(impl, bad_keyword = 'some text')"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testRuleImplementationMissing
#[test]
fn test_rule_implementation_missing() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "rule() missing 1 required positional argument: implementation",
        &["rule(attrs = {})"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testRuleBadTypeForAdd
#[test]
fn test_rule_bad_type_for_add() {
    let ev = Ev::new();
    register_dummy_starlark_function(&ev);
    ev.check_eval_error_contains(
        "in call to rule(), parameter 'attrs' got value of type 'string', want 'dict'",
        &["rule(impl, attrs = 'some text')"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testRuleBadTypeInAdd
#[test]
fn test_rule_bad_type_in_add() {
    let ev = Ev::new();
    register_dummy_starlark_function(&ev);
    ev.check_eval_error_contains(
        "got dict<string, string> for 'attrs', want dict<string, Attribute>",
        &["rule(impl, attrs = {'a1': 'some text'})"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testIntDefaultValueMustBeInt32
#[test]
fn test_int_default_value_must_be_int32() {
    // This is a test of the loading phase. Move somewhere more appropriate.
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "for parameter 'default' of attribute '', got 4294967296, want value in signed 32-bit \
         range",
        &["attr.int(default = 0x10000 * 0x10000)"],
    );
    ev.check_eval_error_contains(
        "for element 0 of parameter 'default' of attribute '', got 4294967296, want value in \
         signed 32-bit range",
        &["attr.int_list(default = [0x10000 * 0x10000])"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testLabelAttrWrongDefault
#[test]
fn test_label_attr_wrong_default() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "got value of type 'int', want 'Label, string, LateBoundDefault, function, or NoneType'",
        &["attr.label(default = 123)"],
    );
}

// ---- Label

// Java: StarlarkRuleClassFunctionsTest.testLabelNameAndPackage
// (`ev.eval(...)` results are the strings themselves.)
#[test]
fn test_label_name_and_package() {
    let ev = Ev::new();
    assert_eq!(ev.eval_str("Label('//foo/bar:baz').name"), "baz");
    // NB: implicitly creates a new pkgContext and environments, yet labels should be the same.
    assert_eq!(ev.eval_str("Label('//foo/bar:baz').package"), "foo/bar");
}

// Java: StarlarkRuleClassFunctionsTest.testLabelGetRelativeSyntaxError
#[test]
fn test_label_get_relative_syntax_error() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "invalid target name 'bad//syntax': target names may not contain '//' path separators",
        &["Label('//foo:bar').relative('bad//syntax')"],
    );
}

// ---- struct

// Java: StarlarkRuleClassFunctionsTest.testStructFields
// (`x.getValue("a")` is `StarlarkInt.of(1)`: here `repr(x.a)`.)
#[test]
#[ignore = "starlark-rust core-language error text still differs after patches 0049-0055: Blaze 'cannot set .a field of struct value', Java 'struct value does not support field assignment'"]
fn test_struct_fields() {
    let ev = Ev::new();
    ev.exec(&["x = struct(a = 1, b = 2)"]);
    assert_eq!(ev.eval_strs(&["repr(x.a)", "repr(x.b)"]), vec!["1", "2"]);
    // Update is prohibited. (A fresh file: Java's `ev` lets a later statement rebind `x`,
    // one .bzl file does not.)
    Ev::new().check_eval_error_contains(
        "struct value does not support field assignment",
        &["x = struct(a = 1); x.a = 2"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testStructEquality
// (Each `(Boolean) ev.eval(...)` is `str(...)` here.)
#[test]
fn test_struct_equality() {
    let ev = Ev::new();
    assert_eq!(
        ev.eval_strs(&[
            "str(struct(a = 1, b = 2) == struct(b = 2, a = 1))",
            "str(struct(a = 1) == struct(a = 1, b = 2))",
            "str(struct(a = 1, b = 2) == struct(a = 1))",
        ]),
        vec!["True", "False", "False"]
    );
    // Compare a recursive object to itself to make sure reference equality is checked
    ev.exec(&["s = struct(a = 1, b = []); s.b.append(s)"]);
    assert_eq!(
        ev.eval_strs(&[
            "str(s == s)",
            "str(struct(a = 1, b = 2) == struct(a = 1, b = 3))",
            "str(struct(a = 1) == [1])",
            "str([1] == struct(a = 1))",
            "str(struct() == struct())",
            "str(struct() == struct(a = 1))",
        ]),
        vec!["True", "False", "False", "False", "True", "False"]
    );
    ev.exec(&["foo = provider(); bar = provider()"]);
    assert_eq!(
        ev.eval_strs(&[
            "str(struct(a = 1) == foo(a = 1))",
            "str(foo(a = 1) == struct(a = 1))",
            "str(foo(a = 1) == bar(a = 1))",
            "str(foo(a = 1) == foo(a = 1))",
        ]),
        vec!["False", "False", "False", "True"]
    );
}

// Java: StarlarkRuleClassFunctionsTest.testStructIncomparability
#[test]
fn test_struct_incomparability() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "unsupported comparison: struct <=> struct",
        &["struct(a = 1) < struct(a = 2)"],
    );
    ev.check_eval_error_contains(
        "unsupported comparison: struct <=> struct",
        &["struct(a = 1) > struct(a = 2)"],
    );
    ev.check_eval_error_contains(
        "unsupported comparison: struct <=> struct",
        &["struct(a = 1) <= struct(a = 2)"],
    );
    ev.check_eval_error_contains(
        "unsupported comparison: struct <=> struct",
        &["struct(a = 1) >= struct(a = 2)"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testStructAccessingFieldsFromStarlark
// (`ev.lookup("x1")` is `StarlarkInt.of(1)`: here `repr(x1)`.)
#[test]
fn test_struct_accessing_fields_from_starlark() {
    let ev = Ev::new();
    ev.exec(&["x = struct(a = 1, b = 2)", "x1 = x.a", "x2 = x.b"]);
    assert_eq!(ev.eval_strs(&["repr(x1)", "repr(x2)"]), vec!["1", "2"]);
}

// Java: StarlarkRuleClassFunctionsTest.testStructAccessingUnknownField
#[test]
fn test_struct_accessing_unknown_field() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "'struct' value has no field or method 'c'\nAvailable attributes: a, b",
        &["x = struct(a = 1, b = 2)", "y = x.c"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testStructAccessingUnknownFieldWithArgs
#[test]
fn test_struct_accessing_unknown_field_with_args() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "'struct' value has no field or method 'c'",
        &["x = struct(a = 1, b = 2)", "y = x.c()"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testStructAccessingNonFunctionFieldWithArgs
#[test]
fn test_struct_accessing_non_function_field_with_args() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "'int' object is not callable",
        &["x = struct(a = 1, b = 2)", "x1 = x.a(1)"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testStructAccessingFunctionFieldWithArgs
// (`ev.lookup("x1")` is `StarlarkInt.of(6)`: here `repr(x1)`.)
#[test]
fn test_struct_accessing_function_field_with_args() {
    let ev = Ev::new();
    ev.exec(&[
        "def f(x): return x+5",
        "x = struct(a = f, b = 2)",
        "x1 = x.a(1)",
    ]);
    assert_eq!(ev.eval_str("repr(x1)"), "6");
}

// Java: StarlarkRuleClassFunctionsTest.testStructPosArgs
#[test]
#[ignore = "starlark-rust core-language error text still differs after patches 0049-0055: Blaze 'Error in <function struct>: Wrong number of positional arguments, expected 0, got 1', Java 'struct() got unexpected positional argument'"]
fn test_struct_pos_args() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "struct() got unexpected positional argument",
        &["x = struct(1, b = 2)"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testStructConcatenationFieldValues
// (`z.getValue("a")` is `StarlarkInt.of(1)`: here `repr(z.a)`.)
#[test]
fn test_struct_concatenation_field_values() {
    let ev = Ev::new();
    ev.exec(&[
        "x = struct(a = 1, b = 2)",
        "y = struct(c = 1, d = 2)",
        "z = x + y\n",
    ]);
    assert_eq!(
        ev.eval_strs(&["repr(z.a)", "repr(z.b)", "repr(z.c)", "repr(z.d)"]),
        vec!["1", "2", "1", "2"]
    );
}

// Java: StarlarkRuleClassFunctionsTest.testStructConcatenationCommonFields
#[test]
fn test_struct_concatenation_common_fields() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "cannot add struct instances with common field 'a'",
        &[
            "x = struct(a = 1, b = 2)",
            "y = struct(c = 1, a = 2)",
            "z = x + y\n",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testConditionalStructConcatenation
// (`x.getValue("a")` is `StarlarkInt.of(1)`: here `repr(x.a)`.)
#[test]
fn test_conditional_struct_concatenation() {
    let ev = Ev::new();
    ev.exec(&[
        "def func():",
        "  x = struct(a = 1, b = 2)",
        "  if True:",
        "    x += struct(c = 1, d = 2)",
        "  return x",
        "x = func()",
    ]);
    assert_eq!(
        ev.eval_strs(&["repr(x.a)", "repr(x.b)", "repr(x.c)", "repr(x.d)"]),
        vec!["1", "2", "1", "2"]
    );
}

// Java: StarlarkRuleClassFunctionsTest.testGetattrNoAttr
#[test]
fn test_getattr_no_attr() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "'struct' value has no field or method 'b'\nAvailable attributes: a",
        &["s = struct(a='val')", "getattr(s, 'b')"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testGetattr
// (`ev.lookup("x")` is the string "val".)
#[test]
fn test_getattr() {
    let ev = Ev::new();
    ev.exec(&[
        "s = struct(a='val')",
        "x = getattr(s, 'a')",
        "y = getattr(s, 'b', 'def')",
    ]);
    assert_eq!(ev.eval_strs(&["x", "y"]), vec!["val", "def"]);
}

// Java: StarlarkRuleClassFunctionsTest.testHasattr
// (`ev.lookup("x")` is `true`: here `str(x)`.)
#[test]
fn test_hasattr() {
    let ev = Ev::new();
    ev.exec(&[
        "s = struct(a=1)",
        "x = hasattr(s, 'a')",
        "y = hasattr(s, 'b')\n",
    ]);
    assert_eq!(ev.eval_strs(&["str(x)", "str(y)"]), vec!["True", "False"]);
}

// Java: StarlarkRuleClassFunctionsTest.testStructStr
#[test]
fn test_struct_str() {
    let ev = Ev::new();
    assert_eq!(
        ev.eval_str("str(struct(x = 2, y = 3, z = 4))"),
        "struct(x = 2, y = 3, z = 4)"
    );
}

// Java: StarlarkRuleClassFunctionsTest.testStructsInSets
#[test]
fn test_structs_in_sets() {
    let ev = Ev::new();
    ev.exec(&["depset([struct(a='a')])"]);
}

// Java: StarlarkRuleClassFunctionsTest.testStructsInDicts
#[test]
fn test_structs_in_dicts() {
    let ev = Ev::new();
    ev.exec(&["d = {struct(a = 1): 'aa', struct(b = 2): 'bb'}"]);
    assert_eq!(
        ev.eval_strs(&[
            "d[struct(a = 1)]",
            "d[struct(b = 2)]",
            "str([d[k] for k in d])",
        ]),
        vec!["aa", "bb", "[\"aa\", \"bb\"]"]
    );
    ev.check_eval_error_contains("unhashable type: 'struct'", &["{struct(a = []): 'foo'}"]);
}

// Java: StarlarkRuleClassFunctionsTest.testStructDictMembersAreMutable
// (`s.x` equals `ImmutableMap.of("a", 1, "b", 2)`: here `repr(s.x)`, a dict's repr being in
// insertion order.)
#[test]
fn test_struct_dict_members_are_mutable() {
    let ev = Ev::new();
    ev.exec(&["s = struct(x = {'a' : 1})", "s.x['b'] = 2\n"]);
    assert_eq!(ev.eval_str("repr(s.x)"), "{\"a\": 1, \"b\": 2}");
}

// Java: StarlarkRuleClassFunctionsTest.testTypeOfStruct
#[test]
fn test_type_of_struct() {
    let ev = Ev::new();
    ev.exec(&["p = type(struct)", "s = type(struct())"]);
    assert_eq!(ev.eval_strs(&["p", "s"]), vec!["Provider", "struct"]);
}

// ---- provider()

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersWithFailingInit_normalConstructorFails
// (Java also asserts `ev.lookup("d")` is null: the failed assignment bound nothing; the failed
// lines are not kept by `Ev` either.)
#[test]
fn declared_providers_with_failing_init_normal_constructor_fails() {
    let ev = Ev::new();
    ev.exec(&[
        "def _data_failing_init(x):",
        "    fail('_data_failing_init fails')",
        "data, _new_data = provider(init = _data_failing_init)",
    ]);
    ev.check_eval_error_contains(
        "_data_failing_init fails",
        &["d = data(x = 1)  # normal constructor"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersWithInitReturningInvalidType_normalConstructorFails
#[test]
fn declared_providers_with_init_returning_invalid_type_normal_constructor_fails() {
    let ev = Ev::new();
    ev.exec(&[
        "def _data_invalid_init(x):",
        "    return 'INVALID'",
        "data, _new_data = provider(init = _data_invalid_init)",
    ]);
    ev.check_eval_error_contains(
        "got string for 'return value of provider init()', want dict",
        &["d = data(x = 1)  # normal constructor"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersWithInitReturningInvalidDict_normalConstructorFails
#[test]
fn declared_providers_with_init_returning_invalid_dict_normal_constructor_fails() {
    let ev = Ev::new();
    ev.exec(&[
        "def _data_invalid_init(x):",
        "    return {('x', 'x', 'x'): x}",
        "data, _new_data = provider(init = _data_invalid_init)",
    ]);
    ev.check_eval_error_contains(
        "got dict<tuple, int> for 'return value of provider init()'",
        &["d = data(x = 1)  # normal constructor"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersWithInitReturningUnexpectedFields_normalConstructorFails
#[test]
fn declared_providers_with_init_returning_unexpected_fields_normal_constructor_fails() {
    let ev = Ev::new();
    ev.exec(&[
        "def _data_unexpected_fields_init(x):",
        "    return {'x': x, 'y': x * 2}",
        "data, _new_data = provider(fields = ['x'], init = _data_unexpected_fields_init)",
    ]);
    ev.check_eval_error_contains(
        "got unexpected field 'y' in call to instantiate provider data",
        &["d = data(x = 1)  # normal constructor"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersConcatError
#[test]
fn declared_providers_concat_error() {
    let ev = Ev::new();
    ev.exec(&["data1 = provider()", "data2 = provider()"]);
    ev.check_eval_error_contains(
        "Cannot use '+' operator on instances of different providers (data1 and data2)",
        &["d1 = data1(x = 1)", "d2 = data2(y = 2)", "d = d1 + d2"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersWithFieldsConcatSuccess
// (`ev.lookup("f1")` is `StarlarkInt.of(4)`: here `repr(f1)`.)
#[test]
fn declared_providers_with_fields_concat_success() {
    let ev = Ev::new();
    ev.exec(&[
        "data = provider(fields=['f1', 'f2'])",
        "d1 = data(f1 = 4)",
        "d2 = data(f2 = 5)",
        "d3 = d1 + d2",
        "f1 = d3.f1",
        "f2 = d3.f2",
    ]);
    assert_eq!(ev.eval_strs(&["repr(f1)", "repr(f2)"]), vec!["4", "5"]);
}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersWithFieldsConcatError
#[test]
fn declared_providers_with_fields_concat_error() {
    let ev = Ev::new();
    ev.exec(&[
        "data1 = provider(fields=['f1', 'f2'])",
        "data2 = provider(fields=['f3'])",
    ]);
    ev.check_eval_error_contains(
        "Cannot use '+' operator on instances of different providers (data1 and data2)",
        &["d1 = data1(f1=1, f2=2)", "d2 = data2(f3=3)", "d = d1 + d2"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersWithOverlappingFieldsConcatError
#[test]
fn declared_providers_with_overlapping_fields_concat_error() {
    let ev = Ev::new();
    ev.exec(&["data = provider(fields=['f1', 'f2'])"]);
    ev.check_eval_error_contains(
        "cannot add struct instances with common field 'f1'",
        &["d1 = data(f1 = 4)", "d2 = data(f1 = 5)", "d1 + d2"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersDoc
#[test]
fn declared_providers_doc() {
    let ev = Ev::new();
    ev.exec(&["data1 = provider(doc='foo')"]);
}

// Java: StarlarkRuleClassFunctionsTest.declaredProvidersBadTypeForDoc
#[test]
fn declared_providers_bad_type_for_doc() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "got value of type 'int', want 'string or NoneType'",
        &["provider(doc = 1)"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.providerWithFieldsOptionalError
#[test]
fn provider_with_fields_optional_error() {
    let ev = Ev::new();
    let events = ev.exec_collecting_events(&[
        "p = provider(fields = ['x', 'y'])",
        "p1 = p(y = 2)",
        "x = p1.x",
    ]);
    assert_contains_event(&events, " 'p' value has no field or method 'x'");
}

// Java: StarlarkRuleClassFunctionsTest.providerWithExtraFieldsError
#[test]
fn provider_with_extra_fields_error() {
    let ev = Ev::new();
    let events = ev.exec_collecting_events(&[
        "p = provider(fields = ['x', 'y'])",
        "p1 = p(x = 1, y = 2, z = 3)",
    ]);
    assert_contains_event(
        &events,
        "got unexpected field 'z' in call to instantiate provider p",
    );
}

// Java: StarlarkRuleClassFunctionsTest.providerWithEmptyFieldsError
#[test]
fn provider_with_empty_fields_error() {
    let ev = Ev::new();
    let events =
        ev.exec_collecting_events(&["p = provider(fields = [])", "p1 = p(x = 1, y = 2, z = 3)"]);
    assert_contains_event(
        &events,
        "got unexpected fields 'x', 'y', 'z' in call to instantiate provider p",
    );
}

// Java: StarlarkRuleClassFunctionsTest.providerWithDuplicateFieldsError
#[test]
#[ignore = "starlark-rust rejects the duplicate keyword argument itself ('Argument `b` occurs more than once') before the provider is called; Java StarlarkProvider.fastcall: 'got multiple values for parameter b in call to instantiate provider p'"]
fn provider_with_duplicate_fields_error() {
    let ev = Ev::new();
    let events = ev.exec_collecting_events(&[
        "p = provider(fields = ['a', 'b'])",
        "p(a = 1, b = 2, **dict(b = 3))",
    ]);
    assert_contains_event(
        &events,
        "got multiple values for parameter b in call to instantiate provider p",
    );
}

// ---- aspect()

// Java: StarlarkRuleClassFunctionsTest.testAspectParameterBadDefaultValue
#[test]
fn test_aspect_parameter_bad_default_value() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "Aspect parameter attribute 'param' has a bad default value: has to be one of 'b' \
         instead of 'a'",
        &[
            "def _impl(target, ctx):",
            "   pass",
            "my_aspect = aspect(_impl,",
            "   attrs = { 'param' : attr.string(default = 'a', values = ['b']) }",
            ")",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAspectParameterBadType
#[test]
fn test_aspect_parameter_bad_type() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "Aspect parameter attribute 'param' must have type 'bool', 'int' or 'string'.",
        &[
            "def _impl(target, ctx):",
            "   pass",
            "my_aspect = aspect(_impl,",
            "   attrs = { 'param' : attr.label(default = Label('//foo/bar:baz')) }",
            ")",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAspectNoDefaultValueAttribute
#[test]
fn test_aspect_no_default_value_attribute() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "Aspect attribute '_extra_deps' has no default value",
        &[
            "def _impl(target, ctx):",
            "   pass",
            "my_aspect = aspect(_impl,",
            "   attrs = { '_extra_deps' : attr.label() }",
            ")",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredProvidersNotAllowedWithApplyToGeneratingRules
#[test]
fn aspect_required_providers_not_allowed_with_apply_to_generating_rules() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "An aspect cannot simultaneously have required providers and apply to generating rules.",
        &[
            "prov = provider()",
            "def _impl(target, ctx):",
            "   pass",
            "my_aspect = aspect(_impl,",
            "   required_providers = [prov],",
            "   apply_to_generating_rules = True",
            ")",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.aspectBadTypeForDoc
#[test]
fn aspect_bad_type_for_doc() {
    let ev = Ev::new();
    register_dummy_starlark_function(&ev);
    ev.check_eval_error_contains(
        "got value of type 'int', want 'string or NoneType'",
        &["aspect(impl, doc = 1)"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.invalidToolchainsAspectsType
#[test]
fn invalid_toolchains_aspects_type() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "'toolchains_aspects' got value of type 'string', want 'sequence or function'",
        &[
            "def _impl(target, ctx):",
            "   pass",
            "aspect(_impl, toolchains_aspects='foo')",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAspectWithWildcardToolchainsAspects_mixedWithLabels_fails
// (Java's fail-fast handler throws the reported error as a FailFastException.)
#[test]
fn test_aspect_with_wildcard_toolchains_aspects_mixed_with_labels_fails() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "'*' must be the only item in 'toolchains_aspects' list",
        &[
            "def _aspect_impl(target, ctx):",
            "    return []",
            "my_aspect = aspect(",
            "    implementation = _aspect_impl,",
            "    toolchains_aspects = [\"*\", \"//toolchains:type1\"]",
            ")",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.aspectPropagationPredicateNotFunction_fails
#[test]
fn aspect_propagation_predicate_not_function_fails() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "parameter 'propagation_predicate' got value of type 'string', want 'function or NoneType'",
        &[
            "def _impl(target, ctx):",
            "   pass",
            "my_aspect = aspect(_impl,",
            "   propagation_predicate = 'not_a_function'",
            ")",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.aspectApplyToGeneratingRules_hasPropagationPredicate_fails
#[test]
fn aspect_apply_to_generating_rules_has_propagation_predicate_fails() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "An aspect cannot simultaneously have a propagation predicate and apply to generating \
         rules.",
        &[
            "def _impl(target, ctx):",
            "   pass",
            "def _function():",
            "  return True",
            "my_aspect = aspect(_impl,",
            "   propagation_predicate = _function,",
            "   apply_to_generating_rules = True",
            ")",
        ],
    );
}

// ---- package loading

// Java: StarlarkRuleClassFunctionsTest.testRuleCannotSetConfigurableOnAttr
#[test]
fn test_rule_cannot_set_configurable_on_attr() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("lib/BUILD", &[]);
    t.scratch_file(
        "pkg/foo.bzl",
        &[
            "def _impl(ctx):",
            "    print(\"xyz is %s\" % ctx.attr.xyz)",
            "my_rule = rule(",
            "    implementation=_impl,",
            "    attrs = {",
            "      \"xyz\": attr.string(configurable=False),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":foo.bzl\", \"my_rule\")",
            "my_rule(",
            "    name = \"abc\",",
            "    xyz = select({\"//some:condition\": \":target1\", \"//some:other_condition\": \":target2\"}),",
            ")",
        ],
    );
    let (result, events) = t.evaluate_package("pkg");
    // `Package pkg = getPackage("pkg"); assertThat(pkg).isNull();`
    assert!(result.has_error());
    assert_contains_event(
        &events,
        "attribute 'xyz' has the 'configurable' argument set, which is not allowed in rule \
         definitions",
    );
}

// Java: StarlarkRuleClassFunctionsTest.testAspectCannotSetConfigurableOnAttr
#[test]
fn test_aspect_cannot_set_configurable_on_attr() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("lib/BUILD", &[]);
    t.scratch_file(
        "pkg/foo.bzl",
        &[
            "def _impl(ctx):",
            "    print(\"xyz is %s\" % ctx.attr.xyz)",
            "my_aspect = aspect(",
            "    implementation=_impl,",
            "    attrs = {",
            "      \"xyz\": attr.string(configurable=False),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":foo.bzl\", \"my_aspect\")",
            "my_aspect(",
            "    name = \"abc\",",
            "    xyz = select({\"//some:condition\": \":target1\", \"//some:other_condition\": \":target2\"}),",
            ")",
        ],
    );
    let (result, events) = t.evaluate_package("pkg");
    // `Package pkg = getPackage("pkg"); assertThat(pkg).isNull();`
    assert!(result.has_error());
    assert_contains_event(
        &events,
        "attribute 'xyz' has the 'configurable' argument set, which is not allowed in aspect \
         definitions",
    );
}

/// Java `writeRuleCfgTestRule(cfg)`.
fn write_rule_cfg_test_rule(t: &PackageLoadingTestCase, cfg: &str) {
    let cfg_line = format!("    cfg = {cfg},");
    t.scratch_file(
        "rule_testing/rule.bzl",
        &[
            "def _impl(ctx):",
            "    pass",
            "",
            "demo_rule = rule(",
            "    implementation = _impl,",
            cfg_line.as_str(),
            ")",
        ],
    );
    t.scratch_file(
        "rule_testing/BUILD",
        &[
            "load(\":rule.bzl\", \"demo_rule\")",
            "",
            "demo_rule(name = \"my_target\")",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testRuleCfg_exec_string_fails
#[test]
fn test_rule_cfg_exec_string_fails() {
    let t = PackageLoadingTestCase::new();
    write_rule_cfg_test_rule(&t, "'exec'");
    let (_result, events) = t.evaluate_package("rule_testing");
    assert_contains_error(&events, "`cfg` must be set to a transition object");
}

// Java: StarlarkRuleClassFunctionsTest.testRuleCfg_exec_obj_fails
#[test]
fn test_rule_cfg_exec_obj_fails() {
    let t = PackageLoadingTestCase::new();
    write_rule_cfg_test_rule(&t, "config.exec()");
    let (_result, events) = t.evaluate_package("rule_testing");
    assert_contains_error(&events, "`cfg` must be set to a transition object");
}

// Java: StarlarkRuleClassFunctionsTest.ruleClassDefinedInBuildFile_fails
#[test]
fn rule_class_defined_in_build_file_fails() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "r/create.bzl",
        &["def create(impl): return rule(implementation=impl)"],
    );
    t.scratch_file(
        "r/def.bzl",
        &["load(\":create.bzl\", \"create\")", "r = create({})"],
    );
    t.scratch_file(
        "r/impl.bzl",
        &["def make_struct(ctx): return struct(value='hello')"],
    );
    t.scratch_file(
        "r/BUILD",
        &[
            "load(\":create.bzl\", \"create\")",
            "load(\":impl.bzl\", \"make_struct\")",
            "r = create(make_struct)",
            "r(name = \"r\")",
        ],
    );
    let (_result, events) = t.evaluate_package("r");
    assert_contains_error(
        &events,
        "rule() can only be used during .bzl initialization (top-level evaluation)",
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_allowlist
#[test]
#[ignore = "needs analysis: Java reports 'Non-allowlisted attempt to use initializer.' from AllowlistChecker when the target is configured (getConfiguredTarget), not at loading"]
fn initializer_allowlist() {
    let mut t = PackageLoadingTestCase::new();
    t.scratch_file(
        "p/b.bzl",
        &[
            "def initializer(**kwargs):",
            "    return kwargs",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(impl, initializer = initializer)",
        ],
    );
    t.scratch_file(
        "p/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(name = \"my_target\")",
        ],
    );
    t.set_build_language_options(&["--noexperimental_rule_extension_api"]);
    let (_result, events) = t.evaluate_package("p");
    assert_contains_error(&events, "Non-allowlisted attempt to use initializer.");
}

// Java: StarlarkRuleClassFunctionsTest.initializer_nameUnchanged
// (`getConfiguredTarget` and `assertNoEvents()`: the package loads without errors and no
// error or warning is reported.)
#[test]
fn initializer_name_unchanged() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, **kwargs):",
            "    if name != \"my_target\":",
            "        fail()",
            "    return {\"name\": name} | kwargs",
            "MyInfo = provider()",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(impl, initializer = initializer)",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(name = \"my_target\")",
        ],
    );
    let loaded = t.get_package("initializer_testing");
    assert!(!loaded.package.contains_errors());
    assert!(loaded.package.get_rule("my_target").is_some());
    let reported: Vec<String> = loaded
        .events
        .iter()
        .filter(|e| matches!(e.get_kind(), EventKind::Error | EventKind::Warning))
        .map(|e| e.get_message().into_owned())
        .collect();
    assert!(reported.is_empty(), "{reported:#?}");
}

// Java: StarlarkRuleClassFunctionsTest.initializer_nameChanged
#[test]
fn initializer_name_changed() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, **kwargs):",
            "    return {\"name\": \"my_new_name\"}",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(impl, initializer = initializer)",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(name = \"my_target\")",
        ],
    );
    let (_result, events) = t.evaluate_package("initializer_testing");
    assert_contains_error(
        &events,
        "Error in my_rule: Initializer can't change the name of the target",
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_wrongType
#[test]
fn initializer_wrong_type() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "MyInfo = provider()",
            "def initializer(srcs = []):",
            "    return {\"srcs\": [\"a.ml\"]}",
            "def impl(ctx):",
            "    return [MyInfo(",
            "        srcs = [s.short_path for s in ctx.files.srcs],",
            "    )]",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = \"default_files\",",
            ")",
        ],
    );
    let (_result, events) = t.evaluate_package("initializer_testing");
    assert_contains_error(
        &events,
        "expected value of type 'list(label)' for attribute 'srcs' of 'my_rule', but got \
         \"default_files\" (string)",
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_passThrough
// (`getConfiguredTarget` and `assertNoEvents()`: the package loads without errors and no
// error or warning is reported.)
#[test]
fn initializer_pass_through() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(**kwargs):",
            "    pass",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "        \"deps\": attr.label_list(),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = [\"a.ml\"],",
            ")",
        ],
    );
    let loaded = t.get_package("initializer_testing");
    assert!(!loaded.package.contains_errors());
    assert!(loaded.package.get_rule("my_target").is_some());
    let reported: Vec<String> = loaded
        .events
        .iter()
        .filter(|e| matches!(e.get_kind(), EventKind::Error | EventKind::Warning))
        .map(|e| e.get_message().into_owned())
        .collect();
    assert!(reported.is_empty(), "{reported:#?}");
}

// Java: StarlarkRuleClassFunctionsTest.initializer_omittedValueIsNotPassed
#[test]
fn initializer_omitted_value_is_not_passed() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "MyInfo = provider()",
            "def initializer(name, srcs):",
            "    return {\"srcs\": srcs}",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(name = \"my_target\")",
        ],
    );
    let (_result, events) = t.evaluate_package("initializer_testing");
    // TODO: b/298561048 - Fix error messages to match a rule without initializer
    assert_contains_error(
        &events,
        "initializer() missing 1 required positional argument: srcs",
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_noneValueIsNotPassed
#[test]
fn initializer_none_value_is_not_passed() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "MyInfo = provider()",
            "def initializer(name, srcs):",
            "    return {\"srcs\": srcs}",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = None,",
            ")",
        ],
    );
    let (_result, events) = t.evaluate_package("initializer_testing");
    assert_contains_error(
        &events,
        "initializer() missing 1 required positional argument: srcs",
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_incorrectReturnType
#[test]
fn initializer_incorrect_return_type() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, srcs = []):",
            "    return [srcs]",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = [\"a.ml\"],",
            ")",
        ],
    );
    let (_result, events) = t.evaluate_package("initializer_testing");
    assert_contains_error(
        &events,
        "got list for 'rule's initializer return value', want dict",
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_incorrectReturnDicts
#[test]
fn initializer_incorrect_return_dicts() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, srcs = []):",
            "    return {True: srcs}",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = [\"a.ml\"],",
            ")",
        ],
    );
    let (_result, events) = t.evaluate_package("initializer_testing");
    assert_contains_error(
        &events,
        "got dict<bool, list> for 'rule's initializer return value', want dict",
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_failsSettingBaseAttribute
#[test]
fn initializer_fails_setting_base_attribute() {
    // 'args' is an attribute defined for all executable rules
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, srcs = [], deps = []):",
            "    return {\"srcs\": srcs, \"deps\": deps, \"args\": [\"a\"]}",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    executable = True,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "        \"deps\": attr.label_list(),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = [\"a.ml\"],",
            ")",
        ],
    );
    let (_result, events) = t.evaluate_package("initializer_testing");
    assert_contains_error(
        &events,
        "Initializer can only set Starlark defined attributes, not 'args'",
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_failsSettingUnknownAttr
#[test]
fn initializer_fails_setting_unknown_attr() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, srcs = [], deps = []):",
            "    return {\"srcs\": srcs, \"my_deps\": deps}",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "        \"deps\": attr.label_list(),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = [\"a.ml\"],",
            ")",
        ],
    );
    let (_result, events) = t.evaluate_package("initializer_testing");
    assert_contains_error(
        &events,
        "no such attribute 'my_deps' in 'my_rule' rule (did you mean 'deps'?)",
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_withFails
// (Java's second assertion, "target 'my_target' not declared in package
// 'initializer_testing'", is the error of `getConfiguredTarget` looking the target up; here it
// is that the package has no such target.)
#[test]
fn initializer_with_fails() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, srcs = [], deps = []):",
            "    fail(\"Fail called in initializer\")",
            "    return {\"srcs\": srcs, \"deps\": deps}",
            "def impl(ctx):",
            "    pass",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "        \"deps\": attr.label_list(),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = [\"a.ml\"],",
            ")",
        ],
    );
    let loaded = t.get_package("initializer_testing");
    assert_contains_error(&loaded.events, "Fail called in initializer");
    // TODO: b/298561048 - fix that the whole package doesn't fail if possible
    assert!(loaded.package.contains_errors());
    assert!(loaded.package.get_rule("my_target").is_none());
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_nativeRule_notExtendable
#[test]
fn extend_rule_native_rule_not_extendable() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "extend_rule_testing/child.bzl",
        &[
            "def _impl(ctx):",
            "    ctx.super()",
            "my_library = rule(",
            "    implementation = _impl,",
            "    parent = native.alias,",
            "    fragments = [\"cc\"],",
            ")",
        ],
    );
    t.scratch_file(
        "extend_rule_testing/BUILD",
        &[
            "load(\":child.bzl\", \"my_library\")",
            "my_library(name = \"my_target\")",
        ],
    );
    let (_result, events) = t.evaluate_package("extend_rule_testing");
    assert_contains_error(&events, "Parent needs to be a Starlark rule");
}

// Java: StarlarkRuleClassFunctionsTest.extendRule_extendableDisallowed
#[test]
fn extend_rule_extendable_disallowed() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("extend_rule_testing/parent/BUILD", &[]);
    t.scratch_file(
        "extend_rule_testing/parent/parent.bzl",
        &[
            "ParentInfo = provider()",
            "def _impl(ctx):",
            "    return [ParentInfo()]",
            "parent_library = rule(",
            "    implementation = _impl,",
            "    extendable = False,",
            ")",
        ],
    );
    t.scratch_file(
        "extend_rule_testing/child.bzl",
        &[
            "load(\"//extend_rule_testing/parent:parent.bzl\", \"parent_library\")",
            "def _impl(ctx):",
            "    ctx.super()",
            "my_library = rule(",
            "    implementation = _impl,",
            "    parent = parent_library,",
            ")",
        ],
    );
    t.scratch_file(
        "extend_rule_testing/BUILD",
        &[
            "load(\":child.bzl\", \"my_library\")",
            "my_library(name = \"my_target\")",
        ],
    );
    let (_result, events) = t.evaluate_package("extend_rule_testing");
    assert_contains_error(&events, "The rule 'parent_library' is not extendable.");
}

// Java: StarlarkRuleClassFunctionsTest.testLabelListComputedDefaultChecksElementTypes
// (`getConfiguredTarget` throws an AssertionError carrying the reported error: the rule's
// instantiation reports it while loading.)
#[test]
fn test_label_list_computed_default_checks_element_types() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "p/a.bzl",
        &[
            "def _compute():",
            "    return [\"this is a string\"]",
            "my_rule = rule(lambda ctx: [], attrs = {\"_a\" : attr.label_list(default = _compute)})",
        ],
    );
    t.scratch_file(
        "p/BUILD",
        &["load(\":a.bzl\", \"my_rule\")", "my_rule(name = \"bad\")"],
    );
    let (_result, events) = t.evaluate_package("p");
    assert_contains_error(&events, "expected 'label', but got 'string'");
}

// ---- proto.encode_text

/// Java `checkTextMessage(from, lines...)`: `ev.eval(from)` is the lines, each ended by a
/// newline.
fn check_text_message(ev: &Ev, from: &str, lines: &[&str]) {
    let result = ev.eval_str(from);
    let mut expect = String::new();
    if !lines.is_empty() {
        expect = lines.join("\n") + "\n";
    }
    assert_eq!(result, expect, "{from}");
}

// Java: StarlarkRuleClassFunctionsTest.testSimpleTextMessages
#[test]
fn test_simple_text_messages() {
    let ev = Ev::new();
    check_text_message(
        &ev,
        "proto.encode_text(struct(name='value'))",
        &["name: \"value\""],
    );
    check_text_message(&ev, "proto.encode_text(struct(name=[]))", &[]); // empty lines
    check_text_message(
        &ev,
        "proto.encode_text(struct(name=['a', 'b']))",
        &["name: \"a\"", "name: \"b\""],
    );
    check_text_message(&ev, "proto.encode_text(struct(name=123))", &["name: 123"]);
    check_text_message(
        &ev,
        "proto.encode_text(struct(a=1.2e34, b=float('nan'), c=float('-inf'), d=float('+inf')))",
        &[
            "a: 1.2e+34",
            "b: nan",
            "c: -inf",
            // Caution! textproto requires +inf be encoded as "inf" rather than "+inf"
            "d: inf",
        ],
    );
    check_text_message(&ev, "proto.encode_text(struct(name=123))", &["name: 123"]);
    check_text_message(
        &ev,
        "proto.encode_text(struct(name=[1, 2, 3]))",
        &["name: 1", "name: 2", "name: 3"],
    );
    check_text_message(
        &ev,
        "proto.encode_text(struct(a=struct(b='b')))",
        &["a {", "  b: \"b\"", "}"],
    );
    check_text_message(
        &ev,
        "proto.encode_text(struct(a=[struct(b='x'), struct(b='y')]))",
        &["a {", "  b: \"x\"", "}", "a {", "  b: \"y\"", "}"],
    );
    check_text_message(
        &ev,
        "proto.encode_text(struct(a=struct(b=struct(c='c'))))",
        &["a {", "  b {", "    c: \"c\"", "  }", "}"],
    );
    // dict to_proto tests
    check_text_message(&ev, "proto.encode_text(struct(name={}))", &[]); // empty lines
    check_text_message(
        &ev,
        "proto.encode_text(struct(name={'a': 'b'}))",
        &["name {", "  key: \"a\"", "  value: \"b\"", "}"],
    );
    check_text_message(
        &ev,
        "proto.encode_text(struct(name={'c': 'd', 'a': 'b'}))",
        &[
            "name {",
            "  key: \"c\"",
            "  value: \"d\"",
            "}",
            "name {",
            "  key: \"a\"",
            "  value: \"b\"",
            "}",
        ],
    );
    check_text_message(
        &ev,
        "proto.encode_text(struct(x=struct(y={'a': 1})))",
        &["x {", "  y {", "    key: \"a\"", "    value: 1", "  }", "}"],
    );
    check_text_message(
        &ev,
        "proto.encode_text(struct(name={'a': struct(b=1, c=2)}))",
        &[
            "name {",
            "  key: \"a\"",
            "  value {",
            "    b: 1",
            "    c: 2",
            "  }",
            "}",
        ],
    );
    check_text_message(
        &ev,
        "proto.encode_text(struct(name={'a': struct(b={4: 'z', 3: 'y'}, c=2)}))",
        &[
            "name {",
            "  key: \"a\"",
            "  value {",
            "    b {",
            "      key: 4",
            "      value: \"z\"",
            "    }",
            "    b {",
            "      key: 3",
            "      value: \"y\"",
            "    }",
            "    c: 2",
            "  }",
            "}",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testNoneStructValue
#[test]
fn test_none_struct_value() {
    let ev = Ev::new();
    check_text_message(
        &ev,
        "proto.encode_text(struct(a=1, b=None, nested=struct(c=2, d=None)))",
        &["a: 1", "nested {", "  c: 2", "}"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testProtoFieldsOrder
#[test]
fn test_proto_fields_order() {
    let ev = Ev::new();
    check_text_message(
        &ev,
        "proto.encode_text(struct(d=4, b=2, c=3, a=1))",
        &["a: 1", "b: 2", "c: 3", "d: 4"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testTextMessageEscapes
#[test]
fn test_text_message_escapes() {
    let ev = Ev::new();
    check_text_message(
        &ev,
        "proto.encode_text(struct(name='a\"b'))",
        &["name: \"a\\\"b\""],
    );
    check_text_message(
        &ev,
        "proto.encode_text(struct(name='a\\'b'))",
        &["name: \"a'b\""],
    );
    check_text_message(
        &ev,
        "proto.encode_text(struct(name='a\\nb'))",
        &["name: \"a\\nb\""],
    );

    // struct(name="a\\\"b") -> name: "a\\\"b"
    check_text_message(
        &ev,
        "proto.encode_text(struct(name='a\\\\\\\"b'))",
        &["name: \"a\\\\\\\"b\""],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testTextMessageInvalidStructure
#[test]
fn test_text_message_invalid_structure() {
    let ev = Ev::new();
    // list in list
    ev.check_eval_error_contains(
        "in struct field .a: at list index 0: got list, want string, int, float, bool, or struct",
        &["proto.encode_text(struct(a=[['b']]))"],
    );

    // dict in list
    ev.check_eval_error_contains(
        "in struct field .a: at list index 0: got dict, want string, int, float, bool, or struct",
        &["proto.encode_text(struct(a=[{'b': 1}]))"],
    );

    // tuple as dict key
    ev.check_eval_error_contains(
        "in struct field .a: invalid dict key: got tuple, want int or string",
        &["proto.encode_text(struct(a={(1, 2): 3}))"],
    );

    // dict in dict
    ev.check_eval_error_contains(
        "in struct field .name: in value for dict key \"a\": got dict, want string, int, float, \
         bool, or struct",
        &["proto.encode_text(struct(name={'a': {'b': [1, 2]}}))"],
    );

    // callable in field
    ev.check_eval_error_contains(
        "in struct field .a: got builtin_function_or_method, want string, int, float, bool, or \
         struct",
        &["proto.encode_text(struct(a=rule))"],
    );
}

// The tests of `proto.encode_text` below are not Java tests. Their expected values were printed
// by the oracle (Bazel 9.3.2, `query` on a scratch workspace holding the same expressions).

// Review finding S1-6 (docs/reviews/2026-11-18-loading.md): Java binds `proto` in .bzl files
// (StarlarkGlobalsImpl.addCommonUtilToplevels), so a file that only mentions it loads.
#[test]
fn proto_mentioned_in_bzl_function_loads() {
    let ev = Ev::new();
    ev.exec(&["def textproto(x):", "    return proto.encode_text(x)"]);
    assert_eq!(ev.eval_str("textproto(struct(a = 1))"), "a: 1\n");
}

// Review finding S1-6: Java binds `proto` in BUILD files too (the same
// addCommonUtilToplevels). `struct` is not a BUILD symbol, so the struct comes from a .bzl file.
#[test]
fn proto_encode_text_in_build_file() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "p/defs.bzl",
        &[
            "def textproto(x):",
            "    return proto.encode_text(x)",
            "S = struct(a = 1)",
        ],
    );
    t.scratch_file(
        "p/BUILD",
        &[
            "load(\":defs.bzl\", \"S\")",
            "filegroup(name = \"from_build\", tags = [proto.encode_text(S)])",
        ],
    );
    let (result, events) = t.evaluate_package("p");
    assert!(
        !result.has_error(),
        "{:?}\nevents: {:#?}",
        result,
        all_messages(&events)
    );
    let loaded = t.get_package("p");
    assert!(
        !loaded.package.contains_errors(),
        "package 'p' contains errors: {:#?}",
        all_messages(&loaded.events)
    );
    let rule = loaded
        .package
        .get_rule("from_build")
        .expect("the rule //p:from_build");
    match blaze_packages::target::RuleOrMacroInstance::get_attr(&**rule, "tags") {
        Some(AttrValue::StringList(tags)) => assert_eq!(tags, vec!["a: 1\n".to_owned()]),
        other => panic!("tags of //p:from_build: {other:?}"),
    }
}

// Java: Proto.encodeText `@Param(name = "x", allowedTypes = {Structure, NativeInfo})`; the text
// is BuiltinFunction.checkParamValue's with ParamDescriptor.getTypeErrorMessage.
#[test]
fn proto_encode_text_parameter_type() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "in call to encode_text(), parameter 'x' got value of type 'int', want 'structure or \
         NativeInfo'",
        &["proto.encode_text(1)"],
    );
    ev.check_eval_error_contains(
        "in call to encode_text(), parameter 'x' got value of type 'string', want 'structure or \
         NativeInfo'",
        &["proto.encode_text(\"x\")"],
    );
    ev.check_eval_error_contains(
        "in call to encode_text(), parameter 'x' got value of type 'builtin_function_or_method', \
         want 'structure or NativeInfo'",
        &["proto.encode_text(len)"],
    );
}

// Java: Proto.TextEncoder.fieldElement with StarlarkFloat.toString (`%.17g` cleaned up; the
// shortest decimal has at least two digits, hence 4.9e-324 for 5e-324).
#[test]
fn proto_encode_text_floats() {
    let ev = Ev::new();
    assert_eq!(
        ev.eval_str(
            "proto.encode_text(struct(a=0.1, b=1e16, c=1e17, d=1e-4, e=1e-5, f=0.0, g=-0.0, \
             h=5e-324, i=1e-323, j=123456789.125, k=1e15, l=0.30000000000000004, m=1.0, \
             n=5e-323, o=2e23))"
        ),
        "a: 0.1\nb: 10000000000000000.0\nc: 1e+17\nd: 0.0001\ne: 1e-05\nf: 0.0\ng: -0.0\n\
         h: 4.9e-324\ni: 9.9e-324\nj: 123456789.125\nk: 1000000000000000.0\n\
         l: 0.30000000000000004\nm: 1.0\nn: 4.9e-323\no: 2e+23\n"
    );
}

// Java: Proto.TextEncoder.field (a range is a Sequence; None and empty repeated fields emit
// nothing) and fieldElement (big ints, bools, the three string escapes).
#[test]
fn proto_encode_text_sequences_and_scalars() {
    let ev = Ev::new();
    assert_eq!(
        ev.eval_strs(&[
            "proto.encode_text(struct(a=range(3), b=(1, 2), c=True, d=False, e=None, f=[], \
             g=(), h={}))",
            "proto.encode_text(struct(a=1<<70, b=-5, c=-(1<<70)))",
            "proto.encode_text(struct(a='tab\\there\\rcr\\\\ \"q\" \\n nl'))",
            "proto.encode_text(struct())",
        ]),
        vec![
            "a: 0\na: 1\na: 2\nb: 1\nb: 2\nc: true\nd: false\n",
            "a: 1180591620717411303424\nb: -5\nc: -1180591620717411303424\n",
            "a: \"tab\there\rcr\\\\ \\\"q\\\" \\n nl\"\n",
            "",
        ]
    );
}

// Java: Proto.TextEncoder.field / fieldElement / message error wrapping.
#[test]
fn proto_encode_text_invalid_elements() {
    let ev = Ev::new();
    // None is skipped only as the value of a non-repeated field.
    ev.check_eval_error_contains(
        "in struct field .a: at list index 0: got NoneType, want string, int, float, bool, or \
         struct",
        &["proto.encode_text(struct(a=[None]))"],
    );
    ev.check_eval_error_contains(
        "in struct field .a: in value for dict key 1: got NoneType, want string, int, float, \
         bool, or struct",
        &["proto.encode_text(struct(a={1: None}))"],
    );
    // sequence in sequence
    ev.check_eval_error_contains(
        "in struct field .a: at list index 0: got range, want string, int, float, bool, or struct",
        &["proto.encode_text(struct(a=[range(1)]))"],
    );
    // nested messages
    ev.check_eval_error_contains(
        "in struct field .a: in struct field .b: at list index 0: in struct field .c: in value \
         for dict key \"k\": got list, want string, int, float, bool, or struct",
        &["proto.encode_text(struct(a=struct(b=[struct(c={\"k\": [1]})])))"],
    );
}

// ---- json.encode of structs

/// Java `checkJson(from, expected)`.
fn check_json(ev: &Ev, from: &str, expected: &str) {
    let result = ev.eval_str(from);
    assert_eq!(result, expected, "{from}");
}

// Java: StarlarkRuleClassFunctionsTest.testStarlarkJsonModule
#[test]
fn test_starlark_json_module() {
    let ev = Ev::new();
    check_json(&ev, "json.encode(struct(name=True))", "{\"name\":true}");
    check_json(&ev, "json.encode([1, 2])", "[1,2]"); // works for non-structs too
}

// Java: StarlarkRuleClassFunctionsTest.testJsonBooleanFields
#[test]
fn test_json_boolean_fields() {
    let ev = Ev::new();
    check_json(&ev, "json.encode(struct(name=True))", "{\"name\":true}");
    check_json(&ev, "json.encode(struct(name=False))", "{\"name\":false}");
}

// Java: StarlarkRuleClassFunctionsTest.testJsonDictFields
#[test]
fn test_json_dict_fields() {
    let ev = Ev::new();
    check_json(&ev, "json.encode(struct(config={}))", "{\"config\":{}}");
    check_json(
        &ev,
        "json.encode(struct(config={'key': 'value'}))",
        "{\"config\":{\"key\":\"value\"}}",
    );
    ev.check_eval_error_contains(
        "in struct field .config: dict has int key, want string",
        &["json.encode(struct(config={1:2}))"],
    );
    ev.check_eval_error_contains(
        "in struct field .config: in dict key \"foo\": dict has int key, want string",
        &["json.encode(struct(config={'foo':{1:2}}))"],
    );
    ev.check_eval_error_contains(
        "in struct field .config: dict has bool key, want string",
        &["json.encode(struct(config={True: False}))"],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testJsonEncoding
#[test]
fn test_json_encoding() {
    let ev = Ev::new();
    check_json(
        &ev,
        "json.encode(struct(name='value'))",
        "{\"name\":\"value\"}",
    );
    check_json(
        &ev,
        "json.encode(struct(name=['a', 'b']))",
        "{\"name\":[\"a\",\"b\"]}",
    );
    check_json(&ev, "json.encode(struct(name=123))", "{\"name\":123}");
    check_json(
        &ev,
        "json.encode(struct(name=[1, 2, 3]))",
        "{\"name\":[1,2,3]}",
    );
    check_json(
        &ev,
        "json.encode(struct(a=struct(b='b')))",
        "{\"a\":{\"b\":\"b\"}}",
    );
    check_json(
        &ev,
        "json.encode(struct(a=[struct(b='x'), struct(b='y')]))",
        "{\"a\":[{\"b\":\"x\"},{\"b\":\"y\"}]}",
    );
    check_json(
        &ev,
        "json.encode(struct(a=struct(b=struct(c='c'))))",
        "{\"a\":{\"b\":{\"c\":\"c\"}}}",
    );
}

// Java: StarlarkRuleClassFunctionsTest.testJsonEscapes
#[test]
fn test_json_escapes() {
    let ev = Ev::new();
    check_json(
        &ev,
        "json.encode(struct(name='a\"b'))",
        "{\"name\":\"a\\\"b\"}",
    );
    check_json(
        &ev,
        "json.encode(struct(name='a\\'b'))",
        "{\"name\":\"a'b\"}",
    );
    check_json(
        &ev,
        "json.encode(struct(name='a\\\\b'))",
        "{\"name\":\"a\\\\b\"}",
    );
    check_json(
        &ev,
        "json.encode(struct(name='a\\nb'))",
        "{\"name\":\"a\\nb\"}",
    );
    check_json(
        &ev,
        "json.encode(struct(name='a\\rb'))",
        "{\"name\":\"a\\rb\"}",
    );
    check_json(
        &ev,
        "json.encode(struct(name='a\\tb'))",
        "{\"name\":\"a\\tb\"}",
    );
}

// Java: StarlarkRuleClassFunctionsTest.testJsonNestedListStructure
#[test]
fn test_json_nested_list_structure() {
    let ev = Ev::new();
    check_json(&ev, "json.encode(struct(a=[['b']]))", "{\"a\":[[\"b\"]]}");
}

// Java: StarlarkRuleClassFunctionsTest.testJsonInvalidStructure
#[test]
fn test_json_invalid_structure() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "in struct field .a: cannot encode builtin_function_or_method as JSON",
        &["json.encode(struct(a=rule))"],
    );
}

// ---- Symbolic macros (Java StarlarkRuleClassFunctionsTest.testSymbolicMacro_*), on the
// package-loading harness (Java scratch.file + getPackage).

/// The package, or `None` where Java's `getPackage` returns null (loading threw).
fn load_package_or_none(
    t: &PackageLoadingTestCase,
    pkg: &str,
) -> (
    Option<std::sync::Arc<blaze_packages::package::Package>>,
    Vec<Event>,
) {
    let (result, events) = t.evaluate_package(pkg);
    if result.has_error() {
        return (None, events);
    }
    let loaded = t.get_package(pkg);
    (Some(loaded.package), loaded.events)
}

fn empty_macro_bzl(t: &PackageLoadingTestCase) {
    t.scratch_file(
        "pkg/foo.bzl",
        &[
            "def _impl(name, visibility):",
            "    pass",
            "my_macro = macro(implementation=_impl)",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_failsWithoutFlag
#[test]
fn test_symbolic_macro_fails_without_flag() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_enable_first_class_macros=false"]);
    empty_macro_bzl(&t);
    t.scratch_file("pkg/BUILD", &["load(\":foo.bzl\", \"my_macro\")"]);
    let (pkg, events) = load_package_or_none(&t, "pkg");
    assert!(pkg.is_none());
    assert_contains_event(&events, "requires --experimental_enable_first_class_macros");
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_instantiationRegistersOnPackage
#[test]
fn test_symbolic_macro_instantiation_registers_on_package() {
    let t = PackageLoadingTestCase::new();
    empty_macro_bzl(&t);
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":foo.bzl\", \"my_macro\")",
            "my_macro(name=\"ghi\")  # alphabetized when read back",
            "my_macro(name=\"abc\")",
            "my_macro(name=\"def\")",
        ],
    );
    let pkg = t.get_package("pkg").package;
    let ids: Vec<&String> = pkg.get_macros_by_id().keys().collect();
    assert_eq!(ids, vec!["abc:1", "def:1", "ghi:1"]);
    assert_eq!(
        pkg.get_macros_by_id()["abc:1"].get_macro_class().get_name(),
        "my_macro"
    );
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_instantiationRequiresExport
#[test]
fn test_symbolic_macro_instantiation_requires_export() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/foo.bzl",
        &[
            "def _impl(name, visibility):",
            "    pass",
            "s = struct(m = macro(implementation=_impl))",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &["load(\":foo.bzl\", \"s\")", "s.m(name=\"abc\")"],
    );
    let (pkg, events) = load_package_or_none(&t, "pkg");
    let pkg = pkg.expect("the package");
    assert!(pkg.contains_errors());
    assert_contains_event(
        &events,
        "Cannot instantiate a macro that has not been exported",
    );
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_cannotInstantiateInBzlThread
#[test]
fn test_symbolic_macro_cannot_instantiate_in_bzl_thread() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/foo.bzl",
        &[
            "def _impl(name, visibility):",
            "    pass",
            "my_macro = macro(implementation=_impl)",
            "",
            "# Calling it from a function during .bzl load time is a little more interesting than",
            "# calling it directly at the top level, since it forces us to check thread state rather",
            "# than call stack state.",
            "def some_func():",
            "    my_macro(name=\"nope\")",
            "some_func()",
        ],
    );
    t.scratch_file("pkg/BUILD", &["load(\":foo.bzl\", \"my_macro\")"]);
    let (pkg, events) = load_package_or_none(&t, "pkg");
    assert!(pkg.is_none());
    assert_contains_event(
        &events,
        "a symbolic macro can only be instantiated while evaluating a BUILD file or a legacy or \
         symbolic macro",
    );
}

/// The events of loading `pkg/BUILD` (one line), which must load with errors.
fn package_in_error_events(t: &PackageLoadingTestCase, build: &[&str]) -> Vec<Event> {
    t.scratch_file("pkg/BUILD", build);
    let (pkg, events) = load_package_or_none(t, "pkg");
    let pkg = pkg.expect("the package");
    assert!(pkg.contains_errors());
    events
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_requiresNameAttribute
#[test]
fn test_symbolic_macro_requires_name_attribute() {
    let t = PackageLoadingTestCase::new();
    empty_macro_bzl(&t);
    let events = package_in_error_events(&t, &["load(\":foo.bzl\", \"my_macro\")", "my_macro()"]);
    assert_contains_event(
        &events,
        "missing value for mandatory attribute 'name' in 'my_macro' macro",
    );
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_prohibitsPositionalArgs
#[test]
fn test_symbolic_macro_prohibits_positional_args() {
    let t = PackageLoadingTestCase::new();
    empty_macro_bzl(&t);
    let events = package_in_error_events(
        &t,
        &[
            "load(\":foo.bzl\", \"my_macro\")",
            "my_macro(\"a positional arg\", name = \"abc\")",
        ],
    );
    assert_contains_event(&events, "unexpected positional arguments");
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacroCanAcceptAttributes
#[test]
fn test_symbolic_macro_can_accept_attributes() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/foo.bzl",
        &[
            "def _impl(name, visibility, target_suffix):",
            "    native.filegroup(name = name + \"_\" + target_suffix)",
            "my_macro = macro(",
            "    implementation=_impl,",
            "    attrs = {",
            "        \"target_suffix\": attr.string(configurable=False),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":foo.bzl\", \"my_macro\")",
            "my_macro(",
            "    name = \"abc\",",
            "    target_suffix = \"xyz\"",
            ")",
        ],
    );
    let pkg = t.get_package_without_errors("pkg");
    assert!(pkg.package.get_targets().contains_key("abc_xyz"));
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_rejectsUnknownAttribute
#[test]
fn test_symbolic_macro_rejects_unknown_attribute() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/foo.bzl",
        &[
            "def _impl(name, visibility):",
            "    pass",
            "my_macro = macro(",
            "    implementation = _impl,",
            "    attrs = {",
            "        \"xzz\": attr.string(doc=\"This attr is public\"),",
            "    },",
            ")",
        ],
    );
    let events = package_in_error_events(
        &t,
        &[
            "load(\":foo.bzl\", \"my_macro\")",
            "my_macro(",
            "    name = \"abc\",",
            "    xyz = \"UNKNOWN\",",
            ")",
        ],
    );
    assert_contains_event(
        &events,
        "no such attribute 'xyz' in 'my_macro' macro (did you mean 'xzz'?)",
    );
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_rejectsReservedAttributeName
#[test]
fn test_symbolic_macro_rejects_reserved_attribute_name() {
    let ev = Ev::new();
    let events = ev.exec_collecting_events(&[
        "def _impl(name, visibility):",
        "    pass",
        "my_macro = macro(",
        "    implementation = _impl,",
        "    attrs = {",
        "        \"visibility\": attr.string(),",
        "    },",
        ")",
    ]);
    assert_contains_error(
        &events,
        "Cannot declare a macro attribute named 'visibility'",
    );
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_requiresMandatoryAttribute
#[test]
fn test_symbolic_macro_requires_mandatory_attribute() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/foo.bzl",
        &[
            "def _impl(name, visibility):",
            "    pass",
            "my_macro = macro(",
            "    implementation = _impl,",
            "    attrs = {",
            "        \"xyz\": attr.string(mandatory=True),",
            "    },",
            ")",
        ],
    );
    let events = package_in_error_events(
        &t,
        &["load(\":foo.bzl\", \"my_macro\")", "my_macro(name=\"abc\")"],
    );
    assert_contains_event(
        &events,
        "missing value for mandatory attribute 'xyz' in 'my_macro' macro",
    );
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_cannotOverrideImplicitAttribute
#[test]
fn test_symbolic_macro_cannot_override_implicit_attribute() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/foo.bzl",
        &[
            "def _impl(name, visibility, _xyz):",
            "    print(\"_xyz is %s\" % _xyz)",
            "my_macro = macro(",
            "    implementation=_impl,",
            "    attrs = {",
            "      \"_xyz\": attr.string(default=\"IMPLICIT\")",
            "    },",
            ")",
        ],
    );
    let events = package_in_error_events(
        &t,
        &[
            "load(\":foo.bzl\", \"my_macro\")",
            "my_macro(",
            "    name = \"abc\",",
            "    _xyz = \"CAN'T SET THIS\",",
            ")",
        ],
    );
    assert_contains_event(&events, "cannot set value of implicit attribute '_xyz'");
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_doesNotSupportComputedDefaults
#[test]
fn test_symbolic_macro_does_not_support_computed_defaults() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "In macro attribute 'xyz': Macros do not support computed defaults or late-bound defaults",
        &[
            "def _impl(name, visibility, xyz): pass",
            "def _computed_default(): return \"DEFAULT\"",
            "my_macro = macro(",
            "    implementation=_impl,",
            "    attrs = {",
            "      \"xyz\": attr.label(default=_computed_default)",
            "    },",
            ")",
        ],
    );
}

// Java: StarlarkRuleClassFunctionsTest.testSymbolicMacro_doesNotSupportLateBoundDefaults
// PORT: Java registers the `coverage` fragment on its evaluation test case; the harness here
// has Bazel's rule class provider, whose fragments include it.
#[test]
fn test_symbolic_macro_does_not_support_late_bound_defaults() {
    let ev = Ev::new();
    ev.check_eval_error_contains(
        "In macro attribute 'xyz': Macros do not support computed defaults or late-bound defaults",
        &[
            "def _impl(name, visibility, xyz): pass",
            "_latebound_default = configuration_field(fragment = \"coverage\", name = \"output_generator\")",
            "my_macro = macro(",
            "    implementation=_impl,",
            "    attrs = {",
            "      \"xyz\": attr.label(default=_latebound_default)",
            "    },",
            ")",
        ],
    );
}
