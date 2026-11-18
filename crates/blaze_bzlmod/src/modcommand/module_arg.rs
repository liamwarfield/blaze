//! The resolution half of `bazel/bzlmod/modcommand/ModuleArg.java` (`resolveToModuleKeys`,
//! `resolveToRepoNames`). Parsing (`ModuleArgConverter`) is transcribed in
//! `blaze_optdefs::misc::module_arg`, which has its own light `ModuleKey`/`Version`; this module
//! converts those into `blaze_bzlmod_model` types.

use std::fmt;

use blaze_bzlmod_model::{ModuleKey, Version};
use blaze_cmdline::{RepositoryMapping, RepositoryName};
use blaze_optdefs::misc::module_arg as opt;
use blaze_options::OptionsParsingException;
use indexmap::{IndexMap, IndexSet};

use super::invalid_argument_exception::{InvalidArgumentException, ModCommandCode};
use crate::inspector::AugmentedModule;

/// Represents a reference to one or more modules in the external dependency graph, used for
/// modquery. This is parsed from a command-line argument (either as the value of a flag, or just
/// as a bare argument), and can take one of various forms.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:44 ModuleArg
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleArg {
    /// Refers to a specific version of a module. Parsed from `<module>@<version>`. `<version>`
    /// can be the special string `_` to signify the empty version (for non-registry overrides).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:70 ModuleArg.SpecificVersionOfModule
    SpecificVersionOfModule(ModuleKey),
    /// Refers to all versions of a module. Parsed from `<module>`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:151 ModuleArg.AllVersionsOfModule
    AllVersionsOfModule(String),
    /// Refers to a module with the given apparent repo name, in the context of `--base_module`
    /// (or when parsing that flag itself, in the context of the root module). Parsed from
    /// `@<name>`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:229 ModuleArg.ApparentRepoName
    ApparentRepoName(String),
    /// Refers to a module with the given canonical repo name. Parsed from `@@<name>`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:291 ModuleArg.CanonicalRepoName
    CanonicalRepoName(RepositoryName),
}

/// Guava `ImmutableCollection.toString()` of module keys: `[a@1, b@2]`.
pub(crate) fn module_key_set_to_string<'a>(
    keys: impl IntoIterator<Item = &'a ModuleKey>,
) -> String {
    let parts: Vec<String> = keys.into_iter().map(|k| k.to_string()).collect();
    format!("[{}]", parts.join(", "))
}

/// Converts the option-parsing `ModuleKey` of `blaze_optdefs` into the model's `ModuleKey`.
pub fn module_key_from_option(key: &opt::ModuleKey) -> ModuleKey {
    // The option's version was validated by `Version.parse`; its normalized form parses to the
    // same version.
    let version = if key.version.is_empty() {
        Version::empty()
    } else {
        match Version::parse(key.version.get_normalized()) {
            Ok(v) => v,
            Err(e) => panic!("IllegalStateException: {}", e.get_message()),
        }
    };
    ModuleKey::new(key.name.clone(), version)
}

impl ModuleArg {
    /// Converts a parsed option value (`blaze_optdefs`) into this type.
    pub fn from_option(arg: &opt::ModuleArg) -> ModuleArg {
        match arg {
            opt::ModuleArg::SpecificVersionOfModule(k) => {
                ModuleArg::SpecificVersionOfModule(module_key_from_option(k))
            }
            opt::ModuleArg::AllVersionsOfModule(n) => ModuleArg::AllVersionsOfModule(n.clone()),
            opt::ModuleArg::ApparentRepoName(n) => ModuleArg::ApparentRepoName(n.clone()),
            opt::ModuleArg::CanonicalRepoName(r) => ModuleArg::CanonicalRepoName(r.clone()),
        }
    }

