//! Port of `analysis/actions/TemplateExpansionActionTest.java`, plus action keys and aquery
//! template contents checked against the oracle.

#![allow(dead_code)]

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use blaze_actions::ArtifactOwner;
use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::analysis::actions::substitution::Substitution;
use blaze_actions::analysis::actions::template::Template;
use blaze_actions::analysis::actions::template_expansion_action::{
    TemplateExpansionAction, get_template_content,
};
use blaze_actions::{
    Action, ActionKeyContext, ActionLookupKeyRef, ActionOwner, Artifact, ArtifactRoot,
    PlatformInfoRef, RootType,
};
use blaze_cmdline::Label;
use blaze_events::location::Location;
use blaze_util::Fingerprint;
use blaze_vfs::Root;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

mod oracle_common;

// ---------------------------------------------------------------------------------------------
// Test utilities (ActionsTestUtil / BuildViewTestCase / ActionTester equivalents)
// ---------------------------------------------------------------------------------------------

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NullArtifactOwner;

impl fmt::Display for NullArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NULL_ARTIFACT_OWNER")
    }
}

impl ActionLookupKey for NullArtifactOwner {
    fn get_label(&self) -> Option<Label> {
        Some(Label::parse_canonical("//null/action:owner").unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "NULL_ARTIFACT_OWNER"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<NullArtifactOwner>()
    }
    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn owner_with_platform(platform: PlatformInfoRef) -> ActionOwner {
    ActionOwner::create_dummy(
        Some(Label::parse_canonical("//null/action:owner").unwrap()),
        Location::new("dummy-file", 0, 0),
        "dummy-kind",
        "dummy-configuration-mnemonic",
        "dummy-configuration",
        None,
        false,
        Some(platform),
        Vec::new(),
        IndexMap::new(),
    )
}

/// `ActionsTestUtil.NULL_ACTION_OWNER`.
fn null_action_owner() -> ActionOwner {
    owner_with_platform(empty_platform_info())
}

/// `getBinArtifactWithNoOwner(rootRelativePath)`.
fn get_bin_artifact_with_no_owner(root_relative_path: &str) -> Artifact {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/execroot/_main"), &fs);
    let root = ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-fastbuild", "bin"],
    );
    let exec_path = root.get_exec_path().get_relative_str(root_relative_path);
    Artifact::create_derived_artifact(root, exec_path, ActionLookupKeyRef::new(NullArtifactOwner))
}

/// `Actions.canBeShared`.
fn can_be_shared(ctx: &ActionKeyContext, a: &dyn Action, b: &dyn Action) -> bool {
    if !(a.get_mnemonic() == b.get_mnemonic()
        && a.get_key(ctx, None).unwrap() == b.get_key(ctx, None).unwrap())
    {
        return false;
    }
    // Java: com/google/devtools/build/lib/actions/Actions.java:145 Actions.artifactsEqualWithoutOwner
    fn equal_without_owner(x: &[Artifact], y: &[Artifact]) -> bool {
        x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.equals_without_owner(q))
    }
    if a.is_shareable() && b.is_shareable() {
        return equal_without_owner(
            &a.get_mandatory_inputs().to_list()[..],
            &b.get_mandatory_inputs().to_list()[..],
        ) && equal_without_owner(&a.get_outputs()[..], &b.get_outputs()[..]);
    }
    if a.is_shareable() || b.is_shareable() {
        return false;
    }
    *a.get_mandatory_inputs().to_list() == *b.get_mandatory_inputs().to_list()
        && a.get_outputs() == b.get_outputs()
}

