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

/// [`get_flag_aliases`] over a resolved dep graph: also resolves `flag_alias()` labels with the
/// defining module's full repo mapping (Java `depGraphValue.getFullRepoMapping(moduleKey)`).
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3325 SkyframeExecutor.getFlagAliases
pub fn get_flag_aliases_from_dep_graph(
    dep_graph_value: &blaze_bzlmod_model::BazelDepGraphValue,
    ensure_py_aliases: bool,
    ensure_bazel_py_aliases: bool,
) -> IndexMap<String, String> {
    let module_key_to_repo = dep_graph_value.module_key_to_repository_names();
    let mut modules = Vec::with_capacity(dep_graph_value.dep_graph.len());
    let mut mappings = Vec::with_capacity(dep_graph_value.dep_graph.len());
    for (module_key, module) in &dep_graph_value.dep_graph {
        // depGraphValue.getCanonicalRepoNameLookup().inverse().get(moduleKey)
        let canonical_repo_name = module_key_to_repo
            .get(module_key)
            .cloned()
            .expect("every module of the dep graph has a canonical repo name");
        mappings.push(
            (!module.flag_aliases.is_empty())
                .then(|| dep_graph_value.get_full_repo_mapping(module_key)),
        );
        modules.push(FlagAliasModule {
            name: module.name.clone(),
            version: module.version.to_string(),
            canonical_repo_name,
            flag_aliases: module.flag_aliases.clone(),
        });
    }
    get_flag_aliases_impl(
        &modules,
        &mappings,
        ensure_py_aliases,
        ensure_bazel_py_aliases,
    )
}

/// Resolves a `flag_alias()` label against the defining module's repo mapping and returns its
/// unambiguous canonical form. Falls back to the original string if it can't be resolved, in
/// which case parsing it with the main repo mapping reports the error.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3402 SkyframeExecutor.toCanonicalLabelString
fn to_canonical_label_string(
    starlark_flag: &str,
    repo_context: &blaze_cmdline::LabelRepoContextImpl,
) -> String {
    let label = match blaze_cmdline::Label::parse_with_repo_context(starlark_flag, repo_context) {
        Ok(label) => label,
        Err(_) => return starlark_flag.to_owned(),
    };
    if !label.get_repository().is_visible() {
        return starlark_flag.to_owned();
    }
    label.get_unambiguous_canonical_form()
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
///
/// PORT: without repo mappings (this entry point), a module with `flag_alias()` definitions is
/// not ported; use [`get_flag_aliases_from_dep_graph`].
pub fn get_flag_aliases(
    dep_graph: &[FlagAliasModule],
    ensure_py_aliases: bool,
    ensure_bazel_py_aliases: bool,
) -> IndexMap<String, String> {
    let mappings = vec![None; dep_graph.len()];
    get_flag_aliases_impl(
        dep_graph,
        &mappings,
        ensure_py_aliases,
        ensure_bazel_py_aliases,
    )
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3325 SkyframeExecutor.getFlagAliases
fn get_flag_aliases_impl(
    dep_graph: &[FlagAliasModule],
    repo_mappings: &[Option<blaze_cmdline::RepositoryMapping>],
    ensure_py_aliases: bool,
    ensure_bazel_py_aliases: bool,
) -> IndexMap<String, String> {
    // rules_python at this version or older disables Python flag aliases.
    let min_bazel_version_for_python_aliases =
        Version::parse("1.6.100").expect("Hard-coded rules_python version should always parse.");
    let mut aliases_map: IndexMap<String, String> = IndexMap::new();
    for (module, repo_mapping) in dep_graph.iter().zip(repo_mappings) {
        let flag_aliases = &module.flag_aliases;
        if !flag_aliases.is_empty() {
            // flag_alias() labels are stored in the apparent form seen from the defining module
            // (e.g. "@rules_python//python/config_settings:python_path"), so resolve them with
            // that module's repo mapping rather than the main repo's.
            let Some(repo_mapping) = repo_mapping else {
                blaze_util::unported!(
                    "SkyframeExecutor.toCanonicalLabelString",
                    "flag_alias() in module {} without its repo mapping",
                    module.name
                );
            };
            let repo_context = blaze_cmdline::LabelRepoContextImpl::of(
                module.canonical_repo_name.clone(),
                repo_mapping.clone(),
            );
            for (alias, flag) in flag_aliases {
                aliases_map.insert(
                    alias.clone(),
                    to_canonical_label_string(flag, &repo_context),
                );
            }
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
