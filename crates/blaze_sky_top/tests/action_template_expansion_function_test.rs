//! Port of `ActionTemplateExpansionFunctionTest`
//! (src/test/java/com/google/devtools/build/lib/skyframe), on a real temporary directory.
//!
//! The tests over `SpawnActionTemplate` (`ActionsTestUtil.createDummySpawnActionTemplate`, the
//! conflict tests) are ignored: `SpawnActionTemplate` is not ported.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use blaze_actions::actions::action::{
    AbstractAction, Action, ActionRef, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::actions::artifact::SpecialArtifactType;
use blaze_actions::{
    ActionLookupData, ActionLookupKeyRef, Artifact, ArtifactOwner, ArtifactRoot, RootType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::StoredEventHandler;
use blaze_events::handler::EventHandler;
use blaze_exec::actions::action_adapters::artifact_input;
use blaze_exec::actions::{
    ActionTemplate, ActionTemplateError, FileArtifactValue, TreeArtifactValue,
    register_action_template,
};
use blaze_sky_exec::action_template_expansion_value::{
    ActionTemplateExpansionKey, ActionTemplateExpansionValue,
};
use blaze_sky_exec::analysis_seam::{ActionLookupValue, AnalysisSeam, ConfiguredTargetView};
use blaze_sky_exec::artifact_function::ArtifactValue;
use blaze_sky_exec::keys::ArtifactKey;
use blaze_sky_exec::skyframe_action_executor::{
    ActionContexts, OutputPermissions, SkyframeActionExecutor, SkyframeActionExecutorConfig,
};
use blaze_sky_top::action_template_expansion_function::ActionTemplateExpansionFunction;
use blaze_skyframe::{ComputeError, Environment, EvaluationContext, EvaluatorBuilder, SkyKey};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

/// `InjectedActionLookupKey("key")`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InjectedActionLookupKey(&'static str);

impl std::fmt::Display for InjectedActionLookupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InjectedActionLookupKey{{name={}}}", self.0)
    }
}

impl ActionLookupKey for InjectedActionLookupKey {
    fn get_label(&self) -> Option<Label> {
        None
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "INJECTED"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other
            .as_any()
            .downcast_ref::<InjectedActionLookupKey>()
            .is_some_and(|o| o == self)
    }
    fn key_hash(&self) -> u64 {
        7
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn ctkey() -> ActionLookupKeyRef {
    ActionLookupKeyRef::new(InjectedActionLookupKey("key"))
}

/// `ActionsTestUtil.NULL_ACTION_OWNER`.
fn null_action_owner() -> ActionOwner {
    ActionOwner::create_dummy(
        None,
        blaze_events::location::Location::new("dummy-file", 0, 0),
        "dummy-kind",
        "dummy-configuration-mnemonic",
        "dummy-configuration",
        None,
        false,
        None,
        Vec::new(),
        IndexMap::new(),
    )
}

/// `DummyAction(inputs, output)` (lib/actions/util/DummyAction.java).
#[derive(Debug)]
struct DummyAction {
    base: AbstractAction,
}

impl DummyAction {
    fn new(inputs: NestedSet<Artifact>, output: Artifact) -> ActionRef {
        Arc::new(DummyAction {
            base: AbstractAction::new(null_action_owner(), inputs, [output]),
        })
    }
    fn of(input: Artifact, output: Artifact) -> ActionRef {
        DummyAction::new(
            NestedSetBuilder::create(Order::STABLE_ORDER, [input]),
            output,
        )
    }
}

impl Action for DummyAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn get_mnemonic(&self) -> String {
        "Dummy".to_owned()
    }
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(&self.get_primary_output().get_exec_path_string());
        Ok(())
    }
}

type Generator =
    dyn Fn(&TestActionTemplate, Vec<Artifact>, ActionLookupKeyRef) -> Vec<ActionRef> + Send + Sync;

