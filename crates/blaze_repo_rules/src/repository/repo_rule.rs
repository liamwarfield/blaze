//! `RepoRule`: a repository rule as defined by `repository_rule()`.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/RepoRule.java. `RepoRule.instantiate`
//! and `AttributeUtils` are in [`super::attribute_utils`].

use std::fmt;
use std::sync::Arc;

use blaze_bzlmod_model::RepoRuleId;
use blaze_cmdline::RepositoryName;
use blaze_packages::attribute::Attribute;
use blaze_packages::bzl_context::BazelModuleContext;
use indexmap::IndexMap;
use indexmap::IndexSet;
use starlark::values::OwnedFrozenValue;

/// A list of forbidden attribute names. These used to be present on all repo rules simply
/// because they were built-in attributes for all rules.
// Java: com/google/devtools/build/lib/bazel/repository/RepoRule.java:58 RepoRule.LEGACY_BUILTIN_ATTRIBUTES
pub const LEGACY_BUILTIN_ATTRIBUTES: [&str; 4] = ["name", "tags", "deprecation", "visibility"];

/// A repository rule (Java record `RepoRule`). Immutable; built once per exported
/// `repository_rule` value.
// Java: com/google/devtools/build/lib/bazel/repository/RepoRule.java:46 RepoRule
pub struct RepoRule {
    pub id: RepoRuleId,
    pub transitive_bzl_digest: Vec<u8>,
    /// Java `ImmutableTable<RepositoryName, String, RepositoryName>` cells
    /// `(fromRepo, apparentName, canonicalName)`, in the table's cell order.
    pub recorded_repo_mapping_entries: Vec<(RepositoryName, String, RepositoryName)>,
    /// The implementation function. PORT: keep-alive: the handle does not own the frozen heap of
    /// the defining .bzl, which the build arena keeps (see `repository_module::get_repo_rule`).
    /// Read it with [`RepoRule::get_impl`], which checks [`RepoRule::impl_stamp`].
    pub impl_: OwnedFrozenValue,
    /// The build arena generation `impl_` was made in (`blaze_util::build_arena::ArenaStamp`).
    pub impl_stamp: blaze_util::build_arena::ArenaStamp,
    pub doc: Option<String>,
    pub attributes: Vec<Attribute>,
    pub attribute_indices: IndexMap<String, usize>,
    pub local: bool,
    pub configure: bool,
    pub remotable: bool,
    pub environ: IndexSet<String>,
    /// The `BazelModuleContext` of the .bzl that exported the rule (Java finds the innermost
    /// .bzl of the running function; see `crate::starlark::thread::call_implementation`).
    pub module_context: Option<Arc<BazelModuleContext>>,
}

impl RepoRule {
    /// The implementation function; panics if the build arena that keeps its heap was released
    /// since the handle was made (a holder that outlived its command).
    pub fn get_impl(&self) -> &OwnedFrozenValue {
        self.impl_stamp.check("RepoRule.impl_");
        &self.impl_
    }
}

impl fmt::Debug for RepoRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RepoRule")
            .field("id", &self.id)
            .field("local", &self.local)
            .field("configure", &self.configure)
            .field("environ", &self.environ)
            .finish_non_exhaustive()
    }
}

/// Builder type for [`RepoRule`] (Java `RepoRule.Builder`, an `@AutoBuilder`).
// Java: com/google/devtools/build/lib/bazel/repository/RepoRule.java:118 RepoRule.Builder
#[derive(Default)]
pub struct RepoRuleBuilder {
    attr_names: IndexSet<String>,
    pub attributes: Vec<Attribute>,
    pub attribute_indices: IndexMap<String, usize>,
}

impl RepoRuleBuilder {
    // Java: com/google/devtools/build/lib/bazel/repository/RepoRule.java:136 RepoRule.Builder.addAttribute
    pub fn add_attribute(&mut self, attribute: Attribute) -> &mut Self {
        let public_name = attribute.get_public_name();
        self.attribute_indices
            .insert(public_name.clone(), self.attr_names.len());
        self.attr_names.insert(public_name);
        self.attributes.push(attribute);
        self
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepoRule.java:143 RepoRule.Builder.hasAttribute
    pub fn has_attribute(&self, attr_name: &str) -> bool {
        self.attr_names.contains(attr_name) || LEGACY_BUILTIN_ATTRIBUTES.contains(&attr_name)
    }
}
