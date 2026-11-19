//! Ports of `ModuleFileValue` and `Discovery`.

use std::collections::{HashMap, HashSet};

use blaze_cmdline::RepositoryName;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use crate::checksum::Checksum;
use crate::external_deps_exception::{ExternalDepsCode, ExternalDepsException};
use crate::module::{DepSpec, InterimModule, ModuleOverride};
use crate::module_key::ModuleKey;
use crate::version::Version;

/// The `ModuleFileValue` for non-root modules.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:44 ModuleFileValue.NonRootModuleFileValue
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonRootModuleFileValue {
    /// The module from the module file evaluation; its name and version might not match the
    /// requested key (e.g. with non-registry overrides).
    pub module: InterimModule,
    /// Hashes of files obtained (or known to be missing) from registries while obtaining it.
    pub registry_file_hashes: IndexMap<String, Option<Checksum>>,
}

/// The `ModuleFileValue` for the root module, with the overrides it specifies.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:64 ModuleFileValue.RootModuleFileValue
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootModuleFileValue {
    pub module: InterimModule,
    /// Module name to override.
    pub overrides: IndexMap<String, ModuleOverride>,
    pub non_registry_override_canonical_repo_to_module_name: IndexMap<RepositoryName, String>,
    pub non_registry_override_module_to_repo_name: IndexMap<String, String>,
    /// Relative paths to the root MODULE.bazel and all its transitive includes.
    pub module_file_paths: IndexSet<PathFragment>,
}

impl RootModuleFileValue {
    /// A root module value with no overrides.
    pub fn new(module: InterimModule, overrides: IndexMap<String, ModuleOverride>) -> Self {
        RootModuleFileValue {
            module,
            overrides,
            non_registry_override_canonical_repo_to_module_name: IndexMap::new(),
            non_registry_override_module_to_repo_name: IndexMap::new(),
            module_file_paths: IndexSet::new(),
        }
    }
}

/// Produces the `ModuleFileValue` of a non-root module (`MODULE_FILE` in Skyframe): fetches the
/// module file from the right registry (honoring the root module's overrides) and evaluates it.
/// Implemented by the Starlark-aware crate; tests use fakes.
pub trait ModuleFileLoader {
    fn get_module_file(
        &self,
        key: &ModuleKey,
    ) -> Result<NonRootModuleFileValue, ExternalDepsException>;

    /// Loads several module files (Java requests them as one Skyframe batch). Results are in the
    /// order of `keys`.
    fn get_module_files(
        &self,
        keys: &[ModuleKey],
    ) -> Vec<Result<NonRootModuleFileValue, ExternalDepsException>> {
        keys.iter().map(|k| self.get_module_file(k)).collect()
    }

    /// Whether `e` stands for a value that is not available yet (Java: `getOrThrow` returned
    /// null) rather than a failure. Discovery then goes on with the rest of the round, as Java
    /// does, and returns `e` at its end (Java: `if (env.valuesMissing()) return null`).
    fn is_values_missing(&self, _e: &ExternalDepsException) -> bool {
        false
    }
}

/// The result of discovery.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Discovery.java:45 Discovery.Result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryResult {
    pub dep_graph: IndexMap<ModuleKey, InterimModule>,
    pub registry_file_hashes: IndexMap<String, Option<Checksum>>,
}

/// Runs module discovery: reads the root module, adds its direct `bazel_dep`s to the graph, and
/// repeats for added dependencies until the entire graph is discovered. Port of `Discovery`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Discovery.java:41 Discovery
pub struct Discovery;

