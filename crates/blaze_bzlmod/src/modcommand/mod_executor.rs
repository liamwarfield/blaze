//! Port of `bazel/bzlmod/modcommand/ModExecutor.java`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;

use blaze_bzlmod_model::{ModuleExtensionId, ModuleExtensionUsage, ModuleKey, Table, Version};
use blaze_optdefs::misc::mod_options::{Charset, ModOptions};
use indexmap::{IndexMap, IndexSet};

use super::invalid_argument_exception::InvalidArgumentException;
use super::maybe_complete_set::MaybeCompleteSet;
use super::output_formatters::{
    ExtensionRepoImports, FormatterContext, Printer, get_formatter, java_string_cmp,
};
use crate::inspector::AugmentedModule;

/// Whether the node should be expanded from this edge.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IsExpanded {
    FALSE,
    TRUE,
}

/// Whether the edge is a direct edge or an indirect (transitive) one.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IsIndirect {
    FALSE,
    TRUE,
}

/// Whether the edge is cycling back inside the flattened graph.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IsCycle {
    FALSE,
    TRUE,
}

/// Detailed edge type for the [`ResultNode`] graph.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:728 ModExecutor.ResultNode.NodeMetadata
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeMetadata {
    pub is_expanded: IsExpanded,
    pub is_indirect: IsIndirect,
    pub is_cycle: IsCycle,
}

impl NodeMetadata {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:735 ModExecutor.ResultNode.NodeMetadata.create
    fn create(is_expanded: IsExpanded, is_indirect: IsIndirect, is_cycle: IsCycle) -> Self {
        NodeMetadata {
            is_expanded,
            is_indirect,
            is_cycle,
        }
    }

    pub fn is_expanded(&self) -> IsExpanded {
        self.is_expanded
    }

    pub fn is_indirect(&self) -> IsIndirect {
        self.is_indirect
    }

    pub fn is_cycle(&self) -> IsCycle {
        self.is_cycle
    }
}

/// `ImmutableSetMultimap<ModuleKey, NodeMetadata>`: keys in first-insertion order, values in
/// insertion order per key. Equality ignores order (Guava `Multimap.equals`).
pub type ChildrenMultimap = IndexMap<ModuleKey, IndexSet<NodeMetadata>>;

/// A node representing a module that forms the result graph.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:700 ModExecutor.ResultNode
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultNode {
    /// Whether the module is one of the targets in a paths query.
    is_target: bool,
    /// List of children mapped to detailed edge types.
    children: ChildrenMultimap,
}

impl ResultNode {
    pub fn is_target(&self) -> bool {
        self.is_target
    }

    /// `getChildren()`.
    pub fn get_children(&self) -> &ChildrenMultimap {
        &self.children
    }

    /// `getChildren().size()`: the number of (key, value) entries.
    pub fn children_size(&self) -> usize {
        self.children.values().map(|v| v.len()).sum()
    }

    /// `getChildren().entries()` in multimap order.
    pub fn children_entries(&self) -> Vec<(ModuleKey, NodeMetadata)> {
        self.children
            .iter()
            .flat_map(|(k, vs)| vs.iter().map(move |v| (k.clone(), *v)))
            .collect()
    }