    /// `ModuleArgConverter.INSTANCE.convert(input)`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:356 ModuleArg.ModuleArgConverter.convert
    pub fn convert(input: &str) -> Result<ModuleArg, OptionsParsingException> {
        Ok(ModuleArg::from_option(
            &opt::ModuleArgConverter.convert_arg(input)?,
        ))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:79 ModuleArg.SpecificVersionOfModule.throwIfNonexistent
    fn throw_if_nonexistent(
        module_key: &ModuleKey,
        modules_index: &IndexMap<String, IndexSet<ModuleKey>>,
        dep_graph: &IndexMap<ModuleKey, AugmentedModule>,
        include_unused: bool,
        warn_unused: bool,
    ) -> Result<(), InvalidArgumentException> {
        let module = dep_graph.get(module_key);
        if let Some(m) = module
            && !include_unused
            && warn_unused
            && !m.is_used()
        {
            // Warn the user when unused modules are allowed and the specified version exists, but
            // the --include_unused flag was not set.
            return Err(InvalidArgumentException::with_code(
                format!(
                    "Module version {module_key} is unused as a result of module resolution. Use \
                     the --include_unused flag to include it."
                ),
                ModCommandCode::INVALID_ARGUMENTS,
            ));
        }
        if module.is_none_or(|m| !include_unused && !m.is_used()) {
            let Some(existing_keys) = modules_index.get(module_key.name()) else {
                return Err(InvalidArgumentException::with_code(
                    format!(
                        "Module {} does not exist in the dependency graph.",
                        module_key.name()
                    ),
                    ModCommandCode::INVALID_ARGUMENTS,
                ));
            };
            // If --include_unused is not true, unused modules will be considered non-existent
            // and an error will be thrown.
            let filtered_keys: Vec<&ModuleKey> = existing_keys
                .iter()
                .filter(|k| include_unused || dep_graph[*k].is_used())
                .collect();
            return Err(InvalidArgumentException::with_code(
                format!(
                    "Module version {module_key} does not exist, available versions: {}.",
                    module_key_set_to_string(filtered_keys)
                ),
                ModCommandCode::INVALID_ARGUMENTS,
            ));
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:160 ModuleArg.AllVersionsOfModule.resolveInternal
    fn resolve_all_versions_internal(
        module_name: &str,
        modules_index: &IndexMap<String, IndexSet<ModuleKey>>,
        dep_graph: &IndexMap<ModuleKey, AugmentedModule>,
        include_unused: bool,
        warn_unused: bool,
    ) -> Result<IndexSet<ModuleKey>, InvalidArgumentException> {
        let Some(existing_keys) = modules_index.get(module_name) else {
            return Err(InvalidArgumentException::with_code(
                format!("Module {module_name} does not exist in the dependency graph."),
                ModCommandCode::INVALID_ARGUMENTS,
            ));
        };
        let filtered_keys: IndexSet<ModuleKey> = existing_keys
            .iter()
            .filter(|k| include_unused || dep_graph[*k].is_used())
            .cloned()
            .collect();
        if filtered_keys.is_empty() {
            if warn_unused {
                return Err(InvalidArgumentException::with_code(
                    format!(
                        "Module {module_name} is unused as a result of module resolution. Use the \
                         --include_unused flag to include it."
                    ),
                    ModCommandCode::INVALID_ARGUMENTS,
                ));
            }
            return Err(InvalidArgumentException::with_code(
                format!("Module {module_name} does not exist in the dependency graph."),
                ModCommandCode::INVALID_ARGUMENTS,
            ));
        }
        Ok(filtered_keys)
    }

    /// Resolves this module argument to a set of module keys.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:47 ModuleArg.resolveToModuleKeys
    pub fn resolve_to_module_keys(
        &self,
        modules_index: &IndexMap<String, IndexSet<ModuleKey>>,
        dep_graph: &IndexMap<ModuleKey, AugmentedModule>,
        module_key_to_canonical_names: &IndexMap<ModuleKey, RepositoryName>,
        base_module_deps: &IndexMap<String, ModuleKey>,
        base_module_unused_deps: &IndexMap<String, ModuleKey>,
        include_unused: bool,
        warn_unused: bool,
    ) -> Result<IndexSet<ModuleKey>, InvalidArgumentException> {
        match self {
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:119 ModuleArg.SpecificVersionOfModule.resolveToModuleKeys
            ModuleArg::SpecificVersionOfModule(module_key) => {
                Self::throw_if_nonexistent(
                    module_key,
                    modules_index,
                    dep_graph,
                    include_unused,
                    warn_unused,
                )?;
                Ok(IndexSet::from([module_key.clone()]))
            }
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:193 ModuleArg.AllVersionsOfModule.resolveToModuleKeys
            ModuleArg::AllVersionsOfModule(module_name) => Self::resolve_all_versions_internal(
                module_name,
                modules_index,
                dep_graph,
                include_unused,
                warn_unused,
            ),
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:239 ModuleArg.ApparentRepoName.resolveToModuleKeys
            ModuleArg::ApparentRepoName(name) => {
                let mut builder: IndexSet<ModuleKey> = IndexSet::new();
                if let Some(dep) = base_module_deps.get(name) {
                    builder.insert(dep.clone());
                }
                let unused_dep = base_module_unused_deps.get(name);
                if include_unused && let Some(unused_dep) = unused_dep {
                    builder.insert(unused_dep.clone());
                }
                if builder.is_empty() {
                    return Err(InvalidArgumentException::with_code(
                        format!(
                            "No module with the apparent repo name @{name} exists in the \
                             dependency graph"
                        ),
                        ModCommandCode::INVALID_ARGUMENTS,
                    ));
                }
                Ok(builder)
            }
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:301 ModuleArg.CanonicalRepoName.resolveToModuleKeys
            ModuleArg::CanonicalRepoName(repo_name) => {
                let module = dep_graph
                    .values()
                    .find(|m| module_key_to_canonical_names.get(&m.key) == Some(repo_name));
                if let Some(m) = module
                    && !include_unused
                    && warn_unused
                    && !m.is_used()
                {
                    // Warn the user when unused modules are allowed and the specified version
                    // exists, but the --include_unused flag was not set.
                    return Err(InvalidArgumentException::with_code(
                        format!(
                            "Module version {} is unused as a result of module resolution. Use \
                             the --include_unused flag to include it.",
                            m.key
                        ),
                        ModCommandCode::INVALID_ARGUMENTS,
                    ));
                }
                match module {
                    Some(m) if include_unused || m.is_used() => Ok(IndexSet::from([m.key.clone()])),
                    // If --include_unused is not true, unused modules will be considered
                    // non-existent and an error will be thrown.
                    _ => Err(InvalidArgumentException::with_code(
                        format!(
                            "No module with the canonical repo name @@{} exists in the \
                             dependency graph",
                            repo_name.get_name()
                        ),
                        ModCommandCode::INVALID_ARGUMENTS,
                    )),
                }
            }
        }
    }

    /// Resolves this module argument to a set of repo names.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:58 ModuleArg.resolveToRepoNames
    pub fn resolve_to_repo_names(
        &self,
        modules_index: &IndexMap<String, IndexSet<ModuleKey>>,
        dep_graph: &IndexMap<ModuleKey, AugmentedModule>,
        module_key_to_canonical_names: &IndexMap<ModuleKey, RepositoryName>,
        mapping: &RepositoryMapping,
    ) -> Result<IndexMap<String, RepositoryName>, InvalidArgumentException> {
        let canonical_name_of = |k: &ModuleKey| -> RepositoryName {
            match module_key_to_canonical_names.get(k) {
                Some(r) => r.clone(),
                None => panic!("java.lang.NullPointerException"),
            }
        };
        match self {
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:133 ModuleArg.SpecificVersionOfModule.resolveToRepoNames
            ModuleArg::SpecificVersionOfModule(module_key) => {
                Self::throw_if_nonexistent(
                    module_key,
                    modules_index,
                    dep_graph,
                    /* include_unused= */ false,
                    /* warn_unused= */ false,
                )?;
                Ok(IndexMap::from([(
                    module_key.to_string(),
                    canonical_name_of(module_key),
                )]))
            }
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:206 ModuleArg.AllVersionsOfModule.resolveToRepoNames
            ModuleArg::AllVersionsOfModule(module_name) => {
                let keys = Self::resolve_all_versions_internal(
                    module_name,
                    modules_index,
                    dep_graph,
                    /* include_unused= */ false,
                    /* warn_unused= */ false,
                )?;
                let mut result: IndexMap<String, RepositoryName> = IndexMap::new();
                for k in &keys {
                    let s = k.to_string();
                    if result.contains_key(&s) {
                        panic!("IllegalArgumentException: Multiple entries with same key: {s}");
                    }
                    result.insert(s, canonical_name_of(k));
                }
                Ok(result)
            }
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:268 ModuleArg.ApparentRepoName.resolveToRepoNames
            ModuleArg::ApparentRepoName(name) => {
                let repo_name = mapping.get(name);
                if !repo_name.is_visible() {
                    return Err(InvalidArgumentException::with_code(
                        format!(
                            "No repo visible as {name} from @{}",
                            repo_name.get_context_repo_display_string()
                        ),
                        ModCommandCode::INVALID_ARGUMENTS,
                    ));
                }
                Ok(IndexMap::from([(self.to_string(), repo_name)]))
            }
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:337 ModuleArg.CanonicalRepoName.resolveToRepoNames
            ModuleArg::CanonicalRepoName(repo_name) => {
                Ok(IndexMap::from([(self.to_string(), repo_name.clone())]))
            }
        }
    }
}

impl fmt::Display for ModuleArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:145 ModuleArg.SpecificVersionOfModule.toString
            ModuleArg::SpecificVersionOfModule(k) => write!(f, "{k}"),
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:219 ModuleArg.AllVersionsOfModule.toString
            ModuleArg::AllVersionsOfModule(name) => f.write_str(name),
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:285 ModuleArg.ApparentRepoName.toString
            ModuleArg::ApparentRepoName(name) => write!(f, "@{name}"),
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:346 ModuleArg.CanonicalRepoName.toString
            ModuleArg::CanonicalRepoName(r) => write!(f, "@@{}", r.get_name()),
        }
    }
}

/// Java `List.toString()` of module args, e.g. `[<root>]` (used in `--from %s` messages).
pub fn module_arg_list_to_string(args: &[ModuleArg]) -> String {
    let parts: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    format!("[{}]", parts.join(", "))
}
