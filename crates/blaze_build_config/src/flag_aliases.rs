//! Port of `SkyframeExecutor.getFlagAliases`: the `--flag_alias` entries `BlazeCommandDispatcher`
//! injects for commands that analyze (MODULE.bazel `flag_alias()` and the rules_python stopgap).

use blaze_bzlmod_model::Version;
use indexmap::IndexMap;

/// A module of the resolved dependency graph, as `getFlagAliases` reads it.
#[derive(Clone, Debug)]
pub struct FlagAliasModule {
    pub name: String,
    pub version: String,
    /// `depGraphValue.getCanonicalRepoNameLookup().inverse().get(moduleKey)`.
    pub canonical_repo_name: blaze_cmdline::RepositoryName,
    /// `module.getFlagAliases()`: alias -> Starlark flag label as written in MODULE.bazel.
    pub flag_aliases: IndexMap<String, String>,
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3283 SkyframeExecutor.PY_FLAG_ALIASES
const PY_FLAG_ALIASES: [(&str, &str); 2] = [
    (
        "build_python_zip",
        "//python/config_settings:build_python_zip",
    ),
    (
        "incompatible_default_to_explicit_init_py",
        "//python/config_settings:incompatible_default_to_explicit_init_py",
    ),
];

// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3299 SkyframeExecutor.BAZEL_PY_FLAG_ALIASES
const BAZEL_PY_FLAG_ALIASES: [(&str, &str); 2] = [
    ("python_path", "//python/config_settings:python_path"),
    (
        "experimental_python_import_all_repositories",
        "//python/config_settings:experimental_python_import_all_repositories",
    ),
];

// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3325 SkyframeExecutor.getFlagAliases
/// Returns flag aliases from `MODULE.bazel` `flag_alias()` definitions (plus the hard-coded
/// Python aliases for rules_python versions that do not define them), in dep graph order.
pub fn get_flag_aliases(
    dep_graph: &[FlagAliasModule],
    ensure_py_aliases: bool,
    ensure_bazel_py_aliases: bool,
) -> IndexMap<String, String> {
    // rules_python at this version or older disables Python flag aliases.
    let min_bazel_version_for_python_aliases =
        Version::parse("1.6.100").expect("Hard-coded rules_python version should always parse.");
    let mut aliases_map: IndexMap<String, String> = IndexMap::new();
    for module in dep_graph {
        let flag_aliases = &module.flag_aliases;
        if !flag_aliases.is_empty() {
            // PORT: toCanonicalLabelString resolves each label with the defining module's repo
            // mapping (BazelDepGraphValue.getFullRepoMapping); that needs the bzlmod dep graph
            // (blaze_bzlmod).
            blaze_util::unported!(
                "SkyframeExecutor.toCanonicalLabelString",
                "flag_alias() in module {}",
                module.name
            );
        }
        if module.name != "rules_python" {
            continue;
        }
        // Don't apply hard-coded aliases for python version < 1.6.100
        let version = Version::parse(&module.version).expect("valid module version");
        let is_allowed_version = version > min_bazel_version_for_python_aliases
            || !version.to_string().starts_with("1.");
        if !is_allowed_version || !flag_aliases.is_empty() {
            continue;
        }
        let at = module.canonical_repo_name.get_name_with_at();
        if ensure_py_aliases {
            for (k, v) in PY_FLAG_ALIASES {
                if !flag_aliases.contains_key(k) {
                    aliases_map.insert(k.to_string(), format!("{}{}", at, v));
                }
            }
        }
        if ensure_bazel_py_aliases {
            for (k, v) in BAZEL_PY_FLAG_ALIASES {
                if !flag_aliases.contains_key(k) {
                    aliases_map.insert(k.to_string(), format!("{}{}", at, v));
                }
            }
        }
    }
    aliases_map
}

/// The `--flag_alias=<k>=<v>` arguments `BlazeCommandDispatcher` parses at `RC_FILE` priority
/// with source "module resolution".
pub fn flag_alias_args(aliases: &IndexMap<String, String>) -> Vec<String> {
    aliases
        .iter()
        .map(|(k, v)| format!("--flag_alias={}={}", k, v))
        .collect()
}
