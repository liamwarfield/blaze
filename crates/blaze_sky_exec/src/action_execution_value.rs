//! `ActionExecutionValue`: the metadata of an executed action's outputs.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use blaze_actions::Artifact;
use blaze_actions::actions::action::Action;
use blaze_actions::actions::artifact::OwnerlessArtifactWrapper;
use blaze_exec::actions::action_adapters::{artifact_input, as_blaze_artifact};
use blaze_exec::actions::{FileArtifactValue, RunfilesArtifactValue, TreeArtifactValue};
use indexmap::IndexMap;

/// A value representing an executed action.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:56 ActionExecutionValue
// PORT: Java's four size-specialized subclasses (a memory optimization) are one struct. The
// "rich artifact data" (`RunfilesArtifactValue` of a runfiles tree action, `FilesetOutputTree`)
// is `rich_artifact_data`.
#[derive(Clone, Debug, Default)]
pub struct ActionExecutionValue {
    artifact_data: IndexMap<Artifact, FileArtifactValue>,
    tree_artifact_data: IndexMap<Artifact, Arc<TreeArtifactValue>>,
    rich_artifact_data: Option<Arc<RunfilesArtifactValue>>,
    /// `RunfilesArtifactValue.getMetadata()` of `rich_artifact_data` (Java computes it in the
    /// value's constructor from the producing action's input metadata).
    rich_artifact_metadata: Option<FileArtifactValue>,
}

/// `TreeArtifactValue.MISSING_TREE_ARTIFACT`: a value that represents a missing tree artifact.
///
/// PORT: Java's marker is an anonymous `TreeArtifactValue` subclass whose `getChildValues`,
/// `getChildren`, `getMetadata` and `getDigest` throw `UnsupportedOperationException` and whose
/// `equals` is identity. Blaze's `TreeArtifactValue` (in `blaze_exec`) has no marker variant, so
/// the marker is this one shared instance, recognized by pointer identity
/// ([`is_missing_tree_artifact`]); code that would call one of the throwing methods on it checks
/// for it first and panics with Java's text.
// Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:428 TreeArtifactValue.MISSING_TREE_ARTIFACT
pub static MISSING_TREE_ARTIFACT: LazyLock<Arc<TreeArtifactValue>> = LazyLock::new(|| {
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:431 TreeArtifactValue.createMarker
    Arc::new(TreeArtifactValue {
        child_values: Vec::new(),
        archived_representation: None,
        metadata: None,
        resolved_path: None,
    })
});

/// `TreeArtifactValue.MISSING_TREE_ARTIFACT.equals(value)` (identity).
pub fn is_missing_tree_artifact(value: &Arc<TreeArtifactValue>) -> bool {
    Arc::ptr_eq(value, &MISSING_TREE_ARTIFACT)
}

/// The argument checks at the start of `ActionExecutionValue.create(artifactData,
/// treeArtifactData, richArtifactData, discoveredModules)`, in Java's order.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:69 ActionExecutionValue.create
fn check_output_data(
    artifact_data: &IndexMap<Artifact, FileArtifactValue>,
    tree_artifact_data: &IndexMap<Artifact, Arc<TreeArtifactValue>>,
) {
    // Use forEach instead of entrySet to avoid instantiating an EntrySet in ImmutableMap.
    for (artifact, value) in artifact_data {
        blaze_util::check_argument!(
            !artifact.is_child_of_declared_directory(),
            "{} should only be stored in a TreeArtifactValue",
            artifact
        );
        blaze_util::check_argument!(
            !value.get_type().is_file() || value.get_digest().is_some(),
            "Missing digest for {}",
            artifact
        );
    }
    for (tree, tree_value) in tree_artifact_data {
        // MISSING_TREE_ARTIFACT.getChildValues() throws.
        // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:445 TreeArtifactValue.createMarker
        if is_missing_tree_artifact(tree_value) {
            panic!("UnsupportedOperationException: MISSING_TREE_ARTIFACT");
        }
        for (child, child_value) in &tree_value.child_values {
            // Ignore symlinks to directories, which don't have a digest.
            if !(!child_value.get_type().is_file() || child_value.get_digest().is_some()) {
                // PORT: the child is a `TreeFileArtifact`; its `toString` is the blaze_actions
                // artifact's `Display` (the execution-layer input's `Display` is only the exec
                // path).
                let child = match as_blaze_artifact(child) {
                    Some(artifact) => artifact.to_string(),
                    None => child.to_string(),
                };
                panic!(
                    "IllegalArgumentException: Missing digest for file {} in tree artifact {}",
                    child, tree
                );
            }
        }
    }
}

