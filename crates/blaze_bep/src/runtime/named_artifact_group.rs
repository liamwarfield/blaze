//! Port of `NamedArtifactGroup`, with the `NestedSet<?>` the streamer builds from expanded
//! artifacts.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_collect::nestedset::{NestedSet, NestedSetChild, NestedSetChildren};
use blaze_exec::actions::file_artifact_value::FileArtifactValue;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_util::{check_argument, check_state};

use crate::buildeventstream::build_event::{BuildEvent, Interrupted, LocalFile, LocalFileType};
use crate::buildeventstream::build_event_context::{ArtifactGroupNode, BuildEventContext};
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::proto_chaining_with;
use crate::events::completion_context::{ArtifactReceiver, CompletionContext};
use crate::events::target_complete_event::new_file;

/// `NamedArtifactGroup.ExpandedArtifact`: an artifact whose metadata is known (tree artifacts
/// already expanded to their children).
// Java: com/google/devtools/build/lib/runtime/NamedArtifactGroup.java:186 NamedArtifactGroup.ExpandedArtifact
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpandedArtifact {
    // Java: com/google/devtools/build/lib/runtime/NamedArtifactGroup.java:189 NamedArtifactGroup.NormalExpandedArtifact
    Normal {
        artifact: Artifact,
        metadata: FileArtifactValue,
    },
    // PORT: FilesetExpandedArtifact is not ported (filesets are not ported).
}

impl ExpandedArtifact {
    // Java: com/google/devtools/build/lib/runtime/NamedArtifactGroup.java:196 NormalExpandedArtifact.<init>
    fn normal(artifact: Artifact, metadata: FileArtifactValue) -> Self {
        check_state!(
            !artifact.is_directory(),
            "artifact must be expanded: {}",
            artifact
        );
        ExpandedArtifact::Normal { artifact, metadata }
    }
}

/// A `NestedSet<?>` of the streamer: either an artifact set reported by an event, or a set the
/// streamer built whose leaves are [`ExpandedArtifact`]s.
///
/// PORT: Java's `NestedSet<?>` mixes `Artifact` and `ExpandedArtifact` elements; the Rust
/// `NestedSet` is typed, so a built set is a node of this enum. Its successors are kept in the
/// order Java's `NestedSet` constructor gives them (`STABLE_ORDER`: transitive sets, then
/// direct elements).
#[derive(Clone, Debug)]
pub enum ArtifactSet {
    Artifacts(NestedSet<Artifact>),
    Expanded(Arc<ExpandedNode>),
}

/// The successors of a built set node.
#[derive(Debug)]
pub struct ExpandedNode {
    children: Vec<ExpandedChild>,
}

#[derive(Clone, Debug)]
enum ExpandedChild {
    Leaf(ExpandedArtifact),
    Set(ArtifactSet),
}

impl ArtifactSet {
    /// `NestedSet.toNode()`.
    pub fn to_node(&self) -> ArtifactGroupNode {
        match self {
            ArtifactSet::Artifacts(set) => ArtifactGroupNode::Artifacts(set.to_node()),
            ArtifactSet::Expanded(node) => {
                ArtifactGroupNode::Expanded(Arc::as_ptr(node) as *const () as usize)
            }
        }
    }

    /// `NestedSet.getNonLeaves()`.
    pub fn get_non_leaves(&self) -> Vec<ArtifactSet> {
        match self {
            ArtifactSet::Artifacts(set) => set
                .get_non_leaves()
                .into_iter()
                .map(ArtifactSet::Artifacts)
                .collect(),
            ArtifactSet::Expanded(node) => node
                .children
                .iter()
                .filter_map(|c| match c {
                    ExpandedChild::Set(s) => Some(s.clone()),
                    ExpandedChild::Leaf(_) => None,
                })
                .collect(),
        }
    }

    /// The leaves of a built set (`NestedSet.getLeaves()`).
    fn expanded_leaves(&self) -> Vec<ExpandedArtifact> {
        match self {
            ArtifactSet::Artifacts(_) => {
                panic!("IllegalStateException: leaves of an unexpanded set")
            }
            ArtifactSet::Expanded(node) => node
                .children
                .iter()
                .filter_map(|c| match c {
                    ExpandedChild::Leaf(l) => Some(l.clone()),
                    ExpandedChild::Set(_) => None,
                })
                .collect(),
        }
    }

