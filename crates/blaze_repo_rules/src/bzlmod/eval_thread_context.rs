//! The thread context of a module extension evaluation: records the repo rule calls the
//! extension makes and instantiates them when the extension is done.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalStarlarkThreadContext.java.
//!
//! PORT: Java stores this object in the `StarlarkThread` (it extends `StarlarkThreadContext`,
//! whose main repo mapping is the thread's); Blaze keeps it in the thread's
//! `crate::starlark::context::ExternalContext` payload (`ExternalContextKind::ModuleExtension`)
//! and passes the main repo mapping to `crate::starlark::thread::call_implementation`. The
//! `ExistingRulesShouldBeNoOp` thread local (a hack for `native.existing_rules()` in macros
//! called from extensions) is not ported: `native` is not available to extensions in Blaze's
//! module extension environment.

use std::sync::Arc;

use anyhow::anyhow;
use blaze_bzlmod_model::AttributeValue;
use blaze_bzlmod_model::ExternalDepsException;
use blaze_bzlmod_model::ModuleExtensionId;
use blaze_bzlmod_model::RepoSpec;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use blaze_events::ExtendedEventHandler;
use blaze_packages::starlark_defs::util::call_stack;
use blaze_packages::starlark_defs::util::caller_location;
use blaze_packages::types::label_converter::LabelConverter;
use blaze_starlark::Location;
use blaze_starlark::error::StackEntry;
use indexmap::IndexMap;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::values::Value;

use crate::attr_values::attribute_value_to_starlark;
use crate::attr_values::starlark_to_attribute_value;
use crate::repository::repo_rule::RepoRule;

/// A deferred call to a repo rule.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalStarlarkThreadContext.java:60 ModuleExtensionEvalStarlarkThreadContext.RepoRuleCall
struct RepoRuleCall {
    repo_rule: Arc<RepoRule>,
    /// Deep-cloned keyword arguments (Java: an immutable `Dict<String, Object>`).
    kwargs: Vec<(String, AttributeValue)>,
    location: Location,
    call_stack: Vec<StackEntry>,
}

/// A context object that should be stored in a Starlark thread for use during module extension
/// evaluation.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalStarlarkThreadContext.java:46 ModuleExtensionEvalStarlarkThreadContext
pub struct ModuleExtensionEvalStarlarkThreadContext {
    extension_id: ModuleExtensionId,
    repo_prefix: String,
    base_package_id: PackageIdentifier,
    base_repo_mapping: RepositoryMapping,
    repo_overrides: IndexMap<String, RepositoryName>,
    /// Java `StarlarkThreadContext.getMainRepoMappingSupplier`.
    main_repo_mapping: RepositoryMapping,
    deferred_repos: IndexMap<String, RepoRuleCall>,
}

impl ModuleExtensionEvalStarlarkThreadContext {
    /// PORT: Java also takes the `ExtendedEventHandler` used by `createRepos`; it is passed to
    /// [`Self::create_repos`] instead.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalStarlarkThreadContext.java:74 ModuleExtensionEvalStarlarkThreadContext.<init>
    pub fn new(
        extension_id: ModuleExtensionId,
        repo_prefix: String,
        base_package_id: PackageIdentifier,
        base_repo_mapping: RepositoryMapping,
        repo_overrides: IndexMap<String, RepositoryName>,
        main_repo_mapping: RepositoryMapping,
    ) -> Self {
        ModuleExtensionEvalStarlarkThreadContext {
            extension_id,
            repo_prefix,
            base_package_id,
            base_repo_mapping,
            repo_overrides,
            main_repo_mapping,
            deferred_repos: IndexMap::new(),
        }
    }

    /// The main repo mapping (Java `getMainRepoMappingSupplier().get()`).
    pub fn main_repo_mapping(&self) -> &RepositoryMapping {
        &self.main_repo_mapping
    }