impl ActionExecutionValue {
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:63 ActionExecutionValue.create
    pub fn create(
        artifact_data: IndexMap<Artifact, FileArtifactValue>,
        tree_artifact_data: IndexMap<Artifact, Arc<TreeArtifactValue>>,
        rich_artifact_data: Option<Arc<RunfilesArtifactValue>>,
    ) -> ActionExecutionValue {
        check_output_data(&artifact_data, &tree_artifact_data);
        if rich_artifact_data.is_some() {
            blaze_util::check_argument!(
                artifact_data.len() == 1,
                "actions with rich artifact data should have a single output file (the \
                 manifest): {:?}",
                artifact_data
            );
            blaze_util::check_argument!(
                tree_artifact_data.is_empty(),
                "actions with rich artifact data do not output tree artifacts: {:?}",
                tree_artifact_data
            );
        }
        ActionExecutionValue {
            artifact_data,
            tree_artifact_data,
            rich_artifact_data,
            rich_artifact_metadata: None,
        }
    }

    /// [`ActionExecutionValue::create`] with the rich artifact data's metadata
    /// (`RunfilesArtifactValue.getMetadata()`).
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:64 ActionExecutionValue.create
    pub fn create_with_rich_metadata(
        artifact_data: IndexMap<Artifact, FileArtifactValue>,
        tree_artifact_data: IndexMap<Artifact, Arc<TreeArtifactValue>>,
        rich_artifact_data: Option<(Arc<RunfilesArtifactValue>, FileArtifactValue)>,
    ) -> ActionExecutionValue {
        check_output_data(&artifact_data, &tree_artifact_data);
        if rich_artifact_data.is_some() {
            blaze_util::check_argument!(
                artifact_data.len() == 1,
                "actions with rich artifact data should have a single output file (the \
                 manifest): {:?}",
                artifact_data
            );
            blaze_util::check_argument!(
                tree_artifact_data.is_empty(),
                "actions with rich artifact data do not output tree artifacts: {:?}",
                tree_artifact_data
            );
        }
        let (rich_artifact_data, rich_artifact_metadata) = match rich_artifact_data {
            Some((d, m)) => (Some(d), Some(m)),
            None => (None, None),
        };
        ActionExecutionValue {
            artifact_data,
            tree_artifact_data,
            rich_artifact_data,
            rich_artifact_metadata,
        }
    }

    /// Returns metadata for a given artifact, if that metadata is cached in this value.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:162 ActionExecutionValue.getExistingFileArtifactValue
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:358 ActionExecutionValue.SingleOutputFile.getExistingFileArtifactValue
    pub fn get_existing_file_artifact_value(
        &self,
        artifact: &Artifact,
    ) -> Option<&FileArtifactValue> {
        if artifact.is_child_of_declared_directory() {
            let tree = self.get_tree_artifact_value(artifact.get_parent()?)?;
            return tree
                .child_values
                .iter()
                .find(|(child, _)| child.get_exec_path() == artifact.get_exec_path())
                .map(|(_, value)| value);
        }
        self.artifact_data.get(artifact)
    }