/// `ActionTester.runTest(attributeClass, factory, actionKeyContext)` for `attribute_count`
/// attributes; the factory gets the flipped attributes as a bit set.
fn action_tester_run_test(
    attribute_count: usize,
    factory: impl Fn(&[bool]) -> Box<dyn Action>,
    ctx: &ActionKeyContext,
) {
    let flags =
        |seed: usize| -> Vec<bool> { (0..attribute_count).map(|i| seed & (1 << i) != 0).collect() };
    let count = 1usize << attribute_count;
    let mut actions = Vec::new();
    for i in 0..count {
        let action = factory(&flags(i));
        // Check that creating the same action twice results in equal actions.
        assert!(can_be_shared(ctx, &*action, &*factory(&flags(i))));
        actions.push(action);
    }
    // Check that the count is correct.
    assert!(can_be_shared(ctx, &*actions[0], &*factory(&flags(count))));
    for i in 0..actions.len() {
        for j in i + 1..actions.len() {
            assert!(
                !can_be_shared(ctx, &*actions[i], &*actions[j]),
                "{} and {}",
                i,
                j
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// TemplateExpansionActionTest
// ---------------------------------------------------------------------------------------------

const TEMPLATE: &str = "key=%key%\nvalue=%value%";
const SPECIAL_CHARS: &str = "Š©±½_strøget";

/// The fixture of `createDirectoriesAndTools` / `createArtifacts`: a scratch workspace with
/// `input.txt` (the template plus the newline `scratch.overwriteFile` appends) as a source
/// artifact and `out/destination.txt` as the output.
struct Fixture {
    output_root: ArtifactRoot,
    input_artifact: Artifact,
    output_artifact: Artifact,
    substitutions: Vec<Substitution>,
    _dir: TempDir,
}

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(name: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!(
        "blaze_actions_template_test_{}_{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

// Java: TemplateExpansionActionTest.createArtifacts
fn create_artifacts(name: &str, template: &str) -> Fixture {
    let dir = temp_dir(name);
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let workspace_path = Path::create(PathFragment::create(dir.0.to_str().unwrap()), &fs);
    let workspace = ArtifactRoot::as_source_root(Root::from_path(workspace_path.clone()));
    std::fs::create_dir_all(dir.0.join("out")).unwrap();
    let output_root = ArtifactRoot::as_derived_root(&workspace_path, RootType::OUTPUT, &["out"]);
    // scratch.overwriteFile appends a newline.
    std::fs::write(dir.0.join("input.txt"), format!("{}\n", template)).unwrap();
    let input_artifact = Artifact::new_source_artifact(
        workspace,
        PathFragment::create("input.txt"),
        ArtifactOwner::NullOwner,
    );
    let output_artifact = Artifact::create_derived_artifact(
        output_root.clone(),
        PathFragment::create("out/destination.txt"),
        ActionLookupKeyRef::new(NullArtifactOwner),
    );
    Fixture {
        output_root,
        input_artifact,
        output_artifact,
        substitutions: vec![
            Substitution::of("%key%", "foo"),
            Substitution::of("%value%", "bar"),
        ],
        _dir: dir,
    }
}

// Java: TemplateExpansionActionTest.createDirectoriesAndTools
fn setup(name: &str) -> Fixture {
    create_artifacts(name, TEMPLATE)
}

// Java: TemplateExpansionActionTest.create
fn create(f: &Fixture) -> TemplateExpansionAction {
    TemplateExpansionAction::new(
        null_action_owner(),
        f.output_artifact.clone(),
        Template::for_string(TEMPLATE),
        f.substitutions.clone(),
        false,
    )
}

// Java: TemplateExpansionActionTest.createWithArtifact
fn create_with_artifact(f: &Fixture, substitutions: Vec<Substitution>) -> TemplateExpansionAction {
    TemplateExpansionAction::for_template_artifact(
        null_action_owner(),
        f.input_artifact.clone(),
        f.output_artifact.clone(),
        substitutions,
        false,
    )
}

// Java: TemplateExpansionActionTest.computeKey
fn compute_key(action: &TemplateExpansionAction) -> String {
    let mut fp = Fingerprint::new();
    action
        .compute_key(&ActionKeyContext::new(), None, &mut fp)
        .unwrap();
    fp.hex_digest_and_reset()
}

fn output_artifact_2(f: &Fixture) -> Artifact {
    Artifact::create_derived_artifact(
        f.output_root.clone(),
        PathFragment::create("out/destination.txt"),
        ActionLookupKeyRef::new(NullArtifactOwner),
    )
}

fn key_pair(f: &Fixture, template_b: &str, value_b: &str, executable_b: bool) -> (String, String) {
    let a = TemplateExpansionAction::new(
        null_action_owner(),
        f.output_artifact.clone(),
        Template::for_string(TEMPLATE),
        vec![Substitution::of("%key%", "foo")],
        false,
    );
    let b = TemplateExpansionAction::new(
        null_action_owner(),
        output_artifact_2(f),
        Template::for_string(template_b),
        vec![Substitution::of("%key%", value_b)],
        executable_b,
    );
    (compute_key(&a), compute_key(&b))
}

#[test]
fn test_inputs_is_empty() {
    let f = setup("inputs_is_empty");
    assert!(create(&f).get_inputs().to_list().is_empty());
}

#[test]
fn test_destination_artifact_is_output() {
    let f = setup("destination_artifact_is_output");
    assert_eq!(create(&f).get_outputs(), vec![f.output_artifact.clone()]);
}

// PORT: `execute` is wave W6; `getFileContents` runs the same
// `LocalTemplateExpansionStrategy.getExpandedTemplateUnsafe` that `execute` writes out.
#[test]
fn test_expansion() {
    let f = setup("expansion");
    let content = create(&f).get_file_contents().unwrap();
    let expected = "key=foo\nvalue=bar";
    assert_eq!(content, expected);
}

#[test]
fn test_key_same_if_same() {
    let f = setup("key_same_if_same");
    let (a, b) = key_pair(&f, TEMPLATE, "foo", false);
    assert_eq!(a, b);
}

#[test]
fn test_key_differs_for_substitution() {
    let f = setup("key_differs_for_substitution");
    let (a, b) = key_pair(&f, TEMPLATE, "foo2", false);
    assert_ne!(a, b);
}

#[test]
fn test_key_differs_for_executable() {
    let f = setup("key_differs_for_executable");
    let (a, b) = key_pair(&f, TEMPLATE, "foo", true);
    assert_ne!(a, b);
}

#[test]
fn test_key_differs_for_templates() {
    let f = setup("key_differs_for_templates");
    let (a, b) = key_pair(&f, &format!("{} ", TEMPLATE), "foo", false);
    assert_ne!(a, b);
}

#[test]
fn test_artifact_template_has_input() {
    let f = setup("artifact_template_has_input");
    let action = create_with_artifact(&f, f.substitutions.clone());
    assert_eq!(
        *action.get_inputs().to_list(),
        vec![f.input_artifact.clone()]
    );
}

#[test]
fn test_artifact_template_has_output() {
    let f = setup("artifact_template_has_output");
    let action = create_with_artifact(&f, f.substitutions.clone());
    assert_eq!(action.get_outputs(), vec![f.output_artifact.clone()]);
}

// PORT: executes through `getFileContents` (see `test_expansion`).
#[test]
fn test_artifact_template_expansion() {
    let f = setup("artifact_template_expansion");
    // The trailing "" is needed because scratch.overwriteFile implicitly appends "\n".
    let expected = "key=foo\nvalue=bar\n";
    let actual = create_with_artifact(&f, f.substitutions.clone())
        .get_file_contents()
        .unwrap();
    assert_eq!(actual, expected);
}

// PORT: executes through `getFileContents` (see `test_expansion`).
#[test]
fn test_with_special_characters() {
    // We have to overwrite the artifacts since we need our template in "inputs"
    let f = create_artifacts(
        "with_special_characters",
        &format!("{}%key%", SPECIAL_CHARS),
    );

    // scratch.overwriteFile appends a newline, so we need an additional \n here
    let expected = format!("{}{}\n", SPECIAL_CHARS, SPECIAL_CHARS);

    // PORT: StringEncoding.unicodeToInternal is the identity in Rust.
    let actual = create_with_artifact(&f, vec![Substitution::of("%key%", SPECIAL_CHARS)])
        .get_file_contents()
        .unwrap();
    assert_eq!(actual, expected);
}

// ---------------------------------------------------------------------------------------------
// Other analysis-time behaviour (not in the Java tests).
// ---------------------------------------------------------------------------------------------

#[test]
fn mnemonic_progress_message_and_describe_key() {
    let f = setup("mnemonic_progress_message");
    let action = create(&f);
    assert_eq!(action.get_mnemonic(), "TemplateExpand");
    assert_eq!(
        action.get_progress_message(None).unwrap(),
        "Expanding template destination.txt"
    );
    assert_eq!(
        action.describe_key().unwrap(),
        "GUID: 786c1fe0-dca8-407a-b108-e1ecd6d1bc7f\nmakeExecutable: false\ntemplate: key=%key%\n\
         value=%value%\nsubstitutions: [Substitution(%key% -> foo), Substitution(%value% -> bar)]\n"
    );
    assert_eq!(
        action
            .get_starlark_substitutions()
            .unwrap()
            .get("%value%")
            .map(String::as_str),
        Some("bar")
    );
}

#[test]
fn template_content_of_derived_template_is_its_key() {
    let f = setup("derived_template");
    let action = TemplateExpansionAction::for_template_artifact(
        null_action_owner(),
        output_artifact_2(&f),
        f.output_artifact.clone(),
        Vec::new(),
        false,
    );
    assert_eq!(
        get_template_content(&action).unwrap(),
        "ARTIFACT: out/destination.txt"
    );
}

// Expected key from `bazel-9.3.2 aquery --output=jsonproto //:g` on a workspace whose rule does
//   ctx.actions.expand_template(template = ctx.file.template, output = <g_tpl.txt>,
//       substitutions = {"%NAME%": "wörld", "%EMPTY%": ""}, is_executable = True)
// with tpl.txt = "hello %NAME%\nempty=[%EMPTY%]\n" in the workspace root (k8-fastbuild,
// execution platform @@platforms//host:host). aquery reported templateContent = the file and
// substitutions [{%NAME%, wörld}, {%EMPTY%}].
#[test]
fn oracle_template_expansion_key_and_content() {
    let dir = temp_dir("oracle");
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let workspace_path = Path::create(PathFragment::create(dir.0.to_str().unwrap()), &fs);
    let workspace = ArtifactRoot::as_source_root(Root::from_path(workspace_path));
    std::fs::write(dir.0.join("tpl.txt"), "hello %NAME%\nempty=[%EMPTY%]\n").unwrap();
    let template = Artifact::new_source_artifact(
        workspace,
        PathFragment::create("tpl.txt"),
        ArtifactOwner::NullOwner,
    );
    let action = TemplateExpansionAction::for_template_artifact(
        owner_with_platform(oracle_common::host_platform()),
        template,
        get_bin_artifact_with_no_owner("g_tpl.txt"),
        vec![
            Substitution::of("%NAME%", "w\u{f6}rld"),
            Substitution::of("%EMPTY%", ""),
        ],
        true,
    );
    assert_eq!(
        action.get_key(&ActionKeyContext::new(), None).unwrap(),
        "866889b25f65e3f271b8be36d621d62e381833e76f005ed9770cf51a486da8d3"
    );
    let (content, substitutions) = action.aquery_template_expansion().unwrap().unwrap();
    assert_eq!(content, "hello %NAME%\nempty=[%EMPTY%]\n");
    assert_eq!(
        substitutions,
        vec![
            ("%NAME%".to_string(), "w\u{f6}rld".to_string()),
            ("%EMPTY%".to_string(), String::new()),
        ]
    );
}

/// The TemplateExpand actions of the re2 recording: the key of each, rebuilt from the recorded
/// template input and substitutions, is the oracle's for one of the two `makeExecutable` values
/// (aquery does not report the flag); the template content matches when the template file is
/// still on disk.
#[test]
fn oracle_re2_template_expansion_keys() {
    let Some(rec) = oracle_common::Recording::load("re2") else {
        eprintln!("no oracle recording; skipped");
        return;
    };
    let mut checked = 0;
    for a in rec
        .actions()
        .iter()
        .filter(|a| a["mnemonic"] == "TemplateExpand")
    {
        let inputs = rec.flattened_inputs(a);
        assert_eq!(inputs.len(), 1);
        let substitutions: Vec<Substitution> = a["substitutions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                Substitution::of(
                    s["key"].as_str().unwrap(),
                    s.get("value").and_then(|v| v.as_str()).unwrap_or(""),
                )
            })
            .collect();
        let keys: Vec<String> = [false, true]
            .iter()
            .map(|&executable| {
                TemplateExpansionAction::for_template_artifact(
                    rec.action_owner(a),
                    inputs[0].clone(),
                    rec.primary_output(a),
                    substitutions.clone(),
                    executable,
                )
                .get_key(&ActionKeyContext::new(), None)
                .unwrap()
            })
            .collect();
        let expected = a["actionKey"].as_str().unwrap().to_string();
        assert!(keys.contains(&expected), "{:?} not in {:?}", expected, keys);
        let action = TemplateExpansionAction::for_template_artifact(
            rec.action_owner(a),
            inputs[0].clone(),
            rec.primary_output(a),
            substitutions,
            false,
        );
        if inputs[0].get_path().exists() {
            assert_eq!(
                get_template_content(&action).unwrap(),
                a["templateContent"].as_str().unwrap()
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 4);
}
