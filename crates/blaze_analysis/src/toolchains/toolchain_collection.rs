//! Port of `analysis/ToolchainCollection.java`.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::rule_class::DEFAULT_EXEC_GROUP_NAME;
use indexmap::IndexMap;

use crate::platform::PlatformInfo;
use crate::toolchains::toolchain_context::ToolchainContext;

/// A wrapper class for a map of exec_group names to their relevant [`ToolchainContext`].
///
/// `T` is any [`ToolchainContext`], so the collection is used both before and after toolchain
/// resolution.
// Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:39 ToolchainCollection
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolchainCollection<T> {
    /// A map of execution group names to toolchain contexts.
    context_map: IndexMap<String, T>,
}

impl<T: ToolchainContext> ToolchainCollection<T> {
    pub fn context_map(&self) -> &IndexMap<String, T> {
        &self.context_map
    }

    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:44 ToolchainCollection.getDefaultToolchainContext
    // PORT: the builder guarantees the default context exists (Java returns it unchecked).
    pub fn get_default_toolchain_context(&self) -> &T {
        self.context_map
            .get(DEFAULT_EXEC_GROUP_NAME)
            .expect("NullPointerException: no default toolchain context")
    }

    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:48 ToolchainCollection.hasToolchainContext
    pub fn has_toolchain_context(&self, exec_group: &str) -> bool {
        self.context_map.contains_key(exec_group)
    }

    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:52 ToolchainCollection.getToolchainContext
    pub fn get_toolchain_context(&self, exec_group: &str) -> Option<&T> {
        self.context_map.get(exec_group)
    }

    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:56 ToolchainCollection.getResolvedToolchains
    pub fn get_resolved_toolchains(&self) -> Vec<Label> {
        let mut labels: Vec<Label> = Vec::new();
        for c in self.context_map.values() {
            for l in c.resolved_toolchain_labels() {
                if !labels.contains(&l) {
                    labels.push(l);
                }
            }
        }
        labels
    }

    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:62 ToolchainCollection.getExecGroupNames
    pub fn get_exec_group_names(&self) -> Vec<&str> {
        self.context_map.keys().map(String::as_str).collect()
    }

    /// This is safe because all toolchain context in a toolchain collection should have the same
    /// target platform
    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:70 ToolchainCollection.getTargetPlatform
    pub fn get_target_platform(&self) -> &Arc<PlatformInfo> {
        self.get_default_toolchain_context().target_platform()
    }

    /// Returns a new builder for [`ToolchainCollection`] instances.
    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:80 ToolchainCollection.builder
    pub fn builder() -> ToolchainCollectionBuilder<T> {
        ToolchainCollectionBuilder {
            toolchain_contexts: IndexMap::new(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:84 ToolchainCollection.builderWithExpectedSize
    pub fn builder_with_expected_size(expected_size: usize) -> ToolchainCollectionBuilder<T> {
        ToolchainCollectionBuilder {
            toolchain_contexts: IndexMap::with_capacity(expected_size),
        }
    }
}

/// Builder for [`ToolchainCollection`].
// Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:89 ToolchainCollection.Builder
#[derive(Clone, Debug)]
pub struct ToolchainCollectionBuilder<T> {
    toolchain_contexts: IndexMap<String, T>,
}

impl<T: ToolchainContext + Clone> ToolchainCollectionBuilder<T> {
    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:101 ToolchainCollection.Builder.build
    pub fn build(&self) -> ToolchainCollection<T> {
        blaze_util::check_argument!(
            self.toolchain_contexts
                .contains_key(DEFAULT_EXEC_GROUP_NAME)
        );
        ToolchainCollection {
            context_map: self.toolchain_contexts.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:107 ToolchainCollection.Builder.addContext
    pub fn add_context(&mut self, exec_group: &str, context: T) {
        blaze_util::check_argument!(
            !self.toolchain_contexts.contains_key(exec_group),
            "Duplicate add of '{}' exec group to toolchain collection.",
            exec_group
        );
        self.toolchain_contexts
            .insert(exec_group.to_owned(), context);
    }

    // Java: com/google/devtools/build/lib/analysis/ToolchainCollection.java:116 ToolchainCollection.Builder.addDefaultContext
    pub fn add_default_context(&mut self, context: T) -> &mut Self {
        self.add_context(DEFAULT_EXEC_GROUP_NAME, context);
        self
    }
}