/// `TestActionTemplate` with its `generateActionsForInputArtifacts` (and optionally
/// `getOutputs`) given per test.
struct TestActionTemplate {
    base: AbstractAction,
    input_tree_artifacts: Vec<Artifact>,
    output_tree_artifact: Artifact,
    generate: Box<Generator>,
}

impl std::fmt::Debug for TestActionTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.pretty_print())
    }
}

impl TestActionTemplate {
    fn new(
        input_tree_artifacts: Vec<Artifact>,
        output_tree_artifact: Artifact,
        outputs: Option<Vec<Artifact>>,
        generate: Box<Generator>,
    ) -> TestActionTemplate {
        for input_tree_artifact in &input_tree_artifacts {
            assert!(input_tree_artifact.is_tree_artifact());
        }
        assert!(output_tree_artifact.is_tree_artifact());
        let inputs = NestedSetBuilder::wrap(Order::STABLE_ORDER, input_tree_artifacts.clone());
        TestActionTemplate {
            base: AbstractAction::new(
                null_action_owner(),
                inputs,
                outputs.unwrap_or_else(|| vec![output_tree_artifact.clone()]),
            ),
            input_tree_artifacts,
            output_tree_artifact,
            generate,
        }
    }
}

impl Action for TestActionTemplate {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn get_mnemonic(&self) -> String {
        "TestActionTemplate".to_owned()
    }
    fn is_action_execution_metadata(&self) -> bool {
        false
    }
    fn is_shareable(&self) -> bool {
        false
    }
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        for input_tree_artifact in &self.input_tree_artifacts {
            fp.add_string(input_tree_artifact.get_path().get_path_string());
        }
        fp.add_string(self.output_tree_artifact.get_path().get_path_string());
        Ok(())
    }
    fn pretty_print(&self) -> String {
        format!("TestActionTemplate for {}", self.output_tree_artifact)
    }
    fn describe(&self) -> String {
        self.pretty_print()
    }
    fn to_string_java(&self) -> String {
        self.pretty_print()
    }
    fn get_mandatory_inputs(&self) -> NestedSet<Artifact> {
        NestedSetBuilder::wrap(Order::STABLE_ORDER, self.input_tree_artifacts.clone())
    }
}

impl ActionTemplate for TestActionTemplate {
    fn generate_actions_for_input_artifacts(
        &self,
        input_tree_file_artifacts: Vec<Artifact>,
        artifact_owner: ActionLookupKeyRef,
        _event_handler: &dyn EventHandler,
    ) -> Result<Vec<ActionRef>, ActionTemplateError> {
        Ok((self.generate)(
            self,
            input_tree_file_artifacts,
            artifact_owner,
        ))
    }
    fn get_input_tree_artifacts(&self) -> Vec<Artifact> {
        self.input_tree_artifacts.clone()
    }
}

struct BasicActionLookupValue(Vec<ActionRef>);

impl ActionLookupValue for BasicActionLookupValue {
    fn get_actions(&self) -> &[ActionRef] {
        &self.0
    }
}

struct InjectedSeam(Arc<BasicActionLookupValue>);

impl AnalysisSeam for InjectedSeam {
    fn get_action_lookup_value(
        &self,
        key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ActionLookupValue>>, ComputeError> {
        assert!(*key == ctkey());
        Ok(Some(self.0.clone() as Arc<dyn ActionLookupValue>))
    }
    fn get_configured_target(
        &self,
        _key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ConfiguredTargetView>>, ComputeError> {
        unreachable!()
    }
}

/// A temporary directory removed on drop.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("blaze-ate-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    _dir: TempDir,
    root_directory: Path,
    artifact_value_map: HashMap<Artifact, Arc<TreeArtifactValue>>,
}

impl Fixture {
    fn new() -> Fixture {
        register_action_template::<TestActionTemplate>();
        let dir = TempDir::new();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root_directory = fs.get_path(dir.0.to_str().unwrap());
        Fixture {
            _dir: dir,
            root_directory,
            artifact_value_map: HashMap::new(),
        }
    }

