//! SINGLE_EXTENSION_EVAL: evaluates a single module extension. This function loads the .bzl file
//! containing the extension, runs its implementation function with a `module_ctx` object
//! containing all relevant information, and returns the generated repos.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java.

use std::sync::Arc;

use blaze_bzlmod::skyframe::keys::BazelLockFileKey;
use blaze_bzlmod::skyframe::keys::SingleExtensionEvalKey;
use blaze_bzlmod::skyframe::keys::SingleExtensionValue;
use blaze_bzlmod::skyframe::precomputed::LOCKFILE_MODE;
use blaze_bzlmod_model::ExternalDepsCode;
use blaze_bzlmod_model::ExternalDepsException;
use blaze_bzlmod_model::Facts;
use blaze_bzlmod_model::LockFileModuleExtension;
use blaze_bzlmod_model::LockFileModuleExtensionWithFactors;
use blaze_bzlmod_model::LockfileMode;
use blaze_bzlmod_model::LockfileModuleExtensionMetadata;
use blaze_bzlmod_model::ModuleExtensionEvalFactors;
use blaze_bzlmod_model::ModuleExtensionId;
use blaze_bzlmod_model::ModuleKey;
use blaze_bzlmod_model::RepoSpec;
use blaze_cmdline::repository_name::MAIN;
use blaze_events::Event;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::precomputed::STARLARK_SEMANTICS;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_skyframe::ComputeError;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_skyframe::SkyFunctionException;
use blaze_skyframe::SkyKey;
use blaze_skyframe::Transience;
use indexmap::IndexMap;
use indexmap::IndexSet;

use super::facts::facts_repr;
use super::module_extension_metadata::generate_fixup;
use super::module_extension_metadata::lockfile_module_extension_metadata_of;
use super::runnable_extension::ExtensionError;
use super::runnable_extension::InnateRunnableExtension;
use super::runnable_extension::RegularRunnableExtension;
use super::runnable_extension::RunModuleExtensionResult;
use super::runnable_extension::RunnableExtension;
use super::single_extension_usages::SingleExtensionUsagesKey;
use super::single_extension_usages::SingleExtensionUsagesValue;
use crate::precomputed::REQUIRE_REPO_EXTENSION_METADATA;
use crate::repository::repo_recorded_input::RepoRecordedInput;
use crate::repository::repo_recorded_input::RepoRecordedInputWithValue;
use crate::services::RepoServices;

/// `throw new SingleExtensionEvalFunctionException(cause)` (always PERSISTENT).
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:514 SingleExtensionEvalFunction.SingleExtensionEvalFunctionException
fn single_extension_eval_function_exception(cause: ExternalDepsException) -> ComputeError {
    SkyFunctionException::new(cause, Transience::Persistent).into()
}

impl From<ExtensionError> for ComputeError {
    fn from(e: ExtensionError) -> Self {
        match e {
            ExtensionError::Interrupted(i) => ComputeError::Interrupted(i),
            ExtensionError::ExternalDeps(e) => single_extension_eval_function_exception(e),
        }
    }
}

/// Java `NeedsSkyframeRestartException` thrown by `tryGettingValueFromLockFile`, or one of its
/// declared exceptions.
enum LockFileError {
    NeedsSkyframeRestart,
    Compute(ComputeError),
}

impl From<Interrupted> for LockFileError {
    fn from(i: Interrupted) -> Self {
        LockFileError::Compute(ComputeError::Interrupted(i))
    }
}

/// Converts a recorded input to its lockfile form (`REPO_RECORDED_INPUT_WITH_VALUE_TYPE_ADAPTER`
/// writes `toString()`).
fn to_lockfile_recorded_input(
    value: &RepoRecordedInputWithValue,
) -> blaze_bzlmod_model::RepoRecordedInputWithValue {
    blaze_bzlmod_model::RepoRecordedInputWithValue(value.to_string())
}

