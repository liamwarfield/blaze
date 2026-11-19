//! Port of the non-Skyframe parts of `packages/PackageFactory.java`: creating package builders,
//! the non-Skyframe globber, executing a BUILD file into a builder, and the post-load checks.
//!
//! `PackageFactory.checkBuildSyntax` (glob prefetch patterns and generator names) is ported in
//! `blaze_starlark::syntax` (`CompiledFile::build_facts`). Agent D's PACKAGE function calls
//! [`PackageFactory::new_package_builder`], [`PackageFactory::execute_build_file`] and
//! [`PackageFactory::after_done_loading_package`].

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_events::EventHandler;
use blaze_events::ExtendedEventHandler;
use blaze_starlark::EvalError;
use blaze_starlark::Location;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::eval::CompiledFile;
use blaze_starlark::eval::EvalOptions;
use blaze_starlark::loader::LoadMap;
use blaze_vfs::Path;
use blaze_vfs::RootedPath;
use indexmap::IndexMap;
use starlark::environment::Globals;

use crate::build::glob_cache::CachingPackageLocator;
use crate::build::glob_cache::GlobCache;
use crate::build::glob_cache::IgnoredSubdirectoriesMatcher;
use crate::build::glob_cache::NonSkyframeGlobber;
use crate::exceptions::DetailedExitCode;
use crate::exceptions::PackageLoadingCode;
use crate::native_rules::RuleClassProvider;
use crate::package::ConfigSettingVisibilityPolicy;
use crate::package::Globber;
use crate::package::GlobberOperation;
use crate::package::InvalidPackageException;
use crate::package::Package;
use crate::package::PackageSettings;
use crate::package::PackageValidator;
use crate::package::TargetDefinitionContext;

/// The BUILD file evaluation was interrupted (Java `InterruptedException`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("interrupted")]
pub struct BuildFileInterrupted;

/// Creates packages (Java `PackageFactory`).
// Java: com/google/devtools/build/lib/packages/PackageFactory.java:84 PackageFactory
pub struct PackageFactory {
    rule_class_provider: Arc<RuleClassProvider>,
    max_directories_to_eagerly_visit_in_globbing: i32,
    package_settings: PackageSettings,
    package_validator: Arc<dyn PackageValidator>,
}

impl PackageFactory {
    /// Constructs a `PackageFactory` instance with a specific glob path translator and rule
    /// factory.
    // PORT: the globbing executor, package overhead estimator and loading listener are not
    // ported (single build per process; memory accounting; metrics).
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:145 PackageFactory.<init>
    pub fn new(
        rule_class_provider: Arc<RuleClassProvider>,
        package_settings: PackageSettings,
        package_validator: Arc<dyn PackageValidator>,
    ) -> Self {
        PackageFactory {
            rule_class_provider,
            max_directories_to_eagerly_visit_in_globbing: 0,
            package_settings,
            package_validator,
        }
    }

    /// Sets the max number of directories to eagerly traverse on the first glob for a given
    /// package (`--experimental_max_directories_to_eagerly_visit_in_globbing`); `-2` enables
    /// the glob prefetch of [`PackageFactory::execute_build_file`].
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:201 PackageFactory.setMaxDirectoriesToEagerlyVisitInGlobbing
    pub fn set_max_directories_to_eagerly_visit_in_globbing(
        &mut self,
        max_directories_to_eagerly_visit_in_globbing: i32,
    ) {
        self.max_directories_to_eagerly_visit_in_globbing =
            max_directories_to_eagerly_visit_in_globbing;
    }

    /// Returns the immutable, unordered set of names of all the known rule classes.
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:207 PackageFactory.getRuleClassProvider
    pub fn get_rule_class_provider(&self) -> &Arc<RuleClassProvider> {
        &self.rule_class_provider
    }

    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:132 PackageFactory.getPackageSettingsForTesting
    pub fn get_package_settings(&self) -> &PackageSettings {
        &self.package_settings
    }

    /// A builder for the package `package_id` (also used by PACKAGE to create empty packages
    /// in case of error before BUILD execution).
    // PORT: the cpu-bound semaphore is not ported (single build per process).
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:215 PackageFactory.newPackageBuilder
    pub fn new_package_builder(
        &self,
        package_id: PackageIdentifier,
        filename: RootedPath,
        associated_module_name: Option<String>,
        associated_module_version: Option<String>,
        starlark_semantics: &StarlarkSemantics,
        repository_mapping: RepositoryMapping,
        main_repository_mapping: RepositoryMapping,
        generator_map: Option<IndexMap<Location, String>>,
        config_setting_visibility_policy: Option<ConfigSettingVisibilityPolicy>,
        globber: Option<Arc<dyn Globber>>,
    ) -> TargetDefinitionContext {
        Package::new_package_builder(
            self.package_settings,
            package_id,
            filename,
            self.rule_class_provider
                .get_runfiles_prefix()
                .expect("setRunfilesPrefix was not called")
                .to_owned(),
            associated_module_name,
            associated_module_version,
            starlark_semantics.incompatible_no_implicit_file_export,
            starlark_semantics.incompatible_simplify_unconditional_selects_in_rule_attrs,
            repository_mapping,
            main_repository_mapping,
            generator_map,
            config_setting_visibility_policy,
            globber,
            /* enable_name_conflict_checking= */ true,
            /* track_full_macro_information= */ true,
            self.package_validator.get_package_limits(),
        )
    }

