//! Port of `analysis/actions/BuildInfoFileWriteActionTest.java`.

#![allow(dead_code)]

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use blaze_actions::ArtifactOwner;
use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::analysis::actions::build_info_file_write_action::BuildInfoFileWriteAction;
use blaze_actions::analysis::workspace_status_action::BUILD_INFO_KEY;
use blaze_actions::{
    Action, ActionKeyContext, ActionLookupKeyRef, ActionOwner, Artifact, ArtifactRoot,
    PlatformInfoRef, RootType,
};
use blaze_cmdline::Label;
use blaze_events::location::Location;
use blaze_vfs::Root;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;
use starlark::environment::{Globals, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::OwnedFrozenValue;

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
    let fs = shared_fs();
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
// BuildInfoFileWriteActionTest
// ---------------------------------------------------------------------------------------------

/// `exec(lines...)`: evaluates the lines as a module and returns the frozen function `t`.
fn exec(lines: &[&str]) -> OwnedFrozenValue {
    let ast = AstModule::parse("test/label.bzl", lines.join("\n"), &Dialect::Extended).unwrap();
    let globals = Globals::standard();
    Module::with_temp_heap(|module| {
        {
            let mut eval = Evaluator::new(&module);
            eval.eval_module(ast, &globals).unwrap();
        }
        module.freeze().unwrap().get("t").unwrap()
    })
}

/// `getSourceArtifact(name)` / `getSourceArtifact(name, owner)`.
fn get_source_artifact(name: &str, owner: ArtifactOwner) -> Artifact {
    let fs = shared_fs();
    let root = ArtifactRoot::as_source_root(Root::from_path(Path::create(
        PathFragment::create("/workspace"),
        &fs,
    )));
    Artifact::new_source_artifact(root, PathFragment::create(name), owner)
}

fn build_info_owner() -> ArtifactOwner {
    ArtifactOwner::ActionLookupKey(ActionLookupKeyRef::new(BUILD_INFO_KEY))
}

#[test]
#[ignore = "needs action execution and the Starlark translation callback (wave W6)"]
fn execute_writes_to_output_file() {}

#[test]
#[ignore = "needs action execution and the Starlark translation callback (wave W6)"]
fn key_missing_template_partially_expanded() {}

#[test]
fn action_inputs_vary_check_compute_key_results() {
    let output_file = get_bin_artifact_with_no_owner("output.txt");
    let starlark_func_object = exec(&[
        "def t(d):",
        " for i in range(1, 10):",
        "   for j in range (1, 10):",
        "       a = 5",
        " return {}",
    ]);
    let ctx = ActionKeyContext::new();
    // INPUT, TEMPLATE, IS_VOLATILE
    action_tester_run_test(
        3,
        |flip| {
            Box::new(BuildInfoFileWriteAction::new(
                null_action_owner(),
                if flip[0] {
                    get_source_artifact("input1.txt", build_info_owner())
                } else {
                    get_source_artifact("input2.txt", build_info_owner())
                },
                output_file.clone(),
                starlark_func_object.clone(),
                if flip[1] {
                    get_source_artifact("template1.txt", ArtifactOwner::NullOwner)
                } else {
                    get_source_artifact("template2.txt", ArtifactOwner::NullOwner)
                },
                flip[2],
                // bzlTransitiveDigest of the test module.
                Vec::new(),
            ))
        },
        &ctx,
    );
}

#[test]
#[ignore = "needs action execution and the Starlark translation callback (wave W6)"]
fn wrong_key_read_exception_thrown() {}

#[test]
#[ignore = "needs action execution and the Starlark translation callback (wave W6)"]
fn callback_return_value_invalid_type_exception_thrown() {}

#[test]
#[ignore = "needs action execution and the Starlark translation callback (wave W6)"]
fn callback_return_dict_contains_invalid_type_exception_thrown() {}

#[test]
#[ignore = "needs action execution and the Starlark translation callback (wave W6)"]
fn callback_fails_exception_thrown() {}

#[test]
#[should_panic(
    expected = "input artifact of BuildInfoFileWriteAction must be one of workspace status artifacts: ctx.info_file or ctx.version_file"
)]
fn wrong_artifact_owner_on_input_source_file_exception_thrown() {
    let starlark_func_object = exec(&["def t(d):", " pass"]);
    BuildInfoFileWriteAction::new(
        null_action_owner(),
        // Set no artifact owner.
        get_source_artifact("input.txt", ArtifactOwner::NullOwner),
        get_bin_artifact_with_no_owner("output.txt"),
        starlark_func_object,
        get_source_artifact("template.txt", ArtifactOwner::NullOwner),
        false,
        Vec::new(),
    );
}

#[test]
fn mnemonic_progress_message_and_volatility() {
    let f = exec(&["def t(d):", " return {}"]);
    let make = |is_volatile: bool| {
        BuildInfoFileWriteAction::new(
            null_action_owner(),
            get_source_artifact("volatile-status.txt", build_info_owner()),
            get_bin_artifact_with_no_owner("output.txt"),
            f.clone(),
            get_source_artifact("template.txt", ArtifactOwner::NullOwner),
            is_volatile,
            Vec::new(),
        )
    };
    let stable = make(false);
    let volatile = make(true);
    assert_eq!(stable.get_mnemonic(), "TranslateBuildInfo");
    assert_eq!(
        stable.get_progress_message(None).unwrap(),
        "Translating stable BuildInfo file"
    );
    assert_eq!(
        volatile.get_progress_message(None).unwrap(),
        "Translating volatile BuildInfo file"
    );
    assert!(!stable.is_volatile() && !stable.execute_unconditionally());
    assert!(volatile.is_volatile() && volatile.execute_unconditionally());
    assert_eq!(stable.get_inputs().to_list().len(), 2);
}

/// One file system for every artifact of a test (Java's `Scratch`): paths compare by file system.
fn shared_fs() -> blaze_vfs::FileSystemRef {
    static FS: std::sync::LazyLock<blaze_vfs::FileSystemRef> =
        std::sync::LazyLock::new(|| UnixFileSystem::create(DigestHashFunction::Sha256));
    FS.clone()
}
