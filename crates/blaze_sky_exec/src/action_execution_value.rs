//! `ActionExecutionValue`: the metadata of an executed action's outputs.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::action::Action;
use blaze_actions::actions::artifact::OwnerlessArtifactWrapper;
use blaze_exec::actions::action_adapters::artifact_input;
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

impl ActionExecutionValue {
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:93 ActionExecutionValue.create
    pub fn create(
        artifact_data: IndexMap<Artifact, FileArtifactValue>,
        tree_artifact_data: IndexMap<Artifact, Arc<TreeArtifactValue>>,
        rich_artifact_data: Option<Arc<RunfilesArtifactValue>>,
    ) -> ActionExecutionValue {
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
    pub fn get_tree_artifact_value(&self, artifact: &Artifact) -> Option<&Arc<TreeArtifactValue>> {
        self.tree_artifact_data.get(artifact)
    }

    /// `getAllFileValues()`.
    pub fn get_all_file_values(&self) -> &IndexMap<Artifact, FileArtifactValue> {
        &self.artifact_data
    }

    /// `getAllTreeArtifactValues()`.
    pub fn get_all_tree_artifact_values(&self) -> &IndexMap<Artifact, Arc<TreeArtifactValue>> {
        &self.tree_artifact_data
    }

    /// `getRichArtifactData()` (a runfiles tree's `RunfilesArtifactValue`).
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