    /// A builder for a package assembled from package pieces (lazy macro expansion).
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:252 PackageFactory.newPackageFromPackagePiecesBuilder
    pub fn new_package_from_package_pieces_builder(
        &self,
        metadata: Arc<crate::package::PackageMetadata>,
        declarations: Arc<crate::package::PackageDeclarations>,
        starlark_semantics: &StarlarkSemantics,
        main_repository_mapping: RepositoryMapping,
        generator_map: Option<IndexMap<Location, String>>,
        config_setting_visibility_policy: Option<ConfigSettingVisibilityPolicy>,
        globber: Option<Arc<dyn Globber>>,
        build_file: Arc<crate::target::InputFile>,
    ) -> TargetDefinitionContext {
        // PORT: Java passes configSettingVisibilityPolicy here but the metadata (from the BUILD
        // file piece) already carries it; Package.newPackageFromPackagePiecesBuilder ignores it.
        let _ = config_setting_visibility_policy;
        TargetDefinitionContext::new_package_from_package_pieces_builder(
            self.package_settings,
            metadata,
            declarations,
            starlark_semantics.incompatible_no_implicit_file_export,
            starlark_semantics.incompatible_simplify_unconditional_selects_in_rule_attrs,
            main_repository_mapping,
            generator_map,
            globber,
            /* enable_name_conflict_checking= */ true,
            /* track_full_macro_information= */ true,
            self.package_validator.get_package_limits(),
            build_file,
        )
    }

    /// A builder for the BUILD file piece of a package (lazy macro expansion).
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:283 PackageFactory.newPackagePieceForBuildFileBuilder
    pub fn new_package_piece_for_build_file_builder(
        &self,
        package_piece_id: crate::package::PackagePieceIdentifierForBuildFile,
        filename: RootedPath,
        associated_module_name: Option<String>,
        associated_module_version: Option<String>,
        starlark_semantics: &StarlarkSemantics,
        repository_mapping: RepositoryMapping,
        main_repository_mapping: RepositoryMapping,
        generator_map: Option<IndexMap<Location, String>>,
        config_setting_visibility_policy: Option<ConfigSettingVisibilityPolicy>,
        globber: Option<Arc<dyn Globber>>,
    ) -> TargetDefinitionContext {
        TargetDefinitionContext::new_package_piece_for_build_file_builder(
            self.package_settings,
            package_piece_id,
            filename,
            self.rule_class_provider
                .get_runfiles_prefix()
                .expect("setRunfilesPrefix was not called")
                .to_owned(),
            associated_module_name,
            associated_module_version,
            starlark_semantics.incompatible_no_implicit_file_export,
            starlark_semantics.incompatible_simplify_unconditional_selects_in_rule_attrs,
            repository_mapping,
            main_repository_mapping,
            generator_map,
            config_setting_visibility_policy,
            globber,
            /* enable_name_conflict_checking= */ true,
            /* track_full_macro_information= */ true,
            self.package_validator.get_package_limits(),
        )
    }

    /// A builder for the piece of one symbolic macro (lazy macro expansion).
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:320 PackageFactory.newPackagePieceForMacroBuilder
    pub fn new_package_piece_for_macro_builder(
        &self,
        metadata: Arc<crate::package::PackageMetadata>,
        declarations: Arc<crate::package::PackageDeclarations>,
        macro_: Arc<crate::macros::MacroInstance>,
        parent_identifier: crate::package::PackagePieceIdentifier,
        starlark_semantics: &StarlarkSemantics,
        main_repository_mapping: RepositoryMapping,
        existing_rules_map_for_finalizer: Option<IndexMap<String, Arc<crate::target::Rule>>>,
    ) -> TargetDefinitionContext {
        TargetDefinitionContext::new_package_piece_for_macro_builder(
            metadata,
            declarations,
            macro_,
            parent_identifier,
            starlark_semantics.incompatible_simplify_unconditional_selects_in_rule_attrs,
            main_repository_mapping,
            /* enable_name_conflict_checking= */ true,
            /* track_full_macro_information= */ true,
            self.package_validator.get_package_limits(),
            existing_rules_map_for_finalizer,
        )
    }