    fn create_tree_artifact(&self, path: &str) -> Artifact {
        let exec_path = PathFragment::create("out").get_relative(&PathFragment::create(path));
        Artifact::create_special_artifact(
            ArtifactRoot::as_derived_root(&self.root_directory, RootType::OUTPUT, &["out"]),
            exec_path,
            ctkey(),
            SpecialArtifactType::TREE,
        )
    }

    fn create_and_populate_tree_artifact(&mut self, path: &str, children: &[&str]) -> Artifact {
        let tree_artifact = self.create_tree_artifact(path);
        tree_artifact.set_generating_action_key(ActionLookupData::create(ctkey(), 0));
        let mut tree = Vec::new();
        for child_relative_path in children {
            let tree_file_artifact = Artifact::create_tree_output(
                &tree_artifact,
                PathFragment::create(child_relative_path),
            );
            let p = tree_file_artifact.get_path();
            std::fs::create_dir_all(std::path::Path::new(p.get_path_string()).parent().unwrap())
                .unwrap();
            std::fs::write(p.get_path_string(), child_relative_path).unwrap();
            // We do not care about the FileArtifactValues in this test.
            tree.push((
                artifact_input(&tree_file_artifact),
                FileArtifactValue::create_for_testing(&p).unwrap(),
            ));
        }
        self.artifact_value_map.insert(
            tree_artifact.clone(),
            Arc::new(TreeArtifactValue::build(tree, None, None)),
        );
        tree_artifact
    }

    /// Evaluates the expansion of `action_template`; `Err` is the panic message (Java's
    /// `RuntimeException` caused by an `IllegalStateException`).
    fn evaluate(&self, action_template: TestActionTemplate) -> Result<Vec<ActionRef>, String> {
        let action_template: ActionRef = Arc::new(action_template);
        // createActionLookupValue: Actions.assignOwnersAndThrowIfConflict then a value.
        blaze_analysis::rule::actions_util::assign_owners_and_throw_if_conflict(
            &ActionKeyContext::new(),
            std::slice::from_ref(&action_template),
            &ctkey(),
        )
        .unwrap();
        let seam = Arc::new(InjectedSeam(Arc::new(BasicActionLookupValue(vec![
            action_template,
        ]))));
        let executor = Arc::new(SkyframeActionExecutor::new(SkyframeActionExecutorConfig {
            exec_root: self.root_directory.clone(),
            action_out_dir: self.root_directory.get_child("action_outs"),
            external_directory: self.root_directory.get_child("external"),
            reporter: Arc::new(StoredEventHandler::new()),
            contexts: ActionContexts::default(),
            client_env: IndexMap::new(),
            keep_going: false,
            verbose_failures: false,
            output_permissions: OutputPermissions::READONLY,
        }));
        let function = Arc::new(ActionTemplateExpansionFunction {
            skyframe_action_executor: executor,
            seam,
        });
        let values = self.artifact_value_map.clone();
        let evaluator = EvaluatorBuilder::new()
            .typed_function::<ArtifactKey, _>(move |k, _env| {
                Ok(Some(ArtifactValue::Tree(
                    values.get(&k.0).cloned().expect("checkNotNull"),
                )))
            })
            .typed_function::<ActionTemplateExpansionKey, _>(move |k, env| function.compute(k, env))
            .build();
        let template_key = ActionTemplateExpansionValue::key(ctkey(), 0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            evaluator.evaluate(
                &[SkyKey::new(template_key.clone())],
                &EvaluationContext::default(),
            )
        }));
        let result = match result {
            Ok(r) => r.unwrap(),
            Err(p) => {
                return Err(p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default());
            }
        };
        let value = result.get(&template_key).expect("no error");
        Ok(value.get_actions().to_vec())
    }
}

fn exec_paths(artifacts: impl IntoIterator<Item = Artifact>) -> Vec<String> {
    artifacts
        .into_iter()
        .map(|a| a.get_exec_path_string().to_owned())
        .collect()
}

#[test]
#[ignore = "needs SpawnActionTemplate (ActionsTestUtil.createDummySpawnActionTemplate), not ported"]
fn test_action_template_expansion_function() {}

