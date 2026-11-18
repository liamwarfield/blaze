//! The external repositories seam of the command's evaluator (Java: the repository functions
//! `BazelRepositoryModule` registers with the `SkyframeExecutor`).
//!
//! PORT: until the repo-rules lead's `BzlmodExternalRepos` (crates/blaze_repo_rules/src/
//! external_repos.rs) replaces it, external repositories are the ones recorded from the oracle
//! (`blaze_sky_loading::repos::OracleRepos`, written by `scripts/w3/oracle-repos`). This is the
//! one function to change when real fetching lands.

use std::path::PathBuf;
use std::sync::Arc;

use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::repos::{ExternalRepos, OracleRepos};

/// The environment variable naming the oracle repository facts to use.
pub const BLAZE_ORACLE_REPOS: &str = "BLAZE_ORACLE_REPOS";

/// The oracle repository facts file of a workspace: `$BLAZE_ORACLE_REPOS` if set, else
/// `$HOME/.cache/blaze/w3/<basename of the workspace>.repos.json`.
pub fn oracle_repos_path(directories: &LoadingDirectories) -> PathBuf {
    if let Ok(path) = std::env::var(BLAZE_ORACLE_REPOS) {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let basename = directories.get_workspace().get_base_name().to_string();
    PathBuf::from(home)
        .join(".cache/blaze/w3")
        .join(format!("{basename}.repos.json"))
}

/// Creates the external repositories of the command (REPOSITORY_MAPPING, REPOSITORY_DIRECTORY).
/// The error is a message for the user.
pub fn create_external_repos(
    directories: &LoadingDirectories,
) -> Result<Arc<dyn ExternalRepos>, String> {
    let path = oracle_repos_path(directories);
    match OracleRepos::load(&path, directories.clone()) {
        Ok(repos) => Ok(Arc::new(repos)),
        Err(e) => Err(e.0),
    }
}