    /// `getTreeArtifactValue(Artifact)`.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:184 ActionExecutionValue.getTreeArtifactValue
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:439 ActionExecutionValue.SingleTree.getTreeArtifactValue
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:473 ActionExecutionValue.MultiTree.getTreeArtifactValue
    pub fn get_tree_artifact_value(&self, artifact: &Artifact) -> Option<&Arc<TreeArtifactValue>> {
        self.tree_artifact_data.get(artifact)
    }

    /// `getAllFileValues()`.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:208 ActionExecutionValue.getAllFileValues
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:367 ActionExecutionValue.SingleOutputFile.getAllFileValues
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:421 ActionExecutionValue.MultiOutputFile.getAllFileValues
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:451 ActionExecutionValue.SingleTree.getAllFileValues
    pub fn get_all_file_values(&self) -> &IndexMap<Artifact, FileArtifactValue> {
        &self.artifact_data
    }

    /// `getAllTreeArtifactValues()`.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:211 ActionExecutionValue.getAllTreeArtifactValues
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:446 ActionExecutionValue.SingleTree.getAllTreeArtifactValues
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:480 ActionExecutionValue.MultiTree.getAllTreeArtifactValues
    pub fn get_all_tree_artifact_values(&self) -> &IndexMap<Artifact, Arc<TreeArtifactValue>> {
        &self.tree_artifact_data
    }

    /// `getRichArtifactData()` (a runfiles tree's `RunfilesArtifactValue`).
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:215 ActionExecutionValue.getRichArtifactData
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:384 ActionExecutionValue.WithRichData.getRichArtifactData
    pub fn get_rich_artifact_data(&self) -> Option<&Arc<RunfilesArtifactValue>> {
        self.rich_artifact_data.as_ref()
    }

    /// Creates a new `ActionExecutionValue` by transforming this one's outputs so that artifact
    /// owners match the given action's outputs.
    ///
    /// The given action must be shareable (`Actions.canBeShared`) with the action that originally
    /// produced this `ActionExecutionValue`. `Err` is Java's `ActionTransformException` message.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:310 ActionExecutionValue.transformForSharedAction
    pub fn transform_for_shared_action(
        &self,
        action: &dyn Action,
    ) -> Result<ActionExecutionValue, String> {
        let outputs = action.get_outputs();
        if outputs.len() != self.artifact_data.len() + self.tree_artifact_data.len() {
            return Err(format!(
                "Cannot share {:?} with {}",
                self,
                action.to_string_java()
            ));
        }
        // Maps.uniqueIndex: duplicate keys are an IllegalArgumentException.
        let mut new_artifact_map: HashMap<OwnerlessArtifactWrapper, Artifact> = HashMap::new();
        for output in &outputs {
            if new_artifact_map
                .insert(OwnerlessArtifactWrapper(output.clone()), output.clone())
                .is_some()
            {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {}",
                    output
                );
            }
        }
        Ok(ActionExecutionValue {
            artifact_data: transform_map(
                &self.artifact_data,
                &new_artifact_map,
                action,
                |_, v| v.clone(),
            )?,
            tree_artifact_data: transform_map(
                &self.tree_artifact_data,
                &new_artifact_map,
                action,
                |new_artifact, tree| Arc::new(transform_shared_tree(new_artifact, tree)),
            )?,
            rich_artifact_data: self.rich_artifact_data.clone(),
            rich_artifact_metadata: self.rich_artifact_metadata.clone(),
        })
    }

    /// `((RunfilesArtifactValue) getRichArtifactData()).getMetadata()`, if known.
    // Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:125 RunfilesArtifactValue.getMetadata
    pub fn get_rich_artifact_metadata(&self) -> Option<&FileArtifactValue> {
        self.rich_artifact_metadata.as_ref()
    }
}

// Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:253 ActionExecutionValue.transformMap
fn transform_map<V>(
    data: &IndexMap<Artifact, V>,
    new_artifact_map: &HashMap<OwnerlessArtifactWrapper, Artifact>,
    action: &dyn Action,
    transform: impl Fn(&Artifact, &V) -> V,
) -> Result<IndexMap<Artifact, V>, String> {
    let mut result = IndexMap::with_capacity(data.len());
    for (artifact, value) in data {
        let Some(new_artifact) = new_artifact_map.get(&OwnerlessArtifactWrapper(artifact.clone()))
        else {
            return Err(format!(
                "No output matching {}, cannot share with {}",
                artifact,
                action.to_string_java()
            ));
        };
        let transformed = transform(new_artifact, value);
        if result.insert(new_artifact.clone(), transformed).is_some() {
            panic!(
                "IllegalArgumentException: Multiple entries with same key: {}",
                new_artifact
            );
        }
    }
    Ok(result)
}

/// Transforms the children of a `TreeArtifactValue` so that owners are consistent.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:279 ActionExecutionValue.transformSharedTree
fn transform_shared_tree(new_artifact: &Artifact, tree: &TreeArtifactValue) -> TreeArtifactValue {
    blaze_util::check_state!(
        new_artifact.is_tree_artifact(),
        "Expected tree artifact, got {}",
        new_artifact
    );
    let mut children = Vec::with_capacity(tree.child_values.len());
    for (child, value) in &tree.child_values {
        let parent_relative_path = child
            .as_artifact()
            .and_then(|a| a.get_parent_relative_path())
            .unwrap_or_else(|| panic!("tree artifact child {child} is not a TreeFileArtifact"));
        children.push((
            artifact_input(&Artifact::create_tree_output(
                new_artifact,
                parent_relative_path,
            )),
            value.clone(),
        ));
    }
    if tree.archived_representation.is_some() {
        blaze_util::unported!("ArchivedTreeArtifact.createForTree");
    }
    TreeArtifactValue::build(children, None, None)
}

#[cfg(test)]
mod transform_for_shared_action_tests {
    //! `transformForSharedAction` re-keys the outputs to the second owner's artifacts (Java
    //! `ActionExecutionValueTest` covers equality only; these check the transform itself).

    use std::any::Any;
    use std::fmt;
    use std::hash::{DefaultHasher, Hash, Hasher};

    use blaze_actions::actions::action::Action;
    use blaze_actions::actions::action_lookup::ActionLookupKey;
    use blaze_actions::actions::platform_info::empty_platform_info;
    use blaze_actions::analysis::actions::file_write_action::{Compression, FileWriteAction};
    use blaze_actions::{ActionLookupKeyRef, ActionOwner, Artifact, ArtifactRoot, RootType};
    use blaze_cmdline::Label;
    use blaze_events::location::Location;
    use blaze_exec::actions::FileArtifactValue;
    use blaze_vfs::{DigestHashFunction, FileSystemExt, UnixFileSystem};
    use indexmap::IndexMap;

    use super::ActionExecutionValue;

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct Key(String);

