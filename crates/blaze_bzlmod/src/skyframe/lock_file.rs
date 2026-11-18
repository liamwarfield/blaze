//! `BazelLockFileFunction` (BAZEL_LOCK_FILE) and `BazelLockFileModule.afterCommand`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use blaze_bzlmod_model::{
    BazelDepGraphValue, BazelLockFileFunction, BazelLockFileModule, BazelLockFileValue,
    BazelModuleResolutionValue, Facts, LOCK_FILE_VERSION, LockFileModuleExtensionWithFactors,
    LockfileMode, ModuleExtensionId,
};
use blaze_cmdline::label_constants::MODULE_LOCKFILE_NAME;
use blaze_skyframe::{ComputeError, Environment, Evaluator, Key, SkyKey, Transience};
use indexmap::IndexMap;

use super::keys::{
    BazelDepGraphKey, BazelLockFileKey, BazelModuleResolutionKey, SingleExtensionValue,
};
use super::precomputed::LOCKFILE_MODE;
use super::{java_io_message, throw};

/// Reads the contents of the lockfiles into [`BazelLockFileValue`]s.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileFunction.java:69 BazelLockFileFunction.compute
pub fn compute(
    root_directory: &Path,
    output_base: &Path,
    sky_key: &BazelLockFileKey,
    env: &mut Environment<'_>,
) -> Result<Option<BazelLockFileValue>, ComputeError> {
    let lockfile_mode = *LOCKFILE_MODE.get_required(env)?;
    if lockfile_mode == LockfileMode::Off {
        return Ok(Some(BazelLockFileValue::empty_lockfile()));
    }

    let for_hidden_lockfile = sky_key.is_hidden();
    let lockfile_path: PathBuf = if for_hidden_lockfile {
        output_base
    } else {
        root_directory
    }
    .join(MODULE_LOCKFILE_NAME.get_path_string());

    // PORT: Java requests FileValue.key(lockfilePath) to recognize changes to the lockfile; FILE
    // is not ported here and a single build per process needs no invalidation.

    // PORT: profiling ("parse lockfile") skipped.
    // FileSystemUtils.readContent(path, UTF_8): malformed input is replaced.
    let contents = match std::fs::read(&lockfile_path) {
        // PORT: byte-strings (Java also replaces malformed input here (readContent(path, UTF_8)))
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        // FileNotFoundException: the lockfile does not exist.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(std::io::Error::new(
            e.kind(),
            java_io_message(&lockfile_path, &e),
        )),
    };
    BazelLockFileFunction::compute(contents, for_hidden_lockfile, lockfile_mode)
        .map(Some)
        .map_err(|e| throw(e, Transience::Persistent))
}

/// The lockfile modes in which the lockfiles are written.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:53 BazelLockFileModule.ENABLED_IN_MODES
pub const ENABLED_IN_MODES: &[LockfileMode] = &[LockfileMode::Update, LockfileMode::Refresh];

fn existing<K: Key>(evaluator: &Evaluator, key: K) -> Option<Arc<K::Value>> {
    evaluator
        .get_existing_value(&SkyKey::new(key))
        .map(|v| match v.downcast::<K::Value>() {
            Ok(v) => v,
            Err(_) => panic!("ClassCastException: value of {:?}", K::FUNCTION),
        })
}

