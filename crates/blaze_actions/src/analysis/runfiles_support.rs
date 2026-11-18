//! Port of `lib/analysis/RunfilesSupport.java`.
//!
//! PORT: Java's factory methods take a `RuleContext` / `ActionConstructionContext`, which live
//! in the analysis layer above this crate. Here they take a
//! [`RunfilesActionConstructionContext`] (the `ActionConstructionContext` and
//! `BuildConfigurationValue` values used) and, for [`RunfilesSupport::with_executable`], a
//! [`RunfilesSupportRuleValues`] (the values read from the `RuleContext`). The analysis lead
//! implements the trait and fills the struct. Actions are registered through
//! [`RunfilesActionConstructionContext::register_action`] in Java's order.

use std::sync::{Arc, Weak};

use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_util::check_state;
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use crate::actions::action::ActionRef;
use crate::actions::action_environment::ActionEnvironment;
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::actions::artifact_root::ArtifactRoot;
use crate::actions::command_line::{self, CommandLineRef};
use crate::actions::runfiles_tree::{RunfilesMapping, RunfilesTree, RunfilesTreeRef};
use crate::actions::runfiles_tree_action::RunfilesTreeAction;
use crate::analysis::actions::symlink_tree_action::{RunfileSymlinksMode, SymlinkTreeAction};
use crate::analysis::repo_mapping_manifest_action::{
    RepoMappingManifestAction, RepoMappingPackageMetadata,
};
use crate::analysis::runfiles::{Runfiles, RunfilesBuilder};
use crate::analysis::source_manifest_action::{ManifestType, SourceManifestAction};
use crate::analysis::symlink_entry::SymlinkEntry;
use blaze_util::Fingerprint;

// Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:83 RunfilesSupport.RUNFILES_DIR_EXT
const RUNFILES_DIR_EXT: &str = ".runfiles";
// Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:84 RunfilesSupport.INPUT_MANIFEST_EXT
pub const INPUT_MANIFEST_EXT: &str = ".runfiles_manifest";
// Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:85 RunfilesSupport.OUTPUT_MANIFEST_BASENAME
const OUTPUT_MANIFEST_BASENAME: &str = "MANIFEST";
// Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:86 RunfilesSupport.REPO_MAPPING_MANIFEST_EXT
const REPO_MAPPING_MANIFEST_EXT: &str = ".repo_mapping";

/// The values of Java's `ActionConstructionContext` (usually a `RuleContext`) and of its
/// `BuildConfigurationValue` that the runfiles actions read. The analysis lead implements this
/// over its rule context.
pub trait RunfilesActionConstructionContext {
    /// `context.getActionOwner()`.
    fn get_action_owner(&self) -> ActionOwner;

    /// `context.getBinDirectory()`.
    fn get_bin_directory(&self) -> ArtifactRoot;

    /// `context.getDerivedArtifact(rootRelativePath, root)`.
    fn get_derived_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact;

    /// `context.getAnalysisEnvironment().getRunfilesArtifact(rootRelativePath, root)`: a
    /// `SpecialArtifact` of type `RUNFILES`.
    fn get_runfiles_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact;

    /// `context.registerAction(action)` / `context.getAnalysisEnvironment().registerAction(...)`.
    fn register_action(&self, action: ActionRef);

    /// `context.getConfiguration().getRunfileSymlinksMode()`.
    fn get_runfile_symlinks_mode(&self) -> RunfileSymlinksMode;

    /// `context.getConfiguration().buildRunfileManifests()`.
    fn build_runfile_manifests(&self) -> bool;

    /// `context.getConfiguration().buildRunfileLinks()`.
    fn build_runfile_links(&self) -> bool;

    /// `context.getConfiguration().isSiblingRepositoryLayout()`.
    fn is_sibling_repository_layout(&self) -> bool;

    /// `context.getConfiguration().remotableSourceManifestActions()`.
    fn remotable_source_manifest_actions(&self) -> bool;

    /// `context.getConfiguration().getActionEnvironment()` (passed to `SymlinkTreeAction`).
    fn get_configuration_action_environment(&self) -> ActionEnvironment;

    /// `context.getConfiguration().getWorkspaceName()` (passed to `SymlinkTreeAction`).
    fn get_configuration_workspace_name(&self) -> String;
}

