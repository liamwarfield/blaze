//! The Skyframe-independent parts of `BazelModuleResolutionFunction`, plus
//! `BazelModuleResolutionValue`, `YankedVersionsUtil`, `BazelVersion` and the option enums they
//! use.

use std::fmt;

use blaze_cmdline::repository_name::is_valid_module_name;
use blaze_events::{Event, EventHandler};
use blaze_util::profiler::{Profiler, ProfilerTask};
use indexmap::{IndexMap, IndexSet};

use crate::checksum::Checksum;
use crate::discovery::{Discovery, ModuleFileLoader, RootModuleFileValue};
use crate::external_deps_exception::{ExternalDepsCode, ExternalDepsException};
use crate::module::{InterimModule, Module, ModuleOverride};
use crate::module_key::ModuleKey;
use crate::registry::YankedVersionsValue;
use crate::repo_spec::RepoSpec;
use crate::selection::{Selection, SelectionResult};
use crate::version::Version;

/// `RepositoryOptions.CheckDirectDepsMode`.
// PORT: the options class lives in blaze_optdefs; this crate cannot depend on it, so the enum is
// mirrored here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckDirectDepsMode {
    Off,
    Warning,
    Error,
}

/// `RepositoryOptions.BazelCompatibilityMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BazelCompatibilityMode {
    Error,
    Warning,
    Off,
}

/// `RepositoryOptions.LockfileMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockfileMode {
    Off,
    Update,
    Refresh,
    Error,
}

/// The result of module resolution. Port of `BazelModuleResolutionValue`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionValue.java:30 BazelModuleResolutionValue
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BazelModuleResolutionValue {
    /// The final dep graph, in BFS order from the root.
    pub resolved_dep_graph: IndexMap<ModuleKey, Module>,
    /// The un-pruned dep graph (for `mod graph` and friends).
    pub unpruned_dep_graph: IndexMap<ModuleKey, InterimModule>,
    /// Hashes of files obtained (or known to be missing) from registries.
    pub registry_file_hashes: IndexMap<String, Option<Checksum>>,
    /// Selected module versions that are known to be yanked (and explicitly allowed).
    pub selected_yanked_versions: IndexMap<ModuleKey, String>,
}

/// Port of `YankedVersionsUtil`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsUtil.java:31 YankedVersionsUtil
pub struct YankedVersionsUtil;

// Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsUtil.java:36 YankedVersionsUtil.BZLMOD_ALLOWED_YANKED_VERSIONS_ENV
pub const BZLMOD_ALLOWED_YANKED_VERSIONS_ENV: &str = "BZLMOD_ALLOW_YANKED_VERSIONS";

impl YankedVersionsUtil {
    /// Returns `None` if all yanked versions are allowed, otherwise the allowed set.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsUtil.java:38 YankedVersionsUtil.parseAllowedYankedVersions
    pub fn parse_allowed_yanked_versions(
        allowed_yanked_versions_from_env: Option<&str>,
        allowed_yanked_versions_from_flag: &[String],
    ) -> Result<Option<IndexSet<ModuleKey>>, ExternalDepsException> {
        let mut builder = IndexSet::new();
        if let Some(env) = allowed_yanked_versions_from_env
            && Self::parse_module_keys_from_string(
                env,
                &mut builder,
                &format!("environment variable {BZLMOD_ALLOWED_YANKED_VERSIONS_ENV}={env}"),
            )?
        {
            return Ok(None);
        }
        for allowed in allowed_yanked_versions_from_flag {
            if Self::parse_module_keys_from_string(
                allowed,
                &mut builder,
                &format!("command line flag --allow_yanked_versions={allowed}"),
            )? {
                return Ok(None);
            }
        }
        Ok(Some(builder))
    }

    /// Returns true if `input` allows all yanked versions.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsUtil.java:69 YankedVersionsUtil.parseModuleKeysFromString
    fn parse_module_keys_from_string(
        input: &str,
        builder: &mut IndexSet<ModuleKey>,
        context: &str,
    ) -> Result<bool, ExternalDepsException> {
        for module_str in input.split(',') {
            if module_str == "all" {
                return Ok(true);
            }
            if module_str.is_empty() {
                continue;
            }
            let Some((name, version)) = module_str.split_once('@') else {
                return Err(ExternalDepsException::with_message(
                    ExternalDepsCode::VersionResolutionError,
                    format!(
                        "Parsing {context} failed, module versions must be of the form \
                         '<module name>@<version>'"
                    ),
                ));
            };
            if !is_valid_module_name(name) {
                return Err(ExternalDepsException::with_message(
                    ExternalDepsCode::VersionResolutionError,
                    format!(
                        "Parsing {context} failed, invalid module name '{name}': valid names must \
                         1) only contain lowercase letters (a-z), digits (0-9), dots (.), hyphens \
                         (-), and underscores (_); 2) begin with a lowercase letter; 3) end with a \
                         lowercase letter or digit."
                    ),
                ));
            }
            let version = match Version::parse(version) {
                Ok(v) => v,
                Err(e) => {
                    return Err(ExternalDepsException::with_cause_and_message(
                        ExternalDepsCode::VersionResolutionError,
                        e,
                        &format!(
                            "Parsing {context} failed, invalid version specified for module: \
                             {version}"
                        ),
                    ));
                }
            };
            builder.insert(ModuleKey::new(name, version));
        }
        Ok(false)
    }
}

