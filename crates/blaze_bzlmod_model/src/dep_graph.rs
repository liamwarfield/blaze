//! Ports of `BazelDepGraphValue` and the Skyframe-independent part of `BazelDepGraphFunction`,
//! plus the Guava `ImmutableTable` semantics they need.

use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{
    Label, LabelPackageContext, PackageIdentifier, RepositoryMapping, RepositoryName,
};
use blaze_util::check_argument;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::external_deps_exception::{ExternalDepsCode, ExternalDepsException};
use crate::module::{AbridgedModule, Module};
use crate::module_extension::{ModuleExtensionId, ModuleExtensionUsage};
use crate::module_key::ModuleKey;
use crate::version::Version;

/// Guava `ImmutableTable` built with `ImmutableTable.builder()` (no row or column comparator).
///
/// Rows and columns are ordered by first appearance (`rowKeySet()`, `columnKeySet()`). The
/// order inside `row(r)` and `column(c)` depends on the implementation Guava picks in
/// `RegularImmutableTable.forOrderedComponents`:
///
/// - `DenseImmutableTable`, when there are more cells than half of rows times columns:
///   `row(r)` lists the columns in column order and `column(c)` lists the rows in row order
///   (`ImmutableArrayMap` over the key-to-index maps);
/// - `SparseImmutableTable` otherwise: `row(r)` and `column(c)` are in cell insertion order
///   (`LinkedHashMap`s filled from the cell list).
///
/// An empty table (`ImmutableTable.of()`) and a `SingletonImmutableTable` have nothing to order.
///
/// PORT: Java builds the table once (`Builder.buildOrThrow()`); here `put` adds to the table
/// directly, and the table is at every moment what `buildOrThrow()` would return for the cells
/// put so far. The Guava sources are not in the pinned tree; the orders were checked against
/// `ImmutableTable` of Guava 33.5.0-jre (the version Bazel 9.3.2 builds with), see the tests.
// Java: com/google/common/collect/RegularImmutableTable.java RegularImmutableTable.forOrderedComponents
// Java: com/google/common/collect/DenseImmutableTable.java DenseImmutableTable
// Java: com/google/common/collect/SparseImmutableTable.java SparseImmutableTable
#[derive(Debug, Clone)]
pub struct Table<R: Hash + Eq, C: Hash + Eq, V> {
    /// `rowMap()`: row keys by first appearance; each row in the order described above.
    rows: IndexMap<R, IndexMap<C, V>>,
    /// `columnMap()`: column keys by first appearance; each column in the order described above.
    columns: IndexMap<C, IndexMap<R, V>>,
    /// The keys of `cellSet()`, in insertion order.
    cells: Vec<(R, C)>,
    /// Whether `rows` and `columns` are currently in `DenseImmutableTable` order.
    dense: bool,
}

impl<R: Hash + Eq, C: Hash + Eq, V: PartialEq> PartialEq for Table<R, C, V> {
    /// `AbstractTable.equals`: equal cell sets, whatever the order.
    fn eq(&self, other: &Self) -> bool {
        self.rows == other.rows
    }
}

impl<R: Hash + Eq, C: Hash + Eq, V: Eq> Eq for Table<R, C, V> {}

