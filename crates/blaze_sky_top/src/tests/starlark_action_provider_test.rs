//! Port of `src/test/java/com/google/devtools/build/lib/starlark/StarlarkActionProviderTest.java`
//! (an `AnalysisTestCase`): the Starlark `Action` values `target.actions` gives an aspect
//! (`RuleConfiguredTarget.rawGetStarlarkProvider("actions")`, Java `ActionApi`).

use blaze_analysis::starlark::starlark_action::StarlarkAction;
use blaze_packages::rule_class::ProviderKey;
use blaze_starlark::structs::StarlarkInfo;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::{OwnedFrozenValue, Value};

use crate::testing::build_view_test_case::parse_label;
use crate::testing::{AnalysisTestCase, BuildViewTestCase, assert_contains_exactly};

fn new_test() -> AnalysisTestCase {
    BuildViewTestCase::analysis_test_case()
}

/// `new StarlarkProvider.Key(keyForBuild(Label.parseCanonical(bzl)), name)`.
fn starlark_provider_key(bzl: &str, name: &str) -> ProviderKey {
    ProviderKey::Starlark {
        extension_label: parse_label(bzl),
        exported_name: name.to_string(),
    }
}

/// `Iterables.getOnlyElement(analysisResult.getAspectsMap().values()).get(fooKey)`.
fn only_aspect_foo_provider(t: &mut AnalysisTestCase, aspect: &str) -> OwnedFrozenValue {
    let result = t
        .update_with_aspects(&[aspect], &["//test:xxx"])
        .unwrap_or_else(|e| panic!("update: {e:?}"));
    let aspects = result.get_aspects_map();
    assert_eq!(aspects.len(), 1, "expected one configured aspect");
    let configured_aspect = aspects.values().next().unwrap().get_configured_object();
    configured_aspect
        .get(&starlark_provider_key("//test:aspect.bzl", "foo"))
        .expect("NullPointerException: no foo provider")
}

/// `((StructImpl) info).getValue(name)`.
fn field<'v>(info: &'v OwnedFrozenValue, name: &str) -> Value<'v> {
    let v = info.value();
    StarlarkInfo::from_value(v)
        .unwrap_or_else(|| panic!("ClassCastException: {v} is not a StructImpl"))
        .get_field(name)
        .unwrap_or_else(|| panic!("no field '{name}' in {v}"))
}

/// `(Sequence<?>) value`.
fn sequence(v: Value<'_>) -> Vec<Value<'_>> {
    ListRef::from_value(v)
        .unwrap_or_else(|| panic!("ClassCastException: {v} is not a Sequence"))
        .iter()
        .collect()
}

/// A `Dict<String, String>` as (key, value) pairs in order.
fn string_dict(v: Value<'_>) -> Vec<(String, String)> {
    DictRef::from_value(v)
        .unwrap_or_else(|| panic!("ClassCastException: {v} is not a Dict"))
        .iter()
        .map(|(k, v)| {
            (
                k.unpack_str().unwrap().into(),
                v.unpack_str().unwrap().into(),
            )
        })
        .collect()
}

/// `flattenArtifactNames`: the file names of the artifacts of each list.
fn flatten_artifact_names(v: Value<'_>) -> Vec<String> {
    sequence(v)
        .into_iter()
        .flat_map(sequence)
        .map(|a| {
            let artifact = blaze_actions::analysis::starlark::starlark_file::artifact_from_value(a)
                .unwrap_or_else(|| panic!("ClassCastException: {a} is not an Artifact"));
            artifact.get_filename().to_string()
        })
        .collect()
}

// Java: StarlarkActionProviderTest.aspectGetsActionProviderForNativeRule
#[test]
fn aspect_gets_action_provider_for_native_rule() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"foo = provider()

def _impl(target, ctx):
    return [foo(actions = target.actions)]

MyAspect = aspect(implementation = _impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "xxx",
    outs = ["mygen.out"],
    cmd = 'echo "hello" > $@',
)
"#],
    );

    let foo_provider = only_aspect_foo_provider(&mut t, "test/aspect.bzl%MyAspect");
    let actions = sequence(field(&foo_provider, "actions"));
    assert!(!actions.is_empty());

    let action = StarlarkAction::action_of(actions[0])
        .unwrap_or_else(|| panic!("not an AbstractAction: {}", actions[0]));
    assert_eq!(action.get_mnemonic(), "Genrule");
}