/// A Bazel version, for `bazel_compatibility` checks. Port of `lib/bazel/BazelVersion`.
// Java: com/google/devtools/build/lib/bazel/BazelVersion.java:35 BazelVersion
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BazelVersion {
    release: Vec<i32>,
    suffix: String,
    original: String,
}

fn java_integer_value_of(s: &str) -> i32 {
    s.parse::<i32>()
        .unwrap_or_else(|_| panic!("NumberFormatException: For input string: \"{s}\""))
}

impl BazelVersion {
    // Java: com/google/devtools/build/lib/bazel/BazelVersion.java:56 BazelVersion.parse
    pub fn parse(version: &str) -> BazelVersion {
        // (?<release>(?:\d+\.)*\d+)(?<suffix>(.*))?
        let b = version.as_bytes();
        let mut i = 0;
        let mut end = 0;
        loop {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == start {
                break;
            }
            end = i;
            if i < b.len() && b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
                i += 1;
            } else {
                break;
            }
        }
        if end == 0 {
            panic!("IllegalArgumentException: bad version (does not match regex): {version}");
        }
        let release = &version[..end];
        BazelVersion {
            release: release.split('.').map(java_integer_value_of).collect(),
            suffix: version[end..].to_string(),
            original: version.to_string(),
        }
    }

    pub fn get_original(&self) -> &str {
        &self.original
    }

    // Java: com/google/devtools/build/lib/bazel/BazelVersion.java:50 BazelVersion.isPrerelease
    fn is_prerelease(&self) -> bool {
        self.suffix.starts_with("-pre")
    }

    /// Checks if this version satisfies `compat_version` (e.g. `>=7.0.0`).
    // Java: com/google/devtools/build/lib/bazel/BazelVersion.java:69 BazelVersion.satisfiesCompatibility
    pub fn satisfies_compatibility(&self, compat_version: &str) -> bool {
        let cut_index = if compat_version.contains('=') { 2 } else { 1 };
        let sign = &compat_version[..cut_index];
        let compat_split: Vec<i32> = compat_version[cut_index..]
            .split('.')
            .map(java_integer_value_of)
            .collect();
        let mut result = match self.release.cmp(&compat_split) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        if result == 0 && self.is_prerelease() {
            result = -1;
        }
        (result == 0 && sign.contains('='))
            || (result > 0 && (sign.contains('>') || sign.contains('-')))
            || (result < 0 && (sign.contains('<') || sign.contains('-')))
    }
}

/// Yanked-version information for the selected modules (`YANKED_VERSIONS` in Skyframe).
pub trait YankedVersionsLookup {
    /// `YankedVersionsFunction` for (`module_name`, `registry_url`).
    fn get_yanked_versions(&self, module_name: &str, registry_url: &str) -> YankedVersionsValue;
}

/// The intermediate result of discovery and selection.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:68 BazelModuleResolutionFunction.Result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoverAndSelectResult {
    pub selection_result: SelectionResult,
    pub registry_file_hashes: IndexMap<String, Option<Checksum>>,
    pub selected_yanked_versions: IndexMap<ModuleKey, String>,
}

/// Port of the Skyframe-independent parts of `BazelModuleResolutionFunction`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:62 BazelModuleResolutionFunction
pub struct BazelModuleResolutionFunction;

