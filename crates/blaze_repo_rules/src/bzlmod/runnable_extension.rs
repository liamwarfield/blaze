//! The two "flavors" of module extensions: the "regular", which is declared using
//! `module_extension` in a .bzl file; and the "innate", which is fabricated from usages of
//! `use_repo_rule` in MODULE.bazel files.
//!
//! The general idiom is to "load" such a [`RunnableExtension`] by getting as much information
//! about it as needed to determine whether it can be reused from the lockfile (hence
//! [`RunnableExtension::get_eval_factors`] and [`RunnableExtension::get_bzl_transitive_digest`]).
//! Then [`RunnableExtension::run`] can be called if the cached results in the lockfile cannot be
//! reused and the extension has to be re-run.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/RunnableExtension.java,
//! RegularRunnableExtension.java, InnateRunnableExtension.java and
//! ModuleExtensionEvaluationProgress.java.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use blaze_bzlmod_model::ExternalDepsCode;
use blaze_bzlmod_model::ExternalDepsException;
use blaze_bzlmod_model::ModuleExtensionEvalFactors;
use blaze_bzlmod_model::ModuleExtensionId;
use blaze_bzlmod_model::ModuleKey;
use blaze_bzlmod_model::RepoSpec;
use blaze_bzlmod_model::Tag;
use blaze_cmdline::Label;
use blaze_cmdline::LabelRepoContextImpl;
use blaze_cmdline::LabelSimpleRepoMappingRecorder;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::label_constants::MODULE_EXTENSION_WORKING_DIRECTORY_LOCATION;
use blaze_events::Event;
use blaze_events::FetchProgress;
use blaze_events::Postable;
use blaze_options::spelling::SpellChecker;
use blaze_packages::types::label_converter::LabelConverter;
use blaze_packages::types::label_converter::SharedRepoMappingRecorder;
use blaze_sky_loading::bzl_load::check_valid_load_label;
use blaze_sky_loading::starlark_util::event_location;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_starlark::EvalError;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::error::StackEntry;
use indexmap::IndexMap;
use indexmap::IndexSet;
use parking_lot::Mutex;
use starlark::environment::FrozenModule;
use starlark::environment::Module;
use starlark::values::FrozenHeap;
use starlark::values::FrozenValue;
use starlark::values::OwnedFrozenValue;
use starlark::values::Value;
use starlark::values::ValueLike;

use super::eval_thread_context::ModuleExtensionEvalStarlarkThreadContext;
use super::facts::FactsValue;
use super::module_extension::ModuleExtension;
use super::module_extension_context::ModuleCtxFields;
use super::module_extension_context::ModuleExtensionContext;
use super::module_extension_metadata::ModuleExtensionMetadata;
use super::single_extension_usages::SingleExtensionUsagesValue;
use super::starlark_bazel_module;
use super::starlark_bazel_module::starlark_location;
use crate::attr_values::attribute_value_to_starlark;
use crate::bzl_load::BzlLoadValue;
use crate::bzl_load::load_bzl_for_bzlmod;
use crate::env_handle::SkyEnv;
use crate::precomputed::RequireRepoExtensionMetadataMode;
use crate::repository::repo_environment_function::get_environment_view;
use crate::repository::repo_recorded_input::RepoRecordedInputWithValue;
use crate::services::RepoServices;
use crate::starlark::base_external_context::CloseError;
use crate::starlark::context::EvaluationAbort;
use crate::starlark::context::ExternalContext;
use crate::starlark::context::ExternalContextKind;
use crate::starlark::context::ModuleCtx;
use crate::starlark::repository_module::module_extension_from_global;
use crate::starlark::repository_module::repo_rule_from_global;
use crate::starlark::thread::call_implementation;

// ---------------------------------------------------------------------------------------------
// ModuleExtensionEvaluationProgress

/// Reports the progress of the evaluation of a module extension.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvaluationProgress.java:23 ModuleExtensionEvaluationProgress
#[derive(Debug)]
pub struct ModuleExtensionEvaluationProgress {
    extension_id: ModuleExtensionId,
    finished: bool,
    message: String,
}

