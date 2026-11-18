//! Port of `Selection`: Minimal Version Selection over the discovered dependency graph.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use blaze_util::check_state;
use indexmap::IndexMap;

use crate::external_deps_exception::{ExternalDepsCode, ExternalDepsException};
use crate::module::{DepSpec, InterimModule, ModuleOverride};
use crate::module_key::ModuleKey;
use crate::version::Version;

/// The result of selection.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:87 Selection.Result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionResult {
    /// Final dep graph sorted in BFS iteration order, with unused modules removed.
    pub resolved_dep_graph: IndexMap<ModuleKey, InterimModule>,
    /// Un-pruned dep graph, with updated dep keys, additionally containing the unused modules
    /// which were initially discovered.
    pub unpruned_dep_graph: IndexMap<ModuleKey, InterimModule>,
}

/// During selection, a version is selected for each distinct "selection group".
// Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:97 Selection.SelectionGroup
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SelectionGroup {
    module_name: String,
    compatibility_level: i32,
    /// Only used for modules with multiple-version overrides.
    target_allowed_version: Version,
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:99 Selection.ModuleNameAndCompatibilityLevel
type ModuleNameAndCompatibilityLevel = (String, i32);

/// Runs module selection. Port of `Selection`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:81 Selection
pub struct Selection;

impl Selection {
    /// Computes (moduleName, compatibilityLevel) -> allowed versions, for modules with
    /// multiple-version overrides.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:107 Selection.computeAllowedVersionSets
    fn compute_allowed_version_sets(
        overrides: &IndexMap<String, ModuleOverride>,
        dep_graph: &IndexMap<ModuleKey, InterimModule>,
    ) -> Result<HashMap<ModuleNameAndCompatibilityLevel, BTreeSet<Version>>, ExternalDepsException>
    {
        let mut allowed_version_sets: HashMap<ModuleNameAndCompatibilityLevel, BTreeSet<Version>> =
            HashMap::new();
        for (module_name, override_) in overrides {
            let ModuleOverride::MultipleVersion(mvo) = override_ else {
                continue;
            };
            for allowed_version in &mvo.versions {
                let Some(allowed_version_module) = dep_graph.get(&ModuleKey::new(
                    module_name.clone(),
                    allowed_version.clone(),
                )) else {
                    return Err(ExternalDepsException::with_message(
                        ExternalDepsCode::VersionResolutionError,
                        format!(
                            "multiple_version_override for module {module_name} contains version \
                             {allowed_version}, but it doesn't exist in the dependency graph"
                        ),
                    ));
                };
                allowed_version_sets
                    .entry((
                        module_name.clone(),
                        allowed_version_module.compatibility_level,
                    ))
                    // The empty version compares greater than any other version, so it serves
                    // as a sentinel value.
                    .or_insert_with(|| BTreeSet::from([Version::empty()]))
                    .insert(allowed_version.clone());
            }
        }
        Ok(allowed_version_sets)
    }

    /// Computes the selection group for the given module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:144 Selection.computeSelectionGroup
    fn compute_selection_group(
        module: &InterimModule,
        allowed_version_sets: &HashMap<ModuleNameAndCompatibilityLevel, BTreeSet<Version>>,
    ) -> SelectionGroup {
        let Some(allowed_version_set) =
            allowed_version_sets.get(&(module.name.clone(), module.compatibility_level))
        else {
            // This module has no multiple-version override.
            return SelectionGroup {
                module_name: module.key.name().to_string(),
                compatibility_level: module.compatibility_level,
                target_allowed_version: Version::empty(),
            };
        };
        SelectionGroup {
            module_name: module.key.name().to_string(),
            compatibility_level: module.compatibility_level,
            // `ceiling`: the lowest allowed version that's no lower than this module's version.
            // If it's higher than any allowed version, EMPTY results, which is an error later
            // unless the module becomes unreferenced.
            target_allowed_version: allowed_version_set
                .range(module.version.clone()..)
                .next()
                .cloned()
                .expect("EMPTY sentinel"),
        }
    }

    /// Computes the possible versions a single DepSpec can resolve to.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:172 Selection.computePossibleResolutionResultsForOneDepSpec
    fn compute_possible_resolution_results_for_one_dep_spec(
        dep_spec: &DepSpec,
        selection_groups: &IndexMap<ModuleKey, SelectionGroup>,
        selected_versions: &HashMap<SelectionGroup, Version>,
    ) -> Vec<Version> {
        let min_compatibility_level =
            selection_groups[&dep_spec.to_module_key()].compatibility_level;
        let max_compatibility_level = if dep_spec.max_compatibility_level < 0 {
            min_compatibility_level
        } else {
            dep_spec.max_compatibility_level
        };
        // Sorted by compatibility level; only the lowest version per level (Comparators::min).
        let mut by_level: BTreeMap<i32, Version> = BTreeMap::new();
        for (group, version) in selected_versions {
            if group.module_name == dep_spec.name
                && group.compatibility_level >= min_compatibility_level
                && group.compatibility_level <= max_compatibility_level
                && group.target_allowed_version >= dep_spec.version
            {
                by_level
                    .entry(group.compatibility_level)
                    .and_modify(|v| {
                        if *version < *v {
                            *v = version.clone();
                        }
                    })
                    .or_insert_with(|| version.clone());
            }
        }
        by_level.into_values().collect()
    }

    /// Computes the possible versions for all distinct DepSpecs, in BFS order of first appearance.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:205 Selection.computePossibleResolutionResults
    fn compute_possible_resolution_results(
        dep_graph: &IndexMap<ModuleKey, InterimModule>,
        selection_groups: &IndexMap<ModuleKey, SelectionGroup>,
        selected_versions: &HashMap<SelectionGroup, Version>,
    ) -> IndexMap<DepSpec, Vec<Version>> {
        let mut results: IndexMap<DepSpec, Vec<Version>> = IndexMap::new();
        for module in dep_graph.values() {
            for dep_spec in module.deps.values().chain(module.nodep_deps.iter()) {
                if !results.contains_key(dep_spec) {
                    let r = Self::compute_possible_resolution_results_for_one_dep_spec(
                        dep_spec,
                        selection_groups,
                        selected_versions,
                    );
                    results.insert(dep_spec.clone(), r);
                }
            }
        }
        results
    }

    /// Runs module selection (aka version resolution).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:245 Selection.run
    pub fn run(
        dep_graph: &IndexMap<ModuleKey, InterimModule>,
        overrides: &IndexMap<String, ModuleOverride>,
    ) -> Result<SelectionResult, ExternalDepsException> {
        let allowed_version_sets = Self::compute_allowed_version_sets(overrides, dep_graph)?;

        let selection_groups: IndexMap<ModuleKey, SelectionGroup> = dep_graph
            .iter()
            .map(|(k, m)| {
                (
                    k.clone(),
                    Self::compute_selection_group(m, &allowed_version_sets),
                )
            })
            .collect();

        // Figure out the version to select for every selection group.
        let mut selected_versions: HashMap<SelectionGroup, Version> = HashMap::new();
        for (key, selection_group) in &selection_groups {
            selected_versions
                .entry(selection_group.clone())
                .and_modify(|v| {
                    if *key.version() > *v {
                        *v = key.version().clone();
                    }
                })
                .or_insert_with(|| key.version().clone());
        }

        let possible_resolution_results = Self::compute_possible_resolution_results(
            dep_graph,
            &selection_groups,
            &selected_versions,
        );
        for (dep_spec, versions) in &possible_resolution_results {
            if versions.is_empty() {
                return Err(ExternalDepsException::with_message(
                    ExternalDepsCode::VersionResolutionError,
                    format!("Unexpected error: {dep_spec} has no valid resolution result"),
                ));
            }
        }

        // Enumerate the cartesian product of the possible resolution results ("resolution
        // strategies"), in Lists.cartesianProduct order (last position varies fastest). Report the
        // failure from the first strategy if none yields a valid graph.
        let positions: HashMap<&DepSpec, usize> = possible_resolution_results
            .keys()
            .enumerate()
            .map(|(i, d)| (d, i))
            .collect();
        let choices_lists: Vec<&Vec<Version>> = possible_resolution_results.values().collect();
        let walker = DepGraphWalker {
            old_dep_graph: dep_graph,
            overrides,
            selection_groups: &selection_groups,
        };
        let mut indices = vec![0usize; choices_lists.len()];
        let mut first_failure: Option<ExternalDepsException> = None;
        loop {
            let strategy = |dep_spec: &DepSpec| -> Version {
                let pos = positions[dep_spec];
                choices_lists[pos][indices[pos]].clone()
            };
            match walker.walk(&strategy, false) {
                Ok(_) => {
                    // Walk again ignoring nodep edges, so that modules only reachable via nodep
                    // edges don't end up in the final graph. This cannot fail.
                    let pruned_dep_graph = walker.walk(&strategy, true)?;
                    let unpruned_dep_graph = dep_graph
                        .iter()
                        .map(|(k, m)| {
                            (
                                k.clone(),
                                m.with_deps_transformed(|d| d.with_version(strategy(d))),
                            )
                        })
                        .collect();
                    return Ok(SelectionResult {
                        resolved_dep_graph: pruned_dep_graph,
                        unpruned_dep_graph,
                    });
                }
                Err(e) => {
                    if first_failure.is_none() {
                        first_failure = Some(e);
                    }
                }
            }
            // Advance to the next combination.
            let mut i = indices.len();
            loop {
                if i == 0 {
                    // firstFailure cannot be null: no element of possibleResolutionResults is
                    // empty, so there is at least one strategy.
                    return Err(first_failure.unwrap());
                }
                i -= 1;
                indices[i] += 1;
                if indices[i] < choices_lists[i].len() {
                    break;
                }
                indices[i] = 0;
            }
        }
    }
}

/// Walks the dependency graph from the root node, collecting any reachable nodes into a new dep
/// graph and checking that nothing conflicts.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:340 Selection.DepGraphWalker
struct DepGraphWalker<'a> {
    old_dep_graph: &'a IndexMap<ModuleKey, InterimModule>,
    overrides: &'a IndexMap<String, ModuleOverride>,
    selection_groups: &'a IndexMap<ModuleKey, SelectionGroup>,
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:476 DepGraphWalker.ExistingModule
struct ExistingModule {
    module_key: ModuleKey,
    compatibility_level: i32,
    dependent: Option<ModuleKey>,
}

impl DepGraphWalker<'_> {
    /// Builds a new dep graph containing only deps reachable from the root module, in
    /// breadth-first order.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:359 DepGraphWalker.walk
    fn walk(
        &self,
        resolution_strategy: &dyn Fn(&DepSpec) -> Version,
        ignore_nodeps: bool,
    ) -> Result<IndexMap<ModuleKey, InterimModule>, ExternalDepsException> {
        let mut module_by_name: HashMap<String, ExistingModule> = HashMap::new();
        let mut new_dep_graph: IndexMap<ModuleKey, InterimModule> = IndexMap::new();
        let mut known: HashSet<ModuleKey> = HashSet::new();
        let mut to_visit: VecDeque<(ModuleKey, Option<ModuleKey>)> = VecDeque::new();
        to_visit.push_back((ModuleKey::root(), None));
        known.insert(ModuleKey::root());
        while let Some((key, dependent)) = to_visit.pop_front() {
            let module = self.old_dep_graph[&key]
                .with_deps_transformed(|d| d.with_version(resolution_strategy(d)));
            self.visit(&key, &module, dependent.as_ref(), &mut module_by_name)?;

            let deps: Vec<&DepSpec> = if ignore_nodeps {
                module.deps.values().collect()
            } else {
                module
                    .deps
                    .values()
                    .chain(module.nodep_deps.iter())
                    .collect()
            };
            for dep_spec in deps {
                if known.insert(dep_spec.to_module_key()) {
                    to_visit.push_back((dep_spec.to_module_key(), Some(key.clone())));
                }
            }
            // PORT: ImmutableMap.Builder.buildOrThrow; keys are unique by `known`.
            new_dep_graph.insert(key, module);
        }
        Ok(new_dep_graph)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/Selection.java:393 DepGraphWalker.visit
    fn visit(
        &self,
        key: &ModuleKey,
        module: &InterimModule,
        from: Option<&ModuleKey>,
        module_by_name: &mut HashMap<String, ExistingModule>,
    ) -> Result<(), ExternalDepsException> {
        if let Some(ModuleOverride::MultipleVersion(override_)) = self.overrides.get(key.name()) {
            if self.selection_groups[key].target_allowed_version.is_empty() {
                // This module has no target allowed version: there's no allowed version higher
                // than its version at the same compatibility level.
                check_state!(
                    from.is_some(),
                    "the root module cannot have a multiple version override"
                );
                return Err(ExternalDepsException::with_message(
                    ExternalDepsCode::VersionResolutionError,
                    format!(
                        "{} depends on {} which is not allowed by the multiple_version_override on \
                         {}, which allows only [{}]",
                        from.unwrap(),
                        key,
                        key.name(),
                        override_
                            .versions
                            .iter()
                            .map(|v| v.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        } else {
            let existing_module_with_same_name = module_by_name.insert(
                module.name.clone(),
                ExistingModule {
                    module_key: key.clone(),
                    compatibility_level: module.compatibility_level,
                    dependent: from.cloned(),
                },
            );
            if let Some(existing) = existing_module_with_same_name {
                // A module with the same name but a different compatibility level was also
                // selected.
                check_state!(
                    from.is_some() && existing.dependent.is_some(),
                    "the root module cannot possibly exist more than once in the dep graph"
                );
                return Err(ExternalDepsException::with_message(
                    ExternalDepsCode::VersionResolutionError,
                    format!(
                        "{} depends on {} with compatibility level {}, but {} depends on {} with \
                         compatibility level {} which is different",
                        from.unwrap(),
                        key,
                        module.compatibility_level,
                        existing.dependent.unwrap(),
                        existing.module_key,
                        existing.compatibility_level
                    ),
                ));
            }
        }

        // Make sure that we don't have `module` depending on the same dependency version twice.
        let mut dep_key_to_repo_name: HashMap<ModuleKey, &str> = HashMap::new();
        for (repo_name, dep_spec) in &module.deps {
            if let Some(previous_repo_name) =
                dep_key_to_repo_name.insert(dep_spec.to_module_key(), repo_name)
            {
                return Err(ExternalDepsException::with_message(
                    ExternalDepsCode::VersionResolutionError,
                    format!(
                        "{} depends on {} at least twice (with repo names {} and {}). Consider \
                         adding a multiple_version_override if you want to depend on multiple \
                         versions of {} simultaneously",
                        key,
                        dep_spec.to_module_key(),
                        repo_name,
                        previous_repo_name,
                        dep_spec.name
                    ),
                ));
            }
        }
        Ok(())
    }
}