/// Parses a recorded input from its lockfile form (`REPO_RECORDED_INPUT_WITH_VALUE_TYPE_ADAPTER`
/// reads `WithValue.parse(s)`, or `new WithValue(PARSE_FAILURE, "")`).
fn from_lockfile_recorded_input(
    value: &blaze_bzlmod_model::RepoRecordedInputWithValue,
) -> RepoRecordedInputWithValue {
    RepoRecordedInputWithValue::parse(&value.0).unwrap_or_else(|| {
        RepoRecordedInputWithValue::new(RepoRecordedInput::parse_failure(), Some(String::new()))
    })
}

/// Evaluates a single module extension.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:61 SingleExtensionEvalFunction
pub struct SingleExtensionEvalFunction {
    /// Java `directories`, `repoEnvSupplier`, `nonstrictRepoEnvSupplier`, `timeoutScaling`,
    /// `processWrapper`, `downloadManager`. PORT: `repositoryRemoteExecutor` is not ported.
    services: Arc<RepoServices>,
}

impl SingleExtensionEvalFunction {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:71 SingleExtensionEvalFunction.<init>
    pub fn new(services: Arc<RepoServices>) -> SingleExtensionEvalFunction {
        SingleExtensionEvalFunction { services }
    }

    fn directories(&self) -> &LoadingDirectories {
        &self.services.directories
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:98 SingleExtensionEvalFunction.compute
    pub fn compute(
        &self,
        key: &SingleExtensionEvalKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<SingleExtensionValue>, ComputeError> {
        let Some(starlark_semantics) = STARLARK_SEMANTICS.get(env)? else {
            return Ok(None);
        };
        let require_repo_extension_metadata_mode =
            *REQUIRE_REPO_EXTENSION_METADATA.get_required(env)?;
        let Some(main_repo_mapping_value) =
            env.get_value(RepositoryMappingKey::key(MAIN.clone()))?
        else {
            return Ok(None);
        };

        let extension_id = key.argument();
        let Some(usages_value) =
            env.get_value(SingleExtensionUsagesKey::key(extension_id.clone()))?
        else {
            return Ok(None);
        };
        let extension = if extension_id.is_innate() {
            InnateRunnableExtension::load(extension_id, &usages_value, &starlark_semantics, env)?
                .map(RunnableExtension::Innate)
        } else {
            RegularRunnableExtension::load(
                extension_id,
                &usages_value,
                &starlark_semantics,
                env,
                &self.services,
            )?
            .map(RunnableExtension::Regular)
        };
        let Some(extension) = extension else {
            return Ok(None);
        };

        // Check the lockfile first for that module extension
        let lockfile_mode = *LOCKFILE_MODE.get_required(env)?;
        let current_facts_version = extension.get_facts_version();
        let mut lockfile_facts = Facts::empty();
        // Store workspace lockfile facts separately for validation in ERROR mode
        let mut workspace_lockfile_facts = Facts::empty();
        if lockfile_mode != LockfileMode::Off {
            let keys = [
                SkyKey::new(BazelLockFileKey::KEY),
                SkyKey::new(BazelLockFileKey::HIDDEN_KEY),
            ];
            let lockfiles = env.get_values_and_exceptions(&keys)?;
            let workspace_lockfile = lockfiles.get_typed(&BazelLockFileKey::KEY);
            let hidden_lockfile = lockfiles.get_typed(&BazelLockFileKey::HIDDEN_KEY);
            let (Some(workspace_lockfile), Some(hidden_lockfile)) =
                (workspace_lockfile, hidden_lockfile)
            else {
                return Ok(None);
            };
            // The facts recorded by the most recent actual evaluation of the extension, if there
            // is such a record at the current facts_version. Unlike the workspace lockfile, the
            // hidden lockfile is not subject to manual edits or merges, so this can be compared
            // against the workspace lockfile's facts to detect that they have been modified.
            let mut hidden_lockfile_facts: Option<Facts> = None;
            if hidden_lockfile.facts.contains_key(extension_id)
                && *hidden_lockfile
                    .facts_versions
                    .get(extension_id)
                    .unwrap_or(&0)
                    == current_facts_version
            {
                hidden_lockfile_facts = hidden_lockfile.facts.get(extension_id).cloned();
            }
            // Prefer the workspace lockfile facts when present, falling back to the hidden
            // lockfile. In both cases, facts whose stored factsVersion differs from the current
            // extension's facts_version are discarded: the extension's schema may have changed.
            if let Some(facts) = workspace_lockfile.facts.get(extension_id) {
                let workspace_facts_version = *workspace_lockfile
                    .facts_versions
                    .get(extension_id)
                    .unwrap_or(&0);
                if workspace_facts_version == current_facts_version {
                    workspace_lockfile_facts = facts.clone();
                    lockfile_facts = workspace_lockfile_facts.clone();
                }
            } else if let Some(hidden) = &hidden_lockfile_facts {
                lockfile_facts = hidden.clone();
            }
            let eval_factors = extension.get_eval_factors();
            let mut locked_extension = workspace_lockfile
                .module_extensions
                .get(extension_id)
                .and_then(|m| m.get(&eval_factors));
            if locked_extension.is_none() {
                locked_extension = hidden_lockfile
                    .module_extensions
                    .get(extension_id)
                    .and_then(|m| m.get(&eval_factors));
            }
            if let Some(locked_extension) = locked_extension {
                // PORT: profiling ("check lockfile for <id>") is not ported.
                match self.try_getting_value_from_lock_file(
                    env,
                    extension_id,
                    &extension,
                    &usages_value,
                    &eval_factors,
                    locked_extension,
                    &lockfile_facts,
                    hidden_lockfile_facts.as_ref(),
                ) {
                    Ok(Some(single_extension_value)) => return Ok(Some(single_extension_value)),
                    Ok(None) => {}
                    Err(LockFileError::NeedsSkyframeRestart) => return Ok(None),
                    Err(LockFileError::Compute(e)) => return Err(e),
                }
            }
        }

        // Run that extension!
        let Some(module_extension_result) = extension.run(
            env,
            &usages_value,
            &starlark_semantics,
            extension_id,
            main_repo_mapping_value
                .repository_mapping()
                .expect("the main repo always has a repository mapping"),
            &lockfile_facts,
            require_repo_extension_metadata_mode,
        )?
        else {
            return Ok(None);
        };
        let RunModuleExtensionResult {
            recorded_inputs,
            generated_repo_specs,
            module_extension_metadata,
        } = module_extension_result;

        if lockfile_mode != LockfileMode::Off {
            let non_visible_repo_names = recorded_inputs
                .iter()
                .filter_map(|input_and_value| match &input_and_value.input {
                    RepoRecordedInput::RecordedRepoMapping { apparent_name, .. }
                        if input_and_value.value.is_none() =>
                    {
                        Some(format!("@{apparent_name}"))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(", ");
            if !non_visible_repo_names.is_empty() {
                env.listener().handle(&Event::warn(format!(
                    "The module extension {extension_id} produced an invalid lockfile entry \
                     because it referenced {non_visible_repo_names}. Please report this issue to \
                     its maintainers."
                )));
            }
        }
        if lockfile_mode == LockfileMode::Error && !module_extension_metadata.get_reproducible() {
            // The extension is not reproducible and can't be in the lockfile, since an existing
            // (but possibly out-of-date) entry would have been handled by
            // tryGettingValueFromLockFile above.
            let eval_factors = extension.get_eval_factors();
            return Err(single_extension_eval_function_exception(
                ExternalDepsException::with_message(
                    ExternalDepsCode::BadLockfile,
                    format!(
                        "The module extension '{}'{} does not exist in the lockfile",
                        extension_id,
                        if eval_factors.is_empty() {
                            String::new()
                        } else {
                            format!(" for platform {eval_factors}")
                        }
                    ),
                ),
            ));
        }
        let new_facts = module_extension_metadata.get_facts().clone();
        // In ERROR mode, validate facts only against the workspace lockfile, not the hidden
        // lockfile. The hidden lockfile may contain stale facts from a different version (e.g.,
        // after a rollback), which would cause false-positive validation errors.
        if lockfile_mode == LockfileMode::Error && new_facts != workspace_lockfile_facts {
            let reason = format!(
                "the extension '{}' has changed its facts: {} != {}",
                extension_id,
                facts_repr(&new_facts),
                facts_repr(&workspace_lockfile_facts)
            );
            return Err(create_outdated_lockfile_exception(&reason));
        }

        let lockfile_module_extension_metadata =
            lockfile_module_extension_metadata_of(&module_extension_metadata);
        // At this point the extension has been evaluated successfully, but
        // SingleExtensionEvalFunction may still fail if imported repositories were not generated.
        // However, since imports do not influence the evaluation of the extension and the
        // validation also runs when the extension result is taken from the lockfile, we can
        // already populate the lockfile info. This is necessary to prevent the extension from
        // rerunning when only the imports change.
        let lock_file_info = if lockfile_mode == LockfileMode::Update
            || lockfile_mode == LockfileMode::Refresh
        {
            Some(LockFileModuleExtensionWithFactors {
                extension_factors: extension.get_eval_factors(),
                module_extension: LockFileModuleExtension {
                    bzl_transitive_digest: extension.get_bzl_transitive_digest(),
                    usages_digest: SingleExtensionUsagesValue::hash_for_evaluation(&usages_value),
                    recorded_inputs: recorded_inputs
                        .iter()
                        .map(to_lockfile_recorded_input)
                        .collect(),
                    generated_repo_specs: generated_repo_specs.clone(),
                    module_extension_metadata: lockfile_module_extension_metadata.clone(),
                },
            })
        } else {
            None
        };
        self.create_single_extension_value(
            generated_repo_specs,
            lockfile_module_extension_metadata.as_ref(),
            extension_id,
            &usages_value,
            lock_file_info,
            new_facts,
            current_facts_version,
            env,
        )
        .map(Some)
    }

    /// Tries to get the evaluation result from the lockfile, if it's still up-to-date. Otherwise,
    /// returns `Ok(None)`.
    ///
    /// `facts` are the facts to attach to the reused result (workspace lockfile facts if present,
    /// otherwise the hidden lockfile's); `hidden_lockfile_facts` are the facts recorded by the
    /// most recent actual evaluation of the extension (kept in the hidden lockfile), or `None` if
    /// there is no such record. `Err(LockFileError::NeedsSkyframeRestart)` in case we need a
    /// skyframe restart. Note that we <em>don't</em> return `Ok(None)` in this case!
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:332 SingleExtensionEvalFunction.tryGettingValueFromLockFile
    fn try_getting_value_from_lock_file(
        &self,
        env: &mut Environment<'_>,
        extension_id: &ModuleExtensionId,
        extension: &RunnableExtension,
        usages_value: &SingleExtensionUsagesValue,
        eval_factors: &ModuleExtensionEvalFactors,
        locked_extension: &LockFileModuleExtension,
        facts: &Facts,
        hidden_lockfile_facts: Option<&Facts>,
    ) -> Result<Option<SingleExtensionValue>, LockFileError> {
        let lockfile_mode = *LOCKFILE_MODE
            .get_required(env)
            .map_err(LockFileError::Compute)?;
        let mut diff_recorder = DiffRecorder::new(
            /* record_messages= */ lockfile_mode == LockfileMode::Error,
        );
        // The try block: DiffFoundEarlyExitException ends it early.
        let result = (|| -> Result<(), TryError> {
            // Put faster diff detections earlier, so that we can short-circuit in UPDATE mode.
            if extension.get_bzl_transitive_digest() != locked_extension.bzl_transitive_digest {
                diff_recorder.record(format!(
                    "the implementation of the extension '{extension_id}' or one of its \
                     transitive .bzl files has changed"
                ))?;
            }
            // Check extension data in lockfile is still valid, disregarding usage information
            // that is not relevant for the evaluation of the extension.
            if SingleExtensionUsagesValue::hash_for_evaluation(usages_value)
                != locked_extension.usages_digest
            {
                diff_recorder.record(format!(
                    "the usages of the extension '{extension_id}' have changed"
                ))?;
            }
            let recorded_inputs: Vec<RepoRecordedInputWithValue> = locked_extension
                .recorded_inputs
                .iter()
                .map(from_lockfile_recorded_input)
                .collect();
            let reason = did_recorded_inputs_change(env, self.directories(), &recorded_inputs)?;
            if let Some(reason) = reason {
                diff_recorder.record(format!(
                    "an input to the extension '{extension_id}' changed: {reason}"
                ))?;
            }
            // The results of a reproducible extension are persisted in the hidden lockfile, so
            // whether such an extension reruns is determined by output base state. This could
            // result in stale facts in the workspace lockfile (e.g., modified externally by an
            // edit or VCS merge) not being recognized (in ERROR mode) or repaired (in UPDATE mode)
            // depending on this essentially invisible state. Since the hidden lockfile is meant to
            // be a fully transparent optimization, we avoid this by comparing the facts to the
            // ones persisted in it, which have been obtained from a prior evaluation of the
            // extension. Since it is reproducible and no other diffs have been detected up to this
            // point, the facts must be what the workspace lockfile should contain.
            if locked_extension.is_reproducible()
                && *facts != Facts::empty()
                && Some(facts) != hidden_lockfile_facts
            {
                diff_recorder.record(format!(
                    "the facts recorded in the lockfile for the extension '{extension_id}' do not \
                     match the result of its most recent evaluation"
                ))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {}
            // ignored
            Err(TryError::DiffFoundEarlyExit) => {}
            Err(TryError::LockFile(e)) => return Err(e),
        }
        if !diff_recorder.any_diffs_detected() {
            return self
                .create_single_extension_value(
                    locked_extension.generated_repo_specs.clone(),
                    locked_extension.module_extension_metadata.as_ref(),
                    extension_id,
                    usages_value,
                    Some(LockFileModuleExtensionWithFactors {
                        extension_factors: eval_factors.clone(),
                        module_extension: locked_extension.clone(),
                    }),
                    facts.clone(),
                    extension.get_facts_version(),
                    env,
                )
                .map(Some)
                .map_err(LockFileError::Compute);
        }
        // Reproducible extensions are always locked in the hidden lockfile to provide
        // best-effort speedups, but should never result in an error if out-of-date.
        if lockfile_mode == LockfileMode::Error && !locked_extension.is_reproducible() {
            return Err(LockFileError::Compute(create_outdated_lockfile_exception(
                &diff_recorder.get_recorded_diff_messages(),
            )));
        }
        Ok(None)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:456 SingleExtensionEvalFunction.createSingleExtensionValue
    fn create_single_extension_value(
        &self,
        generated_repo_specs: IndexMap<String, RepoSpec>,
        module_extension_metadata: Option<&LockfileModuleExtensionMetadata>,
        extension_id: &ModuleExtensionId,
        usages_value: &SingleExtensionUsagesValue,
        lock_file_info: Option<LockFileModuleExtensionWithFactors>,
        facts: Facts,
        facts_version: i32,
        env: &mut Environment<'_>,
    ) -> Result<SingleExtensionValue, ComputeError> {
        let mut fixup = None;
        if let Some(module_extension_metadata) = module_extension_metadata
            && let Some(root_usage) = usages_value.get_extension_usages().get(&ModuleKey::root())
        {
            // TODO: ModuleExtensionMetadata#generateFixup should throw ExternalDepsException
            // instead of EvalException.
            let all_repos: IndexSet<String> = generated_repo_specs.keys().cloned().collect();
            match generate_fixup(module_extension_metadata, root_usage, &all_repos) {
                Ok(f) => fixup = f,
                Err(message) => {
                    // An EvalException from Starlark.errorf: no location, no call stack.
                    env.listener().handle(&Event::error_at(None, message));
                    return Err(single_extension_eval_function_exception(
                        ExternalDepsException::with_message(
                            ExternalDepsCode::BadModule,
                            format!(
                                "error evaluating module extension {} in {}",
                                extension_id.extension_name(),
                                extension_id.bzl_file_label()
                            ),
                        ),
                    ));
                }
            }
        }

        let canonical_repo_name_to_internal_names = generated_repo_specs
            .keys()
            .map(|e| {
                (
                    SingleExtensionValue::repository_name(
                        usages_value.get_extension_unique_name(),
                        e,
                    ),
                    e.clone(),
                )
            })
            .collect();
        Ok(SingleExtensionValue {
            generated_repo_specs,
            canonical_repo_name_to_internal_names,
            lock_file_info,
            // PORT: `SingleExtensionValue::fixup` is `Option<()>` until RootModuleFileFixup moves
            // to blaze_bzlmod (see `super::module_extension_metadata::RootModuleFileFixup`); the
            // fixup's warning and commands are dropped here.
            fixup: fixup.map(|_| ()),
            facts,
            facts_version,
        })
    }
}

/// Checks recorded inputs in batches to prevent Skyframe cycles caused by outdated dependencies.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:437 SingleExtensionEvalFunction.didRecordedInputsChange
fn did_recorded_inputs_change(
    env: &mut Environment<'_>,
    directories: &LoadingDirectories,
    recorded_inputs: &[RepoRecordedInputWithValue],
) -> Result<Option<String>, LockFileError> {
    // Check inputs in batches to prevent Skyframe cycles caused by outdated dependencies.
    for batch in RepoRecordedInputWithValue::split_into_batches(recorded_inputs) {
        let outdated = RepoRecordedInput::is_any_value_outdated(env, directories, &batch)?;
        if env.values_missing() {
            return Err(LockFileError::NeedsSkyframeRestart);
        }
        if outdated.is_some() {
            return Ok(outdated);
        }
    }
    Ok(None)
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:409 SingleExtensionEvalFunction.DiffFoundEarlyExitException
struct DiffFoundEarlyExitException;

/// What the try block of `tryGettingValueFromLockFile` throws.
enum TryError {
    DiffFoundEarlyExit,
    LockFile(LockFileError),
}

impl From<DiffFoundEarlyExitException> for TryError {
    fn from(_: DiffFoundEarlyExitException) -> Self {
        TryError::DiffFoundEarlyExit
    }
}

impl From<LockFileError> for TryError {
    fn from(e: LockFileError) -> Self {
        TryError::LockFile(e)
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:411 SingleExtensionEvalFunction.DiffRecorder
struct DiffRecorder {
    diff_detected: bool,
    diff_messages: Option<Vec<String>>,
}

impl DiffRecorder {
    fn new(record_messages: bool) -> DiffRecorder {
        DiffRecorder {
            diff_detected: false,
            diff_messages: if record_messages {
                Some(Vec::new())
            } else {
                None
            },
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:419 SingleExtensionEvalFunction.DiffRecorder.record
    fn record(&mut self, message: String) -> Result<(), DiffFoundEarlyExitException> {
        self.diff_detected = true;
        match &mut self.diff_messages {
            Some(messages) => {
                messages.push(message);
                Ok(())
            }
            None => Err(DiffFoundEarlyExitException),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:428 SingleExtensionEvalFunction.DiffRecorder.anyDiffsDetected
    fn any_diffs_detected(&self) -> bool {
        self.diff_detected
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:432 SingleExtensionEvalFunction.DiffRecorder.getRecordedDiffMessages
    fn get_recorded_diff_messages(&self) -> String {
        self.diff_messages
            .as_ref()
            .expect("NullPointerException: diff messages are not recorded")
            .join(",")
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:504 SingleExtensionEvalFunction.createOutdatedLockfileException
fn create_outdated_lockfile_exception(reason: &str) -> ComputeError {
    single_extension_eval_function_exception(ExternalDepsException::with_message(
        ExternalDepsCode::BadLockfile,
        format!(
            "MODULE.bazel.lock is no longer up-to-date because {reason}. Please run `bazel mod \
             deps --lockfile_mode=update` to update your lockfile."
        ),
    ))
}