impl ModuleExtensionEvaluationProgress {
    /// Returns the unique identifying string for a module extension evaluation event.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvaluationProgress.java:30 ModuleExtensionEvaluationProgress.moduleExtensionEvaluationContextString
    pub fn module_extension_evaluation_context_string(extension_id: &ModuleExtensionId) -> String {
        format!("module extension {extension_id}")
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvaluationProgress.java:34 ModuleExtensionEvaluationProgress.ongoing
    pub fn ongoing(extension_id: &ModuleExtensionId, message: &str) -> Self {
        ModuleExtensionEvaluationProgress {
            extension_id: extension_id.clone(),
            finished: false,
            message: message.to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvaluationProgress.java:39 ModuleExtensionEvaluationProgress.finished
    pub fn finished(extension_id: &ModuleExtensionId) -> Self {
        ModuleExtensionEvaluationProgress {
            extension_id: extension_id.clone(),
            finished: true,
            message: "finished.".to_owned(),
        }
    }
}

impl Postable for ModuleExtensionEvaluationProgress {
    fn as_fetch_progress(&self) -> Option<&dyn FetchProgress> {
        Some(self)
    }
}

impl FetchProgress for ModuleExtensionEvaluationProgress {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvaluationProgress.java:50 ModuleExtensionEvaluationProgress.getResourceIdentifier
    fn get_resource_identifier(&self) -> String {
        Self::module_extension_evaluation_context_string(&self.extension_id)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvaluationProgress.java:55 ModuleExtensionEvaluationProgress.getProgress
    fn get_progress(&self) -> String {
        self.message.clone()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvaluationProgress.java:60 ModuleExtensionEvaluationProgress.isFinished
    fn is_finished(&self) -> bool {
        self.finished
    }
}

// ---------------------------------------------------------------------------------------------
// RunnableExtension

/// What loading or running an extension throws (Java `InterruptedException` or
/// `ExternalDepsException`).
#[derive(Debug)]
pub enum ExtensionError {
    Interrupted(Interrupted),
    ExternalDeps(ExternalDepsException),
}

impl From<Interrupted> for ExtensionError {
    fn from(i: Interrupted) -> Self {
        ExtensionError::Interrupted(i)
    }
}

impl From<ExternalDepsException> for ExtensionError {
    fn from(e: ExternalDepsException) -> Self {
        ExtensionError::ExternalDeps(e)
    }
}

/// Holds the result data from running a module extension.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RunnableExtension.java:63 RunnableExtension.RunModuleExtensionResult
#[derive(Debug, Clone)]
pub struct RunModuleExtensionResult {
    pub recorded_inputs: Vec<RepoRecordedInputWithValue>,
    pub generated_repo_specs: IndexMap<String, RepoSpec>,
    pub module_extension_metadata: ModuleExtensionMetadata,
}

/// An internal abstraction to support the two "flavors" of module extensions.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RunnableExtension.java:38 RunnableExtension
pub enum RunnableExtension {
    Regular(RegularRunnableExtension),
    Innate(InnateRunnableExtension),
}

impl RunnableExtension {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RunnableExtension.java:39 RunnableExtension.getEvalFactors
    pub fn get_eval_factors(&self) -> ModuleExtensionEvalFactors {
        match self {
            RunnableExtension::Regular(e) => e.get_eval_factors(),
            RunnableExtension::Innate(e) => e.get_eval_factors(),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/RunnableExtension.java:41 RunnableExtension.getBzlTransitiveDigest
    pub fn get_bzl_transitive_digest(&self) -> Vec<u8> {
        match self {
            RunnableExtension::Regular(e) => e.get_bzl_transitive_digest(),
            RunnableExtension::Innate(e) => e.get_bzl_transitive_digest(),
        }
    }

    /// The current schema version of the facts produced by this extension. Persisted in the
    /// lockfile alongside the facts and compared against the value before the extension runs: if
    /// they differ, the persisted facts are discarded and the extension is invoked with empty
    /// facts.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RunnableExtension.java:48 RunnableExtension.getFactsVersion
    pub fn get_facts_version(&self) -> i32 {
        match self {
            RunnableExtension::Regular(e) => e.get_facts_version(),
            RunnableExtension::Innate(e) => e.get_facts_version(),
        }
    }

    /// Runs the extension. Returns `Ok(None)` if a Skyframe restart is required.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RunnableExtension.java:52 RunnableExtension.run
    pub fn run(
        &self,
        env: &mut Environment<'_>,
        usages_value: &SingleExtensionUsagesValue,
        starlark_semantics: &Arc<StarlarkSemantics>,
        extension_id: &ModuleExtensionId,
        main_repository_mapping: &RepositoryMapping,
        facts: &blaze_bzlmod_model::Facts,
        require_repo_extension_metadata_mode: RequireRepoExtensionMetadataMode,
    ) -> Result<Option<RunModuleExtensionResult>, ExtensionError> {
        match self {
            RunnableExtension::Regular(e) => e.run(
                env,
                usages_value,
                starlark_semantics,
                extension_id,
                main_repository_mapping,
                facts,
                require_repo_extension_metadata_mode,
            ),
            RunnableExtension::Innate(e) => e.run(env, usages_value, extension_id),
        }
    }
}

/// Java `Module.getGlobal(name)` on a loaded .bzl: the value of a global defined by the file,
/// public or private; a name the file only binds by `load` is not a global.
// Java: net/starlark/java/eval/Module.java:280 Module.getGlobal
fn get_global(module: &FrozenModule, name: &str) -> Option<OwnedFrozenValue> {
    module.java_get_global(name)
}

/// Java `Module.getGlobals().keySet()` of a loaded .bzl: its globals (private ones included,
/// load bindings excluded) in binding order.
// Java: net/starlark/java/eval/Module.java:216 Module.getGlobals
fn global_names(module: &FrozenModule) -> Vec<String> {
    module
        .java_globals()
        .map(|n| n.as_str().to_owned())
        .collect()
}

/// `Starlark.type(x)` of a frozen value.
fn starlark_type(value: &OwnedFrozenValue) -> String {
    value.value().to_value().get_type().to_owned()
}

// ---------------------------------------------------------------------------------------------
// RegularRunnableExtension

/// A regular module extension defined with `module_extension` and used with `use_extension`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:71 RegularRunnableExtension
pub struct RegularRunnableExtension {
    bzl_load_value: Arc<BzlLoadValue>,
    extension: Arc<ModuleExtension>,
    static_env_vars: BTreeMap<String, Option<String>>,
    /// Java `directories`, `repoEnv`, `nonstrictRepoEnv`, `timeoutScaling`, `processWrapper`,
    /// `downloadManager`.
    services: Arc<RepoServices>,
}

impl RegularRunnableExtension {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:106 RegularRunnableExtension.loadBzlFile
    fn load_bzl_file(
        bzl_file_label: &Label,
        sample_usage_location: &blaze_events::Location,
        starlark_semantics: &StarlarkSemantics,
        env: &mut Environment<'_>,
    ) -> Result<Option<Arc<BzlLoadValue>>, ExtensionError> {
        // Check that the .bzl label isn't crazy.
        if let Err(e) = check_valid_load_label(bzl_file_label, starlark_semantics) {
            return Err(ExternalDepsException::with_cause_and_message(
                ExternalDepsCode::BadModule,
                e,
                "invalid module extension label",
            )
            .into());
        }

        // Load the .bzl file pointed to by the label.
        match load_bzl_for_bzlmod(env, bzl_file_label, false)? {
            None => Ok(None),
            Some(Ok(v)) => Ok(Some(v)),
            Some(Err(e)) => {
                let message = format!(
                    "Error loading '{}' for module extensions, requested by {}: {}",
                    bzl_file_label,
                    sample_usage_location,
                    e.get_message()
                );
                Err(ExternalDepsException::with_cause_and_message(
                    ExternalDepsCode::BadModule,
                    e,
                    &message,
                )
                .into())
            }
        }
    }

    /// Returns `Ok(None)` if a Skyframe restart is required.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:141 RegularRunnableExtension.load
    pub fn load(
        extension_id: &ModuleExtensionId,
        usages_value: &SingleExtensionUsagesValue,
        starlark_semantics: &StarlarkSemantics,
        env: &mut Environment<'_>,
        services: &Arc<RepoServices>,
    ) -> Result<Option<RegularRunnableExtension>, ExtensionError> {
        let sample_usage = usages_value
            .get_extension_usages()
            .values()
            .next()
            .expect("NoSuchElementException: an extension is only evaluated with usages");
        let sample_usage_location = sample_usage.proxies[0].location.clone();
        let Some(bzl_load_value) = Self::load_bzl_file(
            extension_id.bzl_file_label(),
            &sample_usage_location,
            starlark_semantics,
            env,
        )?
        else {
            return Ok(None);
        };
        // TODO(wyv): Consider whether there's a need to check .bzl load visibility
        // (BzlLoadFunction#checkLoadVisibilities).
        // TODO(wyv): Consider refactoring to use PackageFunction#loadBzlModules, or the simpler
        // API that may be created by b/237658764.

        // Check that the .bzl file actually exports a module extension by our name.
        let module = bzl_load_value.get_module();
        let extension = get_global(module, extension_id.extension_name())
            .and_then(|exported| module_extension_from_global(&exported));
        let Some(extension) = extension else {
            let exported_extensions: Vec<String> = global_names(module)
                .into_iter()
                .filter(|name| {
                    get_global(module, name)
                        .and_then(|v| module_extension_from_global(&v))
                        .is_some()
                })
                .collect();
            return Err(ExternalDepsException::with_message(
                ExternalDepsCode::BadModule,
                format!(
                    "{} does not export a module extension called {}, yet its use is requested \
                     at {}{}",
                    extension_id.bzl_file_label(),
                    extension_id.extension_name(),
                    sample_usage_location,
                    SpellChecker::did_you_mean(extension_id.extension_name(), &exported_extensions)
                ),
            )
            .into());
        };

        let env_variables: IndexSet<String> = extension.env_variables.iter().cloned().collect();
        let Some(static_env_vars) = get_environment_view(env, &env_variables)? else {
            return Ok(None);
        };
        Ok(Some(RegularRunnableExtension {
            bzl_load_value,
            extension,
            static_env_vars,
            services: services.clone(),
        }))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:203 RegularRunnableExtension.getEvalFactors
    pub fn get_eval_factors(&self) -> ModuleExtensionEvalFactors {
        // OS.getCurrent().toString() and the JVM's os.arch, for Blaze's only host platform
        // (Linux x86-64).
        ModuleExtensionEvalFactors::create(
            if self.extension.os_dependent {
                "linux"
            } else {
                ""
            },
            if self.extension.arch_dependent {
                "amd64"
            } else {
                ""
            },
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:210 RegularRunnableExtension.getBzlTransitiveDigest
    pub fn get_bzl_transitive_digest(&self) -> Vec<u8> {
        self.bzl_load_value
            .get_module_context()
            .bzl_transitive_digest
            .clone()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:215 RegularRunnableExtension.getFactsVersion
    pub fn get_facts_version(&self) -> i32 {
        self.extension.facts_version
    }

    /// PORT: Java runs `runInternal` in a worker thread (`WorkerSkyKeyComputeState`) and loops on
    /// `CancellationException`; Blaze's blocking evaluator runs it directly.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:221 RegularRunnableExtension.run
    pub fn run(
        &self,
        env: &mut Environment<'_>,
        usages_value: &SingleExtensionUsagesValue,
        starlark_semantics: &Arc<StarlarkSemantics>,
        extension_id: &ModuleExtensionId,
        main_repository_mapping: &RepositoryMapping,
        facts: &blaze_bzlmod_model::Facts,
        require_repo_extension_metadata_mode: RequireRepoExtensionMetadataMode,
    ) -> Result<Option<RunModuleExtensionResult>, ExtensionError> {
        self.run_internal(
            env,
            usages_value,
            starlark_semantics,
            extension_id,
            main_repository_mapping,
            facts,
            require_repo_extension_metadata_mode,
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:260 RegularRunnableExtension.runInternal
    fn run_internal(
        &self,
        env: &mut Environment<'_>,
        usages_value: &SingleExtensionUsagesValue,
        starlark_semantics: &Arc<StarlarkSemantics>,
        extension_id: &ModuleExtensionId,
        main_repository_mapping: &RepositoryMapping,
        facts: &blaze_bzlmod_model::Facts,
        require_repo_extension_metadata_mode: RequireRepoExtensionMetadataMode,
    ) -> Result<Option<RunModuleExtensionResult>, ExtensionError> {
        env.listener()
            .post(Arc::new(ModuleExtensionEvaluationProgress::ongoing(
                extension_id,
                "starting",
            )));
        let thread_context = ModuleExtensionEvalStarlarkThreadContext::new(
            extension_id.clone(),
            format!("{}+", usages_value.get_extension_unique_name()),
            extension_id
                .bzl_file_label()
                .get_package_identifier()
                .clone(),
            self.bzl_load_value
                .get_module_context()
                .repo_mapping
                .clone(),
            usages_value.get_repo_overrides().clone(),
            main_repository_mapping.clone(),
        );
        // Java creates the context inside the try-with-resources; an ExternalDepsException from
        // it propagates before the working directory exists.
        let prepared =
            self.create_context(usages_value, starlark_semantics, extension_id, facts)?;

        let services = &self.services;
        let extension = &self.extension;
        SkyEnv::scope(
            env,
            move |sky_env| -> Result<Option<RunModuleExtensionResult>, ExtensionError> {
                let (base, ctx) = ModuleExtensionContext::create(
                    prepared.working_directory,
                    services.directories.clone(),
                    sky_env,
                    services.repo_env.clone(),
                    services.nonstrict_repo_env.clone(),
                    services.download_manager.clone(),
                    services.timeout_scaling,
                    services.process_wrapper.clone(),
                    starlark_semantics.clone(),
                    extension_id.clone(),
                    prepared.fields.clone(),
                    &self.static_env_vars,
                    &prepared.static_repo_mapping_entries,
                );
                let base = base.with_process_wrapper_options(
                    services.process_wrapper_kill_delay,
                    services.process_wrapper_graceful_sigterm,
                );
                let module_ctx = ModuleCtx {
                    extension_id: extension_id.to_string(),
                    os: base.get_os(),
                    fields: prepared.fields.clone(),
                };
                let payload = ExternalContext {
                    base,
                    kind: ExternalContextKind::ModuleExtension {
                        ctx,
                        thread_context,
                    },
                    abort: None,
                };
                // PORT: Java's thread is named "module extension <id>" with
                // `SymbolGenerator.create(extensionId)`; the main repo mapping comes from the
                // thread context (`ModuleExtensionEvalStarlarkThreadContext extends
                // StarlarkThreadContext`). Profiling is not ported.
                let outcome = call_implementation(
                    &extension.implementation,
                    extension.module_context.as_ref(),
                    Phase::ModuleExtension,
                    starlark_semantics.clone(),
                    Some(main_repository_mapping.clone()),
                    payload,
                    move |heap| heap.alloc(module_ctx),
                    |return_value, _eval| Ok(extract_metadata(return_value)),
                );
                let mut context = outcome.context;
                // Java: thread.setPrintHandler(Event.makeDebugPrintHandler(env.getListener())):
                // `call_implementation` has reported print() output as it happened.

                // The body of Java's try-with-resources: its outcome, before the context is closed.
                let body = (|| -> Result<Option<RunModuleExtensionResult>, BodyError> {
                    let returned = match outcome.result {
                        Err(e) => {
                            match context.abort {
                                // NeedsSkyframeRestartException: restart by returning null.
                                Some(EvaluationAbort::NeedsSkyframeRestart) => return Ok(None),
                                Some(EvaluationAbort::Interrupted) => {
                                    return Err(BodyError::Interrupted(Interrupted));
                                }
                                None => return Err(BodyError::Eval(e)),
                            }
                        }
                        Ok(returned) => returned,
                    };
                    let module_extension_metadata = match returned {
                        Err(type_name) => {
                            return Err(BodyError::ExternalDeps(
                                ExternalDepsException::with_message(
                                    ExternalDepsCode::ExtensionEvalError,
                                    format!(
                                        "expected module extension {extension_id} to return None or \
                                 extension_metadata, got {type_name}"
                                    ),
                                ),
                            ));
                        }
                        Ok(Some(ret_metadata)) => ret_metadata,
                        Ok(None) => {
                            if should_require_metadata(
                                require_repo_extension_metadata_mode,
                                extension_id,
                            ) {
                                // Java: RegularRunnableExtension.java:309.
                                return Err(BodyError::ExternalDeps(
                                    ExternalDepsException::with_message(
                                        ExternalDepsCode::ExtensionEvalError,
                                        format!(
                                            "module extension {} did not return extension_metadata \
                                             (implementation at {}), but \
                                             --incompatible_require_repo_extension_metadata={} \
                                             requires it",
                                            extension_id,
                                            crate::repository::repository_fetch_function::implementation_location(
                                                &extension.implementation,
                                            ),
                                            require_repo_extension_metadata_mode
                                        ),
                                    ),
                                ));
                            }
                            ModuleExtensionMetadata::default_metadata()
                        }
                    };
                    context.base.mark_successful();
                    let listener = context.base.env.get().listener();
                    listener.post(Arc::new(ModuleExtensionEvaluationProgress::finished(
                        extension_id,
                    )));
                    let recorded_inputs = context.base.get_recorded_inputs();
                    let ExternalContextKind::ModuleExtension { thread_context, .. } = &context.kind
                    else {
                        unreachable!("a module extension context");
                    };
                    let generated_repo_specs = thread_context
                        .create_repos(listener)
                        .map_err(BodyError::ExternalDeps)?;
                    Ok(Some(RunModuleExtensionResult {
                        recorded_inputs,
                        generated_repo_specs,
                        module_extension_metadata,
                    }))
                })();

                // Closing the context (end of the try-with-resources). An exception from close is
                // suppressed if the body threw.
                let should_delete =
                    context.should_delete_working_directory_on_close(context.base.was_successful());
                let close_result = context.base.close(should_delete);
                let listener = context.base.env.get().listener();
                let close_error = match (&body, close_result) {
                    (_, Ok(())) => None,
                    (Err(_), Err(_)) => None,
                    (Ok(_), Err(e)) => Some(e),
                };
                let eval_exception: Option<(Option<blaze_events::Location>, String)> =
                    match (body, close_error) {
                        (Ok(result), None) => return Ok(result),
                        (Ok(_), Some(CloseError::Eval(message))) => {
                            // An EvalException without a call stack: getMessageWithStack() is the
                            // message.
                            Some((None, message))
                        }
                        (Ok(_), Some(CloseError::Io(e))) => {
                            return Err(ExternalDepsException::with_cause_and_message(
                                ExternalDepsCode::ExternalDepsUnknown,
                                e,
                                "Failed to clean up module context directory",
                            )
                            .into());
                        }
                        (Err(BodyError::Interrupted(i)), _) => {
                            return Err(ExtensionError::Interrupted(i));
                        }
                        (Err(BodyError::ExternalDeps(e)), _) => {
                            return Err(ExtensionError::ExternalDeps(e));
                        }
                        (Err(BodyError::Eval(e)), _) => Some((
                            e.location().map(event_location),
                            eval_message_with_stack(&e),
                        )),
                    };
                // catch (EvalException e): ExternalDepsException events (an EvalException caused by
                // one) should already have been reported; no native of module_ctx or repo rules
                // throws such an EvalException in Blaze.
                let (location, message) = eval_exception.expect("an EvalException");
                listener.handle(&Event::error_at(location, message));
                Err(ExternalDepsException::with_message(
                    ExternalDepsCode::ExtensionEvalError,
                    format!("error evaluating module extension {extension_id}"),
                )
                .into())
            },
        )
    }

    /// Everything `createContext` computes that does not need the Skyframe environment: the
    /// working directory, the `module_ctx` field values (allocated on a new frozen heap) and the
    /// statically recorded repo mapping entries.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:354 RegularRunnableExtension.createContext
    fn create_context(
        &self,
        usages_value: &SingleExtensionUsagesValue,
        starlark_semantics: &StarlarkSemantics,
        extension_id: &ModuleExtensionId,
        facts: &blaze_bzlmod_model::Facts,
    ) -> Result<PreparedContext, ExternalDepsException> {
        let static_repo_mapping_recorder =
            Arc::new(Mutex::new(LabelSimpleRepoMappingRecorder::new()));
        {
            use blaze_cmdline::LabelRepoMappingRecorder;
            static_repo_mapping_recorder
                .lock()
                .record_all(self.bzl_load_value.get_recorded_repo_mappings());
        }
        let working_directory = self
            .services
            .directories
            .get_output_base()
            .get_relative(&MODULE_EXTENSION_WORKING_DIRECTORY_LOCATION)
            .get_relative_str(usages_value.get_extension_unique_name());
        let heap = FrozenHeap::new();
        let mut modules: Vec<FrozenValue> = Vec::new();
        let abridged_modules = usages_value.get_abridged_modules();
        for (i, abridged_module) in abridged_modules.iter().enumerate() {
            let module_key = &abridged_module.key;
            let recorder: SharedRepoMappingRecorder = static_repo_mapping_recorder.clone();
            modules.push(starlark_bazel_module::create(
                abridged_module,
                &self.extension,
                usages_value.get_repo_mappings().get(module_key).expect(
                    "NullPointerException: no repo mapping for a module using the extension",
                ),
                usages_value.get_extension_usages().get(module_key),
                recorder,
                i as i32,
                &heap,
            )?);
        }
        let root_usage = usages_value.get_extension_usages().get(&ModuleKey::root());
        let root_module_has_non_dev_dependency =
            root_usage.is_some_and(|u| u.get_has_non_dev_use_extension());
        // StarlarkList.immutableCopyOf(modules)
        let modules_value = heap.alloc(modules);
        let facts_value = heap.alloc(FactsValue::new(facts.clone(), &heap));
        let fields = Arc::new(ModuleCtxFields {
            frozen_heap: heap.into_ref(),
            modules: modules_value,
            facts: facts_value,
            root_module_has_non_dev_dependency,
            is_isolated: extension_id.isolation_key().is_some(),
            experimental_isolated_extension_usages: starlark_semantics
                .experimental_isolated_extension_usages,
        });
        let static_repo_mapping_entries = static_repo_mapping_recorder.lock().recorded_entries();
        Ok(PreparedContext {
            working_directory,
            fields,
            static_repo_mapping_entries,
        })
    }
}

/// The part of `createContext` computed before the Skyframe environment is handed to the
/// context.
struct PreparedContext {
    working_directory: blaze_vfs::Path,
    fields: Arc<ModuleCtxFields>,
    static_repo_mapping_entries: Vec<(
        blaze_cmdline::RepositoryName,
        String,
        blaze_cmdline::RepositoryName,
    )>,
}

/// What the body of `runInternal`'s try-with-resources throws.
enum BodyError {
    Interrupted(Interrupted),
    /// `EvalException` from the implementation function.
    Eval(EvalError),
    ExternalDeps(ExternalDepsException),
}

/// The return value of the implementation function: `Ok(None)` for `None`, `Ok(Some(_))` for
/// `extension_metadata`, else `Err(Starlark.type(returnValue))`.
fn extract_metadata(return_value: Value<'_>) -> Result<Option<ModuleExtensionMetadata>, String> {
    if return_value.is_none() {
        return Ok(None);
    }
    match return_value.downcast_ref::<ModuleExtensionMetadata>() {
        Some(metadata) => Ok(Some(metadata.clone())),
        None => Err(return_value.get_type().to_owned()),
    }
}

/// `EvalException.getMessageWithStack()`.
fn eval_message_with_stack(e: &EvalError) -> String {
    e.message_with_stack()
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:344 RegularRunnableExtension.shouldRequireMetadata
fn should_require_metadata(
    require_repo_extension_metadata_mode: RequireRepoExtensionMetadataMode,
    extension_id: &ModuleExtensionId,
) -> bool {
    match require_repo_extension_metadata_mode {
        RequireRepoExtensionMetadataMode::False => false,
        RequireRepoExtensionMetadataMode::All => true,
        RequireRepoExtensionMetadataMode::Root => {
            extension_id.bzl_file_label().get_repository().is_main()
        }
    }
}

// ---------------------------------------------------------------------------------------------
// InnateRunnableExtension

/// A fabricated module extension "innate" to each module, used to generate all repos defined
/// using `use_repo_rule` for a single repo rule.
// Java: com/google/devtools/build/lib/bazel/bzlmod/InnateRunnableExtension.java:50 InnateRunnableExtension
pub struct InnateRunnableExtension {
    module_key: ModuleKey,
    bzl_label: Label,
    rule_name: String,
    loaded_bzl: Arc<BzlLoadValue>,
    // Never empty.
    tags: Vec<Tag>,
}

impl fmt::Debug for InnateRunnableExtension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "InnateRunnableExtension({} {})",
            self.bzl_label, self.rule_name
        )
    }
}

impl InnateRunnableExtension {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/InnateRunnableExtension.java:58 InnateRunnableExtension.<init>
    fn new(
        module_key: ModuleKey,
        bzl_label: Label,
        rule_name: String,
        loaded_bzl: Arc<BzlLoadValue>,
        tags: Vec<Tag>,
    ) -> Self {
        blaze_util::check_argument!(!tags.is_empty());
        InnateRunnableExtension {
            module_key,
            bzl_label,
            rule_name,
            loaded_bzl,
            tags,
        }
    }

    /// Returns `Ok(None)` if a Skyframe restart is needed.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/InnateRunnableExtension.java:74 InnateRunnableExtension.load
    pub fn load(
        extension_id: &ModuleExtensionId,
        usages_value: &SingleExtensionUsagesValue,
        starlark_semantics: &StarlarkSemantics,
        env: &mut Environment<'_>,
    ) -> Result<Option<InnateRunnableExtension>, ExtensionError> {
        // An innate extension should have a singular usage.
        if usages_value.get_extension_usages().len() > 1 {
            return Err(ExternalDepsException::with_message(
                ExternalDepsCode::BadModule,
                format!(
                    "innate module extension {} is used by multiple modules: [{}]",
                    extension_id,
                    usages_value
                        .get_extension_usages()
                        .keys()
                        .map(|k| k.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .into());
        }
        let (module_key, usage) = usages_value
            .get_extension_usages()
            .iter()
            .next()
            .expect("NoSuchElementException: an innate extension has one usage");
        let module_key = module_key.clone();
        // ModuleFileFunction doesn't add usages for use_repo_rule without any instantiations, so
        // we can assume that there is at least one tag.
        let tags = usage.tags.clone();
        let repo_mapping = usages_value
            .get_repo_mappings()
            .get(&module_key)
            .expect("NullPointerException: no repo mapping for the module")
            .clone();
        let repo_context =
            LabelRepoContextImpl::of(repo_mapping.context_repo().clone(), repo_mapping.clone());

        // The name of the extension is of the form "<bzl_file_label> <rule_name>". Rule names
        // cannot contain spaces, so we can split on the last space.
        let extension_name = extension_id.extension_name();
        let last_space = extension_name
            .rfind(' ')
            .expect("an innate extension name contains a space");
        let raw_label = &extension_name[..last_space];
        let rule_name = extension_name[last_space + 1..].to_owned();
        let location = tags[0].location.clone();
        let bzl_label = match Label::parse_with_repo_context(raw_label, &repo_context)
            .and_then(|label| check_valid_load_label(&label, starlark_semantics).map(|()| label))
        {
            Ok(label) => label,
            Err(e) => {
                let message = format!("bad repo rule .bzl file label at {location}");
                return Err(ExternalDepsException::with_cause_and_message(
                    ExternalDepsCode::BadModule,
                    e,
                    &message,
                )
                .into());
            }
        };
        if rule_name.starts_with('_') {
            return Err(ExternalDepsException::with_message(
                ExternalDepsCode::BadModule,
                format!(
                    "{} does not export a repository_rule called {}, yet its use is requested at \
                     {}",
                    bzl_label, rule_name, tags[0].location
                ),
            )
            .into());
        }

        // Load the .bzl file.
        let loaded_bzl = match load_bzl_for_bzlmod(env, &bzl_label, false)? {
            None => return Ok(None),
            Some(Ok(v)) => v,
            Some(Err(e)) => {
                let message =
                    format!("error loading '{bzl_label}' for repo rules, requested by {location}");
                return Err(ExternalDepsException::with_cause_and_message(
                    ExternalDepsCode::BadModule,
                    e,
                    &message,
                )
                .into());
            }
        };

        Ok(Some(InnateRunnableExtension::new(
            module_key, bzl_label, rule_name, loaded_bzl, tags,
        )))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InnateRunnableExtension.java:142 InnateRunnableExtension.getEvalFactors
    pub fn get_eval_factors(&self) -> ModuleExtensionEvalFactors {
        ModuleExtensionEvalFactors::create("", "")
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InnateRunnableExtension.java:147 InnateRunnableExtension.getBzlTransitiveDigest
    pub fn get_bzl_transitive_digest(&self) -> Vec<u8> {
        self.loaded_bzl.get_transitive_digest().to_vec()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InnateRunnableExtension.java:152 InnateRunnableExtension.getFactsVersion
    pub fn get_facts_version(&self) -> i32 {
        0
    }

    /// PORT: Java's `run` never returns null (`StarlarkRepoRule.getRepoRule` needs no Skyframe
    /// value); Blaze's `repo_rule_from_global` may need one, and then this returns `Ok(None)`
    /// (a Skyframe restart, as `RunnableExtension.run` allows).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/InnateRunnableExtension.java:157 InnateRunnableExtension.run
    pub fn run(
        &self,
        env: &mut Environment<'_>,
        usages_value: &SingleExtensionUsagesValue,
        extension_id: &ModuleExtensionId,
    ) -> Result<Option<RunModuleExtensionResult>, ExtensionError> {
        let module = self.loaded_bzl.get_module();
        let exported = get_global(module, &self.rule_name);
        let Some(exported) = exported else {
            let exported_repo_rules: Vec<String> = global_names(module)
                .into_iter()
                .filter(|name| {
                    get_global(module, name)
                        .is_some_and(|v| v.value().to_value().get_type() == "repository_rule")
                })
                .collect();
            return Err(ExternalDepsException::with_message(
                ExternalDepsCode::BadModule,
                format!(
                    "{} does not export a repository_rule called {}, yet its use is requested at \
                     {}{}",
                    self.bzl_label,
                    self.rule_name,
                    self.tags[0].location,
                    SpellChecker::did_you_mean(&self.rule_name, &exported_repo_rules)
                ),
            )
            .into());
        };
        let repo_rule = match repo_rule_from_global(&exported, env)? {
            Some(Some(repo_rule)) => repo_rule,
            Some(None) => {
                return Err(ExternalDepsException::with_message(
                    ExternalDepsCode::BadModule,
                    format!(
                        "{} exports a value called {} of type {}, yet a repository_rule is \
                         requested at {}",
                        self.bzl_label,
                        self.rule_name,
                        starlark_type(&exported),
                        self.tags[0].location
                    ),
                )
                .into());
            }
            None => return Ok(None),
        };

        let mut generated_repo_specs: IndexMap<String, RepoSpec> =
            IndexMap::with_capacity(self.tags.len());
        // Instantiate the repos one by one.
        for tag in &self.tags {
            let kwargs = tag.attribute_values.attributes();
            // This cast should be safe since it should have been verified at tag creation time.
            let name = match tag.attribute_values.get("name") {
                Some(blaze_bzlmod_model::AttributeValue::String(name)) => name.clone(),
                other => panic!("ClassCastException: tag name {other:?} is not a String"),
            };
            let fake_call_stack = [StackEntry {
                name: "<toplevel>".to_owned(),
                location: Some(starlark_location(&tag.location)),
                source_line: None,
            }];
            let label_converter = LabelConverter::new(
                extension_id.bzl_file_label().get_package_identifier(),
                usages_value
                    .get_repo_mappings()
                    .get(&self.module_key)
                    .expect("NullPointerException: no repo mapping for the module")
                    .clone(),
            );
            let repo_mapping_where = format!("to the {}", self.module_key.to_display_string());
            let repo_spec = Module::with_temp_heap(|temp| {
                let heap = temp.heap();
                let kwargs: Vec<(String, Value)> = kwargs
                    .iter()
                    .map(|(k, v)| (k.clone(), attribute_value_to_starlark(v, heap)))
                    .collect();
                repo_rule.instantiate(
                    &kwargs,
                    &fake_call_stack,
                    &label_converter,
                    env.listener(),
                    &repo_mapping_where,
                )
            })?;
            // ImmutableMap.Builder.buildOrThrow: duplicate names throw.
            if generated_repo_specs
                .insert(name.clone(), repo_spec)
                .is_some()
            {
                panic!("IllegalArgumentException: Multiple entries with same key: {name}");
            }
        }
        Ok(Some(RunModuleExtensionResult {
            recorded_inputs: Vec::new(),
            generated_repo_specs,
            module_extension_metadata: ModuleExtensionMetadata::reproducible_metadata(),
        }))
    }
}