/// The values `RunfilesSupport.create` reads from the `RuleContext` itself.
pub struct RunfilesSupportRuleValues {
    /// `ruleContext.getWorkspaceName()`.
    pub workspace_name: String,
    /// `cacheRunfilesMappings(ruleContext)`; compute with
    /// [`RunfilesSupport::cache_runfiles_mappings`].
    pub cache_runfiles_mappings: bool,
    /// When `getConfiguration().getRunUnder()` is a `LabelRunUnder` and the rule is a test rule
    /// (`TargetUtils.isTestRule(ruleContext.getRule())`): the runfiles of
    /// `ruleContext.getRunUnderPrerequisite()`, computed with
    /// [`RunfilesSupport::get_runfiles_of_target`]. Otherwise `None`.
    pub run_under_runfiles: Option<Runfiles>,
    /// `ruleContext.getTransitivePackagesForRunfileRepoMappingManifest()`, each package
    /// converted to a [`RepoMappingPackageMetadata`]; `None` where Java returns null.
    pub transitive_packages_for_runfile_repo_mapping_manifest:
        Option<NestedSet<RepoMappingPackageMetadata>>,
    /// `getConfiguration().getOptions().get(CoreOptions.class).compactRepoMapping`.
    pub compact_repo_mapping: bool,
}

/// The implementation of [`RunfilesTree`].
// Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:90 RunfilesSupport.RunfilesTreeImpl
#[derive(Debug)]
pub struct RunfilesTreeImpl {
    exec_path: PathFragment,
    runfiles: Runfiles,
    repo_mapping_manifest: Option<Artifact>,

    /// The cached runfiles mapping: `None` if caching is not desired, otherwise a weak
    /// reference to the cached value (dangling if not computed or dropped).
    ///
    /// PORT: Java uses a `WeakReference` cleared by the GC; a Rust `Weak` is cleared when the
    /// last user drops the mapping. Only memory use differs.
    cached_mapping: Option<Mutex<Weak<RunfilesMapping>>>,

    build_runfile_links: bool,
    runfile_symlinks_mode: RunfileSymlinksMode,
}