    /// The successors of this set, as Java's `children` array.
    fn successors(&self) -> Vec<Successor> {
        match self {
            ArtifactSet::Artifacts(set) => match set.children() {
                NestedSetChildren::Empty => Vec::new(),
                NestedSetChildren::Leaf(a) => vec![Successor::Artifact(a)],
                NestedSetChildren::Array(children) => children
                    .into_iter()
                    .map(|c| match c {
                        NestedSetChild::Leaf(a) => Successor::Artifact(a),
                        NestedSetChild::Set(s) => Successor::Set(ArtifactSet::Artifacts(s)),
                    })
                    .collect(),
            },
            ArtifactSet::Expanded(node) => node
                .children
                .iter()
                .map(|c| match c {
                    ExpandedChild::Leaf(l) => Successor::Expanded(l.clone()),
                    ExpandedChild::Set(s) => Successor::Set(s.clone()),
                })
                .collect(),
        }
    }

    /// `NestedSet.splitIfExceedsMaximumSize(maxDegree)`: a set with the same elements whose
    /// node has at most `max_degree` successors.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSet.java:666 NestedSet.splitIfExceedsMaximumSize
    pub fn split_if_exceeds_maximum_size(&self, max_degree: i32) -> ArtifactSet {
        check_argument!(max_degree >= 2, "maxDegree must be at least 2");
        let ArtifactSet::Expanded(node) = self else {
            panic!("IllegalStateException: only built sets are split");
        };
        let nsuccs = node.children.len();
        let max_degree = max_degree as usize;
        if nsuccs <= max_degree {
            return self.clone();
        }
        // TODO(adonovan): (preexisting): if the last piece is a singleton, it must be inlined.
        let pieces: Vec<ExpandedChild> = node
            .children
            .chunks(max_degree)
            .map(|piece| {
                ExpandedChild::Set(ArtifactSet::Expanded(Arc::new(ExpandedNode {
                    children: piece.to_vec(),
                })))
            })
            .collect();
        ArtifactSet::Expanded(Arc::new(ExpandedNode { children: pieces }))
            .split_if_exceeds_maximum_size(max_degree as i32)
    }
}

enum Successor {
    Artifact(Artifact),
    Expanded(ExpandedArtifact),
    Set(ArtifactSet),
}

/// A build event introducing a set of artifacts to be referred to later by its name. Those
/// events are generated by the `BuildEventStreamer` upon seeing an `EventReportingArtifacts`,
/// if necessary.
// Java: com/google/devtools/build/lib/runtime/NamedArtifactGroup.java:46 NamedArtifactGroup
#[derive(Debug)]
pub struct NamedArtifactGroup {
    name: String,
    completion_context: Arc<CompletionContext>,
    /// Of `Artifact` or `ExpandedArtifact`; all its leaf successors are expanded.
    set: ArtifactSet,
}

impl NamedArtifactGroup {
    // Java: com/google/devtools/build/lib/runtime/NamedArtifactGroup.java:55 NamedArtifactGroup.<init>
    pub fn new(name: String, completion_context: Arc<CompletionContext>, set: ArtifactSet) -> Self {
        NamedArtifactGroup {
            name,
            completion_context,
            set,
        }
    }