#[test]
#[ignore = "needs SpawnActionTemplate with an OutputPathMapper, not ported"]
fn test_throws_on_action_conflict() {}

#[test]
#[ignore = "needs SpawnActionTemplate with an OutputPathMapper, not ported"]
fn test_throws_on_artifact_prefix_conflict() {}

#[test]
fn cannot_declare_non_tree_output() {
    let mut f = Fixture::new();
    let input_tree = f.create_and_populate_tree_artifact("input", &["child"]);
    let output_tree = f.create_tree_artifact("output");
    let not_tree = Artifact::create_derived_artifact(
        output_tree.get_root().clone(),
        output_tree
            .get_root()
            .get_exec_path()
            .get_relative(&PathFragment::create("not_tree")),
        ctkey(),
    );
    let template = TestActionTemplate::new(
        vec![input_tree],
        output_tree.clone(),
        Some(vec![output_tree, not_tree]),
        Box::new(|_, _, _| Vec::new()),
    );
    let template_string = template.to_string_java();
    let e = f.evaluate(template).unwrap_err();
    assert!(
        e.contains(&format!(
            "{template_string} declares an output which is not a tree artifact"
        )),
        "{e}"
    );
}

#[test]
fn cannot_generate_output_with_wrong_owner() {
    let mut f = Fixture::new();
    let input_tree = f.create_and_populate_tree_artifact("input", &["child"]);
    let output_tree = f.create_tree_artifact("output");
    let out = output_tree.clone();
    let template = TestActionTemplate::new(
        vec![input_tree],
        output_tree,
        None,
        Box::new(move |_, inputs, artifact_owner| {
            let input = inputs.into_iter().next().unwrap();
            // ActionsTestUtil.NULL_TEMPLATE_EXPANSION_ARTIFACT_OWNER
            let wrong_owner = ActionLookupKeyRef::new(ActionTemplateExpansionValue::key(
                ActionLookupKeyRef::new(InjectedActionLookupKey("null")),
                0,
            ));
            let output_with_wrong_owner = Artifact::create_template_expansion_output(
                &out,
                PathFragment::create("child"),
                wrong_owner,
            );
            assert!(
                output_with_wrong_owner.get_artifact_owner()
                    != ArtifactOwner::ActionLookupKey(artifact_owner)
            );
            vec![DummyAction::of(input, output_with_wrong_owner)]
        }),
    );
    let template_string = template.to_string_java();
    let e = f.evaluate(template).unwrap_err();
    assert!(
        e.contains(&format!(
            "{template_string} generated an action with an output owned by the wrong owner"
        )),
        "{e}"
    );
}

#[test]
fn cannot_generate_non_tree_file_artifact_output() {
    let mut f = Fixture::new();
    let input_tree = f.create_and_populate_tree_artifact("input", &["child"]);
    let output_tree = f.create_tree_artifact("output");
    let template = TestActionTemplate::new(
        vec![input_tree],
        output_tree,
        None,
        Box::new(|_, inputs, artifact_owner| {
            let input = inputs.into_iter().next().unwrap();
            let not_tree_file_artifact = Artifact::create_derived_artifact(
                input.get_root().clone(),
                input
                    .get_root()
                    .get_exec_path()
                    .get_relative(&PathFragment::create("a.txt")),
                artifact_owner,
            );
            assert!(!not_tree_file_artifact.is_tree_artifact());
            vec![DummyAction::of(input, not_tree_file_artifact)]
        }),
    );
    let template_string = template.to_string_java();
    let e = f.evaluate(template).unwrap_err();
    assert!(
        e.contains(&format!(
            "{template_string} generated an action which outputs a non-TreeFileArtifact"
        )),
        "{e}"
    );
}

