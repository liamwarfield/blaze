//! Ports of `BaseCache.java` and the `Known*` id caches of `skyframe/actiongraph/v2`.
//!
//! Each cache assigns ids from 1 in first-seen order and emits the proto for a new key before
//! returning its id (children such as parent path fragments or transitive dep sets are emitted
//! first, because `createProto` requests their ids).

use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;

use blaze_collect::{NestedSet, NestedSetNode};
use blaze_protos::analysis;
use blaze_vfs::PathFragment;

use crate::actions::action_owner::{AspectDescriptor, BuildConfigurationEvent};
use crate::actions::artifact::Artifact;

use super::aquery_output_handler::AqueryOutputHandler;

/// `BaseCache`: the id table (`cache` + `nextId`).
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/BaseCache.java:24 BaseCache
pub struct BaseCache<K> {
    cache: HashMap<K, u32>,
    next_id: u32,
}

impl<K: Eq + Hash> BaseCache<K> {
    pub fn new() -> Self {
        BaseCache {
            cache: HashMap::new(),
            next_id: 1,
        }
    }

    /// Returns `(id, is_new)`: the existing id, or a fresh id the caller must emit a proto for.
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/BaseCache.java:46 BaseCache.dataToIdAndStreamOutputProto
    fn id_for(&mut self, key: K) -> (u32, bool) {
        if let Some(id) = self.cache.get(&key) {
            return (*id, false);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.cache.insert(key, id);
        (id, true)
    }
}

/// `KnownPathFragments`.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownPathFragments.java:22 KnownPathFragments
pub struct KnownPathFragments {
    base: BaseCache<PathFragment>,
}

impl KnownPathFragments {
    pub fn new() -> Self {
        KnownPathFragments {
            base: BaseCache::new(),
        }
    }

    pub fn data_to_id_and_stream_output_proto(
        &mut self,
        path_fragment: &PathFragment,
        out: &mut dyn AqueryOutputHandler,
    ) -> u32 {
        let (id, is_new) = self.base.id_for(path_fragment.clone());
        if is_new {
            // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownPathFragments.java:27 KnownPathFragments.createProto
            let mut proto = analysis::PathFragment {
                id,
                label: path_fragment.get_base_name().to_string(),
                parent_id: 0,
            };
            if has_parent(path_fragment) {
                let parent = path_fragment.get_parent_directory().unwrap();
                proto.parent_id = self.data_to_id_and_stream_output_proto(&parent, out);
            }
            out.output_path_fragment(proto);
        }
        id
    }
}

// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownPathFragments.java:48 KnownPathFragments.hasParent
fn has_parent(path_fragment: &PathFragment) -> bool {
    match path_fragment.get_parent_directory() {
        Some(parent) => !parent.get_base_name().is_empty(),
        None => false,
    }
}

/// `KnownArtifacts`.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownArtifacts.java:21 KnownArtifacts
pub struct KnownArtifacts {
    base: BaseCache<Artifact>,
    known_path_fragments: KnownPathFragments,
}

impl KnownArtifacts {
    pub fn new() -> Self {
        KnownArtifacts {
            base: BaseCache::new(),
            known_path_fragments: KnownPathFragments::new(),
        }
    }

    pub fn data_to_id_and_stream_output_proto(
        &mut self,
        artifact: &Artifact,
        out: &mut dyn AqueryOutputHandler,
    ) -> u32 {
        let (id, is_new) = self.base.id_for(artifact.clone());
        if is_new {
            // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownArtifacts.java:31 KnownArtifacts.createProto
            let path_fragment_id = self
                .known_path_fragments
                .data_to_id_and_stream_output_proto(artifact.get_exec_path(), out);
            out.output_artifact(analysis::Artifact {
                id,
                path_fragment_id,
                is_tree_artifact: artifact.is_tree_artifact(),
            });
        }
        id
    }
}

/// `KnownNestedSets`, keyed by `NestedSet.toNode()`.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownNestedSets.java:25 KnownNestedSets
pub struct KnownNestedSets {
    base: BaseCache<NestedSetNode<Artifact>>,
}

impl KnownNestedSets {
    pub fn new() -> Self {
        KnownNestedSets {
            base: BaseCache::new(),
        }
    }

