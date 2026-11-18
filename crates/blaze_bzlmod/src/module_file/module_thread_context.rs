//! Port of `ModuleThreadContext` (with `ModuleExtensionUsageBuilder`, `RepoOverride` and
//! `RepoNameUsage`). OWNER: agent A.
//!
//! PORT: Java's `ModuleExtensionProxy` holds references to a `ModuleExtensionUsageBuilder` and a
//! `ModuleExtensionUsage.Proxy.Builder` that the context also reaches. Here the context owns every
//! usage builder in an arena (including the detached ones Java creates for ignored dev-dependency
//! proxies) and the Starlark proxy values hold indices ([`ProxyRef`]).

use std::sync::Arc;

use blaze_bzlmod_model::{
    AttributeValue, AttributeValues, DepSpec, InterimModule, InterimModuleBuilder, IsolationKey,
    ModuleExtensionUsage, ModuleExtensionUsageProxy, ModuleKey, ModuleOverride,
    NonRegistryOverride, Registry, RepoOverride as UsageRepoOverride, RepoRuleId, RepoSpec, Tag,
    Version,
};
use blaze_cmdline::label_constants::MODULE_DOT_BAZEL_FILE_NAME;
use blaze_cmdline::{Label, RepositoryName};
use blaze_events::{Event, Location};
use blaze_options::spelling::SpellChecker;
use blaze_starlark::error::StackEntry;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use super::java_hash::JavaHashMap;
use super::{CompiledModuleFile, EvalException, entry_location};

/// Java `RepositoryName.validateUserProvidedRepoName`, as a Starlark error.
pub(crate) fn validate_user_provided_repo_name(name: &str) -> Result<(), EvalException> {
    RepositoryName::validate_user_provided_repo_name(name)
        .map_err(|e| EvalException::new(e.get_message()))
}