impl Discovery {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Discovery.java:54 Discovery.run
    pub fn run(
        loader: &dyn ModuleFileLoader,
        root: &RootModuleFileValue,
    ) -> Result<DiscoveryResult, ExternalDepsException> {
        // Because of the possible existence of nodep edges, we do multiple rounds of discovery.
        // In each round, we keep track of unfulfilled nodep edges, and at the end of the round,
        // if any unfulfilled nodep edge can now be fulfilled, we run another round.
        let mut prev_round_module_names: HashSet<String> =
            HashSet::from([root.module.name.clone()]);
        loop {
            let mut discovery_round = DiscoveryRound::new(loader, root, prev_round_module_names);
            let result = discovery_round.run()?;
            prev_round_module_names = result.dep_graph.values().map(|m| m.name.clone()).collect();
            if !discovery_round
                .unfulfilled_nodep_edge_module_names
                .iter()
                .any(|n| prev_round_module_names.contains(n))
            {
                return Ok(result);
            }
        }
    }
}

struct DiscoveryRound<'a> {
    loader: &'a dyn ModuleFileLoader,
    root: &'a RootModuleFileValue,
    prev_round_module_names: HashSet<String>,
    dep_graph: IndexMap<ModuleKey, InterimModule>,
    /// A module's first dependent in BFS order, for dependency chains in errors.
    predecessors: HashMap<ModuleKey, ModuleKey>,
    /// Module names of the unfulfilled nodep edges seen during this round.
    unfulfilled_nodep_edge_module_names: HashSet<String>,
    /// The keys whose module file is missing: Java puts them in `depGraph` with a null value.
    missing: HashSet<ModuleKey>,
}

impl<'a> DiscoveryRound<'a> {
    fn new(
        loader: &'a dyn ModuleFileLoader,
        root: &'a RootModuleFileValue,
        prev_round_module_names: HashSet<String>,
    ) -> Self {
        DiscoveryRound {
            loader,
            root,
            prev_round_module_names,
            dep_graph: IndexMap::new(),
            predecessors: HashMap::new(),
            unfulfilled_nodep_edge_module_names: HashSet::new(),
            missing: HashSet::new(),
        }
    }

    /// Runs one round of discovery: a breadth-first search from the root module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Discovery.java:111 DiscoveryRound.run
    fn run(&mut self) -> Result<DiscoveryResult, ExternalDepsException> {
        let mut registry_file_hashes: IndexMap<String, Option<Checksum>> = IndexMap::new();
        let root_module = self
            .root
            .module
            .with_deps_transformed(|d| self.apply_overrides(d));
        self.dep_graph.insert(ModuleKey::root(), root_module);
        let mut horizon: Vec<ModuleKey> = vec![ModuleKey::root()];
        let mut values_missing: Option<ExternalDepsException> = None;
        while !horizon.is_empty() {
            let next_horizon_keys = self.advance_horizon(&horizon);
            let results = self.loader.get_module_files(&next_horizon_keys);
            let mut next_horizon = Vec::new();
            for (dep_key, result) in next_horizon_keys.into_iter().zip(results) {
                let module_file_value = match result {
                    Ok(v) => v,
                    Err(e) if self.loader.is_values_missing(&e) => {
                        // Don't return yet. Try to expand any other unexpanded nodes before
                        // returning.
                        self.missing.insert(dep_key);
                        values_missing.get_or_insert(e);
                        continue;
                    }
                    Err(e) => return Err(self.maybe_report_dependency_chain(e, &dep_key)),
                };
                let module = module_file_value
                    .module
                    .with_deps_transformed(|d| self.apply_overrides(d));
                self.dep_graph.insert(dep_key.clone(), module);
                for (k, v) in module_file_value.registry_file_hashes {
                    registry_file_hashes.insert(k, v);
                }
                next_horizon.push(dep_key);
            }
            horizon = next_horizon;
        }
        if let Some(e) = values_missing {
            return Err(e);
        }

        // Remove all unfulfilled nodep edges from the dep graph. It should be just as if they
        // never existed.
        let mut result = IndexMap::with_capacity(self.dep_graph.len());
        for (key, module) in &self.dep_graph {
            if module
                .nodep_deps
                .iter()
                .all(|d| self.dep_graph.contains_key(&d.to_module_key()))
            {
                result.insert(key.clone(), module.clone());
            } else {
                let filtered: Vec<DepSpec> = module
                    .nodep_deps
                    .iter()
                    .filter(|d| self.dep_graph.contains_key(&d.to_module_key()))
                    .cloned()
                    .collect();
                result.insert(
                    key.clone(),
                    module.to_builder().set_nodep_deps(filtered).build(),
                );
            }
        }
        Ok(DiscoveryResult {
            dep_graph: result,
            registry_file_hashes,
        })
    }

