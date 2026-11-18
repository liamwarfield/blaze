//! Port of `lib/exec/local/LocalEnvProvider.java` and `PosixLocalEnvProvider.java`.

use indexmap::IndexMap;

use crate::exec::bin_tools::BinTools;

/// Allows just-in-time rewriting of the environment used for local actions.
// Java: com/google/devtools/build/lib/exec/local/LocalEnvProvider.java:26 LocalEnvProvider
pub trait LocalEnvProvider: Send + Sync {
    /// `rewriteLocalEnv(env, binTools, fallbackTmpDir)`.
    fn rewrite_local_env(
        &self,
        env: &IndexMap<String, String>,
        bin_tools: &BinTools,
        fallback_tmp_dir: &str,
    ) -> std::io::Result<IndexMap<String, String>>;
}

/// `LocalEnvProvider.NOOP`.
// Java: com/google/devtools/build/lib/exec/local/LocalEnvProvider.java:28 LocalEnvProvider.NOOP
#[derive(Debug, Default)]
pub struct NoopLocalEnvProvider;

impl LocalEnvProvider for NoopLocalEnvProvider {
    fn rewrite_local_env(
        &self,
        env: &IndexMap<String, String>,
        _bin_tools: &BinTools,
        _fallback_tmp_dir: &str,
    ) -> std::io::Result<IndexMap<String, String>> {
        Ok(env.clone())
    }
}

/// `LocalEnvProvider.forCurrentOs(clientEnv)`: Linux only, so always the POSIX provider.
// Java: com/google/devtools/build/lib/exec/local/LocalEnvProvider.java:30 LocalEnvProvider.forCurrentOs
pub fn for_current_os(client_env: &IndexMap<String, String>) -> PosixLocalEnvProvider {
    // PORT: skipped, darwin (XcodeLocalEnvProvider) and windows (WindowsLocalEnvProvider) only.
    PosixLocalEnvProvider::new(client_env.clone())
}

/// Sets `TMPDIR` from the client environment (or `/tmp`).
// Java: com/google/devtools/build/lib/exec/local/PosixLocalEnvProvider.java:25 PosixLocalEnvProvider
#[derive(Debug, Clone)]
pub struct PosixLocalEnvProvider {
    client_env: IndexMap<String, String>,
}

impl PosixLocalEnvProvider {
    /// `new PosixLocalEnvProvider(clientEnv)`.
    pub fn new(client_env: IndexMap<String, String>) -> PosixLocalEnvProvider {
        PosixLocalEnvProvider { client_env }
    }
}

impl LocalEnvProvider for PosixLocalEnvProvider {
    /// Compute an environment map for local actions on Unix-like platforms (e.g. Linux, macOS).
    ///
    /// Returns a map with the same keys and values as `env`. Overrides the value of TMPDIR (or
    /// adds it if not present in `env`) by the value of `clientEnv.get("TMPDIR")`, or if that's
    /// empty or null, then by "/tmp".
    // Java: com/google/devtools/build/lib/exec/local/PosixLocalEnvProvider.java:43 PosixLocalEnvProvider.rewriteLocalEnv
    fn rewrite_local_env(
        &self,
        env: &IndexMap<String, String>,
        _bin_tools: &BinTools,
        _fallback_tmp_dir: &str,
    ) -> std::io::Result<IndexMap<String, String>> {
        let mut result: IndexMap<String, String> = env
            .iter()
            .filter(|(k, _)| k.as_str() != "TMPDIR")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let p = match self.client_env.get("TMPDIR") {
            Some(p) if !p.is_empty() => p.clone(),
            _ => "/tmp".to_string(),
        };
        result.insert("TMPDIR".to_string(), p);
        Ok(result)
    }
}