impl BazelModuleResolutionFunction {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:146 BazelModuleResolutionFunction.discoverAndSelect
    #[allow(clippy::too_many_arguments)]
    pub fn discover_and_select(
        loader: &dyn ModuleFileLoader,
        yanked_versions: &dyn YankedVersionsLookup,
        root: &RootModuleFileValue,
        allowed_yanked_versions: Option<&IndexSet<ModuleKey>>,
        check_direct_dependencies: CheckDirectDepsMode,
        bazel_compatibility_mode: BazelCompatibilityMode,
        current_bazel_version: &str,
        event_handler: &dyn EventHandler,
    ) -> Result<DiscoverAndSelectResult, ExternalDepsException> {
        // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:157 BazelModuleResolutionFunction.discoverAndSelect
        let discovery_result = {
            let _c = Profiler::instance().profile(ProfilerTask::BZLMOD, "discovery");
            Discovery::run(loader, root)?
        };
        Self::verify_all_overrides_are_on_existent_modules(
            &discovery_result.dep_graph,
            &root.overrides,
        )?;
        // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:169 BazelModuleResolutionFunction.discoverAndSelect
        let selection_result = {
            let _c = Profiler::instance().profile(ProfilerTask::BZLMOD, "selection");
            Selection::run(&discovery_result.dep_graph, &root.overrides)?
        };
        let resolved_dep_graph = &selection_result.resolved_dep_graph;
        // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:177 BazelModuleResolutionFunction.discoverAndSelect
        let yanked_versions_values = {
            let _c = Profiler::instance().profile(ProfilerTask::BZLMOD, "collect yanked versions");
            Self::collect_yanked_versions_values(yanked_versions, resolved_dep_graph.values())
        };
        // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:185 BazelModuleResolutionFunction.discoverAndSelect
        {
            let _c = Profiler::instance()
                .profile(ProfilerTask::BZLMOD, "verify root module direct deps");
            Self::verify_root_module_direct_deps_are_accurate(
                &discovery_result.dep_graph[&ModuleKey::root()],
                &resolved_dep_graph[&ModuleKey::root()],
                check_direct_dependencies,
                event_handler,
            )?;
        }
        // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:194 BazelModuleResolutionFunction.discoverAndSelect
        {
            let _c =
                Profiler::instance().profile(ProfilerTask::BZLMOD, "check bazel compatibility");
            Self::check_bazel_compatibility(
                resolved_dep_graph.values(),
                bazel_compatibility_mode,
                current_bazel_version,
                event_handler,
            )?;
        }
        let selected_yanked_versions =
            Self::check_no_yanked_versions(&yanked_versions_values, allowed_yanked_versions)?;
        Ok(DiscoverAndSelectResult {
            selection_result,
            registry_file_hashes: discovery_result.registry_file_hashes,
            selected_yanked_versions,
        })
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:207 BazelModuleResolutionFunction.collectYankedVersionsValues
    fn collect_yanked_versions_values<'a>(
        lookup: &dyn YankedVersionsLookup,
        modules: impl Iterator<Item = &'a InterimModule>,
    ) -> IndexMap<ModuleKey, YankedVersionsValue> {
        let mut values = IndexMap::new();
        let mut to_fetch: Vec<(ModuleKey, String, String)> = Vec::new();
        for m in modules {
            let Some(registry) = &m.registry else {
                // Modules with a non-registry override are never yanked.
                values.insert(m.key.clone(), YankedVersionsValue::none_yanked());
                continue;
            };
            match registry.try_get_yanked_versions_from_lockfile(&m.key) {
                Some(v) => {
                    values.insert(m.key.clone(), v);
                }
                None => to_fetch.push((m.key.clone(), m.name.clone(), registry.get_url())),
            }
        }
        // PORT: java-hash-order. Java keeps the modules whose yanked versions must be downloaded
        // in a `HashMap<ModuleKey, YankedVersionsValue.Key>` and appends them to the
        // ImmutableMap.Builder in that map's iteration order. checkNoYankedVersions keeps the
        // order, so it decides which yanked module is reported first and the order of
        // `selectedYankedVersions`, which BazelLockFileModule writes to MODULE.bazel.lock as is
        // (BazelLockFileModule.java:175; an ImmutableMap, not sorted on write).
        let to_fetch = crate::java_hash::java_hash_map_order(
            to_fetch
                .into_iter()
                .map(|(key, name, url)| (key.java_hash_code(), (key, name, url))),
        );
        // Java requests every YankedVersionsValue.Key at once (env.getValuesAndExceptions) and
        // then reads the results in the map's order.
        for (key, name, url) in to_fetch {
            values.insert(key, lookup.get_yanked_versions(&name, &url));
        }
        values
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:246 BazelModuleResolutionFunction.verifyAllOverridesAreOnExistentModules
    pub fn verify_all_overrides_are_on_existent_modules(
        initial_dep_graph: &IndexMap<ModuleKey, InterimModule>,
        overrides: &IndexMap<String, ModuleOverride>,
    ) -> Result<(), ExternalDepsException> {
        let existent_modules: IndexSet<&str> = initial_dep_graph
            .values()
            .map(|m| m.name.as_str())
            .collect();
        let nonexistent: Vec<&str> = overrides
            .keys()
            .map(|k| k.as_str())
            .filter(|k| !existent_modules.contains(k))
            .collect();
        if !nonexistent.is_empty() {
            return Err(ExternalDepsException::with_message(
                ExternalDepsCode::BadModule,
                format!(
                    "the root module specifies overrides on nonexistent module(s): {}",
                    nonexistent.join(", ")
                ),
            ));
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:264 BazelModuleResolutionFunction.verifyRootModuleDirectDepsAreAccurate
    pub fn verify_root_module_direct_deps_are_accurate(
        discovered_root_module: &InterimModule,
        resolved_root_module: &InterimModule,
        mode: CheckDirectDepsMode,
        event_handler: &dyn EventHandler,
    ) -> Result<(), ExternalDepsException> {
        if mode == CheckDirectDepsMode::Off {
            return Ok(());
        }
        let mut failure = false;
        for (repo_name, dep) in &discovered_root_module.deps {
            let resolved = resolved_root_module.deps[repo_name].to_module_key();
            if dep.to_module_key() != resolved {
                let message = format!(
                    "For repository '{}', the root module requires module version {}, but got {} \
                     in the resolved dependency graph. Please update the version in your \
                     MODULE.bazel or set --check_direct_dependencies=off",
                    repo_name,
                    dep.to_module_key(),
                    resolved
                );
                if mode == CheckDirectDepsMode::Warning {
                    event_handler.handle(&Event::warn(message));
                } else {
                    event_handler.handle(&Event::error(message));
                    failure = true;
                }
            }
        }
        if failure {
            return Err(ExternalDepsException::with_message(
                ExternalDepsCode::VersionResolutionError,
                "Direct dependency check failed.",
            ));
        }
        Ok(())
    }

    /// `current_bazel_version` is `BlazeVersionInfo.instance().getVersion()` (may be empty).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:300 BazelModuleResolutionFunction.checkBazelCompatibility
    pub fn check_bazel_compatibility<'a>(
        modules: impl Iterator<Item = &'a InterimModule>,
        mode: BazelCompatibilityMode,
        current_bazel_version: &str,
        event_handler: &dyn EventHandler,
    ) -> Result<(), ExternalDepsException> {
        if mode == BazelCompatibilityMode::Off {
            return Ok(());
        }
        if current_bazel_version.is_empty() {
            return Ok(());
        }
        let cur_version = BazelVersion::parse(current_bazel_version);
        for module in modules {
            for compat_version in &module.bazel_compatibility {
                if !cur_version.satisfies_compatibility(compat_version) {
                    let message = format!(
                        "Bazel version {} is not compatible with module \"{}\" \
                         (bazel_compatibility: [{}])",
                        cur_version.get_original(),
                        module.key,
                        module.bazel_compatibility.join(", ")
                    );
                    if mode == BazelCompatibilityMode::Warning {
                        event_handler.handle(&Event::warn(message));
                    } else {
                        event_handler.handle(&Event::error(message));
                        return Err(ExternalDepsException::with_message(
                            ExternalDepsCode::VersionResolutionError,
                            "Bazel compatibility check failed",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Fails if any selected module is yanked and not explicitly allowed. Returns the yanked
    /// info for each yanked but explicitly allowed module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:339 BazelModuleResolutionFunction.checkNoYankedVersions
    pub fn check_no_yanked_versions(
        yanked_version_values: &IndexMap<ModuleKey, YankedVersionsValue>,
        allowed_yanked_versions: Option<&IndexSet<ModuleKey>>,
    ) -> Result<IndexMap<ModuleKey, String>, ExternalDepsException> {
        let mut selected = IndexMap::new();
        for (key, value) in yanked_version_values {
            let Some(yanked) = &value.yanked_versions else {
                // No yanked version information available for this module.
                continue;
            };
            let Some(yanked_info) = yanked.get(key.version()) else {
                continue;
            };
            if allowed_yanked_versions.is_none_or(|a| a.contains(key)) {
                selected.insert(key.clone(), yanked_info.clone());
                continue;
            }
            return Err(ExternalDepsException::with_message(
                ExternalDepsCode::VersionResolutionError,
                format!(
                    "Yanked version detected in your resolved dependency graph: {key}, for the \
                     reason: {yanked_info}.\nYanked versions may contain serious vulnerabilities \
                     and should not be used. To fix this, use a bazel_dep on a newer version of \
                     this module. To continue using this version, allow it using the \
                     --allow_yanked_versions flag or the BZLMOD_ALLOW_YANKED_VERSIONS env \
                     variable."
                ),
            ));
        }
        Ok(selected)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:376 BazelModuleResolutionFunction.computeFinalDepGraph
    pub fn compute_final_dep_graph(
        resolved_dep_graph: &IndexMap<ModuleKey, InterimModule>,
        overrides: &IndexMap<String, ModuleOverride>,
        remote_repo_specs: &IndexMap<ModuleKey, RepoSpec>,
    ) -> IndexMap<ModuleKey, Module> {
        resolved_dep_graph
            .iter()
            .map(|(key, module)| {
                (
                    key.clone(),
                    InterimModule::to_module(
                        module,
                        overrides.get(key.name()),
                        remote_repo_specs.get(key),
                    ),
                )
            })
            .collect()
    }
}

impl fmt::Display for BazelVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.original)
    }
}