    /// Returns a new [`DepSpec`] transformed according to any override on the dependency module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Discovery.java:166 DiscoveryRound.applyOverrides
    fn apply_overrides(&self, dep_spec: &DepSpec) -> DepSpec {
        if self.root.module.name == dep_spec.name {
            return DepSpec::root_module();
        }
        dep_spec.with_version(match self.root.overrides.get(&dep_spec.name) {
            Some(ModuleOverride::NonRegistry(_)) => Version::empty(),
            Some(ModuleOverride::SingleVersion(svo)) if !svo.version.is_empty() => {
                svo.version.clone()
            }
            _ => dep_spec.version.clone(),
        })
    }

    /// Given the current horizon, returns the next horizon: newly discovered module keys.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Discovery.java:187 DiscoveryRound.advanceHorizon
    fn advance_horizon(&mut self, horizon: &[ModuleKey]) -> Vec<ModuleKey> {
        let mut next_horizon: IndexSet<ModuleKey> = IndexSet::new();
        for module_key in horizon {
            let module = &self.dep_graph[module_key];
            // The main group of module keys to discover are the current horizon's normal deps.
            for dep_spec in module.deps.values() {
                let dep_key = dep_spec.to_module_key();
                if self.dep_graph.contains_key(&dep_key) || self.missing.contains(&dep_key) {
                    continue;
                }
                self.predecessors
                    .entry(dep_key.clone())
                    .or_insert_with(|| module.key.clone());
                next_horizon.insert(dep_key);
            }
            // Nodep deps are discovered ("fulfilled") iff the module they refer to already
            // exists in the dep graph. Otherwise, record them.
            for dep_spec in &module.nodep_deps {
                let dep_key = dep_spec.to_module_key();
                if self.dep_graph.contains_key(&dep_key) || self.missing.contains(&dep_key) {
                    continue;
                }
                if !self.prev_round_module_names.contains(&dep_spec.name) {
                    self.unfulfilled_nodep_edge_module_names
                        .insert(dep_spec.name.clone());
                    continue;
                }
                self.predecessors
                    .entry(dep_key.clone())
                    .or_insert_with(|| module.key.clone());
                next_horizon.insert(dep_key);
            }
        }
        next_horizon.into_iter().collect()
    }

    /// When an exception occurs while discovering a new dep, adds the dependency chain that led
    /// to that dep.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Discovery.java:232 DiscoveryRound.maybeReportDependencyChain
    fn maybe_report_dependency_chain(
        &self,
        e: ExternalDepsException,
        dep_key: &ModuleKey,
    ) -> ExternalDepsException {
        // SHOW_DEP_CHAIN
        if !matches!(
            e.get_code(),
            ExternalDepsCode::BadModule | ExternalDepsCode::ModuleNotFound
        ) {
            // E.g. a lockfile parse error or an I/O exception during registry access.
            return e;
        }
        // Trace back a dependency chain to the root module.
        let mut dep_chain = vec![dep_key.clone()];
        let mut predecessor = dep_key;
        while let Some(p) = self.predecessors.get(predecessor) {
            dep_chain.push(p.clone());
            predecessor = p;
        }
        dep_chain.reverse();
        let dep_chain_string = dep_chain
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>()
            .join(" -> ");
        ExternalDepsException::with_cause_and_message(
            ExternalDepsCode::BadModule,
            e,
            &format!("in module dependency chain {dep_chain_string}"),
        )
    }
}