impl RunfilesTreeImpl {
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:118 RunfilesTreeImpl.<init>
    fn new(
        exec_path: PathFragment,
        runfiles: Runfiles,
        repo_mapping_manifest: Option<Artifact>,
        build_runfile_links: bool,
        cache_mapping: bool,
        runfile_symlinks_mode: RunfileSymlinksMode,
    ) -> RunfilesTreeImpl {
        RunfilesTreeImpl {
            exec_path,
            runfiles,
            repo_mapping_manifest,
            build_runfile_links,
            runfile_symlinks_mode,
            cached_mapping: if cache_mapping {
                Some(Mutex::new(Weak::new()))
            } else {
                None
            },
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:118 RunfilesTreeImpl.<init>
    pub fn new_for_testing(exec_path: PathFragment, runfiles: Runfiles) -> RunfilesTreeImpl {
        RunfilesTreeImpl::new(
            exec_path,
            runfiles,
            /* repo_mapping_manifest= */ None,
            /* build_runfile_links= */ false,
            /* cache_mapping= */ false,
            RunfileSymlinksMode::CREATE,
        )
    }

    /// The runfiles of this tree.
    pub fn get_runfiles(&self) -> &Runfiles {
        &self.runfiles
    }
}

impl RunfilesTree for RunfilesTreeImpl {
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:145 RunfilesTreeImpl.getExecPath
    fn get_exec_path(&self) -> PathFragment {
        self.exec_path.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:150 RunfilesTreeImpl.getMapping
    fn get_mapping(&self) -> Arc<RunfilesMapping> {
        let Some(cached_mapping) = &self.cached_mapping else {
            return Arc::new(
                self.runfiles
                    .get_runfiles_inputs(self.repo_mapping_manifest.as_ref()),
            );
        };

        let mut cached = cached_mapping.lock();
        if let Some(result) = cached.upgrade() {
            return result;
        }
        let result = Arc::new(
            self.runfiles
                .get_runfiles_inputs(self.repo_mapping_manifest.as_ref()),
        );
        *cached = Arc::downgrade(&result);
        result
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:173 RunfilesTreeImpl.getArtifacts
    fn get_artifacts(&self) -> NestedSet<Artifact> {
        self.runfiles.get_all_artifacts()
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:215 RunfilesTreeImpl.getSymlinksMode
    fn get_symlinks_mode(&self) -> RunfileSymlinksMode {
        self.runfile_symlinks_mode
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:220 RunfilesTreeImpl.isBuildRunfileLinks
    fn is_build_runfile_links(&self) -> bool {
        self.build_runfile_links
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:225 RunfilesTreeImpl.getWorkspaceName
    fn get_workspace_name(&self) -> String {
        self.runfiles.get_prefix().to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:178 RunfilesTreeImpl.getArtifactsAtCanonicalLocationsForLogging
    fn get_artifacts_at_canonical_locations_for_logging(&self) -> NestedSet<Artifact> {
        self.runfiles.get_artifacts().clone()
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:183 RunfilesTreeImpl.getEmptyFilenamesForLogging
    fn get_empty_filenames_for_logging(&self) -> Vec<PathFragment> {
        self.runfiles.get_empty_filenames()
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:188 RunfilesTreeImpl.getSymlinksForLogging
    fn get_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry> {
        self.runfiles.get_symlinks().clone()
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:193 RunfilesTreeImpl.getRootSymlinksForLogging
    fn get_root_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry> {
        self.runfiles.get_root_symlinks().clone()
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:199 RunfilesTreeImpl.getRepoMappingManifestForLogging
    fn get_repo_mapping_manifest_for_logging(&self) -> Option<Artifact> {
        self.repo_mapping_manifest.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:204 RunfilesTreeImpl.isMappingCached
    fn is_mapping_cached(&self) -> bool {
        self.cached_mapping.is_some()
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:209 RunfilesTreeImpl.fingerprint
    fn fingerprint(
        &self,
        action_key_context: &ActionKeyContext,
        fp: &mut Fingerprint,
        digest_absolute_paths: bool,
    ) {
        self.runfiles
            .fingerprint(action_key_context, fp, digest_absolute_paths);
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:230 RunfilesTreeImpl.containsConstantMetadata
    fn contains_constant_metadata(&self) -> bool {
        if let Some(cached_mapping) = &self.cached_mapping {
            if let Some(mapping) = cached_mapping.lock().upgrade() {
                return mapping
                    .values()
                    .any(|artifact| artifact.as_ref().is_some_and(|a| a.is_constant_metadata()));
            }
        }
        self.get_artifacts()
            .to_list()
            .iter()
            .any(|a| a.is_constant_metadata())
    }
}

/// This class manages the creation of the runfiles symlink farms.
///
/// For executables that might depend on the existence of files at run-time, we create a symlink
/// farm: a directory which contains symlinks to the right locations for those runfiles.
///
/// The runfiles symlink farm serves two purposes. The first is to allow programs (and
/// programmers) to refer to files using their workspace-relative paths, regardless of whether
/// the files were source files or generated files, and regardless of which part of the package
/// path they came from. The second purpose is to ensure that all run-time dependencies are
/// explicitly declared in the BUILD files; programs may only use files which the build system
/// knows that they depend on.
///
/// The symlink farm contains a MANIFEST file which describes its contents. The MANIFEST file
/// lists the names and contents of all of the symlinks in the symlink farm. For efficiency,
/// Blaze's dependency analysis ignores the actual symlinks and just looks at the MANIFEST file.
// Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:82 RunfilesSupport
#[derive(Debug)]
pub struct RunfilesSupport {
    runfiles_tree: Arc<RunfilesTreeImpl>,
    runfiles_input_manifest: Option<Artifact>,
    runfiles_manifest: Option<Artifact>,
    runfiles_tree_artifact: Artifact,
    owning_executable: Artifact,
    /// Always a `FlatCommandLine` (`CommandLine.empty()` or `CommandLine.of(args)`).
    args: CommandLineRef,
    action_environment: ActionEnvironment,
}

impl RunfilesSupport {
    /// Only cache mappings if there is a chance that more than one action will use it within a
    /// single build. This helps reduce peak memory usage, especially when the value of --jobs is
    /// high, but avoids the additional overhead of a weak reference when it is not needed.
    ///
    /// - `is_test_rule`: `TargetUtils.isTestRule(ruleContext.getTarget())`
    /// - `runs_per_test`: `TestActionBuilder.getRunsPerTest(ruleContext)` (only called for
    ///   test rules)
    /// - `shard_count`: `TestActionBuilder.getShardCount(ruleContext)` (only called for test
    ///   rules with one run per test)
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:254 RunfilesSupport.cacheRunfilesMappings
    pub fn cache_runfiles_mappings(
        is_test_rule: bool,
        runs_per_test: impl FnOnce() -> i32,
        shard_count: impl FnOnce() -> i32,
    ) -> bool {
        if !is_test_rule {
            // Runfiles trees of non-test rules are tools and can thus be used by multiple
            // actions.
            return true;
        }

        // Test runfiles are only used by a single test runner action unless there are multiple
        // runs or shards.
        if runs_per_test() > 1 {
            return true;
        }

        if shard_count() > 1 {
            return true;
        }

        false
    }

    /// Creates the RunfilesSupport helper with the given executable and runfiles.
    ///
    /// - `ctx`: the rule context to create the runfiles support for (its action construction
    ///   and configuration values)
    /// - `rule`: the values read from the rule context itself
    /// - `executable`: the executable for whose runfiles this runfiles support is responsible
    /// - `runfiles`: the runfiles
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:280 RunfilesSupport.create
    fn create(
        ctx: &dyn RunfilesActionConstructionContext,
        rule: &RunfilesSupportRuleValues,
        executable: Artifact,
        runfiles: Runfiles,
        args: CommandLineRef,
        action_environment: ActionEnvironment,
    ) -> RunfilesSupport {
        let mut runfiles = runfiles;
        let runfile_symlinks_mode = ctx.get_runfile_symlinks_mode();
        let build_runfile_manifests = ctx.build_runfile_manifests();
        let build_runfile_links = ctx.build_runfile_links();

        // Adding run_under target to the runfiles manifest so it would become part
        // of runfiles tree and would be executable everywhere.
        if let Some(run_under_runfiles) = &rule.run_under_runfiles {
            runfiles = RunfilesBuilder::new(&rule.workspace_name)
                .merge(run_under_runfiles)
                .merge(&runfiles)
                .build();
        }
        check_state!(!runfiles.is_empty(), "Empty runfiles");

        let repo_mapping_manifest =
            RunfilesSupport::create_repo_mapping_manifest_action(ctx, rule, &runfiles, &executable);

        let runfiles_tree_artifact =
            RunfilesSupport::declare_runfiles_tree_artifact(ctx, &executable);

        let runfiles_input_manifest;
        let runfiles_manifest;
        if build_runfile_manifests {
            let input_manifest =
                RunfilesSupport::create_runfiles_input_manifest_artifact(ctx, &executable);
            runfiles_manifest = Some(RunfilesSupport::create_runfiles_action(
                ctx,
                &runfiles,
                &runfiles_tree_artifact,
                build_runfile_links,
                &input_manifest,
                repo_mapping_manifest.as_ref(),
            ));
            runfiles_input_manifest = Some(input_manifest);
        } else {
            runfiles_input_manifest = None;
            runfiles_manifest = None;
        }

        let runfiles_tree = Arc::new(RunfilesTreeImpl::new(
            runfiles_tree_artifact.get_exec_path().clone(),
            runfiles,
            repo_mapping_manifest.clone(),
            build_runfile_links,
            rule.cache_runfiles_mappings,
            runfile_symlinks_mode,
        ));

        let runfiles_tree_ref: RunfilesTreeRef = runfiles_tree.clone();
        RunfilesSupport::create_runfiles_tree_artifact_action(
            ctx,
            &runfiles_tree_artifact,
            runfiles_tree_ref,
            runfiles_manifest.as_ref(),
            repo_mapping_manifest.as_ref(),
        );

        RunfilesSupport {
            runfiles_tree,
            runfiles_input_manifest,
            runfiles_manifest,
            runfiles_tree_artifact,
            owning_executable: executable,
            args,
            action_environment,
        }
    }

    /// Returns the executable owning this RunfilesSupport.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:367 RunfilesSupport.getExecutable
    pub fn get_executable(&self) -> &Artifact {
        &self.owning_executable
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:371 RunfilesSupport.getRunfiles
    pub fn get_runfiles(&self) -> &Runfiles {
        &self.runfiles_tree.runfiles
    }

    /// Helper method that returns a collection of artifacts that are necessary for the runfiles
    /// of the given target. Note that the runfile symlink tree is never built, so this may
    /// include artifacts that end up not being used (see [`Runfiles`]).
    ///
    /// - `default_runfiles`: `target.getProvider(RunfilesProvider.class).getDefaultRunfiles()`,
    ///   `None` if the target has no `RunfilesProvider`
    /// - `files_to_build`: `target.getProvider(FileProvider.class).getFilesToBuild()` (only
    ///   called when there is no `RunfilesProvider`)
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:371 RunfilesSupport.getRunfiles
    pub fn get_runfiles_of_target(
        default_runfiles: Option<Runfiles>,
        files_to_build: impl FnOnce() -> NestedSet<Artifact>,
        workspace_name: &str,
    ) -> Runfiles {
        match default_runfiles {
            Some(runfiles) => runfiles,
            None => RunfilesBuilder::new(workspace_name)
                .add_transitive_artifacts(&files_to_build())
                .build(),
        }
    }

    /// Returns the .runfiles_manifest file outside of the runfiles symlink farm. Returns `None`
    /// if --nobuild_runfile_manifests is in effect.
    ///
    /// The MANIFEST file represents the contents of all of the symlinks in the symlink farm. For
    /// efficiency, Blaze's dependency analysis ignores the actual symlinks and just looks at the
    /// MANIFEST file. It is an invariant that the MANIFEST file should accurately represent the
    /// contents of the symlinks whenever the MANIFEST file is present.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:403 RunfilesSupport.getRunfilesInputManifest
    pub fn get_runfiles_input_manifest(&self) -> Option<&Artifact> {
        self.runfiles_input_manifest.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:410 RunfilesSupport.createRunfilesInputManifestArtifact
    fn create_runfiles_input_manifest_artifact(
        context: &dyn RunfilesActionConstructionContext,
        owning_executable: &Artifact,
    ) -> Artifact {
        let relative_path =
            owning_executable.get_output_dir_relative_path(context.is_sibling_repository_layout());
        let basename = relative_path.get_base_name();
        let input_manifest_path = relative_path
            .replace_name(&format!("{}{}", basename, INPUT_MANIFEST_EXT))
            .expect("NullPointerException");
        context.get_derived_artifact(&input_manifest_path, &context.get_bin_directory())
    }

    /// Returns the MANIFEST file in the runfiles symlink farm if Bazel is run with
    /// --build_runfile_links. Returns the .runfiles_manifest file outside of the symlink farm,
    /// if Bazel is run with --nobuild_runfile_links. Returns `None` if
    /// --nobuild_runfile_manifests is passed.
    ///
    /// Beware: In most cases [`RunfilesSupport::get_runfiles_input_manifest`] is the more
    /// appropriate function.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:426 RunfilesSupport.getRunfilesManifest
    pub fn get_runfiles_manifest(&self) -> Option<&Artifact> {
        self.runfiles_manifest.as_ref()
    }

    /// Returns the foo.repo_mapping file if Bazel is run with transitive package tracking turned
    /// on and any of the transitive packages come from a repository with strict deps.
    /// Otherwise, returns `None`.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:437 RunfilesSupport.getRepoMappingManifest
    pub fn get_repo_mapping_manifest(&self) -> Option<&Artifact> {
        self.runfiles_tree.repo_mapping_manifest.as_ref()
    }

    /// Returns the root directory of the runfiles symlink farm.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:443 RunfilesSupport.getRunfilesDirectory
    pub fn get_runfiles_directory(&self) -> Path {
        self.runfiles_tree_artifact.get_path()
    }

    /// Returns the runfiles tree artifact that depends on getExecutable(),
    /// getRunfilesManifest(), and getRunfilesSymlinkTargets(). Anything which needs to actually
    /// run the executable should depend on this.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:452 RunfilesSupport.getRunfilesTreeArtifact
    pub fn get_runfiles_tree_artifact(&self) -> &Artifact {
        &self.runfiles_tree_artifact
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:458 RunfilesSupport.declareRunfilesTreeArtifact
    fn declare_runfiles_tree_artifact(
        rule_context: &dyn RunfilesActionConstructionContext,
        owning_executable: &Artifact,
    ) -> Artifact {
        let executable_root_relative_path = owning_executable.get_root_relative_path();
        let runfiles_root_relative_path = executable_root_relative_path
            .replace_name(&format!(
                "{}{}",
                executable_root_relative_path.get_base_name(),
                RUNFILES_DIR_EXT
            ))
            .expect("NullPointerException");
        rule_context
            .get_runfiles_artifact(&runfiles_root_relative_path, owning_executable.get_root())
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:469 RunfilesSupport.createRunfilesTreeArtifactAction
    pub fn create_runfiles_tree_artifact_action(
        context: &dyn RunfilesActionConstructionContext,
        runfiles_tree_artifact: &Artifact,
        runfiles_tree: RunfilesTreeRef,
        runfiles_manifest: Option<&Artifact>,
        repo_mapping_manifest: Option<&Artifact>,
    ) {
        let mut contents_builder: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
        contents_builder.add_transitive(&runfiles_tree.get_artifacts());
        if let Some(runfiles_manifest) = runfiles_manifest {
            contents_builder.add(runfiles_manifest.clone());
        }
        if let Some(repo_mapping_manifest) = repo_mapping_manifest {
            contents_builder.add(repo_mapping_manifest.clone());
        }

        let contents = contents_builder.build();

        let runfiles_tree_action = RunfilesTreeAction::new(
            context.get_action_owner(),
            runfiles_tree,
            contents,
            vec![runfiles_tree_artifact.clone()],
        );
        context.register_action(Arc::new(runfiles_tree_action));
    }

    /// Creates a runfiles action for all of the specified files, and returns the output artifact
    /// (the artifact for the MANIFEST file).
    ///
    /// The "runfiles" action creates a symlink farm that links all the runfiles (which may come
    /// from different places, e.g. different package paths, generated files, etc.) into a
    /// single tree, so that programs can access them using the workspace-relative name.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:504 RunfilesSupport.createRunfilesAction
    fn create_runfiles_action(
        context: &dyn RunfilesActionConstructionContext,
        runfiles: &Runfiles,
        runfiles_tree_artifact: &Artifact,
        create_symlinks: bool,
        input_manifest: &Artifact,
        repo_mapping_manifest: Option<&Artifact>,
    ) -> Artifact {
        // Compute the names of the runfiles directory and its MANIFEST file.
        context.register_action(Arc::new(SourceManifestAction::new(
            Arc::new(ManifestType::SOURCE_SYMLINKS),
            context.get_action_owner(),
            input_manifest.clone(),
            runfiles.clone(),
            repo_mapping_manifest.cloned(),
            context.remotable_source_manifest_actions(),
        )));

        if !create_symlinks {
            // Just return the manifest if that's all the build calls for.
            return input_manifest.clone();
        }

        let runfiles_dir = runfiles_tree_artifact
            .get_output_dir_relative_path(context.is_sibling_repository_layout());
        let output_manifest_path = runfiles_dir.get_relative_str(OUTPUT_MANIFEST_BASENAME);

        let output_manifest =
            context.get_derived_artifact(&output_manifest_path, &context.get_bin_directory());
        context.register_action(Arc::new(SymlinkTreeAction::new(
            context.get_action_owner(),
            input_manifest.clone(),
            Some(runfiles.clone()),
            output_manifest.clone(),
            repo_mapping_manifest.cloned(),
            context.get_configuration_action_environment(),
            context.get_runfile_symlinks_mode(),
            context.get_configuration_workspace_name(),
        )));
        output_manifest
    }

    /// Returns the unmodifiable list of expanded and tokenized 'args' attribute values.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:547 RunfilesSupport.getArgs
    pub fn get_args(&self) -> &CommandLineRef {
        &self.args
    }

    /// Returns the immutable environment from the 'env' and 'env_inherit' attribute values.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:552 RunfilesSupport.getActionEnvironment
    pub fn get_action_environment(&self) -> &ActionEnvironment {
        &self.action_environment
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:556 RunfilesSupport.createSymlinkTree
    pub fn create_symlink_tree(
        rule_context: &dyn RunfilesActionConstructionContext,
        runfiles_tree_artifact: &Artifact,
        runfiles: &Runfiles,
    ) {
        // We always want symlinks to be created because that's the point of a symlink tree.
        let build_runfiles_links = true;
        let runfiles_tree: RunfilesTreeRef = Arc::new(RunfilesTreeImpl::new(
            runfiles_tree_artifact.get_exec_path().clone(),
            runfiles.clone(),
            None,
            build_runfiles_links,
            false,
            rule_context.get_runfile_symlinks_mode(),
        ));
        let root_relative_path = runfiles_tree_artifact.get_root_relative_path();
        let manifest_path = root_relative_path
            .replace_name(&format!(
                "{}.symlink_tree_manifest",
                root_relative_path.get_base_name()
            ))
            .expect("NullPointerException");
        let input_manifest =
            rule_context.get_derived_artifact(&manifest_path, &rule_context.get_bin_directory());
        let runfiles_manifest = RunfilesSupport::create_runfiles_action(
            rule_context,
            runfiles,
            runfiles_tree_artifact,
            build_runfiles_links,
            &input_manifest,
            None,
        );
        RunfilesSupport::create_runfiles_tree_artifact_action(
            rule_context,
            runfiles_tree_artifact,
            runfiles_tree,
            Some(&runfiles_manifest),
            None,
        );
    }

    /// Creates and returns a [`RunfilesSupport`] object for the given rule and executable. Note
    /// that this method calls back into the passed in rule to obtain the runfiles.
    ///
    /// If the executable is a test, runfiles mappings are cached and re-used between shards.
    /// It's a win since when there is a large number of test shards and/or runs per test, the
    /// same runfiles tree is needed many times.
    ///
    /// PORT: Java computes `args` with `computeArgs(ruleContext)` and the environment with
    /// `computeActionEnvironment(ruleContext, runEnvironmentInfo)`; the caller computes them
    /// with [`RunfilesSupport::compute_args`] and
    /// [`RunfilesSupport::compute_action_environment`] first.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:588 RunfilesSupport.withExecutable
    pub fn with_executable(
        ctx: &dyn RunfilesActionConstructionContext,
        rule: &RunfilesSupportRuleValues,
        runfiles: Runfiles,
        executable: Artifact,
        args: CommandLineRef,
        action_environment: ActionEnvironment,
    ) -> RunfilesSupport {
        RunfilesSupport::create(ctx, rule, executable, runfiles, args, action_environment)
    }

    /// `computeArgs(ruleContext)`.
    ///
    /// - `args_attr_defined`: `ruleContext.getRule().isAttrDefined("args", Types.STRING_LIST)`
    /// - `tokenized_args`: `ruleContext.getExpander().withDataLocations().tokenized("args")`
    ///   (only called when the attribute is defined)
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:598 RunfilesSupport.computeArgs
    pub fn compute_args(
        args_attr_defined: bool,
        tokenized_args: impl FnOnce() -> Vec<String>,
    ) -> CommandLineRef {
        if !args_attr_defined {
            // Some non-_binary rules create RunfilesSupport instances; it is fine to not have an
            // args attribute here.
            return command_line::empty();
        }
        let args = tokenized_args();
        if args.is_empty() {
            command_line::empty()
        } else {
            command_line::of(args)
        }
    }

    /// `computeActionEnvironment(ruleContext, runEnvironmentInfo)`.
    ///
    /// - `run_environment_info`: `(getEnvironment(), getInheritedEnvironment())` of the
    ///   `RunEnvironmentInfo`, if any
    /// - `is_native_rule`:
    ///   `ruleContext.getRule().getRuleClassObject().getRuleDefinitionEnvironmentLabel() == null`
    /// - `env_attr_defined` / `env_inherit_attr_defined`: `isAttrDefined("env",
    ///   Types.STRING_DICT)` / `isAttrDefined("env_inherit", Types.STRING_LIST)`
    /// - `expanded_env`: the `env` attribute's entries in attribute order, each value expanded
    ///   with `ruleContext.getExpander().withDataLocations().expand("env", value)` (only called
    ///   when `env` is defined and the rule is native)
    /// - `env_inherit`: the `env_inherit` attribute value (only called when defined)
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:618 RunfilesSupport.computeActionEnvironment
    pub fn compute_action_environment(
        run_environment_info: Option<(IndexMap<String, String>, Vec<String>)>,
        is_native_rule: bool,
        env_attr_defined: bool,
        env_inherit_attr_defined: bool,
        expanded_env: impl FnOnce() -> Vec<(String, String)>,
        env_inherit: impl FnOnce() -> Vec<String>,
    ) -> ActionEnvironment {
        if let Some((environment, inherited_environment)) = run_environment_info {
            // Must be a Starlark rule.
            return ActionEnvironment::create(
                environment,
                inherited_environment
                    .into_iter()
                    .collect::<IndexSet<String>>(),
            );
        }

        if !is_native_rule {
            return ActionEnvironment::Empty;
        }

        if !env_attr_defined && !env_inherit_attr_defined {
            return ActionEnvironment::Empty;
        }

        // TreeMap<String, String>
        let mut fixed_env: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        let mut inherited_env: IndexSet<String> = IndexSet::new();
        if env_attr_defined {
            for (key, value) in expanded_env() {
                fixed_env.insert(key, value);
            }
        }
        if env_inherit_attr_defined {
            for key in env_inherit() {
                if !fixed_env.contains_key(&key) {
                    inherited_env.insert(key);
                }
            }
        }
        ActionEnvironment::create(fixed_env.into_iter().collect(), inherited_env)
    }

    /// Returns the exec path of the `.runfiles` directory for the given executable.
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:655 RunfilesSupport.runfilesDirExecPath
    pub fn runfiles_dir_exec_path(executable: &Artifact) -> PathFragment {
        let executable_path = executable.get_exec_path();
        executable_path
            .replace_name(&format!(
                "{}{}",
                executable_path.get_base_name(),
                RUNFILES_DIR_EXT
            ))
            .expect("NullPointerException")
    }

    /// Returns the exec path of the corresponding `.runfiles_manifest` file for the given
    /// `.runfiles` directory.
    ///
    /// The input manifest is produced by [`SourceManifestAction`] and is an input to
    /// [`SymlinkTreeAction`].
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:667 RunfilesSupport.inputManifestExecPath
    pub fn input_manifest_exec_path(runfiles_dir_exec_path: &PathFragment) -> PathFragment {
        // Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:173 FileSystemUtils.replaceExtension
        runfiles_dir_exec_path
            .replace_name(&format!(
                "{}{}",
                remove_extension(runfiles_dir_exec_path.get_base_name()),
                INPUT_MANIFEST_EXT
            ))
            .expect("NullPointerException")
    }

    /// Returns the exec path of the corresponding `MANIFEST` file for the given `.runfiles`
    /// directory.
    ///
    /// The output manifest is a symlink to the input manifest. It is located in the `.runfiles`
    /// directory and is the output of [`SymlinkTreeAction`].
    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:679 RunfilesSupport.outputManifestExecPath
    pub fn output_manifest_exec_path(runfiles_dir_exec_path: &PathFragment) -> PathFragment {
        runfiles_dir_exec_path.get_relative_str(OUTPUT_MANIFEST_BASENAME)
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:686 RunfilesSupport.createRepoMappingManifestAction
    fn create_repo_mapping_manifest_action(
        rule_context: &dyn RunfilesActionConstructionContext,
        rule: &RunfilesSupportRuleValues,
        runfiles: &Runfiles,
        owning_executable: &Artifact,
    ) -> Option<Artifact> {
        let Some(transitive_packages) = &rule.transitive_packages_for_runfile_repo_mapping_manifest
        else {
            // If transitive packages are not tracked for repo mapping manifest, we don't need the
            // action.
            return None;
        };

        // PORT: Java falls back to `ruleContext.getPackageDirectory().getRelative(label name)`
        // for a null executable; `create` never passes null (it checks the executable first).
        let executable_path = owning_executable
            .get_output_dir_relative_path(rule_context.is_sibling_repository_layout());
        let repo_mapping_manifest = rule_context.get_derived_artifact(
            &executable_path
                .replace_name(&format!(
                    "{}{}",
                    executable_path.get_base_name(),
                    REPO_MAPPING_MANIFEST_EXT
                ))
                .expect("NullPointerException"),
            &rule_context.get_bin_directory(),
        );
        rule_context.register_action(Arc::new(RepoMappingManifestAction::new(
            rule_context.get_action_owner(),
            repo_mapping_manifest.clone(),
            transitive_packages.clone(),
            runfiles.get_artifacts().clone(),
            runfiles.get_symlinks(),
            runfiles.get_root_symlinks().clone(),
            rule.workspace_name.clone(),
            rule.compact_repo_mapping,
        )));
        Some(repo_mapping_manifest)
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:719 RunfilesSupport.getRunfilesTree
    pub fn get_runfiles_tree(&self) -> RunfilesTreeRef {
        self.runfiles_tree.clone()
    }
}

/// `FileSystemUtils.removeExtension(String)`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:128 FileSystemUtils.removeExtension
fn remove_extension(filename: &str) -> &str {
    let Some(last_dot_index) = filename.rfind('.') else {
        return filename;
    };
    if let Some(last_slash_index) = filename.rfind('/') {
        if last_slash_index > last_dot_index {
            return filename;
        }
    }
    &filename[..last_dot_index]
}
