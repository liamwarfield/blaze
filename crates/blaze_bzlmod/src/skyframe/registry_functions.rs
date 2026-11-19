//! `RegistryFunction` (REGISTRY), `RepoSpecFunction` (REPO_SPEC) and `YankedVersionsFunction`
//! (YANKED_VERSIONS).

use std::sync::Arc;

use blaze_bzlmod_model::{
    ExternalDepsCode, ExternalDepsException, LockfileMode, Registry, RegistryException,
    RegistryFileDownloadEvent, YankedVersionsValue,
};
use blaze_events::{Event, StoredEventHandler};
use blaze_skyframe::{ComputeError, Environment, Interrupted, Transience};
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use super::config::BzlmodConfig;
use super::keys::{
    BazelLockFileKey, ModuleFileKey, RegistryKey, RepoSpecKey, RepoSpecValue, YankedVersionsKey,
};
use super::precomputed::{LAST_INVALIDATION, LOCKFILE_MODE, MODULE_MIRRORS, VENDOR_DIRECTORY};
use super::throw;

// Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryFunction.java:63 RegistryFunction.compute
pub fn compute_registry(
    config: &BzlmodConfig,
    key: &RegistryKey,
    env: &mut Environment<'_>,
) -> Result<Option<Arc<dyn Registry>>, ComputeError> {
    let lockfile_mode = *LOCKFILE_MODE.get_required(env)?;
    let vendor_dir = VENDOR_DIRECTORY.get_required(env)?;

    if lockfile_mode == LockfileMode::Refresh {
        LAST_INVALIDATION.get(env)?;
    }

    let Some(lockfile) = env.get_value(BazelLockFileKey::KEY)? else {
        return Ok(None);
    };

    let module_mirrors = MODULE_MIRRORS.get_required(env)?;
    let empty = IndexSet::new();
    config
        .registry_factory
        .create_registry(
            &key.url()
                .replace("%workspace%", &super::path_string(&config.workspace_root)),
            lockfile_mode,
            &lockfile.registry_file_hashes,
            &lockfile.selected_yanked_versions,
            (*vendor_dir)
                .as_ref()
                .map(|p| PathFragment::create(&super::path_string(p))),
            module_mirrors.get(key.url()).unwrap_or(&empty),
        )
        .map(Some)
        .map_err(|e| {
            // RegistryException: TRANSIENT.
            throw(
                ExternalDepsException::with_cause_and_message(
                    ExternalDepsCode::InvalidRegistryUrl,
                    e,
                    &format!("Invalid registry URL: {}", key.url()),
                ),
                Transience::Transient,
            )
        })
}

/// Computes a `RepoSpec` for the given module by fetching required information from its
/// registry.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RepoSpecFunction.java:40 RepoSpecFunction.compute
pub fn compute_repo_spec(
    config: &BzlmodConfig,
    key: &RepoSpecKey,
    env: &mut Environment<'_>,
) -> Result<Option<RepoSpecValue>, ComputeError> {
    let Some(registry) = env.get_value(RegistryKey::create(key.registry_url.clone()))? else {
        return Ok(None);
    };
    let Some(module_file_value) = env.get_value(ModuleFileKey::key(key.module_key.clone()))? else {
        return Ok(None);
    };

    let download_events = StoredEventHandler::new();
    // PORT: profiling ("compute repo spec: <key>") skipped.
    let repo_spec = match registry.get_repo_spec(
        &key.module_key,
        &module_file_value.registry_file_hashes(),
        &download_events,
        &*config.registry_fetcher,
    ) {
        Ok(r) => r,
        Err(RegistryException::Interrupted) => return Err(Interrupted.into()),
        // IOException (NotFound is a FileNotFoundException here).
        Err(e) => {
            // RepoSpecException: TRANSIENT.
            return Err(throw(
                ExternalDepsException::with_cause_and_message(
                    ExternalDepsCode::ErrorAccessingRegistry,
                    e,
                    &format!(
                        "Unable to get module repo spec for {} from registry",
                        key.module_key
                    ),
                ),
                Transience::Transient,
            ));
        }
    };
    Ok(Some(RepoSpecValue::create(
        repo_spec,
        RegistryFileDownloadEvent::collect_to_map(&download_events.get_posts()),
    )))
}

/// Fetches the yanked versions for a given module from its registry.
// Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsFunction.java:39 YankedVersionsFunction.compute
pub fn compute_yanked_versions(
    config: &BzlmodConfig,
    key: &YankedVersionsKey,
    env: &mut Environment<'_>,
) -> Result<Option<YankedVersionsValue>, ComputeError> {
    let Some(registry) = env.get_value(RegistryKey::create(key.registry_url.clone()))? else {
        return Ok(None);
    };

    // PORT: profiling ("getting yanked versions: <name>") skipped.
    match registry.get_yanked_versions(&key.module_name, env.listener(), &*config.registry_fetcher)
    {
        Ok(v) => Ok(Some(YankedVersionsValue::create(v))),
        Err(RegistryException::Interrupted) => Err(Interrupted.into()),
        Err(e) => {
            env.listener().handle(&Event::warn(format!(
                "Could not read metadata file for module {} from registry {}: {}",
                key.module_name,
                key.registry_url,
                e.get_message()
            )));
            // This is failing open: If we can't read the metadata file, we allow yanked modules to
            // be fetched.
            Ok(Some(YankedVersionsValue::create(None)))
        }
    }
}