/// Collects the Bazel module and module extension resolution results of the command and
/// updates the workspace lockfile and the hidden lockfile (under the output base).
///
/// The caller must only call this after a command that evaluated the lockfile values (Java:
/// `env.hasSyncedPackageLoading()`); other commands (e.g. `shutdown`) must skip it, since the
/// values may be stale.
///
/// PORT: Java writes both files on two virtual threads and joins them; they are written one after
/// the other here. Java logs I/O errors at SEVERE and continues; they are dropped here.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:63 BazelLockFileModule.afterCommand
pub fn after_command(
    evaluator: &Evaluator,
    workspace_root: &Path,
    output_base: &Path,
    lockfile_mode: LockfileMode,
) {
    if !ENABLED_IN_MODES.contains(&lockfile_mode) {
        return;
    }
    let module_resolution_value: Option<Arc<BazelModuleResolutionValue>> =
        existing(evaluator, BazelModuleResolutionKey);
    let dep_graph_value: Option<Arc<BazelDepGraphValue>> = existing(evaluator, BazelDepGraphKey);
    let old_lockfile: Option<Arc<BazelLockFileValue>> = existing(evaluator, BazelLockFileKey::KEY);
    let old_hidden_lockfile: Option<Arc<BazelLockFileValue>> =
        existing(evaluator, BazelLockFileKey::HIDDEN_KEY);
    let (Some(module_resolution_value), Some(dep_graph_value), Some(old_lockfile)) =
        (module_resolution_value, dep_graph_value, old_lockfile)
    else {
        // Since these values are required to compute the main repo mapping, which happens in
        // every build, an error must have occurred that prevented the evaluation of these values
        // and that has already been reported at this point.
        return;
    };
    let old_hidden_lockfile: Arc<BazelLockFileValue> =
        old_hidden_lockfile.unwrap_or_else(|| Arc::new(BazelLockFileValue::empty_lockfile()));

    // All nodes corresponding to module extensions that have been evaluated in the current build
    // are done at this point. Look up entries by eval keys to record results even if validation
    // later fails due to invalid imports.
    // Note: This also picks up up-to-date results from previous builds that are not in the
    // transitive closure of the current build. Since extensions are potentially costly to
    // evaluate, this is seen as an advantage. Full reproducibility can be ensured by running
    // 'bazel shutdown' first if needed.
    let extension_usages_table = &dep_graph_value.extension_usages_table;
    let mut new_extension_infos: IndexMap<ModuleExtensionId, LockFileModuleExtensionWithFactors> =
        IndexMap::new();
    let mut combined_facts: IndexMap<ModuleExtensionId, Facts> = old_lockfile.facts.clone();
    let mut combined_facts_versions: IndexMap<ModuleExtensionId, i32> =
        old_lockfile.facts_versions.clone();
    // The hidden lockfile's facts serve as the record of the facts produced by the most recent
    // actual evaluation of each extension, which SingleExtensionEvalFunction compares against the
    // workspace lockfile's facts to detect manual edits. They must thus only be combined with
    // results of the current build, never with the workspace lockfile's (possibly edited) facts.
    let mut combined_hidden_facts: IndexMap<ModuleExtensionId, Facts> =
        old_hidden_lockfile.facts.clone();
    let mut combined_hidden_facts_versions: IndexMap<ModuleExtensionId, i32> =
        old_hidden_lockfile.facts_versions.clone();
    for extension_id in extension_usages_table.row_key_set() {
        if extension_id.is_innate() {
            // The innate extensions are implemented in Java and don't benefit from a lockfile
            // entry.
            continue;
        }
        let value: Option<Arc<SingleExtensionValue>> = existing(
            evaluator,
            SingleExtensionValue::eval_key(extension_id.clone()),
        );
        if let Some(value) = value {
            new_extension_infos.insert(
                extension_id.clone(),
                value
                    .lock_file_info
                    .clone()
                    .expect("NoSuchElementException: No value present"),
            );
            combined_facts.insert(extension_id.clone(), value.facts.clone());
            combined_facts_versions.insert(extension_id.clone(), value.facts_version);
            combined_hidden_facts.insert(extension_id.clone(), value.facts.clone());
            combined_hidden_facts_versions.insert(extension_id.clone(), value.facts_version);
        }
    }
    let has_usages = |id: &ModuleExtensionId| extension_usages_table.contains_row(id);
    let relevant_facts = BazelLockFileModule::filter_relevant_facts(&combined_facts, has_usages);
    let relevant_facts_versions = BazelLockFileModule::filter_relevant_facts_versions(
        &combined_facts_versions,
        &relevant_facts,
    );
    let relevant_hidden_facts =
        BazelLockFileModule::filter_relevant_facts(&combined_hidden_facts, has_usages);
    let relevant_hidden_facts_versions = BazelLockFileModule::filter_relevant_facts_versions(
        &combined_hidden_facts_versions,
        &relevant_hidden_facts,
    );

    // updateLockfile thread.
    {
        let not_reproducible_extension_infos = BazelLockFileModule::combine_module_extensions(
            &old_lockfile.module_extensions,
            &new_extension_infos,
            has_usages,
            /* reproducible= */ false,
        );

        // Bazel may track the hashes of files fetched from local registries for internal
        // purposes, but those should never show up in the lockfile for two reasons:
        // - they are not needed for reproducibility, as local registries are assumed to be under
        //   the user's control, just like CLI flags;
        // - they would contribute absolute paths and thus aren't portable.
        let remote_registry_file_hashes = BazelLockFileModule::remote_registry_file_hashes(
            &module_resolution_value.registry_file_hashes,
        );

        // Create an updated version of the lockfile, keeping only the extension results from the
        // old lockfile that are still up-to-date and adding the newly resolved extension results,
        // as long as any of them are not known to be reproducible.
        let new_lockfile = BazelLockFileValue {
            lock_file_version: LOCK_FILE_VERSION,
            registry_file_hashes: remote_registry_file_hashes,
            selected_yanked_versions: module_resolution_value.selected_yanked_versions.clone(),
            module_extensions: not_reproducible_extension_infos,
            facts: relevant_facts,
            facts_versions: relevant_facts_versions,
        };

        // Write the new values to the files, but only if needed. This is not just a performance
        // optimization: whenever the lockfile is updated, most Skyframe nodes will be marked as
        // dirty on the next build, which breaks commands such as `bazel config` that rely on
        // com.google.devtools.build.skyframe.MemoizingEvaluator#getDoneValues.
        if new_lockfile != *old_lockfile {
            update_lockfile(workspace_root, &new_lockfile);
        }
    }

    // updateHiddenLockfile thread.
    {
        // Results of reproducible extensions do not need to be stored for reproducibility, but
        // avoiding reevaluations on server startups helps cold build performance.
        let reproducible_extension_infos = BazelLockFileModule::combine_module_extensions(
            &old_hidden_lockfile.module_extensions,
            &new_extension_infos,
            has_usages,
            /* reproducible= */ true,
        );
        let new_hidden_lockfile = BazelLockFileValue {
            lock_file_version: LOCK_FILE_VERSION,
            registry_file_hashes: IndexMap::new(),
            selected_yanked_versions: IndexMap::new(),
            module_extensions: reproducible_extension_infos,
            facts: relevant_hidden_facts,
            facts_versions: relevant_hidden_facts_versions,
        };
        if new_hidden_lockfile != *old_hidden_lockfile {
            update_lockfile(output_base, &new_hidden_lockfile);
        }
    }
}

/// Updates the data stored in the lockfile (MODULE.bazel.lock) under `lockfile_root`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:336 BazelLockFileModule.updateLockfile
fn update_lockfile(lockfile_root: &Path, updated_lockfile: &BazelLockFileValue) {
    // PORT: Java logs "Error while updating MODULE.bazel.lock file: %s" at SEVERE; Blaze has no
    // server log yet, so the error is dropped.
    let _ = BazelLockFileModule::update_lockfile(lockfile_root, updated_lockfile);
}