    /// Records a call to a repo rule that should be created at the end of the module extension
    /// evaluation. `eval` is the thread running the repo rule (a builtin) called by the
    /// extension. The error is the `EvalException` message.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalStarlarkThreadContext.java:96 ModuleExtensionEvalStarlarkThreadContext.lazilyCreateRepo
    pub fn lazily_create_repo<'v>(
        &mut self,
        eval: &Evaluator<'v, '_, '_>,
        kwargs: Vec<(String, Value<'v>)>,
        repo_rule: Arc<RepoRule>,
    ) -> anyhow::Result<()> {
        let name_value = kwargs
            .iter()
            .find(|(k, _)| k == "name")
            .map(|(_, v)| *v)
            .unwrap_or_else(Value::new_none);
        let Some(name) = name_value.unpack_str() else {
            return Err(anyhow!(
                "expected string for attribute 'name', got '{}'",
                name_value.get_type()
            ));
        };
        RepositoryName::validate_user_provided_repo_name(name)
            .map_err(|e| anyhow!("{}", e.get_message()))?;
        if let Some(conflict) = self.deferred_repos.get(name) {
            return Err(anyhow!(
                "A repo named {} is already generated by this module extension at {}",
                name,
                conflict.location
            ));
        }
        let mut stack = call_stack(eval);
        // Pop the call to the repo rule itself
        stack.pop();
        // The extension may mutate the values of the kwargs after this function returns.
        let mut cloned_kwargs = Vec::with_capacity(kwargs.len());
        for (k, v) in &kwargs {
            cloned_kwargs.push((
                k.clone(),
                starlark_to_attribute_value(*v).map_err(|e| anyhow!("{e}"))?,
            ));
        }
        self.deferred_repos.insert(
            name.to_owned(),
            RepoRuleCall {
                repo_rule,
                kwargs: cloned_kwargs,
                location: caller_location(eval),
                call_stack: stack,
            },
        );
        Ok(())
    }

    /// Evaluates the repo rule calls recorded by [`Self::lazily_create_repo`] and returns all
    /// repos generated by the extension. The key is the "internal name" (as specified by the
    /// extension) of the repo, and the value is the [`RepoSpec`].
    ///
    /// PORT: the recorded keyword arguments are stored data; they are allocated on a temporary
    /// heap again for `RepoRule::instantiate`, which type-checks Starlark values.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalStarlarkThreadContext.java:127 ModuleExtensionEvalStarlarkThreadContext.createRepos
    pub fn create_repos(
        &self,
        event_handler: &dyn ExtendedEventHandler,
    ) -> Result<IndexMap<String, RepoSpec>, ExternalDepsException> {
        // LINT.IfChange
        // Make it possible to refer to extension repos in the label attributes of another
        // extension repo. Wrapping a label in Label(...) ensures that it is evaluated with respect
        // to the containing module's repo mapping instead.
        // ImmutableMap.Builder.buildKeepingLast: the last value wins, at the key's first position.
        let mut entries: IndexMap<String, RepositoryName> = IndexMap::new();
        for (k, v) in self.base_repo_mapping.entries() {
            entries.insert(k.clone(), v.clone());
        }
        for apparent_name in self.deferred_repos.keys() {
            entries.insert(
                apparent_name.clone(),
                RepositoryName::create_unvalidated(&format!(
                    "{}{}",
                    self.repo_prefix, apparent_name
                )),
            );
        }
        for (k, v) in &self.repo_overrides {
            entries.insert(k.clone(), v.clone());
        }
        let full_repo_mapping =
            RepositoryMapping::create(entries, self.base_repo_mapping.context_repo().clone());
        // LINT.ThenChange(//src/main/java/com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionRepoMappingEntriesFunction.java)

        let label_converter = LabelConverter::new(&self.base_package_id, full_repo_mapping);
        let repo_mapping_where = format!("in the extension '{}'", self.extension_id);
        let mut repo_specs = IndexMap::with_capacity(self.deferred_repos.len());
        for (name, repo_rule_call) in &self.deferred_repos {
            let repo_spec = Module::with_temp_heap(|module| {
                let heap = module.heap();
                let kwargs: Vec<(String, Value)> = repo_rule_call
                    .kwargs
                    .iter()
                    .map(|(k, v)| (k.clone(), attribute_value_to_starlark(v, heap)))
                    .collect();
                repo_rule_call.repo_rule.instantiate(
                    &kwargs,
                    &repo_rule_call.call_stack,
                    &label_converter,
                    event_handler,
                    &repo_mapping_where,
                )
            })?;
            repo_specs.insert(name.clone(), repo_spec);
        }
        Ok(repo_specs)
    }
}