    /// `ImmutableSortedSet.copyOf(comparingByKey(LEXICOGRAPHIC), entries)`: sorted by key, and
    /// entries with an equal key collapse into the first one.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:744 ModExecutor.ResultNode.getChildrenSortedByKey
    pub fn get_children_sorted_by_key(&self) -> Vec<(ModuleKey, NodeMetadata)> {
        let mut entries = self.children_entries();
        // Stable sort, then keep the first of each run of equal keys (Guava `construct`).
        entries.sort_by(|a, b| ModuleKey::lexicographic_cmp(&a.0, &b.0));
        let mut out: Vec<(ModuleKey, NodeMetadata)> = Vec::with_capacity(entries.len());
        for e in entries {
            if let Some(last) = out.last()
                && ModuleKey::lexicographic_cmp(&last.0, &e.0) == std::cmp::Ordering::Equal
            {
                continue;
            }
            out.push(e);
        }
        out
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:749 ModExecutor.ResultNode.getChildrenSortedByEdgeType
    pub fn get_children_sorted_by_edge_type(&self) -> Vec<(ModuleKey, NodeMetadata)> {
        let mut entries = self.children_entries();
        let cmp = |a: &(ModuleKey, NodeMetadata), b: &(ModuleKey, NodeMetadata)| {
            b.1.is_cycle
                .cmp(&a.1.is_cycle)
                .then_with(|| a.1.is_expanded.cmp(&b.1.is_expanded))
                .then_with(|| a.1.is_indirect.cmp(&b.1.is_indirect))
                .then_with(|| ModuleKey::lexicographic_cmp(&a.0, &b.0))
        };
        entries.sort_by(cmp);
        let mut out: Vec<(ModuleKey, NodeMetadata)> = Vec::with_capacity(entries.len());
        for e in entries {
            if let Some(last) = out.last()
                && cmp(last, &e) == std::cmp::Ordering::Equal
            {
                continue;
            }
            out.push(e);
        }
        out
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:759 ModExecutor.ResultNode.builder
    pub fn builder() -> ResultNodeBuilder {
        ResultNodeBuilder {
            target: false,
            children: IndexMap::new(),
        }
    }
}

/// `ResultNode.Builder` (an AutoValue builder with an `ImmutableSetMultimap.Builder`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:764 ModExecutor.ResultNode.Builder
#[derive(Debug, Clone)]
pub struct ResultNodeBuilder {
    target: bool,
    children: ChildrenMultimap,
}

impl ResultNodeBuilder {
    pub fn set_target(&mut self, value: bool) -> &mut Self {
        self.target = value;
        self
    }

    fn put(&mut self, key: ModuleKey, value: NodeMetadata) {
        self.children.entry(key).or_default().insert(value);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:771 ModExecutor.ResultNode.Builder.addChild
    pub fn add_child(
        &mut self,
        value: ModuleKey,
        expanded: IsExpanded,
        indirect: IsIndirect,
    ) -> &mut Self {
        self.put(
            value,
            NodeMetadata::create(expanded, indirect, IsCycle::FALSE),
        );
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:777 ModExecutor.ResultNode.Builder.addChildren
    pub fn add_children(&mut self, children: &ChildrenMultimap) -> &mut Self {
        for (k, vs) in children {
            for v in vs {
                self.put(k.clone(), *v);
            }
        }
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:783 ModExecutor.ResultNode.Builder.addCycle
    pub fn add_cycle(&mut self, value: ModuleKey) -> &mut Self {
        self.put(
            value,
            NodeMetadata::create(IsExpanded::FALSE, IsIndirect::FALSE, IsCycle::TRUE),
        );
        self
    }

    pub fn build(&self) -> ResultNode {
        ResultNode {
            is_target: self.target,
            children: self.children.clone(),
        }
    }
}

/// Executes inspection queries for `ModCommand` and prints the resulted output to the
/// reporter's output stream using the different defined output formatters.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:69 ModExecutor
pub struct ModExecutor<'a> {
    dep_graph: Cow<'a, IndexMap<ModuleKey, AugmentedModule>>,
    extension_usages: Cow<'a, Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage>>,
    /// `ImmutableSetMultimap<ModuleExtensionId, String>`.
    extension_repos: Cow<'a, IndexMap<ModuleExtensionId, IndexSet<String>>>,
    extension_filter: Option<MaybeCompleteSet<ModuleExtensionId>>,
    options: ModOptions,
    printer: Printer<'a>,
    extension_repo_imports: ExtensionRepoImports,
}

impl<'a> ModExecutor<'a> {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:81 ModExecutor.<init>
    pub fn new(
        dep_graph: Cow<'a, IndexMap<ModuleKey, AugmentedModule>>,
        options: ModOptions,
        output_stream: &'a mut dyn Write,
    ) -> ModExecutor<'a> {
        Self::with_extensions(
            dep_graph,
            Cow::Owned(Table::new()),
            Cow::Owned(IndexMap::new()),
            Some(MaybeCompleteSet::complete_set()),
            options,
            output_stream,
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:94 ModExecutor.<init>
    pub fn with_extensions(
        dep_graph: Cow<'a, IndexMap<ModuleKey, AugmentedModule>>,
        extension_usages: Cow<'a, Table<ModuleExtensionId, ModuleKey, ModuleExtensionUsage>>,
        extension_repos: Cow<'a, IndexMap<ModuleExtensionId, IndexSet<String>>>,
        extension_filter: Option<MaybeCompleteSet<ModuleExtensionId>>,
        options: ModOptions,
        output_stream: &'a mut dyn Write,
    ) -> ModExecutor<'a> {
        let ascii = options.charset != Charset::UTF8;
        let mut executor = ModExecutor {
            dep_graph,
            extension_usages,
            extension_repos,
            extension_filter,
            options,
            printer: Printer::new(output_stream, ascii),
            extension_repo_imports: IndexMap::new(),
        };
        // Easier lookup table for repo imports by module.
        // It is updated after pruneByDepthAndLink to filter out pruned modules.
        let keys: IndexSet<ModuleKey> = executor.dep_graph.keys().cloned().collect();
        executor.extension_repo_imports = executor.compute_repo_imports_table(&keys);
        executor
    }

    fn output(&mut self, result: &IndexMap<ModuleKey, ResultNode>) {
        let ctx = FormatterContext {
            result,
            dep_graph: &self.dep_graph,
            extension_repos: &self.extension_repos,
            extension_repo_imports: &self.extension_repo_imports,
            options: &self.options,
        };
        get_formatter(self.options.output_format).output(&ctx, &mut self.printer);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:116 ModExecutor.graph
    pub fn graph(&mut self, from: &IndexSet<ModuleKey>) {
        let targets = self.compute_extension_filter_targets();
        let result = if targets.is_empty() {
            self.expand_and_prune(from)
        } else {
            self.expand_paths_to_targets(from, &targets, false)
        };
        self.output(&result);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:128 ModExecutor.path
    pub fn path(&mut self, from: &IndexSet<ModuleKey>, to: &IndexSet<ModuleKey>) {
        let mut targets = self.compute_extension_filter_targets();
        targets.extend(to.iter().cloned());

        if targets.is_empty() {
            self.printer.println("No target modules specified.");
            self.printer.flush();
            return;
        }

        let result = self.expand_paths_to_targets(from, &targets, true);
        if result.is_empty() {
            self.printer
                .println("No path found to the specified target modules.");
            self.printer.flush();
            return;
        }
        self.output(&result);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:151 ModExecutor.allPaths
    pub fn all_paths(&mut self, from: &IndexSet<ModuleKey>, to: &IndexSet<ModuleKey>) {
        let mut targets = self.compute_extension_filter_targets();
        targets.extend(to.iter().cloned());

        if targets.is_empty() {
            self.printer.println("No target modules specified.");
            self.printer.flush();
            return;
        }

        let result = self.expand_paths_to_targets(from, &targets, false);
        if result.is_empty() {
            self.printer
                .println("No path found to the specified target modules.");
            self.printer.flush();
            return;
        }
        self.output(&result);
    }

    /// Needs `RepoDefinitionValue` and `RepoOutputFormatter`, which are not ported.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:174 ModExecutor.showRepo
    pub fn show_repo(&mut self) {
        blaze_util::unported!("ModExecutor.showRepo")
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:188 ModExecutor.showExtension
    pub fn show_extension(
        &mut self,
        extensions: &[ModuleExtensionId],
        from_usages: &IndexSet<ModuleKey>,
    ) -> Result<(), InvalidArgumentException> {
        for extension in extensions {
            // On an error Java leaves what was printed in the unflushed PrintWriter, where it is
            // never written out; the Printer here buffers the same way.
            self.display_extension(extension, from_usages)?;
        }
        self.printer.flush();
        Ok(())
    }

    /// Reconstructs a path backwards from a child to the root and adds it to the result graph.
    ///
    /// This is a helper function for [`Self::expand_paths_to_targets`]. Once a path to a target
    /// is found, this function is called to walk up the dependency chain (using the
    /// `bfs_parent_map`) and add the necessary nodes and edges to the `result_graph`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:204 ModExecutor.addPathToResultGraph
    fn add_path_to_result_graph(
        &self,
        result_graph: &mut HashMap<ModuleKey, ResultNode>,
        bfs_parent_map: &HashMap<ModuleKey, Option<ModuleKey>>,
        path_parent: Option<ModuleKey>,
        path_child: &ModuleKey,
    ) {
        // Mark the child node as a target in the result graph.
        let mut child_node_builder = ResultNode::builder();
        if let Some(existing) = result_graph.get(path_child) {
            child_node_builder.add_children(existing.get_children());
        }
        result_graph.insert(
            path_child.clone(),
            child_node_builder.set_target(true).build(),
        );

        // Traverse up from the found path to the root, adding the path to the result graph.
        let root_direct_children = self
            .module(&ModuleKey::root())
            .get_all_deps(self.options.include_unused);

        let mut current_child = path_child.clone();
        let mut current_parent = path_parent;

        while let Some(parent) = current_parent {
            let mut parent_node_builder = ResultNode::builder();

            // Preserve existing children if the parent node is already in the graph.
            if let Some(existing_node) = result_graph.get(&parent) {
                parent_node_builder
                    .add_children(existing_node.get_children())
                    .set_target(existing_node.is_target());
            }

            // Add the edge from parent to child.
            let is_indirect =
                parent == ModuleKey::root() && !root_direct_children.contains_key(&current_child);
            parent_node_builder.add_child(
                current_child.clone(),
                IsExpanded::TRUE,
                if is_indirect {
                    IsIndirect::TRUE
                } else {
                    IsIndirect::FALSE
                },
            );

            result_graph.insert(parent.clone(), parent_node_builder.build());

            // Move up the path.
            current_child = parent;
            current_parent = bfs_parent_map.get(&current_child).cloned().flatten();
        }
    }

    fn module(&self, key: &ModuleKey) -> &AugmentedModule {
        match self.dep_graph.get(key) {
            Some(m) => m,
            None => panic!("java.lang.NullPointerException: no module {key} in the dep graph"),
        }
    }

    /// The deps of `key`, filtered by `filterBuiltin`, sorted by `LEXICOGRAPHIC_COMPARATOR`.
    fn sorted_filtered_deps(&self, key: &ModuleKey) -> Vec<ModuleKey> {
        // getAllDeps is already sorted by the same comparator.
        self.module(key)
            .get_all_deps(self.options.include_unused)
            .into_keys()
            .filter(|k| self.filter_builtin(k))
            .collect()
    }

    /// Finds paths from a set of modules to a set of target modules and returns a dependency
    /// graph containing these paths.
    ///
    /// This function performs a breadth-first search (BFS) starting from the `from` modules to
    /// find paths to the `targets`. When a path is found, it's added to the result graph. The
    /// search can be configured to stop after finding a single path to each target or to find
    /// all possible paths. The final graph is then pruned to the depth specified in the options.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:264 ModExecutor.expandPathsToTargets
    pub fn expand_paths_to_targets(
        &mut self,
        from: &IndexSet<ModuleKey>,
        targets: &IndexSet<ModuleKey>,
        find_single_path: bool,
    ) -> IndexMap<ModuleKey, ResultNode> {
        // 1. Perform a BFS to find paths from the "from" modules to the "targets".
        // This map tracks the parent of each visited module to reconstruct paths later.
        let mut bfs_parent_map: HashMap<ModuleKey, Option<ModuleKey>> = HashMap::new();
        let mut sorted_from: Vec<&ModuleKey> =
            from.iter().filter(|k| self.filter_builtin(k)).collect();
        sorted_from.sort_by(|a, b| ModuleKey::lexicographic_cmp(a, b));
        for module_key in sorted_from {
            bfs_parent_map.insert(module_key.clone(), Some(ModuleKey::root()));
        }
        bfs_parent_map.insert(ModuleKey::root(), None); // The root has no parent.

        let mut result_graph: HashMap<ModuleKey, ResultNode> = HashMap::new();
        let mut queue: VecDeque<ModuleKey> = from.iter().cloned().collect();
        let mut found_targets: HashSet<ModuleKey> = HashSet::new();
        let contains_all = |found: &HashSet<ModuleKey>| targets.iter().all(|t| found.contains(t));

        while !queue.is_empty() {
            // If we only need one path to each target, and we've found them all, we can stop.
            if find_single_path && contains_all(&found_targets) {
                break;
            }

            let current_module_key = queue.pop_front().unwrap();

            if targets.contains(&current_module_key)
                && !(find_single_path && found_targets.contains(&current_module_key))
            {
                let parent = bfs_parent_map.get(&current_module_key).cloned().flatten();
                self.add_path_to_result_graph(
                    &mut result_graph,
                    &bfs_parent_map,
                    parent,
                    &current_module_key,
                );
                found_targets.insert(current_module_key.clone());
                if find_single_path && contains_all(&found_targets) {
                    break;
                }
            }

            let dependencies = self.sorted_filtered_deps(&current_module_key);

            for dep_key in dependencies {
                // A path to a target is found.
                if targets.contains(&dep_key)
                    && !(find_single_path && found_targets.contains(&dep_key))
                {
                    self.add_path_to_result_graph(
                        &mut result_graph,
                        &bfs_parent_map,
                        Some(current_module_key.clone()),
                        &dep_key,
                    );
                    found_targets.insert(dep_key.clone());
                }
                // If this dependency hasn't been visited, add it to the queue for traversal.
                if !bfs_parent_map.contains_key(&dep_key) {
                    bfs_parent_map.insert(dep_key.clone(), Some(current_module_key.clone()));
                    queue.push_back(dep_key);
                }
            }
        }

        // 2. Prune the resulting graph containing the found paths to the specified depth.
        let old_result: IndexMap<ModuleKey, ResultNode> = result_graph.into_iter().collect();
        self.prune_by_depth(MaybeCompleteSet::copy_of(targets), old_result)
    }

    /// Expands the full dependency graph starting from a given set of modules and then prunes
    /// it to the depth specified in the options.
    ///
    /// This function first performs a breadth-first traversal to build a complete graph of all
    /// dependencies reachable from the `from` modules. The `from` modules themselves are
    /// "pinned" as direct children of the root node in the resulting graph. Finally, it trims
    /// the graph to the requested depth.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:332 ModExecutor.expandAndPrune
    pub fn expand_and_prune(
        &mut self,
        from: &IndexSet<ModuleKey>,
    ) -> IndexMap<ModuleKey, ResultNode> {
        // This map will store the fully expanded dependency graph as ResultNode objects.
        let mut full_graph_builder: IndexMap<ModuleKey, ResultNode> = IndexMap::new();
        let put = |map: &mut IndexMap<ModuleKey, ResultNode>, k: ModuleKey, v: ResultNode| {
            if map.contains_key(&k) {
                panic!("IllegalArgumentException: Multiple entries with same key: {k}");
            }
            map.insert(k, v);
        };

        // 1. Initialize the graph with the ROOT module and its immediate "pinned" children.
        // "Pinned" children are the modules that are explicitly requested to start the graph
        // from.
        let mut root_builder = ResultNode::builder();
        let root_direct_children = self
            .module(&ModuleKey::root())
            .get_all_deps(self.options.include_unused);
        let root_direct_children_keys: IndexSet<ModuleKey> =
            root_direct_children.keys().cloned().collect();
        let pinned_children: Vec<ModuleKey> = self
            .get_pinned_children_of_root_in_the_result_graph(&root_direct_children_keys, from)
            .into_iter()
            .filter(|k| self.filter_builtin(k))
            .collect();

        for pinned_child in &pinned_children {
            let is_direct = root_direct_children.contains_key(pinned_child);
            root_builder.add_child(
                pinned_child.clone(),
                IsExpanded::TRUE,
                if is_direct {
                    IsIndirect::FALSE
                } else {
                    IsIndirect::TRUE
                },
            );
        }
        put(
            &mut full_graph_builder,
            ModuleKey::root(),
            root_builder.build(),
        );

        // 2. Traverse the dependency graph starting from the pinned children (BFS).
        let mut visited: HashSet<ModuleKey> = pinned_children.iter().cloned().collect();
        let mut queue: VecDeque<ModuleKey> = pinned_children.iter().cloned().collect();
        visited.insert(ModuleKey::root());

        while let Some(current_module_key) = queue.pop_front() {
            let mut node_builder = ResultNode::builder();

            let dependencies = self.sorted_filtered_deps(&current_module_key);

            for dep_key in dependencies {
                if visited.contains(&dep_key) {
                    // This dependency has been seen before, but we add a non-expanded edge to it.
                    node_builder.add_child(dep_key, IsExpanded::FALSE, IsIndirect::FALSE);
                } else {
                    // New dependency found, add it to the queue to visit and mark as expanded.
                    node_builder.add_child(dep_key.clone(), IsExpanded::TRUE, IsIndirect::FALSE);
                    visited.insert(dep_key.clone());
                    queue.push_back(dep_key);
                }
            }
            put(
                &mut full_graph_builder,
                current_module_key,
                node_builder.build(),
            );
        }

        // 3. Prune the fully expanded graph based on the specified depth.
        self.prune_by_depth(MaybeCompleteSet::complete_set(), full_graph_builder)
    }

    /// `new ResultGraphPruner(targets, oldResult).pruneByDepth()`.
    fn prune_by_depth(
        &mut self,
        targets: MaybeCompleteSet<ModuleKey>,
        old_result: IndexMap<ModuleKey, ResultNode>,
    ) -> IndexMap<ModuleKey, ResultNode> {
        let mut pruner = ResultGraphPruner {
            old_result: &old_result,
            arena: Vec::new(),
            result_builder: HashMap::new(),
            parent_stack: HashSet::new(),
            targets,
            depth: self.options.depth,
            cycles: self.options.cycles,
        };
        let Some(result) = pruner.prune_by_depth() else {
            return IndexMap::new();
        };
        // Filter imports for nodes that were pruned during this process.
        let keys: IndexSet<ModuleKey> = result.keys().cloned().collect();
        self.extension_repo_imports = self.compute_repo_imports_table(&keys);
        result
    }

    /// Return a sorted list of modules that will be the direct children of the root in the
    /// result graph (original root's direct dependencies along with the specified targets).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:529 ModExecutor.getPinnedChildrenOfRootInTheResultGraph
    fn get_pinned_children_of_root_in_the_result_graph(
        &self,
        root_direct_deps: &IndexSet<ModuleKey>,
        from_targets: &IndexSet<ModuleKey>,
    ) -> Vec<ModuleKey> {
        let mut target_keys: HashSet<ModuleKey> = from_targets.iter().cloned().collect();
        if from_targets.contains(&ModuleKey::root()) {
            target_keys.remove(&ModuleKey::root());
            target_keys.extend(root_direct_deps.iter().cloned());
        }
        let mut sorted: Vec<ModuleKey> = target_keys.into_iter().collect();
        sorted.sort_by(ModuleKey::lexicographic_cmp);
        sorted
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:539 ModExecutor.intersect
    fn intersect(a: &MaybeCompleteSet<ModuleExtensionId>, b: &[&ModuleExtensionId]) -> bool {
        if a.is_complete() {
            return !b.is_empty();
        }
        let elements = a.get_elements_if_not_complete();
        b.iter().any(|x| elements.contains(*x))
    }

    /// If the extensionFilter option is set, computes the set of target modules that use the
    /// specified extension(s).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:551 ModExecutor.computeExtensionFilterTargets
    fn compute_extension_filter_targets(&self) -> IndexSet<ModuleKey> {
        let Some(extension_filter) = &self.extension_filter else {
            return IndexSet::new();
        };
        self.dep_graph
            .keys()
            .filter(|k| self.filter_unused(k))
            .filter(|k| self.filter_builtin(k))
            .filter(|k| {
                let used: Vec<&ModuleExtensionId> = self
                    .extension_usages
                    .column(k)
                    .map(|c| c.keys().collect())
                    .unwrap_or_default();
                Self::intersect(extension_filter, &used)
            })
            .cloned()
            .collect()
    }

    /// Compute the multimap of repo imports to modules for each extension.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:563 ModExecutor.computeRepoImportsTable
    fn compute_repo_imports_table(
        &self,
        present_modules: &IndexSet<ModuleKey>,
    ) -> ExtensionRepoImports {
        let mut result_builder: ExtensionRepoImports = IndexMap::new();
        for extension in self.extension_usages.row_key_set() {
            if let Some(filter) = &self.extension_filter
                && !filter.contains(extension)
            {
                continue;
            }
            // ImmutableSetMultimap<ModuleKey, String>
            let mut modules_to_imports_builder: IndexMap<ModuleKey, IndexSet<String>> =
                IndexMap::new();
            if let Some(row) = self.extension_usages.row(extension) {
                for (module, usage) in row {
                    if !present_modules.contains(module) {
                        continue;
                    }
                    for proxy in &usage.proxies {
                        let entry = modules_to_imports_builder
                            .entry(module.clone())
                            .or_default();
                        for v in proxy.get_imports().values() {
                            entry.insert(v.clone());
                        }
                    }
                }
            }
            // .inverse(): entries in multimap order, swapped.
            let mut inverse: IndexMap<String, IndexSet<ModuleKey>> = IndexMap::new();
            for (module, repos) in &modules_to_imports_builder {
                for repo in repos {
                    inverse
                        .entry(repo.clone())
                        .or_default()
                        .insert(module.clone());
                }
            }
            if result_builder.contains_key(extension) {
                panic!("IllegalArgumentException: Multiple entries with same key: {extension}");
            }
            result_builder.insert(extension.clone(), inverse);
        }
        result_builder
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:587 ModExecutor.filterUnused
    fn filter_unused(&self, key: &ModuleKey) -> bool {
        let module = self.module(key);
        self.options.include_unused || module.is_used()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:592 ModExecutor.filterBuiltin
    fn filter_builtin(&self, key: &ModuleKey) -> bool {
        self.options.include_builtin || !Self::is_builtin(key)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:596 ModExecutor.tagToFunctionArgs
    fn tag_to_function_args(attributes: &blaze_bzlmod_model::AttributeValues) -> String {
        let mut entries: Vec<&(String, blaze_bzlmod_model::AttributeValue)> =
            attributes.attributes().iter().collect();
        // show 'name' first for readability, similar to buildifier
        let sort_key = |s: &str| -> String {
            if s == "name" {
                String::new()
            } else {
                s.to_string()
            }
        };
        entries.sort_by(|a, b| java_string_cmp(&sort_key(&a.0), &sort_key(&b.0)));
        entries
            .iter()
            .map(|(k, v)| format!("{}={}", k, starlark_repr(v)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Helper to display show_extension info.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:605 ModExecutor.displayExtension
    fn display_extension(
        &mut self,
        extension: &ModuleExtensionId,
        from_usages: &IndexSet<ModuleKey>,
    ) -> Result<(), InvalidArgumentException> {
        self.printer.print(&format!("## {extension}:\n"));
        self.printer.println("");
        self.printer.println("Fetched repositories:");
        let Some(imports) = self.extension_repo_imports.get(extension) else {
            return Err(InvalidArgumentException::new(format!(
                "No extension {extension} exists in the dependency graph"
            )));
        };
        let mut used_repos: Vec<&String> = imports.keys().collect();
        used_repos.sort_by(|a, b| java_string_cmp(a, b));
        let mut unused_repos: Vec<&String> = self
            .extension_repos
            .get(extension)
            .map(|s| s.iter().filter(|r| !imports.contains_key(*r)).collect())
            .unwrap_or_default();
        unused_repos.sort_by(|a, b| java_string_cmp(a, b));
        let mut lines = String::new();
        for repo in &used_repos {
            let mut importers: Vec<&ModuleKey> = imports[*repo].iter().collect();
            importers.sort_by(|a, b| ModuleKey::lexicographic_cmp(a, b));
            let importers: Vec<String> = importers.iter().map(|k| k.to_string()).collect();
            lines.push_str(&format!(
                "  - {} (imported by {})\n",
                repo,
                importers.join(", ")
            ));
        }
        for repo in &unused_repos {
            lines.push_str(&format!("  - {repo}\n"));
        }
        self.printer.print(&lines);
        self.printer.println("");
        let from_usages: IndexSet<ModuleKey> = if from_usages.is_empty() {
            match self.extension_usages.row(extension) {
                Some(row) => row.keys().cloned().collect(),
                None => panic!("java.lang.NullPointerException"),
            }
        } else {
            from_usages.clone()
        };
        for module in &from_usages {
            let Some(usage) = self.extension_usages.get(extension, module) else {
                continue;
            };
            // TODO: maybe consider printing each proxy separately? Might be relevant for included
            //  segments.
            let first = match usage.proxies.first() {
                Some(p) => p,
                None => panic!("java.util.NoSuchElementException"),
            };
            self.printer.print(&format!(
                "## Usage in {} from {}:{}\n",
                module,
                first.location.file(),
                first.location.line()
            ));

            if extension.is_innate() {
                // This is for the special case of "innate" extensions: fake module extensions
                // created by use_repo_rule(). The name of the extension is of the form
                // "<bzl_file_label> <rule_name>". Rule names cannot contain spaces, so we can split
                // on the last space.
                let name = extension.extension_name();
                let last_space = name.rfind(' ').unwrap();
                let raw_label = &name[..last_space];
                let rule_name = &name[last_space + 1..];

                self.printer.print(&format!(
                    "{rule_name} = use_repo_rule(\"{raw_label}\", \"{rule_name}\")\n"
                ));

                for tag in &usage.tags {
                    // use_repo_rule creates a fake repo extension with a single tag 'repo'.
                    // However, code defensively and print the tag name if it's not 'repo'.
                    let callee = if tag.tag_name != "repo" {
                        format!("{}.{}", rule_name, tag.tag_name)
                    } else {
                        rule_name.to_string()
                    };
                    self.printer.print(&format!(
                        "{}({})\n",
                        callee,
                        Self::tag_to_function_args(&tag.attribute_values)
                    ));
                }

                // Skip the use_repo part since every call to the repo rule creates a repo that is
                // imported.
                self.printer.println("");
            } else {
                for tag in &usage.tags {
                    self.printer.print(&format!(
                        "{}.{}({})\n",
                        extension.extension_name(),
                        tag.tag_name,
                        Self::tag_to_function_args(&tag.attribute_values)
                    ));
                }
                self.printer.print("use_repo(\n");
                self.printer
                    .print(&format!("  {},\n", extension.extension_name()));
                for proxy in &usage.proxies {
                    for (k, v) in proxy.get_imports() {
                        let s = if k == v {
                            format!("\"{k}\"")
                        } else {
                            format!("{k}=\"{v}\"")
                        };
                        self.printer.print(&format!("  {s},\n"));
                    }
                }
                self.printer.print(")\n\n");
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:694 ModExecutor.isBuiltin
    fn is_builtin(key: &ModuleKey) -> bool {
        *key == ModuleKey::new("bazel_tools", Version::empty())
    }
}

/// `Starlark.repr` of an attribute value.
// Java: net/starlark/java/eval/Printer.java Printer.repr
// PORT: Java's StarlarkList and Tuple are both `AttributeValue::List` here (repr `[...]`); the
// model crate does not keep tuples apart.
fn starlark_repr(v: &blaze_bzlmod_model::AttributeValue) -> String {
    use blaze_bzlmod_model::AttributeValue;
    match v {
        AttributeValue::None => "None".to_string(),
        AttributeValue::Bool(true) => "True".to_string(),
        AttributeValue::Bool(false) => "False".to_string(),
        AttributeValue::Int(i) => i.to_string(),
        AttributeValue::String(s) => blaze_starlark::printer::quote(s),
        // Java: com/google/devtools/build/lib/cmdline/Label.java:718 Label.repr
        AttributeValue::Label(l) => format!(
            "Label({})",
            blaze_starlark::printer::quote(&l.get_canonical_form())
        ),
        AttributeValue::List(items) => format!(
            "[{}]",
            items
                .iter()
                .map(starlark_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        AttributeValue::Dict(entries) => format!(
            "{{{}}}",
            entries
                .iter()
                .map(|(k, v)| format!("{}: {}", starlark_repr(k), starlark_repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Prunes the result tree after the specified depth.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:387 ModExecutor.ResultGraphPruner
struct ResultGraphPruner<'r> {
    old_result: &'r IndexMap<ModuleKey, ResultNode>,
    /// The builders, by identity (Java keeps `ResultNode.Builder` references in a `HashMap`).
    arena: Vec<ResultNodeBuilder>,
    result_builder: HashMap<ModuleKey, usize>,
    parent_stack: HashSet<ModuleKey>,
    /// If not complete, it means that the result graph contains paths to some specific targets.
    /// This will cause some branches to contain, after the specified depths, some targets or
    /// target parents. As any other nodes omitted, transitive edges (embedding multiple edges)
    /// will be stored as *indirect*.
    targets: MaybeCompleteSet<ModuleKey>,
    depth: i32,
    cycles: bool,
}

impl<'r> ResultGraphPruner<'r> {
    fn new_builder(&mut self) -> usize {
        self.arena.push(ResultNode::builder());
        self.arena.len() - 1
    }

    fn old_node(&self, key: &ModuleKey) -> &'r ResultNode {
        match self.old_result.get(key) {
            Some(n) => n,
            None => panic!("java.lang.NullPointerException: no result node for {key}"),
        }
    }

    /// Prunes the result tree after the specified depth using DFS (because some nodes may still
    /// appear after the max depth). Returns `None` for an empty input (Java returns
    /// `ImmutableMap.of()` without recomputing the repo imports).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:414 ModExecutor.ResultGraphPruner.pruneByDepth
    fn prune_by_depth(&mut self) -> Option<IndexMap<ModuleKey, ResultNode>> {
        if self.old_result.is_empty() {
            return None;
        }

        let root_builder = self.new_builder();
        self.result_builder.insert(ModuleKey::root(), root_builder);

        self.parent_stack.insert(ModuleKey::root());

        let root_children = self
            .old_node(&ModuleKey::root())
            .get_children_sorted_by_key();
        for (key, meta) in root_children {
            self.arena[root_builder].add_child(key.clone(), IsExpanded::TRUE, meta.is_indirect);
            self.visit_visible(&key, 1, &ModuleKey::root(), IsExpanded::TRUE);
        }

        // Build everything at the end to allow children to add themselves to their parent's
        // adjacency list.
        let mut entries: Vec<(ModuleKey, ResultNode)> = self
            .result_builder
            .iter()
            .map(|(k, idx)| (k.clone(), self.arena[*idx].build()))
            .collect();
        entries.sort_by(|a, b| ModuleKey::lexicographic_cmp(&a.0, &b.0));
        Some(entries.into_iter().collect())
    }

    /// Handles graph traversal within the specified depth.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:445 ModExecutor.ResultGraphPruner.visitVisible
    fn visit_visible(
        &mut self,
        module_key: &ModuleKey,
        depth: i32,
        parent_key: &ModuleKey,
        expanded: IsExpanded,
    ) {
        self.parent_stack.insert(module_key.clone());
        let old_node = self.old_node(module_key);
        let existing = self.result_builder.get(module_key).copied();
        let node_builder = match existing {
            Some(idx) => idx,
            None => {
                let idx = self.new_builder();
                self.result_builder.insert(module_key.clone(), idx);
                idx
            }
        };

        self.arena[node_builder].set_target(old_node.is_target());
        if depth > 1 {
            let parent = self.result_builder[parent_key];
            self.arena[parent].add_child(module_key.clone(), expanded, IsIndirect::FALSE);
        }

        if expanded == IsExpanded::FALSE {
            self.parent_stack.remove(module_key);
            return;
        }
        for (child_key, meta) in old_node.get_children_sorted_by_key() {
            let child_expanded = meta.is_expanded;
            if self.not_cycle(&child_key) {
                if depth < self.depth {
                    self.visit_visible(&child_key, depth + 1, module_key, child_expanded);
                } else if !self.targets.is_complete() {
                    self.visit_detached(&child_key, module_key, module_key, child_expanded);
                }
            } else if self.cycles {
                self.arena[node_builder].add_cycle(child_key);
            }
        }
        self.parent_stack.remove(module_key);
    }

    /// Detached mode is only present in withTargets and handles adding targets and target
    /// parents living below the specified depth to the graph.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:479 ModExecutor.ResultGraphPruner.visitDetached
    fn visit_detached(
        &mut self,
        module_key: &ModuleKey,
        parent_key: &ModuleKey,
        last_visible_parent_key: &ModuleKey,
        expanded: IsExpanded,
    ) {
        self.parent_stack.insert(module_key.clone());
        let old_node = self.old_node(module_key);
        let node_builder = self.new_builder();
        self.arena[node_builder].set_target(old_node.is_target());

        let mut last_visible_parent_key = last_visible_parent_key.clone();
        if old_node.is_target() || self.is_target_parent(old_node) {
            let parent_builder = self.result_builder[&last_visible_parent_key];
            let child_indirect = if last_visible_parent_key == *parent_key {
                IsIndirect::FALSE
            } else {
                IsIndirect::TRUE
            };
            self.arena[parent_builder].add_child(module_key.clone(), expanded, child_indirect);
            self.result_builder.insert(module_key.clone(), node_builder);
            last_visible_parent_key = module_key.clone();
        }

        if expanded == IsExpanded::FALSE {
            self.parent_stack.remove(module_key);
            return;
        }
        for (child_key, meta) in old_node.get_children_sorted_by_key() {
            let child_expanded = meta.is_expanded;
            if self.not_cycle(&child_key) {
                self.visit_detached(
                    &child_key,
                    module_key,
                    &last_visible_parent_key,
                    child_expanded,
                );
            } else if self.cycles {
                self.arena[node_builder].add_cycle(child_key);
            }
        }
        self.parent_stack.remove(module_key);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:514 ModExecutor.ResultGraphPruner.notCycle
    fn not_cycle(&self, key: &ModuleKey) -> bool {
        !self.parent_stack.contains(key)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModExecutor.java:518 ModExecutor.ResultGraphPruner.isTargetParent
    fn is_target_parent(&self, node: &ResultNode) -> bool {
        node.get_children()
            .keys()
            .filter(|k| !self.parent_stack.contains(*k))
            .any(|k| self.targets.contains(k))
    }
}