/// The caller location recorded in a call stack: the entry below the innermost (builtin) frame,
/// or `Location.BUILTIN` for stacks with fewer than two entries.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:96 RepoOverride.location
fn stack_location(stack: &[StackEntry]) -> Location {
    if stack.len() < 2 {
        return Location::builtin().clone();
    }
    // Skip over the override_repo builtin frame.
    entry_location(stack[stack.len() - 2].location.as_ref())
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:90 ModuleThreadContext.RepoOverride
#[derive(Debug, Clone)]
struct RepoOverride {
    overridden_repo_name: String,
    overriding_repo_name: String,
    must_exist: bool,
    extension_name: String,
    stack: Vec<StackEntry>,
}

impl RepoOverride {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:96 RepoOverride.location
    fn location(&self) -> Location {
        stack_location(&self.stack)
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:105 ModuleThreadContext.RepoNameUsage
#[derive(Debug, Clone)]
struct RepoNameUsage {
    how: String,
    stack: Vec<StackEntry>,
}

impl RepoNameUsage {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:106 RepoNameUsage.location
    fn location(&self) -> Location {
        stack_location(&self.stack)
    }
}

/// Port of `ModuleExtensionUsage.Proxy.Builder`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionUsage.java:85 ModuleExtensionUsage.Proxy.Builder
#[derive(Debug, Clone)]
pub(crate) struct ProxyBuilder {
    location: Location,
    proxy_name: String,
    containing_module_file_path: PathFragment,
    dev_dependency: bool,
    imports: IndexMap<String, String>,
}

impl ProxyBuilder {
    /// `Proxy.builder().setDevDependency(..).setLocation(..).setContainingModuleFilePath(..)`
    /// (the proxy name starts empty).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionUsage.java:80 ModuleExtensionUsage.Proxy.builder
    pub(crate) fn new(
        dev_dependency: bool,
        location: Location,
        containing_module_file_path: PathFragment,
    ) -> ProxyBuilder {
        ProxyBuilder {
            location,
            proxy_name: String::new(),
            containing_module_file_path,
            dev_dependency,
            imports: IndexMap::new(),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionUsage.java:106 Proxy.Builder.addImport
    fn add_import(&mut self, key: &str, value: &str) {
        // PORT: ImmutableBiMap.Builder rejects duplicate keys or values at build(); the usage
        // builder's checks (repo name usages, exported-name collisions) rule them out first.
        if self.imports.contains_key(key) || self.imports.values().any(|v| v == value) {
            panic!("IllegalArgumentException: duplicate import {key}={value}");
        }
        self.imports.insert(key.to_owned(), value.to_owned());
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionUsage.java:113 Proxy.Builder.build
    fn build(&self) -> ModuleExtensionUsageProxy {
        ModuleExtensionUsageProxy {
            location: self.location.clone(),
            proxy_name: self.proxy_name.clone(),
            containing_module_file_path: self.containing_module_file_path.clone(),
            dev_dependency: self.dev_dependency,
            imports: self.imports.clone(),
        }
    }
}

/// A proxy object: its usage builder and its proxy builder (Java `ModuleExtensionProxy`'s two
/// fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProxyRef {
    pub(crate) usage_builder: usize,
    pub(crate) proxy_builder: usize,
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:186 ModuleThreadContext.ModuleExtensionUsageBuilder
#[derive(Debug, Clone)]
struct ModuleExtensionUsageBuilder {
    extension_bzl_file: String,
    extension_name: String,
    isolate: bool,
    proxy_builders: Vec<ProxyBuilder>,
    /// A `HashBiMap` from local repo name to exported name; only looked up, never iterated.
    imports: IndexMap<String, String>,
    repo_overrides: JavaHashMap<RepoOverride>,
    tags: Vec<Tag>,
}

impl ModuleExtensionUsageBuilder {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:197 ModuleExtensionUsageBuilder.<init>
    fn new(extension_bzl_file: &str, extension_name: &str, isolate: bool) -> Self {
        ModuleExtensionUsageBuilder {
            extension_bzl_file: extension_bzl_file.to_owned(),
            extension_name: extension_name.to_owned(),
            isolate,
            proxy_builders: Vec::new(),
            imports: IndexMap::new(),
            repo_overrides: JavaHashMap::new(),
            tags: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:220 ModuleExtensionUsageBuilder.isForExtension
    fn is_for_extension(&self, extension_bzl_file: &str, extension_name: &str) -> bool {
        self.extension_bzl_file == extension_bzl_file
            && self.extension_name == extension_name
            && !self.isolate
    }

    /// `imports.inverse().get(exportedName)`.
    fn imports_inverse_get(&self, exported_name: &str) -> Option<&str> {
        self.imports
            .iter()
            .find(|(_, v)| v.as_str() == exported_name)
            .map(|(k, _)| k.as_str())
    }
}

/// Where the `AttributeValues` of a kwargs dict go once the module file that passed the kwargs
/// has been executed.
///
/// PORT: Java's `AttributeValues` wraps the live kwargs `Dict` of the call, so a list or dict
/// passed to a tag (or to `archive_override` / `git_override`) and mutated afterwards is seen with
/// its final contents; the values only stop changing when the module file's `Mutability` is
/// closed at the end of `ModuleFileFunction.execModuleFile`. Blaze's model holds plain data, and
/// starlark-rust values cannot outlive their heap, so the tag or override is recorded at the call
/// with empty attribute values and filled in when the file that made the call has been executed
/// (`module_file_globals::resolve_deferred_attribute_values`). Bindings are not visible across
/// `include()`, so no value can change after the end of the file that created it.
// Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValues.java:34 AttributeValues.create
#[derive(Debug, Clone)]
enum DeferredAttributeValues {
    /// `Tag.getAttributeValues()` of tag number `tag` of a usage builder.
    Tag { usage_builder: usize, tag: usize },
    /// `RepoSpec.attributes()` of the `NonRegistryOverride` of a module.
    Override { module_name: String },
    /// Already filled in.
    Resolved,
}

/// The `InterimModule.Builder` of a module file plus the getters Java's AutoValue builder has
/// (`getKey`, `getName`, `getVersion`, `getRepoName`).
#[derive(Debug, Clone)]
struct ModuleBuilder {
    builder: InterimModuleBuilder,
    key: ModuleKey,
    name: String,
    version: Version,
    repo_name: Option<String>,
}

/// Context of a Starlark thread evaluating MODULE.bazel and the files it includes. `Send`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:48 ModuleThreadContext
pub struct ModuleThreadContext {
    module_called: bool,
    had_non_module_call: bool,
    current_module_file_path: PathFragment,

    ignore_dev_deps: bool,
    module: ModuleBuilder,
    builtin_modules: IndexMap<String, NonRegistryOverride>,
    include_label_to_compiled_module_file: Option<Arc<IndexMap<String, CompiledModuleFile>>>,
    deps: IndexMap<String, DepSpec>,
    /// Indices into `usage_builder_arena` of Java's `extensionUsageBuilders` list.
    extension_usage_builders: Vec<usize>,
    usage_builder_arena: Vec<ModuleExtensionUsageBuilder>,
    overrides: IndexMap<String, ModuleOverride>,
    repo_name_usages: JavaHashMap<RepoNameUsage>,
    warnings: Vec<Event>,

    overridden_repos: JavaHashMap<RepoOverride>,
    overriding_repos: JavaHashMap<RepoOverride>,

    /// The labels of the module files that are currently being executed via `include`.
    include_stack: IndexSet<String>,

    /// See [`DeferredAttributeValues`]; the index is the id kept on the Starlark heap.
    deferred_attribute_values: Vec<DeferredAttributeValues>,

    /// The Java call-stack entries of the files that are including the one being executed
    /// (outermost first, ending with the `include` builtin frame); see the module docs.
    pub(crate) call_stack_prefix: Vec<StackEntry>,
}

/// What `include()` needs to run an included file and restore the context afterwards.
pub(crate) struct IncludeFrame {
    pub(crate) compiled_module_file: CompiledModuleFile,
    include_label: String,
    includer: PathFragment,
}

impl ModuleThreadContext {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:78 ModuleThreadContext.<init>
    pub fn new(
        builtin_modules: IndexMap<String, NonRegistryOverride>,
        key: ModuleKey,
        ignore_dev_deps: bool,
        include_label_to_compiled_module_file: Option<Arc<IndexMap<String, CompiledModuleFile>>>,
    ) -> Self {
        let mut builder = InterimModule::builder();
        builder.set_key(key.clone());
        ModuleThreadContext {
            module_called: false,
            had_non_module_call: false,
            current_module_file_path: MODULE_DOT_BAZEL_FILE_NAME.clone(),
            ignore_dev_deps,
            module: ModuleBuilder {
                builder,
                key,
                name: String::new(),
                version: Version::empty(),
                repo_name: None,
            },
            builtin_modules,
            include_label_to_compiled_module_file,
            deps: IndexMap::new(),
            extension_usage_builders: Vec::new(),
            usage_builder_arena: Vec::new(),
            overrides: IndexMap::new(),
            repo_name_usages: JavaHashMap::new(),
            warnings: Vec::new(),
            overridden_repos: JavaHashMap::new(),
            overriding_repos: JavaHashMap::new(),
            include_stack: IndexSet::new(),
            deferred_attribute_values: Vec::new(),
            call_stack_prefix: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:115 ModuleThreadContext.addRepoNameUsage
    pub(crate) fn add_repo_name_usage(
        &mut self,
        repo_name: &str,
        how: &str,
        stack: Vec<StackEntry>,
    ) -> Result<(), EvalException> {
        let incoming = RepoNameUsage {
            how: how.to_owned(),
            stack,
        };
        let collision = self.repo_name_usages.put(repo_name, incoming.clone());
        if let Some(collision) = collision {
            return Err(EvalException::new(format!(
                "The repo name '{}' cannot be defined {} at {} as it is already defined {} at {}",
                repo_name,
                incoming.how,
                incoming.location(),
                collision.how,
                collision.location()
            )));
        }
        Ok(())
    }

    /// Whether the `module()` directive has been called.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:128 ModuleThreadContext.isModuleCalled
    pub fn is_module_called(&self) -> bool {
        self.module_called
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:132 ModuleThreadContext.setModuleCalled
    pub(crate) fn set_module_called(&mut self) {
        self.module_called = true;
    }

    /// Whether any directives other than `module()` have been called.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:137 ModuleThreadContext.hadNonModuleCall
    pub fn had_non_module_call(&self) -> bool {
        self.had_non_module_call
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:141 ModuleThreadContext.setNonModuleCalled
    pub(crate) fn set_non_module_called(&mut self) {
        self.had_non_module_call = true;
    }

    /// `getModuleBuilder().getKey()`.
    pub(crate) fn module_key(&self) -> &ModuleKey {
        &self.module.key
    }

    /// `getModuleBuilder().getName()`.
    pub(crate) fn module_name(&self) -> &str {
        &self.module.name
    }

    /// `getModuleBuilder().getVersion()`.
    pub(crate) fn module_version(&self) -> &Version {
        &self.module.version
    }

    /// `getModuleBuilder().getRepoName()` (empty until `setRepoName` is called).
    pub(crate) fn module_repo_name(&self) -> Option<&str> {
        self.module.repo_name.as_deref()
    }

    /// The `module()` directive's builder calls, in Java's order (`setName`, `setVersion`,
    /// `setCompatibilityLevel`, `addBazelCompatibilityValues`, `setRepoName`).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:188 ModuleFileGlobals.module
    pub(crate) fn set_module_properties(
        &mut self,
        name: &str,
        version: Version,
        compatibility_level: i32,
        bazel_compatibility: Vec<String>,
        repo_name: &str,
    ) {
        self.module
            .builder
            .set_name(name)
            .set_version(version.clone())
            .set_compatibility_level(compatibility_level)
            .add_bazel_compatibility_values(bazel_compatibility)
            .set_repo_name(repo_name);
        self.module.name = name.to_owned();
        self.module.version = version;
        self.module.repo_name = Some(repo_name.to_owned());
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java Builder.addExecutionPlatformsToRegister
    pub(crate) fn add_execution_platforms_to_register(&mut self, values: Vec<String>) {
        self.module
            .builder
            .add_execution_platforms_to_register(values);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java Builder.addToolchainsToRegister
    pub(crate) fn add_toolchains_to_register(&mut self, values: Vec<String>) {
        self.module.builder.add_toolchains_to_register(values);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:167 Builder.addFlagAlias
    pub(crate) fn add_flag_alias(&mut self, native_name: &str, starlark_label: &str) {
        self.module
            .builder
            .add_flag_alias(native_name, starlark_label);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:149 ModuleThreadContext.addWarning
    pub(crate) fn add_warning(&mut self, event: Event) {
        self.warnings.push(event);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:153 ModuleThreadContext.getWarnings
    pub fn get_warnings(&self) -> Vec<Event> {
        self.warnings.clone()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:157 ModuleThreadContext.shouldIgnoreDevDeps
    pub fn should_ignore_dev_deps(&self) -> bool {
        self.ignore_dev_deps
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:161 ModuleThreadContext.addDep
    pub(crate) fn add_dep(&mut self, repo_name: Option<&str>, dep_spec: DepSpec) {
        match repo_name {
            Some(repo_name) => {
                self.deps.insert(repo_name.to_owned(), dep_spec);
            }
            None => {
                self.module.builder.add_nodep_dep(dep_spec);
            }
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:169 ModuleThreadContext.getOrCreateExtensionUsageBuilder
    pub(crate) fn get_or_create_extension_usage_builder(
        &mut self,
        extension_bzl_file: &str,
        extension_name: &str,
        isolate: bool,
    ) -> usize {
        // Isolated extensions have to always get a new builder, non-isolated ones have to reuse an
        // existing one if it exists so that they all contribute usages to the same row in a table.
        if !isolate {
            for &builder in &self.extension_usage_builders {
                if self.usage_builder_arena[builder]
                    .is_for_extension(extension_bzl_file, extension_name)
                {
                    return builder;
                }
            }
        }
        let new_builder =
            self.new_detached_usage_builder(extension_bzl_file, extension_name, isolate);
        self.extension_usage_builders.push(new_builder);
        new_builder
    }

    /// `new ModuleExtensionUsageBuilder(context, ...)` that is not added to
    /// `extensionUsageBuilders` (the no-op proxies of ignored dev dependencies).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:197 ModuleExtensionUsageBuilder.<init>
    pub(crate) fn new_detached_usage_builder(
        &mut self,
        extension_bzl_file: &str,
        extension_name: &str,
        isolate: bool,
    ) -> usize {
        self.usage_builder_arena
            .push(ModuleExtensionUsageBuilder::new(
                extension_bzl_file,
                extension_name,
                isolate,
            ));
        self.usage_builder_arena.len() - 1
    }

    /// `new ModuleExtensionProxy(usageBuilder, proxyBuilder)`: registers the proxy builder with
    /// the usage builder.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:553 ModuleExtensionProxy.<init>
    pub(crate) fn new_module_extension_proxy(
        &mut self,
        usage_builder: usize,
        proxy_builder: ProxyBuilder,
    ) -> ProxyRef {
        // Java: ModuleThreadContext.java:216 ModuleExtensionUsageBuilder.addProxyBuilder
        let builders = &mut self.usage_builder_arena[usage_builder].proxy_builders;
        builders.push(proxy_builder);
        ProxyRef {
            usage_builder,
            proxy_builder: builders.len() - 1,
        }
    }

    /// `proxyBuilder.isDevDependency()`.
    pub(crate) fn proxy_is_dev_dependency(&self, proxy: ProxyRef) -> bool {
        self.usage_builder_arena[proxy.usage_builder].proxy_builders[proxy.proxy_builder]
            .dev_dependency
    }

    /// `ModuleExtensionProxy.isExported()`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:620 ModuleExtensionProxy.isExported
    pub(crate) fn proxy_is_exported(&self, proxy: ProxyRef) -> bool {
        !self.usage_builder_arena[proxy.usage_builder].proxy_builders[proxy.proxy_builder]
            .proxy_name
            .is_empty()
    }

    /// `ModuleExtensionProxy.export`: sets the proxy name.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:625 ModuleExtensionProxy.export
    pub(crate) fn proxy_export(&mut self, proxy: ProxyRef, name: &str) {
        self.usage_builder_arena[proxy.usage_builder].proxy_builders[proxy.proxy_builder]
            .proxy_name = name.to_owned();
    }

    /// `TagCallable.call`: adds a tag to the proxy's usage builder.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:592 TagCallable.call
    pub(crate) fn tag_call(
        &mut self,
        proxy: ProxyRef,
        tag_name: &str,
        attribute_values: AttributeValues,
        location: Location,
    ) {
        let dev_dependency = self.proxy_is_dev_dependency(proxy);
        // Java: ModuleThreadContext.java:265 ModuleExtensionUsageBuilder.addTag
        self.usage_builder_arena[proxy.usage_builder]
            .tags
            .push(Tag {
                tag_name: tag_name.to_owned(),
                attribute_values,
                dev_dependency,
                location,
            });
    }

    /// `TagCallable.call` with a kwargs dict whose values are still live Starlark values: adds
    /// the tag now, with empty attribute values, and returns the id under which
    /// [`ModuleThreadContext::set_deferred_attribute_values`] fills them in. See
    /// [`DeferredAttributeValues`].
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:592 TagCallable.call
    pub(crate) fn tag_call_deferred(
        &mut self,
        proxy: ProxyRef,
        tag_name: &str,
        location: Location,
    ) -> usize {
        self.tag_call(
            proxy,
            tag_name,
            AttributeValues::create(Vec::<(String, AttributeValue)>::new()),
            location,
        );
        let tag = self.usage_builder_arena[proxy.usage_builder].tags.len() - 1;
        self.deferred_attribute_values
            .push(DeferredAttributeValues::Tag {
                usage_builder: proxy.usage_builder,
                tag,
            });
        self.deferred_attribute_values.len() - 1
    }

    /// `addOverride(moduleName, new NonRegistryOverride(new RepoSpec(repoRuleId,
    /// AttributeValues.create(kwargs))))` with a kwargs dict whose values are still live Starlark
    /// values: adds the override now, with empty attributes, and returns the id under which
    /// [`ModuleThreadContext::set_deferred_attribute_values`] fills them in; `None` when the
    /// override is ignored. See [`DeferredAttributeValues`].
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:372 ModuleThreadContext.addOverride
    pub(crate) fn add_override_deferred(
        &mut self,
        module_name: &str,
        repo_rule_id: RepoRuleId,
    ) -> Result<Option<usize>, EvalException> {
        if self.should_ignore_dev_deps() {
            return Ok(None);
        }
        self.add_override(
            module_name,
            ModuleOverride::NonRegistry(NonRegistryOverride::new(RepoSpec::new(
                repo_rule_id,
                AttributeValues::create(Vec::<(String, AttributeValue)>::new()),
            ))),
        )?;
        self.deferred_attribute_values
            .push(DeferredAttributeValues::Override {
                module_name: module_name.to_owned(),
            });
        Ok(Some(self.deferred_attribute_values.len() - 1))
    }

    /// Fills in the attribute values of the tag or override recorded under `deferred`.
    pub(crate) fn set_deferred_attribute_values(
        &mut self,
        deferred: usize,
        attribute_values: AttributeValues,
    ) {
        let target = std::mem::replace(
            &mut self.deferred_attribute_values[deferred],
            DeferredAttributeValues::Resolved,
        );
        match target {
            DeferredAttributeValues::Tag { usage_builder, tag } => {
                self.usage_builder_arena[usage_builder].tags[tag].attribute_values =
                    attribute_values;
            }
            DeferredAttributeValues::Override { module_name } => {
                let repo_rule_id = match self.overrides.get(&module_name) {
                    Some(ModuleOverride::NonRegistry(o)) => o.repo_spec().repo_rule_id().cloned(),
                    _ => None,
                };
                let Some(repo_rule_id) = repo_rule_id else {
                    panic!("no non-registry override with deferred attributes for {module_name}");
                };
                // Replaces the value; the override keeps its position.
                self.overrides.insert(
                    module_name,
                    ModuleOverride::NonRegistry(NonRegistryOverride::new(RepoSpec::new(
                        repo_rule_id,
                        attribute_values,
                    ))),
                );
            }
            DeferredAttributeValues::Resolved => {
                panic!("deferred attribute values {deferred} were already filled in");
            }
        }
    }

    /// Every tag and override must have its attribute values before the module is built.
    fn check_no_deferred_attribute_values(&self) {
        if self
            .deferred_attribute_values
            .iter()
            .any(|d| !matches!(d, DeferredAttributeValues::Resolved))
        {
            panic!(
                "attribute values of a tag or override were not taken from the Starlark heap \
                 (resolve_deferred_attribute_values)"
            );
        }
    }

    /// `ModuleExtensionProxy.addImport`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:560 ModuleExtensionProxy.addImport
    pub(crate) fn proxy_add_import(
        &mut self,
        proxy: ProxyRef,
        local_repo_name: &str,
        exported_name: &str,
        by_what: &str,
        stack: Vec<StackEntry>,
    ) -> Result<(), EvalException> {
        self.usage_add_import(
            proxy.usage_builder,
            local_repo_name,
            exported_name,
            by_what,
            stack,
        )?;
        self.usage_builder_arena[proxy.usage_builder].proxy_builders[proxy.proxy_builder]
            .add_import(local_repo_name, exported_name);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:226 ModuleExtensionUsageBuilder.addImport
    fn usage_add_import(
        &mut self,
        usage_builder: usize,
        local_repo_name: &str,
        exported_name: &str,
        by_what: &str,
        stack: Vec<StackEntry>,
    ) -> Result<(), EvalException> {
        validate_user_provided_repo_name(local_repo_name)?;
        validate_user_provided_repo_name(exported_name)?;
        self.add_repo_name_usage(local_repo_name, by_what, stack)?;
        let builder = &self.usage_builder_arena[usage_builder];
        if let Some(collision_repo_name) = builder.imports_inverse_get(exported_name) {
            let usage = self
                .repo_name_usages
                .get(collision_repo_name)
                .expect("NullPointerException: repoNameUsages.get(collisionRepoName)");
            return Err(EvalException::new(format!(
                "The repo exported as '{}' by module extension '{}' is already imported at {}",
                exported_name,
                builder.extension_name,
                usage.location()
            )));
        }
        // PORT: HashBiMap.put throws if the value is already bound; checked just above.
        self.usage_builder_arena[usage_builder]
            .imports
            .insert(local_repo_name.to_owned(), exported_name.to_owned());
        Ok(())
    }

    /// `ModuleExtensionProxy.addOverride`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:570 ModuleExtensionProxy.addOverride
    pub(crate) fn proxy_add_override(
        &mut self,
        proxy: ProxyRef,
        overridden_repo_name: &str,
        overriding_repo_name: &str,
        must_exist: bool,
        stack: Vec<StackEntry>,
    ) -> Result<(), EvalException> {
        self.usage_add_repo_override(
            proxy.usage_builder,
            overridden_repo_name,
            overriding_repo_name,
            must_exist,
            stack,
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:244 ModuleExtensionUsageBuilder.addRepoOverride
    fn usage_add_repo_override(
        &mut self,
        usage_builder: usize,
        overridden_repo_name: &str,
        overriding_repo_name: &str,
        must_exist: bool,
        stack: Vec<StackEntry>,
    ) -> Result<(), EvalException> {
        validate_user_provided_repo_name(overridden_repo_name)?;
        validate_user_provided_repo_name(overriding_repo_name)?;
        let builder = &mut self.usage_builder_arena[usage_builder];
        let extension_name = builder.extension_name.clone();
        let collision = builder.repo_overrides.put(
            overridden_repo_name,
            RepoOverride {
                overridden_repo_name: overridden_repo_name.to_owned(),
                overriding_repo_name: overriding_repo_name.to_owned(),
                must_exist,
                extension_name: extension_name.clone(),
                stack,
            },
        );
        if let Some(collision) = collision {
            return Err(EvalException::new(format!(
                "The repo exported as '{}' by module extension '{}' is already overridden with '{}' at {}",
                overridden_repo_name,
                extension_name,
                collision.overriding_repo_name,
                collision.location()
            )));
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:269 ModuleExtensionUsageBuilder.buildUsage
    fn build_usage(&mut self, usage_builder: usize) -> Result<ModuleExtensionUsage, EvalException> {
        let builder = self.usage_builder_arena[usage_builder].clone();
        let proxies: Vec<ModuleExtensionUsageProxy> =
            builder.proxy_builders.iter().map(|p| p.build()).collect();
        let isolation_key = if builder.isolate {
            // Iterables.getOnlyElement
            if proxies.len() != 1 {
                panic!(
                    "IllegalArgumentException: expected one element but was: {}",
                    proxies.len()
                );
            }
            let only_proxy = &proxies[0];
            if only_proxy.proxy_name.is_empty() {
                return Err(EvalException::new(format!(
                    "Isolated extension usage at {} must be assigned to a top-level variable",
                    only_proxy.location
                )));
            }
            Some(IsolationKey::create(
                self.module.key.clone(),
                only_proxy.proxy_name.clone(),
            ))
        } else {
            None
        };

        let repo_overrides = builder.repo_overrides.entry_set();
        for (overridden_repo_name, override_) in &repo_overrides {
            let overriding_repo_name = &override_.overriding_repo_name;
            if !self.repo_name_usages.contains_key(overriding_repo_name) {
                return Err(EvalException::with_call_stack(
                    format!(
                        "The repo exported as '{}' by module extension '{}' is overridden with '{}', but no repo is visible under this name{}",
                        overridden_repo_name,
                        builder.extension_name,
                        overriding_repo_name,
                        SpellChecker::did_you_mean(
                            overriding_repo_name,
                            self.repo_name_usages.key_set()
                        )
                    ),
                    override_.stack.clone(),
                ));
            }
            if let Some(imported_as) = builder.imports_inverse_get(overridden_repo_name) {
                if !override_.must_exist {
                    let usage_stack = self
                        .repo_name_usages
                        .get(imported_as)
                        .expect("NullPointerException: repoNameUsages.get(importedAs)")
                        .stack
                        .clone();
                    return Err(EvalException::with_call_stack(
                        format!(
                            "Cannot import repo '{}' that has been injected into module extension '{}' at {}. Please refer to @{} directly.",
                            overridden_repo_name,
                            builder.extension_name,
                            override_.location(),
                            overriding_repo_name
                        ),
                        usage_stack,
                    ));
                }
                self.overridden_repos.put(imported_as, (*override_).clone());
            }
            self.overriding_repos
                .put(overriding_repo_name.as_str(), (*override_).clone());
        }
        let repo_overrides: IndexMap<String, UsageRepoOverride> = repo_overrides
            .iter()
            .map(|(k, v)| {
                (
                    (*k).to_owned(),
                    UsageRepoOverride {
                        overriding_repo_name: v.overriding_repo_name.clone(),
                        must_exist: v.must_exist,
                        location: v.location(),
                    },
                )
            })
            .collect();

        Ok(ModuleExtensionUsage {
            extension_bzl_file: builder.extension_bzl_file.clone(),
            extension_name: builder.extension_name.clone(),
            isolation_key,
            proxies,
            tags: builder.tags.clone(),
            repo_overrides,
        })
    }

    /// The first half of `include()`: dev-dependency filtering, lookups, cycle detection and
    /// `currentModuleFilePath`. `None` means the include is ignored. The caller runs
    /// [`IncludeFrame::compiled_module_file`] and then calls [`ModuleThreadContext::include_end`]
    /// (Java's `finally`), without holding the context borrow in between.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:333 ModuleThreadContext.include
    pub(crate) fn include_begin(
        &mut self,
        include_label: &str,
        dev_dependency: bool,
    ) -> Result<Option<IncludeFrame>, EvalException> {
        if self.should_ignore_dev_deps() && dev_dependency {
            return Ok(None);
        }
        let Some(include_label_to_compiled_module_file) =
            &self.include_label_to_compiled_module_file
        else {
            // This should never happen because compiling the non-root module file should have failed, way
            // before evaluation started.
            return Err(EvalException::new(
                "trying to call `include()` from a non-root module",
            ));
        };
        let Some(compiled_module_file) = include_label_to_compiled_module_file.get(include_label)
        else {
            // This should never happen because the file we're trying to include should have already been
            // compiled before evaluation started.
            return Err(EvalException::new(format!(
                "internal error; included file {include_label} not compiled"
            )));
        };
        let compiled_module_file = compiled_module_file.clone();
        if !self.include_stack.insert(include_label.to_owned()) {
            // include() behaves as if the included file were textually inserted at the location of the
            // call, so a cycle would result in unbounded recursion.
            let chain: Vec<&str> = self.include_stack.iter().map(|s| s.as_str()).collect();
            let start = chain
                .iter()
                .position(|l| *l == include_label)
                .expect("label in include stack");
            return Err(EvalException::new(format!(
                "include() cycle detected: {} -> {}",
                chain[start..].join(" -> "),
                include_label
            )));
        }
        let includer = self.current_module_file_path.clone();
        self.current_module_file_path =
            Label::parse_canonical_unchecked(include_label).to_path_fragment();
        Ok(Some(IncludeFrame {
            compiled_module_file,
            include_label: include_label.to_owned(),
            includer,
        }))
    }

    /// The `finally` block of `include()`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:362 ModuleThreadContext.include
    pub(crate) fn include_end(&mut self, frame: &IncludeFrame) {
        self.current_module_file_path = frame.includer.clone();
        self.include_stack.shift_remove(&frame.include_label);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:368 ModuleThreadContext.getCurrentModuleFilePath
    pub fn get_current_module_file_path(&self) -> &PathFragment {
        &self.current_module_file_path
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:372 ModuleThreadContext.addOverride
    pub(crate) fn add_override(
        &mut self,
        module_name: &str,
        override_: ModuleOverride,
    ) -> Result<(), EvalException> {
        if self.should_ignore_dev_deps() {
            return Ok(());
        }
        // Java: overrides.putIfAbsent(moduleName, override)
        match self.overrides.entry(module_name.to_owned()) {
            indexmap::map::Entry::Occupied(_) => Err(EvalException::new(format!(
                "multiple overrides for dep {module_name} found"
            ))),
            indexmap::map::Entry::Vacant(entry) => {
                entry.insert(override_);
                Ok(())
            }
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:382 ModuleThreadContext.buildModule
    pub fn build_module(
        &mut self,
        registry: Option<Arc<dyn Registry>>,
    ) -> Result<InterimModule, EvalException> {
        self.check_no_deferred_attribute_values();
        // Add builtin modules as default deps of the current module.
        let builtin_module_names: Vec<String> = self.builtin_modules.keys().cloned().collect();
        for builtin_module in builtin_module_names {
            if self.module.key.name() == builtin_module {
                // The built-in module does not depend on itself.
                continue;
            }
            self.deps.insert(
                builtin_module.clone(),
                DepSpec::new(builtin_module.clone(), Version::empty(), -1),
            );
            if let Err(e) =
                self.add_repo_name_usage(&builtin_module, "as a built-in dependency", Vec::new())
            {
                return Err(EvalException::new(format!(
                    "{}, '{}' is a built-in dependency and cannot be used by any 'bazel_dep' or 'use_repo' directive",
                    e.message, builtin_module
                )));
            }
        }
        // Build module extension usages and the rest of the module.
        let mut extension_usages = Vec::new();
        for extension_usage_builder in self.extension_usage_builders.clone() {
            if self.usage_builder_arena[extension_usage_builder]
                .proxy_builders
                .is_empty()
            {
                // This can happen for the special extension used for "use_repo_rule" calls.
                continue;
            }
            extension_usages.push(self.build_usage(extension_usage_builder)?);
        }
        // A repo cannot be both overriding and overridden. This ensures that repo overrides can be
        // applied to repo mappings in a single step (and also prevents cycles).
        let overriding_and_overridden = self
            .overriding_repos
            .key_set()
            .into_iter()
            .find(|k| self.overridden_repos.contains_key(k))
            .map(|k| k.to_owned());
        if let Some(overriding_and_overridden) = overriding_and_overridden {
            let override_ = self
                .overriding_repos
                .get(&overriding_and_overridden)
                .expect("key from key_set");
            let override_on_override = self
                .overridden_repos
                .get(&overriding_and_overridden)
                .expect("contains_key checked");
            return Err(EvalException::with_call_stack(
                format!(
                    "The repo '{}' used as an override for '{}' in module extension '{}' is itself overridden with '{}' at {}, which is not supported.",
                    override_.overriding_repo_name,
                    override_.overridden_repo_name,
                    override_.extension_name,
                    override_on_override.overriding_repo_name,
                    override_on_override.location()
                ),
                override_.stack.clone(),
            ));
        }

        Ok(self
            .module
            .builder
            .set_registry(registry)
            .set_deps(self.deps.clone())
            .set_original_deps(self.deps.clone())
            .set_extension_usages(extension_usages)
            .build())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:437 ModuleThreadContext.buildOverrides
    pub fn build_overrides(&mut self) -> IndexMap<String, ModuleOverride> {
        self.check_no_deferred_attribute_values();
        // Add overrides for builtin modules if there is no existing override for them.
        if ModuleKey::root() == self.module.key {
            for (module_name, builtin) in &self.builtin_modules {
                self.overrides
                    .entry(module_name.clone())
                    .or_insert_with(|| ModuleOverride::NonRegistry(builtin.clone()));
            }
        }
        self.overrides.clone()
    }
}