    /// Enforces `--max_computation_steps` on a package piece (Java has no piece validation yet).
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:415 PackageFactory.afterDoneLoadingPackagePiece
    pub fn after_done_loading_package_piece(
        &self,
        pkg_piece: &crate::package::PackagePiece,
        starlark_semantics: &StarlarkSemantics,
    ) {
        // TODO(https://github.com/bazelbuild/bazel/issues/23852): add package piece validation.
        // PORT: starlark-rust does not count Java's computation steps (see
        // executeBuildFileImpl), so the limit fails loudly when set.
        let max_steps = starlark_semantics.max_computation_steps;
        if max_steps > 0 {
            blaze_util::unported!(
                "PackageFactory.afterDoneLoadingPackagePiece",
                "--max_computation_steps={max_steps} for {} (StarlarkThread.getExecutedSteps)",
                pkg_piece.get_short_description()
            );
        }
    }

    /// The non-Skyframe globber of a package (Java `createNonSkyframeGlobber`).
    // PORT: the syscall cache and thread-state receiver are not ported (see GlobCache::new).
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:347 PackageFactory.createNonSkyframeGlobber
    pub fn create_non_skyframe_globber(
        &self,
        package_directory: Path,
        package_id: PackageIdentifier,
        ignored_subdirectories: Arc<dyn IgnoredSubdirectoriesMatcher>,
        locator: Arc<dyn CachingPackageLocator>,
    ) -> NonSkyframeGlobber {
        NonSkyframeGlobber::new(Arc::new(GlobCache::new(
            package_directory,
            package_id,
            ignored_subdirectories,
            locator,
        )))
    }

    /// Runs the package validator and enforces `--max_computation_steps`.
    // PORT: the Metrics argument and the loading listener are not ported (profiling only).
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:371 PackageFactory.afterDoneLoadingPackage
    pub fn after_done_loading_package(
        &self,
        pkg: &Package,
        starlark_semantics: &StarlarkSemantics,
        event_handler: &dyn ExtendedEventHandler,
    ) -> Result<(), InvalidPackageException> {
        self.package_validator.validate(pkg, event_handler)?;

        // Enforce limit on number of compute steps in BUILD file (b/151622307).
        let max_steps = starlark_semantics.max_computation_steps;
        let steps = pkg.get_computation_steps();
        if max_steps > 0 && steps > max_steps {
            let message = format!(
                "BUILD file computation took {steps} steps, but --max_computation_steps={max_steps}"
            );
            return Err(InvalidPackageException::with_detailed_exit_code(
                pkg.get_package_identifier().clone(),
                message.clone(),
                DetailedExitCode::package_loading(
                    message,
                    PackageLoadingCode::MaxComputationStepsExceeded,
                ),
            ));
        }
        Ok(())
    }