#[test]
fn cannot_generate_output_under_undeclared_tree() {
    let mut f = Fixture::new();
    let input_tree = f.create_and_populate_tree_artifact("input", &["child"]);
    let output_tree = f.create_tree_artifact("output");
    let undeclared = f.create_tree_artifact("undeclared");
    let template = TestActionTemplate::new(
        vec![input_tree],
        output_tree,
        None,
        Box::new(move |_, inputs, artifact_owner| {
            let input = inputs.into_iter().next().unwrap();
            let output_under_wrong_tree = Artifact::create_template_expansion_output(
                &undeclared,
                PathFragment::create("child"),
                artifact_owner,
            );
            vec![DummyAction::of(input, output_under_wrong_tree)]
        }),
    );
    let template_string = template.to_string_java();
    let e = f.evaluate(template).unwrap_err();
    // Java also prints the artifacts (`File:[[<execution_root>]out]undeclared/child`); the port's
    // `Artifact` Display is checked by the artifact tests.
    assert!(
        e.contains(&format!(
            "{template_string} generated an action with an output "
        )) && e.contains(" under an undeclared tree not in "),
        "{e}"
    );
}

#[test]
fn can_generate_output_under_additional_declared_tree() {
    let mut f = Fixture::new();
    let input_tree = f.create_and_populate_tree_artifact("input", &["child"]);
    let output_tree = f.create_tree_artifact("output");
    let additional_output_tree = f.create_tree_artifact("additional_output");
    let (out, additional) = (output_tree.clone(), additional_output_tree.clone());
    let template = TestActionTemplate::new(
        vec![input_tree],
        output_tree.clone(),
        Some(vec![output_tree, additional_output_tree]),
        Box::new(move |_, inputs, artifact_owner| {
            let input = inputs.into_iter().next().unwrap();
            vec![
                DummyAction::of(
                    input.clone(),
                    Artifact::create_template_expansion_output(
                        &out,
                        PathFragment::create("child"),
                        artifact_owner.clone(),
                    ),
                ),
                DummyAction::of(
                    input,
                    Artifact::create_template_expansion_output(
                        &additional,
                        PathFragment::create("additional_child"),
                        artifact_owner,
                    ),
                ),
            ]
        }),
    );
    let actions = f.evaluate(template).unwrap();
    assert_eq!(actions.len(), 2);
    let owner = ActionLookupKeyRef::new(ActionTemplateExpansionValue::key(ctkey(), 0));
    for action in &actions {
        for output in action.get_outputs() {
            assert_eq!(
                output.get_artifact_owner(),
                ArtifactOwner::ActionLookupKey(owner.clone())
            );
        }
    }
    assert_eq!(
        exec_paths(actions[0].get_outputs()),
        vec!["out/output/child".to_owned()]
    );
    assert_eq!(
        exec_paths(actions[1].get_outputs()),
        vec!["out/additional_output/additional_child".to_owned()]
    );
}

#[test]
fn can_use_multiple_input_trees() {
    let mut f = Fixture::new();
    let input_tree1 = f.create_and_populate_tree_artifact("input1", &["child1", "child2"]);
    let input_tree2 = f.create_and_populate_tree_artifact("input2", &["child1", "child2"]);
    let output_tree = f.create_tree_artifact("output");
    let out = output_tree.clone();
    let template = TestActionTemplate::new(
        vec![input_tree1, input_tree2],
        output_tree,
        None,
        Box::new(move |template, inputs, artifact_owner| {
            let mut actions = Vec::new();
            let input_tree_artifacts_to_children =
                blaze_exec::actions::action_template::get_input_tree_artifacts_to_children(&inputs);
            for (i, input_tree_artifact) in
                template.get_input_tree_artifacts().into_iter().enumerate()
            {
                actions.push(DummyAction::new(
                    NestedSetBuilder::wrap(
                        Order::STABLE_ORDER,
                        input_tree_artifacts_to_children[&input_tree_artifact].clone(),
                    ),
                    Artifact::create_template_expansion_output(
                        &out,
                        PathFragment::create(&format!("child-{i}")),
                        artifact_owner.clone(),
                    ),
                ));
            }
            actions
        }),
    );
    let actions = f.evaluate(template).unwrap();
    assert_eq!(actions.len(), 2);
}