    /// Given a set whose leaf successors are `Artifact` and `ExpandedArtifact`, returns a new
    /// set whose leaf successors are all `ExpandedArtifact`. Non-leaf successors are unaltered.
    // Java: com/google/devtools/build/lib/runtime/NamedArtifactGroup.java:136 NamedArtifactGroup.expandSet
    pub fn expand_set(ctx: &CompletionContext, artifacts: &ArtifactSet) -> ArtifactSet {
        struct Receiver<'a>(&'a mut Vec<ExpandedArtifact>);
        impl ArtifactReceiver for Receiver<'_> {
            fn accept(&mut self, artifact: &Artifact, metadata: &FileArtifactValue) {
                self.0
                    .push(ExpandedArtifact::normal(artifact.clone(), metadata.clone()));
            }
            fn accept_fileset_mapping(&mut self, _fileset: &Artifact) {
                blaze_util::unported!("NamedArtifactGroup.expandSet (FilesetExpandedArtifact)");
            }
        }

        // NestedSetBuilder<Object> res = NestedSetBuilder.newBuilder(Order.STABLE_ORDER)
        let mut direct: Vec<ExpandedArtifact> = Vec::new();
        let mut non_leaves: Vec<ArtifactSet> = Vec::new();
        for elem in artifacts.successors() {
            match elem {
                Successor::Expanded(expanded) => direct.push(expanded),
                Successor::Artifact(artifact) => {
                    ctx.visit_artifacts([&artifact], &mut Receiver(&mut direct));
                }
                Successor::Set(set) => non_leaves.push(set),
            }
        }
        // The builder's direct elements are a set.
        let mut unique_direct: Vec<ExpandedArtifact> = Vec::with_capacity(direct.len());
        for d in direct {
            if !unique_direct.contains(&d) {
                unique_direct.push(d);
            }
        }
        let no_directs = unique_direct.is_empty();

        // NestedSet(STABLE_ORDER, direct, transitive): transitive successors, then direct ones.
        let mut children: Vec<ExpandedChild> =
            Vec::with_capacity(non_leaves.len() + unique_direct.len());
        children.extend(non_leaves.iter().cloned().map(ExpandedChild::Set));
        children.extend(unique_direct.into_iter().map(ExpandedChild::Leaf));
        let result = if children.len() == 1
            && let ExpandedChild::Set(only) = &children[0]
        {
            // If we ended up wrapping exactly one other set, dereference it.
            only.clone()
        } else {
            ArtifactSet::Expanded(Arc::new(ExpandedNode { children }))
        };

        // If we have executed one call to res.addTransitive(x) and none to res.add, then
        // res.build() simply returns x ("inlining"), which may violate our postcondition on
        // elements. (This can happen if 'artifacts' contains one non-leaf successor and one or
        // more leaf successors for which visitArtifacts is a no-op.)
        //
        // In that case we need to recursively apply the expansion. Ugh.
        if no_directs && non_leaves.len() == 1 {
            return Self::expand_set(ctx, &result);
        }
        result
    }
}

impl BuildEvent for NamedArtifactGroup {
    // Java: com/google/devtools/build/lib/runtime/NamedArtifactGroup.java:62 NamedArtifactGroup.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::from_artifact_group_name(&self.name)
    }

    // Java: com/google/devtools/build/lib/runtime/NamedArtifactGroup.java:67 NamedArtifactGroup.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/runtime/NamedArtifactGroup.java:72 NamedArtifactGroup.referencedLocalFiles
    fn referenced_local_files(&self) -> Vec<LocalFile> {
        self.set
            .expanded_leaves()
            .into_iter()
            .map(|elem| match elem {
                ExpandedArtifact::Normal { artifact, metadata } => LocalFile::new(
                    self.completion_context.to_path(&artifact),
                    LocalFileType::for_artifact(&artifact, Some(&metadata)),
                    Some(metadata),
                ),
            })
            .collect()
    }

    // Java: com/google/devtools/build/lib/runtime/NamedArtifactGroup.java:96 NamedArtifactGroup.asStreamProto
    fn as_stream_proto(
        &self,
        converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let path_converter = converters.path_converter();
        let namer = converters.artifact_group_namer();

        let mut builder = bes::NamedSetOfFiles::default();
        for elem in self.set.expanded_leaves() {
            let file = match elem {
                ExpandedArtifact::Normal { artifact, metadata } => {
                    let uri = path_converter.apply(&self.completion_context.to_path(&artifact));
                    new_file(&artifact, &metadata, uri)
                }
            };
            // Omit files with unknown contents (e.g. if uploading failed).
            if file.file.is_some() {
                builder.files.push(file);
            }
        }

        for succ in self.set.get_non_leaves() {
            // Java: builder.addFileSets(namer.apply(succ.toNode())); null throws.
            builder.file_sets.push(
                namer
                    .apply(&succ.to_node())
                    .expect("NullPointerException: unnamed artifact set"),
            );
        }
        Ok(proto_chaining_with(self, Payload::NamedSetOfFiles(builder)))
    }
}