    /// Executes a compiled BUILD file into the package builder held as the payload of `ctx`
    /// (Java `executeBuildFile`). Evaluation errors are recorded on the builder (an error event
    /// and "contains errors"), as in Java. `transitive_loads` and `transitive_bzl_digest` are
    /// what Java's `setLoads(loadedModules)` derives from the loaded modules.
    // PORT: the cpu-bound semaphore is not ported (single build per process).
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:472 PackageFactory.executeBuildFile
    pub fn execute_build_file(
        &self,
        ctx: &ThreadContext,
        build_file_program: CompiledFile,
        globals: &Globals,
        loaded_modules: &LoadMap,
        transitive_loads: Vec<Label>,
        transitive_bzl_digest: Vec<u8>,
        opts: &EvalOptions,
    ) -> Result<(), BuildFileInterrupted> {
        let globber = with_builder(ctx, |b| b.get_globber().cloned());
        // Prefetch glob patterns asynchronously.
        if self.max_directories_to_eagerly_visit_in_globbing == -2
            && let Some(globber) = &globber
        {
            let facts = &build_file_program.build_facts;
            let allow_empty = true;
            // Ignore exceptions. Errors will be properly reported when the actual globbing is
            // done.
            let _ = globber.run_async(&facts.globs, &[], GlobberOperation::Files, allow_empty);
            let _ = globber.run_async(
                &facts.globs_with_dirs,
                &[],
                GlobberOperation::FilesAndDirs,
                allow_empty,
            );
            let _ = globber.run_async(
                &facts.subpackages,
                &[],
                GlobberOperation::Subpackages,
                allow_empty,
            );
        }

        let result = self.execute_build_file_impl(
            ctx,
            build_file_program,
            globals,
            loaded_modules,
            transitive_loads,
            transitive_bzl_digest,
            opts,
        );
        if let Some(globber) = &globber {
            if result.is_err() {
                globber.on_interrupt();
            }
            globber.on_completion();
        }
        result
    }

    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:505 PackageFactory.executeBuildFileImpl
    fn execute_build_file_impl(
        &self,
        ctx: &ThreadContext,
        build_file_program: CompiledFile,
        globals: &Globals,
        loaded_modules: &LoadMap,
        transitive_loads: Vec<Label>,
        transitive_bzl_digest: Vec<u8>,
        opts: &EvalOptions,
    ) -> Result<(), BuildFileInterrupted> {
        with_builder(ctx, |b| {
            b.set_loads(transitive_loads, transitive_bzl_digest)
        });
        // PORT: the module, thread, loader and print handler are set up by
        // `blaze_starlark::eval::exec_module` from `ctx` (its events must replay into the
        // builder's local event handler; agent D supplies them). The rule definition
        // environment thread-local (for analysis_test) is the payload's rule class provider.
        // Java: `pkgBuilder.updateStartedThreadComputationSteps(thread)` adds the thread's
        // executed steps (StarlarkThread.getExecutedSteps) to the package. starlark-rust does not
        // count Java's steps, so the package's count stays 0 and `--max_computation_steps`
        // could never fire: fail loudly when the limit is set. (Bazel installs no package
        // validator that reads the count.)
        let max_steps = ctx.semantics.max_computation_steps;
        if max_steps > 0 {
            blaze_util::unported!(
                "PackageFactory.afterDoneLoadingPackage",
                "--max_computation_steps={max_steps} (StarlarkThread.getExecutedSteps)"
            );
        }
        match blaze_starlark::eval::exec_module(
            build_file_program,
            globals,
            loaded_modules,
            ctx,
            opts,
        ) {
            Ok(_module) => Ok(()),
            Err(ex) => Self::handle_eval_error(ctx, &ex),
        }
    }

    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:505 PackageFactory.executeBuildFileImpl
    fn handle_eval_error(ctx: &ThreadContext, ex: &EvalError) -> Result<(), BuildFileInterrupted> {
        if ex.kind == EvalErrorKind::Cancelled {
            // Suppress the interrupted exception: we have an error of our own to return.
            if with_builder(ctx, |b| b.contains_errors()) {
                return Ok(());
            }
            return Err(BuildFileInterrupted);
        }
        if let EvalErrorKind::Syntax(errors) = &ex.kind {
            // PORT: starlark-rust reports some static errors (undefined names) when executing;
            // Java reports them before execution, one SYNTAX_ERROR event per error.
            // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1238 PackageFunction.loadPackage
            with_builder(ctx, |b| {
                for err in errors {
                    b.get_local_event_handler().handle(&Package::error(
                        Some(&err.location),
                        &err.message,
                        PackageLoadingCode::SyntaxError,
                    ));
                }
                b.set_contains_errors();
            });
            return Ok(());
        }
        with_builder(ctx, |b| {
            b.get_local_event_handler().handle(&Package::error(
                None,
                &ex.message_with_stack(),
                PackageLoadingCode::StarlarkEvalError,
            ));
            b.set_contains_errors();
        });
        Ok(())
    }
}

/// Runs `f` on the package builder that is `ctx`'s payload.
fn with_builder<R>(ctx: &ThreadContext, f: impl FnOnce(&mut TargetDefinitionContext) -> R) -> R {
    let mut b = ctx
        .payload::<TargetDefinitionContext>()
        .expect("the BUILD thread context must hold the package builder");
    f(&mut b)
}

/// The print handler of a BUILD thread (Java `Event.makeDebugPrintHandler(handler)`): `print()`
/// output becomes a DEBUG event at the caller's location on `handler` (the package builder's
/// local event handler, so prints and errors keep their order). Warnings go to the same handler.
// Java: com/google/devtools/build/lib/events/Event.java:425 Event.makeDebugPrintHandler
pub struct DebugPrintHandler {
    handler: Arc<dyn EventHandler>,
}

impl DebugPrintHandler {
    pub fn new(handler: Arc<dyn EventHandler>) -> Self {
        DebugPrintHandler { handler }
    }
}

impl blaze_starlark::thread::StarlarkEvents for DebugPrintHandler {
    fn debug(&self, location: Option<&Location>, message: &str) {
        self.handler.handle(&blaze_events::Event::debug_at(
            location.map(crate::package::package::to_event_location),
            message,
        ));
    }

    fn warn(&self, location: Option<&Location>, message: &str) {
        self.handler.handle(&blaze_events::Event::warn_at(
            location.map(crate::package::package::to_event_location),
            message,
        ));
    }
}