    impl fmt::Display for Key {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "ConfiguredTargetKey{{label={}}}", self.0)
        }
    }

    impl ActionLookupKey for Key {
        fn get_label(&self) -> Option<Label> {
            Some(Label::parse_canonical(&self.0).unwrap())
        }
        fn get_configuration_checksum(&self) -> Option<String> {
            Some("abc".to_string())
        }
        fn function_name(&self) -> &'static str {
            "CONFIGURED_TARGET"
        }
        fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
            other.as_any().downcast_ref::<Key>() == Some(self)
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

    fn artifact(fs: &blaze_vfs::FileSystemRef, owner: &str) -> Artifact {
        let exec_root = fs.get_path("/execroot");
        let root =
            ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["bazel-out", "bin"]);
        let exec_path = root.get_exec_path().get_relative_str("p/out");
        Artifact::create_derived_artifact(
            root,
            exec_path,
            ActionLookupKeyRef::new(Key(owner.to_string())),
        )
    }

    fn write_action(output: &Artifact) -> FileWriteAction {
        FileWriteAction::create(
            ActionOwner::create_dummy(
                Some(Label::parse_canonical("//p:t").unwrap()),
                Location::from_file_line_column("/ws/BUILD", 1, 1),
                "genrule rule",
                "k8-fastbuild",
                "abc",
                None,
                false,
                Some(empty_platform_info()),
                Vec::new(),
                IndexMap::new(),
            ),
            output.clone(),
            "x".to_string(),
            false,
            Compression::DISALLOW,
        )
    }

    #[test]
    fn transform_for_shared_action_rekeys_outputs_to_the_second_owner() {
        // One file system: paths of different file system instances never compare equal.
        let fs: blaze_vfs::FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        let first = artifact(&fs, "//p:a");
        let second = artifact(&fs, "//p:b");
        let value = FileArtifactValue::create_for_normal_file(Some(vec![1]), None, 1);
        let mut data = IndexMap::new();
        data.insert(first.clone(), value.clone());
        let v = ActionExecutionValue::create(data, IndexMap::new(), None);

        let shared = v
            .transform_for_shared_action(&write_action(&second) as &dyn Action)
            .unwrap();
        assert_eq!(
            shared.get_existing_file_artifact_value(&second),
            Some(&value)
        );
        assert_eq!(shared.get_all_file_values().len(), 1);
        assert!(
            shared
                .get_all_file_values()
                .keys()
                .next()
                .unwrap()
                .get_artifact_owner()
                == second.get_artifact_owner()
        );
    }
}

#[cfg(test)]
mod action_execution_value_test {
    //! Port of `ActionExecutionValueTest`
    //! (src/test/java/com/google/devtools/build/lib/skyframe/ActionExecutionValueTest.java): its
    //! two `@Test` methods, `equality` (:62) and `serialization` (:135), both ignored (see the
    //! reasons). Neither covers the "Missing digest" checks of `ActionExecutionValue.create`
    //! (skyframe/ActionExecutionValue.java:70-93); the tests below them check those against the
    //! Java lines and are marked as not from Java.

    use std::any::Any;
    use std::fmt;
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::panic::AssertUnwindSafe;
    use std::sync::Arc;

    use blaze_actions::actions::action_lookup::ActionLookupKey;
    use blaze_actions::actions::artifact::SpecialArtifactType;
    use blaze_actions::{ActionLookupData, ActionLookupKeyRef, Artifact, ArtifactRoot, RootType};
    use blaze_cmdline::Label;
    use blaze_exec::actions::action_adapters::artifact_input;
    use blaze_exec::actions::{FileArtifactValue, TreeArtifactValue};
    use blaze_vfs::{DigestHashFunction, FileSystemExt, FileSystemRef, UnixFileSystem};
    use indexmap::IndexMap;