    pub fn data_to_id_and_stream_output_proto(
        &mut self,
        nested_set: &NestedSet<Artifact>,
        known_artifacts: &mut KnownArtifacts,
        out: &mut dyn AqueryOutputHandler,
    ) -> u32 {
        let (id, is_new) = self.base.id_for(nested_set.to_node());
        if is_new {
            // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownNestedSets.java:39 KnownNestedSets.createProto
            let mut dep_set = analysis::DepSetOfFiles {
                id,
                transitive_dep_set_ids: Vec::new(),
                direct_artifact_ids: Vec::new(),
            };
            #[allow(clippy::mutable_key_type)]
            let mut visited: HashSet<NestedSetNode<Artifact>> = HashSet::new();
            for succ in nested_set.get_non_leaves() {
                if visited.insert(succ.to_node()) {
                    let succ_id =
                        self.data_to_id_and_stream_output_proto(&succ, known_artifacts, out);
                    dep_set.transitive_dep_set_ids.push(succ_id);
                }
            }
            for elem in nested_set.get_leaves() {
                dep_set
                    .direct_artifact_ids
                    .push(known_artifacts.data_to_id_and_stream_output_proto(&elem, out));
            }
            out.output_dep_set_of_files(dep_set);
        }
        id
    }
}

/// `KnownConfigurations`, keyed by the configuration event.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownConfigurations.java:22 KnownConfigurations
pub struct KnownConfigurations {
    // PORT: BuildConfigurationEvent has no Hash (prost messages); key on the event id and the
    // encoded payload, which is what Java's equals compares.
    base: BaseCache<(String, Vec<u8>)>,
}

impl KnownConfigurations {
    pub fn new() -> Self {
        KnownConfigurations {
            base: BaseCache::new(),
        }
    }

    pub fn data_to_id_and_stream_output_proto(
        &mut self,
        config: &BuildConfigurationEvent,
        out: &mut dyn AqueryOutputHandler,
    ) -> u32 {
        use blaze_protos::JavaMessage;
        let key = (
            config.event_id.clone(),
            config.configuration.to_byte_array(),
        );
        let (id, is_new) = self.base.id_for(key);
        if is_new {
            // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownConfigurations.java:29 KnownConfigurations.createProto
            out.output_configuration(analysis::Configuration {
                id,
                mnemonic: config.configuration.mnemonic.clone(),
                platform_name: config.configuration.platform_name.clone(),
                checksum: config.event_id.clone(),
                is_tool: config.configuration.is_tool,
                fragments: Vec::new(),
                fragment_options: Vec::new(),
            });
        }
        id
    }
}

/// `KnownAspectDescriptors`.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownAspectDescriptors.java:23 KnownAspectDescriptors
pub struct KnownAspectDescriptors {
    base: BaseCache<AspectDescriptor>,
}

impl KnownAspectDescriptors {
    pub fn new() -> Self {
        KnownAspectDescriptors {
            base: BaseCache::new(),
        }
    }

    pub fn data_to_id_and_stream_output_proto(
        &mut self,
        aspect_descriptor: &AspectDescriptor,
        out: &mut dyn AqueryOutputHandler,
    ) -> u32 {
        let (id, is_new) = self.base.id_for(aspect_descriptor.clone());
        if is_new {
            // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownAspectDescriptors.java:31 KnownAspectDescriptors.createProto
            out.output_aspect_descriptor(analysis::AspectDescriptor {
                id,
                name: aspect_descriptor.aspect_class_name.clone(),
                parameters: aspect_descriptor
                    .parameters
                    .iter()
                    .map(|(k, v)| analysis::KeyValuePair {
                        key: k.clone(),
                        value: v.clone(),
                    })
                    .collect(),
            });
        }
        id
    }
}

/// `KnownRuleClassStrings`.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownRuleClassStrings.java:22 KnownRuleClassStrings
pub struct KnownRuleClassStrings {
    base: BaseCache<String>,
}

impl KnownRuleClassStrings {
    pub fn new() -> Self {
        KnownRuleClassStrings {
            base: BaseCache::new(),
        }
    }

    pub fn data_to_id_and_stream_output_proto(
        &mut self,
        rule_class_string: &str,
        out: &mut dyn AqueryOutputHandler,
    ) -> u32 {
        let (id, is_new) = self.base.id_for(rule_class_string.to_string());
        if is_new {
            // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownRuleClassStrings.java:27 KnownRuleClassStrings.createProto
            out.output_rule_class(analysis::RuleClass {
                id,
                name: rule_class_string.to_string(),
            });
        }
        id
    }
}

/// `KnownTargets`, keyed by (label string, rule class string).
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownTargets.java:26 KnownTargets
pub struct KnownTargets {
    base: BaseCache<(String, Option<String>)>,
    known_rule_class_strings: KnownRuleClassStrings,
}

impl KnownTargets {
    pub fn new() -> Self {
        KnownTargets {
            base: BaseCache::new(),
            known_rule_class_strings: KnownRuleClassStrings::new(),
        }
    }

    pub fn data_to_id_and_stream_output_proto(
        &mut self,
        target_identifier: (&str, Option<&str>),
        out: &mut dyn AqueryOutputHandler,
    ) -> u32 {
        let key = (
            target_identifier.0.to_string(),
            target_identifier.1.map(|s| s.to_string()),
        );
        let (id, is_new) = self.base.id_for(key);
        if is_new {
            // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/KnownTargets.java:37 KnownTargets.createProto
            let mut target = analysis::Target {
                id,
                label: target_identifier.0.to_string(),
                rule_class_id: 0,
            };
            if let Some(rule_class_string) = target_identifier.1 {
                target.rule_class_id = self
                    .known_rule_class_strings
                    .data_to_id_and_stream_output_proto(rule_class_string, out);
            }
            out.output_target(target);
        }
        id
    }
}