// Java: StarlarkActionProviderTest.aspectGetsActionProviderForStarlarkRule
#[test]
fn aspect_gets_action_provider_for_starlark_rule() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"foo = provider()

def _impl(target, ctx):
    mnemonics = [a.mnemonic for a in target.actions]
    envs = [a.env for a in target.actions]
    execution_info = [a.execution_info for a in target.actions]
    inputs = [a.inputs.to_list() for a in target.actions]
    outputs = [a.outputs.to_list() for a in target.actions]
    argv = [a.argv for a in target.actions]
    return [foo(
        actions = target.actions,
        mnemonics = mnemonics,
        envs = envs,
        execution_info = execution_info,
        inputs = inputs,
        outputs = outputs,
        argv = argv,
    )]

MyAspect = aspect(implementation = _impl)
"#],
    );
    t.scratch.file(
        "test/rule.bzl",
        &[r#"def impl(ctx):
    output_file0 = ctx.actions.declare_file("myfile0")
    output_file1 = ctx.actions.declare_file("myfile1")
    executable = ctx.actions.declare_file("executable")
    ctx.actions.run(
        outputs = [output_file0],
        executable = executable,
        toolchain = None,
        mnemonic = "MyAction0",
        env = {"foo": "bar", "pet": "puppy"},
    )
    ctx.actions.run_shell(
        outputs = [executable, output_file1],
        command = "fakecmd",
        mnemonic = "MyAction1",
        env = {"pet": "bunny"},
    )
    return None

my_rule = rule(impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:rule.bzl", "my_rule")

my_rule(
    name = "xxx",
)
"#],
    );

    t.use_configuration(&["--experimental_google_legacy_api"]);
    let foo_provider = only_aspect_foo_provider(&mut t, "test/aspect.bzl%MyAspect");

    let actions = sequence(field(&foo_provider, "actions"));
    assert_eq!(actions.len(), 2);

    let mnemonics: Vec<String> = sequence(field(&foo_provider, "mnemonics"))
        .into_iter()
        .map(|m| m.unpack_str().unwrap().to_string())
        .collect();
    assert_contains_exactly(&mnemonics, &["MyAction0", "MyAction1"]);

    let envs: Vec<Vec<(String, String)>> = sequence(field(&foo_provider, "envs"))
        .into_iter()
        .map(string_dict)
        .collect();
    let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
    // containsExactly (any order) of the two dicts.
    let mut envs = envs;
    envs.sort();
    let mut expected = vec![
        vec![pair("foo", "bar"), pair("pet", "puppy")],
        vec![pair("pet", "bunny")],
    ];
    expected.sort();
    assert_eq!(envs, expected);

    // assertThat(executionInfo).isNotNull()
    let _execution_info = sequence(field(&foo_provider, "execution_info"));

    assert_contains_exactly(
        &flatten_artifact_names(field(&foo_provider, "inputs")),
        &["executable"],
    );
    assert_contains_exactly(
        &flatten_artifact_names(field(&foo_provider, "outputs")),
        &["myfile0", "executable", "myfile1"],
    );

    let argv = sequence(field(&foo_provider, "argv"));
    let argv0: Vec<String> = sequence(argv[0])
        .into_iter()
        .map(|s| s.unpack_str().unwrap().to_string())
        .collect();
    assert_eq!(argv0.len(), 1);
    assert!(argv0[0].ends_with("executable"), "{argv0:?}");
    let argv1: Vec<String> = sequence(argv[1])
        .into_iter()
        .map(|s| s.unpack_str().unwrap().to_string())
        .collect();
    assert!(argv1.contains(&"fakecmd".to_string()), "{argv1:?}");
}
