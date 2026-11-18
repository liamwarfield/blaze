//! Port of `lib/actions/Spawns.java`: helpers reading a spawn's execution info.

use std::time::Duration;

use blaze_protos::failure_details::{FailureDetail, failure_detail, spawn};

use super::exec_exception::{ExecException, UserExecException};
use super::execution_requirements as er;
use super::spawn::Spawn;

/// `mayBeCached(Spawn)`.
// Java: com/google/devtools/build/lib/actions/Spawns.java:33 Spawns.mayBeCached
pub fn may_be_cached(spawn: &dyn Spawn) -> bool {
    may_be_cached_info(spawn.get_execution_info())
}

/// `mayBeCached(Map executionInfo)`.
pub fn may_be_cached_info(execution_info: &indexmap::IndexMap<String, String>) -> bool {
    !execution_info.contains_key(er::NO_CACHE) && !execution_info.contains_key(er::LOCAL)
}

/// `mayBeCachedRemotely(Spawn)`.
pub fn may_be_cached_remotely(spawn: &dyn Spawn) -> bool {
    may_be_cached_remotely_info(spawn.get_execution_info())
}

/// `mayBeCachedRemotely(Map executionInfo)`.
// Java: com/google/devtools/build/lib/actions/Spawns.java:45 Spawns.mayBeCachedRemotely
pub fn may_be_cached_remotely_info(info: &indexmap::IndexMap<String, String>) -> bool {
    may_be_cached_info(info)
        && !info.contains_key(er::NO_REMOTE)
        && !info.contains_key(er::NO_REMOTE_CACHE)
}

/// `mayBeExecutedRemotely(Spawn)`.
pub fn may_be_executed_remotely(spawn: &dyn Spawn) -> bool {
    let info = spawn.get_execution_info();
    !info.contains_key(er::LOCAL)
        && !info.contains_key(er::NO_REMOTE)
        && !info.contains_key(er::NO_REMOTE_EXEC)
}

/// `mayBeExecutedLocally(Spawn)`.
pub fn may_be_executed_locally(spawn: &dyn Spawn) -> bool {
    !spawn.get_execution_info().contains_key(er::NO_LOCAL)
}

/// `mayBeSandboxed(Spawn)`.
pub fn may_be_sandboxed(spawn: &dyn Spawn) -> bool {
    let info = spawn.get_execution_info();
    !info.contains_key(er::LEGACY_NOSANDBOX)
        && !info.contains_key(er::NO_SANDBOX)
        && !info.contains_key(er::LOCAL)
}

/// `usesPathMapping(Spawn)`.
pub fn uses_path_mapping(spawn: &dyn Spawn) -> bool {
    !spawn.get_path_mapper().is_noop()
}

/// `requiresNetwork(Spawn, defaultSandboxDisallowNetwork)`.
// Java: com/google/devtools/build/lib/actions/Spawns.java:93 Spawns.requiresNetwork
pub fn requires_network(spawn: &dyn Spawn, default_sandbox_disallow_network: bool) -> bool {
    let info = spawn.get_execution_info();
    if info.contains_key(er::BLOCK_NETWORK) {
        return false;
    }
    if info.contains_key(er::REQUIRES_NETWORK) {
        return true;
    }
    default_sandbox_disallow_network
}

/// `supportsWorkers(Spawn)`.
pub fn supports_workers(spawn: &dyn Spawn) -> bool {
    spawn
        .get_execution_info()
        .get(er::SUPPORTS_WORKERS)
        .map(String::as_str)
        == Some("1")
}

/// `supportsMultiplexWorkers(Spawn)`.
pub fn supports_multiplex_workers(spawn: &dyn Spawn) -> bool {
    spawn
        .get_execution_info()
        .get(er::SUPPORTS_MULTIPLEX_WORKERS)
        .map(String::as_str)
        == Some("1")
}

/// `getWorkerKeyMnemonic(Spawn)`.
pub fn get_worker_key_mnemonic(spawn: &dyn Spawn) -> String {
    spawn
        .get_execution_info()
        .get(er::WORKER_KEY_MNEMONIC)
        .cloned()
        .unwrap_or_else(|| spawn.get_mnemonic())
}

/// `getTimeout(Spawn)`.
pub fn get_timeout(spawn: &dyn Spawn) -> Result<Duration, ExecException> {
    get_timeout_with_default(spawn, Some(Duration::ZERO))
}

/// `getTimeout(Spawn, Duration defaultTimeout)`.
// Java: com/google/devtools/build/lib/actions/Spawns.java:164 Spawns.getTimeout
pub fn get_timeout_with_default(
    spawn: &dyn Spawn,
    default_timeout: Option<Duration>,
) -> Result<Duration, ExecException> {
    let Some(timeout_str) = spawn.get_execution_info().get(er::TIMEOUT) else {
        return Ok(default_timeout.unwrap_or(Duration::ZERO));
    };
    match timeout_str.parse::<i32>() {
        Ok(secs) if secs >= 0 => Ok(Duration::from_secs(secs as u64)),
        Ok(_) => blaze_util::unported!("Spawns.getTimeout(negative timeout)"),
        Err(e) => Err(UserExecException::with_cause(
            &format!("For input string: \"{timeout_str}\" ({e})"),
            FailureDetail {
                message: "could not parse timeout".to_string(),
                category: Some(failure_detail::Category::Spawn(
                    blaze_protos::failure_details::Spawn {
                        code: spawn::Code::InvalidTimeout as i32,
                        ..Default::default()
                    },
                )),
            },
        )
        .into()),
    }
}

/// `shouldPrefetchInputsForLocalExecution(Spawn)`.
pub fn should_prefetch_inputs_for_local_execution(spawn: &dyn Spawn) -> bool {
    match spawn.get_execution_info().get(er::DISABLE_LOCAL_PREFETCH) {
        None => true,
        Some(v) => v == "0",
    }
}

/// `prettyPrint(Spawn)`.
// Java: com/google/devtools/build/lib/actions/Spawns.java:192 Spawns.prettyPrint
pub fn pretty_print(spawn: &dyn Spawn) -> String {
    // PORT: Java's primary output is nullable; the seam's is not. Artifact.prettyPrint() prints the
    // exec path for non-source artifacts.
    let primary = spawn.get_resource_owner().get_primary_output();
    format!(
        "{} for {}",
        spawn.get_simple_class_name(),
        primary.get_exec_path_string()
    )
}