    use super::{ActionExecutionValue, MISSING_TREE_ARTIFACT};

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
            None
        }
        fn get_configuration_checksum(&self) -> Option<String> {
            None
        }
        fn function_name(&self) -> &'static str {
            "NULL_ARTIFACT_OWNER"
        }
        fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
            other.as_any().downcast_ref::<NullArtifactOwner>().is_some()
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

    // Java: ActionExecutionValueTest.java:58 ActionExecutionValueTest.OUTPUT_ROOT
    fn output_root() -> ArtifactRoot {
        let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        ArtifactRoot::as_derived_root(&fs.get_path("/execroot"), RootType::OUTPUT, &["out"])
    }

    // Java: ActionExecutionValueTest.java:214 ActionExecutionValueTest.output
    fn output(root: &ArtifactRoot, root_relative_path: &str) -> Artifact {
        Artifact::create_derived_artifact(
            root.clone(),
            root.get_exec_path().get_relative_str(root_relative_path),
            ActionLookupKeyRef::new(NullArtifactOwner),
        )
    }

    // Java: ActionExecutionValueTest.java:229 ActionExecutionValueTest.tree
    fn tree(root: &ArtifactRoot, root_relative_path: &str) -> Artifact {
        let result = Artifact::create_special_artifact(
            root.clone(),
            root.get_exec_path().get_relative_str(root_relative_path),
            ActionLookupKeyRef::new(NullArtifactOwner),
            SpecialArtifactType::TREE,
        );
        // ACTION_LOOKUP_DATA_1 = ActionLookupData.create(KEY, 1)
        result.set_generating_action_key(ActionLookupData::create(
            ActionLookupKeyRef::new(NullArtifactOwner),
            1,
        ));
        result
    }

    fn panic_message(f: impl FnOnce()) -> String {
        let payload = std::panic::catch_unwind(AssertUnwindSafe(f))
            .expect_err("expected a panic, nothing was thrown");
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    }

    // Java: ActionExecutionValueTest.java:62 ActionExecutionValueTest.equality
    #[test]
    #[ignore = "ActionExecutionValue.equals (the equals of Java's SingleOutputFile, \
                MultiOutputFile, SingleTree, MultiTree, WithRichData and ModuleDiscovering \
                subclasses), FilesetOutputTree rich data and discoveredModules are not ported"]
    fn equality() {
        blaze_util::unported!("ActionExecutionValue.equals");
    }

    // Java: ActionExecutionValueTest.java:135 ActionExecutionValueTest.serialization
    #[test]
    #[ignore = "Skyframe serialization (SerializationTester, the ActionExecutionValue codecs) is \
                not ported"]
    fn serialization() {
        blaze_util::unported!("SerializationTester");
    }

    /// Package I2: `ActionExecutionValueTest.equality`'s groups of output files, over
    /// `Key::values_equal`, with local file values (remote values never prune here, see
    /// `keys::file_values_equal_for_pruning`).
    // Java: ActionExecutionValueTest.java:62 ActionExecutionValueTest.equality
    #[test]
    fn values_equal_of_output_files() {
        use crate::keys::ActionExecutionKey;
        use blaze_skyframe::Key;
        use blaze_skyframe::testing::values_equal_tester;
        let root = output_root();
        let local = |d: u8| FileArtifactValue::create_for_normal_file(Some(vec![d]), None, 1);
        let with = |entries: &[(&str, FileArtifactValue)]| {
            ActionExecutionValue::create(
                entries
                    .iter()
                    .map(|(name, value)| (output(&root, name), value.clone()))
                    .collect(),
                IndexMap::new(),
                None,
            )
        };
        values_equal_tester::<ActionExecutionKey>(&[
            vec![with(&[("file1", local(1))]), with(&[("file1", local(1))])],
            vec![with(&[("file1", local(2))])],
            vec![with(&[("file2", local(1))])],
            vec![with(&[("file1", local(1)), ("file2", local(2))])],
        ]);
        let remote = with(&[(
            "file1",
            FileArtifactValue::create_for_remote_file(vec![1], 1, 1),
        )]);
        assert!(!ActionExecutionKey::values_equal(&remote, &remote));
    }

    /// Package I2: an output artifact recomputed to equal local metadata prunes its parent; a
    /// completion value (a Java singleton) always does.
    #[test]
    fn recomputed_equal_artifact_and_completion_prune_parent() {
        use crate::artifact_function::ArtifactValue;
        use crate::completion_function::TargetCompletionValue;
        use crate::keys::{ArtifactKey, TargetCompletionKey};
        use blaze_skyframe::testing::parent_rebuilt;
        let root = output_root();
        let key = ArtifactKey(output(&root, "file1"));
        let local = |d: u8| {
            ArtifactValue::File(FileArtifactValue::create_for_normal_file(
                Some(vec![d]),
                None,
                1,
            ))
        };
        assert!(!parent_rebuilt(key.clone(), local(1), local(1)));
        assert!(parent_rebuilt(key, local(1), local(2)));
        let completion = TargetCompletionKey {
            action_lookup_key: ActionLookupKeyRef::new(NullArtifactOwner),
            output_groups: vec![],
            will_test: false,
        };
        assert!(!parent_rebuilt(
            completion,
            TargetCompletionValue::default(),
            TargetCompletionValue::default()
        ));
    }

    /// Not a Java test: the first `checkArgument` of `ActionExecutionValue.create`
    /// (skyframe/ActionExecutionValue.java:72-75).
    #[test]
    fn create_rejects_tree_child_in_artifact_data() {
        let root = output_root();
        let tree = tree(&root, "tree");
        let child = Artifact::create_tree_output_str(&tree, "file1");
        let mut data = IndexMap::new();
        data.insert(
            child.clone(),
            FileArtifactValue::create_for_remote_file(vec![1], 1, 1),
        );
        assert_eq!(
            panic_message(|| {
                ActionExecutionValue::create(data, IndexMap::new(), None);
            }),
            format!(
                "IllegalArgumentException: {child} should only be stored in a TreeArtifactValue"
            )
        );
    }

    /// Not a Java test: the "Missing digest" check of `ActionExecutionValue.create` for an output
    /// file (skyframe/ActionExecutionValue.java:76-79).
    #[test]
    fn create_rejects_file_without_digest() {
        let root = output_root();
        let file = output(&root, "file1");
        let mut data = IndexMap::new();
        data.insert(
            file.clone(),
            FileArtifactValue::create_for_normal_file(None, None, 1),
        );
        assert_eq!(
            panic_message(|| {
                ActionExecutionValue::create(data, IndexMap::new(), None);
            }),
            format!("IllegalArgumentException: Missing digest for {file}")
        );
    }

    /// Not a Java test: a value that is not a file needs no digest
    /// (skyframe/ActionExecutionValue.java:77 `!value.getType().isFile()`).
    #[test]
    fn create_accepts_non_file_without_digest() {
        let root = output_root();
        let mut data = IndexMap::new();
        data.insert(
            output(&root, "dir"),
            FileArtifactValue::create_for_directory_with_mtime(0),
        );
        let value = ActionExecutionValue::create(data, IndexMap::new(), None);
        assert_eq!(value.get_all_file_values().len(), 1);
    }

    /// Not a Java test: the "Missing digest" check for a tree artifact child
    /// (skyframe/ActionExecutionValue.java:80-92).
    #[test]
    fn create_rejects_tree_child_without_digest() {
        let root = output_root();
        let tree = tree(&root, "tree");
        let child = Artifact::create_tree_output_str(&tree, "file1");
        let tree_value = TreeArtifactValue {
            child_values: vec![(
                artifact_input(&child),
                FileArtifactValue::create_for_normal_file(None, None, 1),
            )],
            archived_representation: None,
            metadata: None,
            resolved_path: None,
        };
        let mut tree_data = IndexMap::new();
        tree_data.insert(tree.clone(), Arc::new(tree_value));
        assert_eq!(
            panic_message(|| {
                ActionExecutionValue::create(IndexMap::new(), tree_data, None);
            }),
            format!(
                "IllegalArgumentException: Missing digest for file {child} in tree artifact {tree}"
            )
        );
    }

    /// Not a Java test: `MISSING_TREE_ARTIFACT.getChildValues()` throws
    /// (skyframe/TreeArtifactValue.java:445) when `ActionExecutionValue.create` visits the tree
    /// values (skyframe/ActionExecutionValue.java:81-83).
    #[test]
    fn create_with_missing_tree_artifact_throws() {
        let root = output_root();
        let mut tree_data = IndexMap::new();
        tree_data.insert(tree(&root, "tree"), MISSING_TREE_ARTIFACT.clone());
        assert_eq!(
            panic_message(|| {
                ActionExecutionValue::create(IndexMap::new(), tree_data, None);
            }),
            "UnsupportedOperationException: MISSING_TREE_ARTIFACT"
        );
    }
}