impl<R: Hash + Eq + Clone + Debug, C: Hash + Eq + Clone + Debug, V: Clone> Default
    for Table<R, C, V>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Hash + Eq + Clone + Debug, C: Hash + Eq + Clone + Debug, V: Clone> Table<R, C, V> {
    pub fn new() -> Self {
        Table {
            rows: IndexMap::new(),
            columns: IndexMap::new(),
            cells: Vec::new(),
            dense: false,
        }
    }

    /// `ImmutableTable.Builder.put` + `buildOrThrow`: a duplicate cell is a crash in Java.
    pub fn put(&mut self, row: R, column: C, value: V) {
        let r = self.rows.entry(row.clone()).or_default();
        if r.contains_key(&column) {
            panic!("IllegalArgumentException: Duplicate key: (row={row:?}, column={column:?})");
        }
        r.insert(column.clone(), value.clone());
        self.columns
            .entry(column.clone())
            .or_default()
            .insert(row.clone(), value);
        self.cells.push((row.clone(), column.clone()));

        let dense = self.is_dense();
        if dense {
            if self.dense {
                // Only the row and the column of the new cell can be out of order.
                let columns = &self.columns;
                if let Some(r) = self.rows.get_mut(&row) {
                    r.sort_by(|c1, _, c2, _| {
                        columns.get_index_of(c1).cmp(&columns.get_index_of(c2))
                    });
                }
                let rows = &self.rows;
                if let Some(c) = self.columns.get_mut(&column) {
                    c.sort_by(|r1, _, r2, _| rows.get_index_of(r1).cmp(&rows.get_index_of(r2)));
                }
            } else {
                self.order_as_dense();
            }
        } else if self.dense {
            self.order_as_sparse();
        }
        self.dense = dense;
    }

    /// Whether `ImmutableTable.Builder.buildOrThrow()` returns a `DenseImmutableTable` for the
    /// cells put so far (as opposed to `ImmutableTable.of()`, a `SingletonImmutableTable` or a
    /// `SparseImmutableTable`).
    // Java: com/google/common/collect/ImmutableTable.java ImmutableTable.Builder.buildOrThrow
    // Java: com/google/common/collect/RegularImmutableTable.java RegularImmutableTable.forOrderedComponents
    pub fn is_dense(&self) -> bool {
        // use a dense table if more than half of the cells have values
        self.cells.len() > 1
            && self.cells.len() as u64 > (self.rows.len() as u64 * self.columns.len() as u64) / 2
    }

    /// `DenseImmutableTable`: each row lists its columns in column order, each column its rows
    /// in row order.
    fn order_as_dense(&mut self) {
        let columns = &self.columns;
        for r in self.rows.values_mut() {
            r.sort_by(|c1, _, c2, _| columns.get_index_of(c1).cmp(&columns.get_index_of(c2)));
        }
        let rows = &self.rows;
        for c in self.columns.values_mut() {
            c.sort_by(|r1, _, r2, _| rows.get_index_of(r1).cmp(&rows.get_index_of(r2)));
        }
    }

    /// `SparseImmutableTable`: each row and each column in cell insertion order.
    fn order_as_sparse(&mut self) {
        let mut rows: IndexMap<R, IndexMap<C, V>> = self
            .rows
            .keys()
            .map(|r| (r.clone(), IndexMap::new()))
            .collect();
        let mut columns: IndexMap<C, IndexMap<R, V>> = self
            .columns
            .keys()
            .map(|c| (c.clone(), IndexMap::new()))
            .collect();
        for (row, column) in &self.cells {
            let value = self
                .rows
                .get_mut(row)
                .and_then(|r| r.swap_remove(column))
                .expect("cell of the row map");
            rows.get_mut(row)
                .expect("row")
                .insert(column.clone(), value);
            let value = self
                .columns
                .get_mut(column)
                .and_then(|c| c.swap_remove(row))
                .expect("cell of the column map");
            columns
                .get_mut(column)
                .expect("column")
                .insert(row.clone(), value);
        }
        self.rows = rows;
        self.columns = columns;
    }

    /// `rowKeySet()`: the row keys in order of first appearance.
    pub fn row_key_set(&self) -> impl Iterator<Item = &R> {
        self.rows.keys()
    }

    /// `columnKeySet()`: the column keys in order of first appearance.
    pub fn column_key_set(&self) -> impl Iterator<Item = &C> {
        self.columns.keys()
    }

    pub fn contains_row(&self, row: &R) -> bool {
        self.rows.contains_key(row)
    }

    /// `rowMap().get(row)`; `row(row)` is this or an empty map. See the type's documentation for
    /// the order.
    pub fn row(&self, row: &R) -> Option<&IndexMap<C, V>> {
        self.rows.get(row)
    }

    /// `columnMap().get(column)`; `column(column)` is this or an empty map. See the type's
    /// documentation for the order.
    pub fn column(&self, column: &C) -> Option<&IndexMap<R, V>> {
        self.columns.get(column)
    }

    pub fn get(&self, row: &R, column: &C) -> Option<&V> {
        self.rows.get(row)?.get(column)
    }

    /// `cellSet()` (and so `values()`): the cells in insertion order, for both implementations.
    pub fn cell_set(&self) -> impl Iterator<Item = (&R, &C, &V)> {
        self.cells.iter().map(|(r, c)| (r, c, &self.rows[r][c]))
    }

    /// The number of cells.
    pub fn size(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The result of Bazel module resolution: the dependency graph post-version-resolution. Port of
/// `BazelDepGraphValue`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphValue.java:37 BazelDepGraphValue
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BazelDepGraphValue {
    /// The post-selection dep graph, in BFS order from the root module.
    pub dep_graph: IndexMap<ModuleKey, Module>,
    /// Canonical repo name to the key of the module backing it (a bimap in Java).
    pub canonical_repo_name_lookup: IndexMap<RepositoryName, ModuleKey>,
    /// All modules in the same order as `dep_graph`, with limited information.
    pub abridged_modules: Vec<AbridgedModule>,
    /// extensionUsagesTable[ID][moduleKey] is the usage of extension ID in module moduleKey.
    pub extension_usages_table: Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage>,
    /// Extension ID to a workspace-unique name.
    pub extension_unique_names: IndexMap<ModuleExtensionId, String>,
    /// For each extension, exported repo name to the canonical name of the overriding repo.
    pub repo_overrides: Table<ModuleExtensionId, String, RepositoryName>,
}

impl BazelDepGraphValue {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphValue.java:59 BazelDepGraphValue.createEmptyDepGraph
    pub fn create_empty_dep_graph() -> BazelDepGraphValue {
        let mut root = Module::new("", Version::empty(), ModuleKey::root());
        root.repo_name = String::new();
        BazelDepGraphValue {
            dep_graph: IndexMap::from([(ModuleKey::root(), root)]),
            canonical_repo_name_lookup: IndexMap::from([(MAIN.clone(), ModuleKey::root())]),
            abridged_modules: Vec::new(),
            extension_usages_table: Table::new(),
            extension_unique_names: IndexMap::new(),
            repo_overrides: Table::new(),
        }
    }

    /// The inverse of `canonical_repo_name_lookup`.
    pub fn module_key_to_repository_names(&self) -> IndexMap<ModuleKey, RepositoryName> {
        inverse(&self.canonical_repo_name_lookup)
    }

    /// Returns the full repository mapping for the given module, including repos from Bazel
    /// module deps and module extensions.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphValue.java:122 BazelDepGraphValue.getFullRepoMapping
    pub fn get_full_repo_mapping(&self, key: &ModuleKey) -> RepositoryMapping {
        Self::get_repository_mapping(
            key,
            &self.dep_graph,
            &self.extension_usages_table,
            &self.extension_unique_names,
            &self.canonical_repo_name_lookup,
            &self.repo_overrides,
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphValue.java:132 BazelDepGraphValue.getRepositoryMapping
    pub fn get_repository_mapping(
        key: &ModuleKey,
        dep_graph: &IndexMap<ModuleKey, Module>,
        extension_usages_table: &Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage>,
        extension_unique_names: &IndexMap<ModuleExtensionId, String>,
        canonical_repo_name_lookup: &IndexMap<RepositoryName, ModuleKey>,
        repo_overrides: &Table<ModuleExtensionId, String, RepositoryName>,
    ) -> RepositoryMapping {
        let mut mapping: Vec<(String, RepositoryName)> = Vec::new();
        if let Some(column) = extension_usages_table.column(key) {
            for (extension_id, usage) in column {
                let repo_name_prefix = format!("{}+", extension_unique_names[extension_id]);
                for proxy in &usage.proxies {
                    for (local, exported) in &proxy.imports {
                        let default_canonical_repo_name = RepositoryName::create_unvalidated(
                            &format!("{repo_name_prefix}{exported}"),
                        );
                        let value = repo_overrides
                            .get(extension_id, exported)
                            .cloned()
                            .unwrap_or(default_canonical_repo_name);
                        // PORT: ImmutableMap.Builder.buildOrThrow throws on duplicate keys.
                        if mapping.iter().any(|(k, _)| k == local) {
                            panic!(
                                "IllegalArgumentException: Multiple entries with same key: {local}"
                            );
                        }
                        mapping.push((local.clone(), value));
                    }
                }
            }
        }
        dep_graph[key]
            .get_repo_mapping_with_bazel_deps_only(&inverse(canonical_repo_name_lookup))
            .with_additional_mappings(mapping)
    }
}

fn inverse<K: Clone + Hash + Eq, V: Clone + Hash + Eq>(m: &IndexMap<K, V>) -> IndexMap<V, K> {
    m.iter().map(|(k, v)| (v.clone(), k.clone())).collect()
}

/// Port of the Skyframe-independent part of `BazelDepGraphFunction`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphFunction.java:54 BazelDepGraphFunction
pub struct BazelDepGraphFunction;

impl BazelDepGraphFunction {
    /// `compute` given the resolved dep graph of `BazelModuleResolutionValue`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphFunction.java:59 BazelDepGraphFunction.compute
    pub fn compute(
        dep_graph: &IndexMap<ModuleKey, Module>,
    ) -> Result<BazelDepGraphValue, ExternalDepsException> {
        let canonical_repo_name_lookup = Self::compute_canonical_repo_name_lookup(dep_graph);
        let extension_usages_by_id =
            Self::get_extension_usages_by_id(dep_graph, &inverse(&canonical_repo_name_lookup))?;
        let extension_unique_names =
            Self::calculate_unique_name_for_used_extension_id(&extension_usages_by_id);
        let repo_overrides = Self::resolve_repo_overrides(
            dep_graph,
            &extension_usages_by_id,
            &extension_unique_names,
            &canonical_repo_name_lookup,
        );
        Ok(BazelDepGraphValue {
            dep_graph: dep_graph.clone(),
            abridged_modules: dep_graph.values().map(AbridgedModule::from).collect(),
            canonical_repo_name_lookup,
            extension_usages_table: extension_usages_by_id,
            extension_unique_names,
            repo_overrides,
        })
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphFunction.java:95 BazelDepGraphFunction.getExtensionUsagesById
    fn get_extension_usages_by_id(
        dep_graph: &IndexMap<ModuleKey, Module>,
        module_key_to_repository_names: &IndexMap<ModuleKey, RepositoryName>,
    ) -> Result<Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage>, ExternalDepsException>
    {
        let mut table = Table::new();
        for module in dep_graph.values() {
            let repo_mapping =
                module.get_repo_mapping_with_bazel_deps_only(module_key_to_repository_names);
            // LabelConverter(PackageIdentifier.create(contextRepo, EMPTY), repoMapping)
            let base = PackageIdentifier::create(
                repo_mapping.context_repo().clone(),
                PathFragment::empty_fragment(),
            );
            let context = LabelPackageContext::of(&base, repo_mapping.clone());
            for usage in &module.extension_usages {
                let location = || {
                    usage
                        .proxies
                        .first()
                        .expect("NoSuchElementException")
                        .location
                        .to_string()
                };
                let label =
                    match Label::parse_with_package_context(&usage.extension_bzl_file, &context) {
                        Ok(l) => l,
                        Err(e) => {
                            return Err(ExternalDepsException::with_cause_and_message(
                                ExternalDepsCode::BadModule,
                                e,
                                &format!(
                                    "invalid label for module extension found at {}",
                                    location()
                                ),
                            ));
                        }
                    };
                let module_extension_id = ModuleExtensionId::create(
                    label,
                    usage.extension_name.clone(),
                    usage.isolation_key.clone(),
                );
                if !module_extension_id
                    .bzl_file_label()
                    .get_repository()
                    .is_visible()
                {
                    return Err(ExternalDepsException::with_message(
                        ExternalDepsCode::BadModule,
                        format!(
                            "invalid label for module extension found at {}: no repo visible as \
                             '@{}' here",
                            location(),
                            module_extension_id
                                .bzl_file_label()
                                .get_repository()
                                .get_name()
                        ),
                    ));
                }
                table.put(module_extension_id, module.key.clone(), usage.clone());
            }
        }
        Ok(table)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphFunction.java:136 BazelDepGraphFunction.computeCanonicalRepoNameLookup
    fn compute_canonical_repo_name_lookup(
        dep_graph: &IndexMap<ModuleKey, Module>,
    ) -> IndexMap<RepositoryName, ModuleKey> {
        // Modules with multiple versions in the dep graph (only from multiple_version_override).
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for key in dep_graph.keys() {
            *counts.entry(key.name()).or_default() += 1;
        }
        // If there is a unique version of this module in the dep graph, elide the version from
        // the canonical repository name.
        let mut lookup = IndexMap::new();
        for key in dep_graph.keys() {
            let name = if counts[key.name()] > 1 {
                key.get_canonical_repo_name_with_version()
            } else {
                key.get_canonical_repo_name_without_version()
            };
            // PORT: toImmutableBiMap throws on duplicate keys or values.
            if lookup.insert(name.clone(), key.clone()).is_some() {
                panic!("IllegalArgumentException: Multiple entries with same key: {name}");
            }
        }
        lookup
    }

    /// Calculates a unique name for each used extension id such that, after appending a single
    /// `+` to each name, none is a prefix of another.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphFunction.java:166 BazelDepGraphFunction.calculateUniqueNameForUsedExtensionId
    fn calculate_unique_name_for_used_extension_id(
        extension_usages_by_id: &Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage>,
    ) -> IndexMap<ModuleExtensionId, String> {
        // PORT: java-hash-order. Java builds a HashBiMap and copies it into an ImmutableBiMap, so
        // its iteration order follows String hash codes. Here the map is ordered by the table's
        // row order; callers only look names up by id.
        let mut taken: HashMap<String, ModuleExtensionId> = HashMap::new();
        let mut result = IndexMap::new();
        for id in extension_usages_by_id.row_key_set() {
            let mut attempt = 1;
            loop {
                let candidate = Self::make_unique_name_candidate(id, attempt);
                if let std::collections::hash_map::Entry::Vacant(e) = taken.entry(candidate.clone())
                {
                    e.insert(id.clone());
                    result.insert(id.clone(), candidate);
                    break;
                }
                attempt += 1;
            }
        }
        result
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphFunction.java:182 BazelDepGraphFunction.makeUniqueNameCandidate
    pub fn make_unique_name_candidate(id: &ModuleExtensionId, attempt: i32) -> String {
        check_argument!(attempt >= 1);
        let extension_name_disambiguator = if attempt == 1 {
            String::new()
        } else {
            attempt.to_string()
        };
        // An innate extension name is of the form "@repo//path/to/defs.bzl repo_rule_name",
        // which cannot be part of a valid repo name.
        let extension_name = if id.is_innate() {
            let n = id.extension_name();
            &n[n.find(' ').unwrap() + 1..]
        } else {
            id.extension_name()
        };
        let repo = id.bzl_file_label().get_repository().get_name().to_string();
        match id.isolation_key() {
            // With an isolation key, prefix the extension name with "_" to distinguish it from
            // non-isolated usages (exported Starlark identifiers cannot start with "_").
            Some(isolation_key) => format!(
                "{}+_{}{}+{}+{}+{}",
                repo,
                extension_name,
                extension_name_disambiguator,
                isolation_key.module().name(),
                isolation_key.module().version(),
                isolation_key.usage_exported_name()
            ),
            None => format!("{repo}+{extension_name}{extension_name_disambiguator}"),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphFunction.java:216 BazelDepGraphFunction.resolveRepoOverrides
    fn resolve_repo_overrides(
        dep_graph: &IndexMap<ModuleKey, Module>,
        extension_usages_table: &Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage>,
        extension_unique_names: &IndexMap<ModuleExtensionId, String>,
        canonical_repo_name_lookup: &IndexMap<RepositoryName, ModuleKey>,
    ) -> Table<ModuleExtensionId, String, RepositoryName> {
        // ModuleFileFunction ensures that repos that override other repos are not themselves
        // overridden, so an empty table is fine here.
        let root_module_mapping_without_overrides = BazelDepGraphValue::get_repository_mapping(
            &ModuleKey::root(),
            dep_graph,
            extension_usages_table,
            extension_unique_names,
            canonical_repo_name_lookup,
            &Table::new(),
        );
        let mut repo_overrides = Table::new();
        for extension_id in extension_usages_table.row_key_set() {
            let Some(root_usage) = extension_usages_table.get(extension_id, &ModuleKey::root())
            else {
                continue;
            };
            for (name, override_) in &root_usage.repo_overrides {
                repo_overrides.put(
                    extension_id.clone(),
                    name.clone(),
                    root_module_mapping_without_overrides.get(&override_.overriding_repo_name),
                );
            }
        }
        repo_overrides
    }
}
