//! Port of Bazel's action layer (wave W5, action half): artifacts and roots, action owners and
//! keys, the analysis-time action classes (`SpawnAction`, `StarlarkAction`, file writes,
//! templates, symlinks, manifests, param files, workspace status), command lines
//! (`CustomCommandLine`, Starlark `Args`), runfiles, and the aquery dump (`ActionGraphDump`).
//!
//! Module paths follow the Java packages: `actions` is `lib/actions`, `analysis` is
//! `lib/analysis` (with `analysis::actions` and `analysis::starlark`), `skyframe::actiongraph::v2`
//! is `lib/skyframe/actiongraph/v2`.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
#![allow(clippy::collapsible_if, clippy::type_complexity)]

pub mod actions;
pub mod analysis;
pub mod bazel;
pub mod rules;
pub mod skyframe;

pub use actions::action::{
    AbstractAction, Action, ActionRef, CommandAction, ComputeKeyError, FileContentsProvider,
    InputMetadataProvider, KEY_ERROR,
};
pub use actions::action_environment::ActionEnvironment;
pub use actions::action_key_context::ActionKeyContext;
pub use actions::action_lookup::{
    ActionLookupData, ActionLookupKey, ActionLookupKeyRef, ArtifactOwner,
};
pub use actions::action_owner::{
    ActionOwner, AspectDescriptor, BuildConfigurationEvent, BuildConfigurationInfo,
};
pub use actions::artifact::{Artifact, ArtifactClass, SpecialArtifactType};
pub use actions::artifact_root::{ArtifactRoot, RootType};
pub use actions::platform_info::{PlatformInfo, PlatformInfoData, PlatformInfoRef};
