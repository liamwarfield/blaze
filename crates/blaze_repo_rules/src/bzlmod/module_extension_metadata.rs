//! `extension_metadata`: the value a module extension's implementation function may return, its
//! lockfile form, and the `use_repo` fixup computed from it.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionMetadata.java,
//! LockfileModuleExtensionMetadata.java (`of`, `generateFixup`; the data class is
//! `blaze_bzlmod_model::LockfileModuleExtensionMetadata`) and RootModuleFileFixup.java.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;

use allocative::Allocative;
use blaze_bzlmod_model::Facts;
use blaze_bzlmod_model::LockfileModuleExtensionMetadata;
use blaze_bzlmod_model::ModuleExtensionUsage;
use blaze_bzlmod_model::UseAllRepos;
use blaze_cmdline::RepositoryName;
use blaze_events::Event;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use indexmap::IndexSet;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::ListRef;
use starlark::values::starlark_value;
use starlark::values::tuple::TupleRef;

use super::facts;
use super::facts::java_string_cmp;

/// The Starlark object optionally returned by a module extension's implementation function
/// (type `extension_metadata`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionMetadata.java:40 ModuleExtensionMetadata
#[derive(Debug, Clone, PartialEq, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ModuleExtensionMetadata {
    #[allocative(skip)]
    explicit_root_module_direct_deps: Option<IndexSet<String>>,
    #[allocative(skip)]
    explicit_root_module_direct_dev_deps: Option<IndexSet<String>>,
    #[allocative(skip)]
    use_all_repos: UseAllRepos,
    reproducible: bool,
    #[allocative(skip)]
    facts: Facts,
}

starlark_simple_value!(ModuleExtensionMetadata);

impl fmt::Display for ModuleExtensionMetadata {
    // The default `StarlarkValue.repr` of the AutoValue class.
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.bazel.bzlmod.AutoValue_ModuleExtensionMetadata>",
        )
    }
}

#[starlark_value(type = "extension_metadata")]
impl<'v> StarlarkValue<'v> for ModuleExtensionMetadata {
    // AutoValue equals.
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<ModuleExtensionMetadata>()
            .is_some_and(|o| o == self))
    }
}

/// `Sequence.cast(x, String.class, what)`. The error is the `EvalException` message.
// Java: net/starlark/java/eval/Sequence.java:132 Sequence.cast
fn cast_string_sequence(x: Value, what: &str) -> Result<Vec<String>, String> {
    let items: Vec<Value> = if let Some(l) = ListRef::from_value(x) {
        l.iter().collect()
    } else if let Some(t) = TupleRef::from_value(x) {
        t.iter().collect()
    } else {
        return Err(format!("for {what}, got {}, want sequence", x.get_type()));
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        match item.unpack_str() {
            Some(s) => out.push(s.to_owned()),
            None => {
                return Err(format!(
                    "at index {i} of {what}, got element of type {}, want string",
                    item.get_type()
                ));
            }
        }
    }
    Ok(out)
}

/// `x.equals(StarlarkList.immutableOf())`: an empty list (not a tuple).
fn is_empty_list(x: Value) -> bool {
    ListRef::from_value(x).is_some_and(|l| l.is_empty())
}

impl ModuleExtensionMetadata {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionMetadata.java:42 ModuleExtensionMetadata.DEFAULT
    pub fn default_metadata() -> ModuleExtensionMetadata {
        Self::create_internal(None, None, UseAllRepos::No, false, Facts::empty())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionMetadata.java:50 ModuleExtensionMetadata.REPRODUCIBLE
    pub fn reproducible_metadata() -> ModuleExtensionMetadata {
        Self::create_internal(None, None, UseAllRepos::No, true, Facts::empty())
    }

    pub fn get_explicit_root_module_direct_deps(&self) -> Option<&IndexSet<String>> {
        self.explicit_root_module_direct_deps.as_ref()
    }

    pub fn get_explicit_root_module_direct_dev_deps(&self) -> Option<&IndexSet<String>> {
        self.explicit_root_module_direct_dev_deps.as_ref()
    }

    pub fn get_use_all_repos(&self) -> UseAllRepos {
        self.use_all_repos
    }

    pub fn get_reproducible(&self) -> bool {
        self.reproducible
    }

    pub fn get_facts(&self) -> &Facts {
        &self.facts
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionMetadata.java:70 ModuleExtensionMetadata.create
    fn create_internal(
        explicit_root_module_direct_deps: Option<IndexSet<String>>,
        explicit_root_module_direct_dev_deps: Option<IndexSet<String>>,
        use_all_repos: UseAllRepos,
        reproducible: bool,
        facts: Facts,
    ) -> ModuleExtensionMetadata {
        ModuleExtensionMetadata {
            explicit_root_module_direct_deps,
            explicit_root_module_direct_dev_deps,
            use_all_repos,
            reproducible,
            facts,
        }
    }

    /// `module_ctx.extension_metadata(...)`. `facts_obj` has passed `Dict.cast(facts,
    /// String.class, Object.class, "facts")`. The error is the `EvalException` message.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionMetadata.java:88 ModuleExtensionMetadata.create
    pub fn create(
        root_module_direct_deps_unchecked: Value,
        root_module_direct_dev_deps_unchecked: Value,
        reproducible: bool,
        facts_obj: Value,
    ) -> Result<ModuleExtensionMetadata, String> {
        let facts = facts::validate_and_create(facts_obj)?;

        if root_module_direct_deps_unchecked.is_none()
            && root_module_direct_dev_deps_unchecked.is_none()
        {
            return Ok(Self::create_internal(
                None,
                None,
                UseAllRepos::No,
                reproducible,
                facts,
            ));
        }

        let deps_all = root_module_direct_deps_unchecked.unpack_str() == Some("all");
        let dev_deps_all = root_module_direct_dev_deps_unchecked.unpack_str() == Some("all");

        // When root_module_direct_deps = "all", accept both root_module_direct_dev_deps = None and
        // root_module_direct_dev_deps = [], but not root_module_direct_dev_deps = ["some_repo"].
        if deps_all && is_empty_list(root_module_direct_dev_deps_unchecked) {
            return Ok(Self::create_internal(
                None,
                None,
                UseAllRepos::Regular,
                reproducible,
                facts,
            ));
        }

        if dev_deps_all && is_empty_list(root_module_direct_deps_unchecked) {
            return Ok(Self::create_internal(
                None,
                None,
                UseAllRepos::Dev,
                reproducible,
                facts,
            ));
        }

        if deps_all || dev_deps_all {
            return Err(
                "if one of root_module_direct_deps and root_module_direct_dev_deps is \
                        \"all\", the other must be an empty list"
                    .to_owned(),
            );
        }

        if root_module_direct_deps_unchecked.unpack_str().is_some()
            || root_module_direct_dev_deps_unchecked.unpack_str().is_some()
        {
            return Err(
                "root_module_direct_deps and root_module_direct_dev_deps must be \
                        None, \"all\", or a list of strings"
                    .to_owned(),
            );
        }
        if root_module_direct_deps_unchecked.is_none()
            != root_module_direct_dev_deps_unchecked.is_none()
        {
            return Err(
                "root_module_direct_deps and root_module_direct_dev_deps must both be \
                        specified or both be unspecified"
                    .to_owned(),
            );
        }

        let root_module_direct_deps =
            cast_string_sequence(root_module_direct_deps_unchecked, "root_module_direct_deps")?;
        let root_module_direct_dev_deps = cast_string_sequence(
            root_module_direct_dev_deps_unchecked,
            "root_module_direct_dev_deps",
        )?;

        let mut explicit_root_module_direct_deps = IndexSet::new();
        for dep in root_module_direct_deps {
            if let Err(e) = RepositoryName::validate_user_provided_repo_name(&dep) {
                return Err(format!("in root_module_direct_deps: {}", e.get_message()));
            }
            if explicit_root_module_direct_deps.contains(&dep) {
                return Err(format!(
                    "in root_module_direct_deps: duplicate entry '{dep}'"
                ));
            }
            explicit_root_module_direct_deps.insert(dep);
        }

        let mut explicit_root_module_direct_dev_deps = IndexSet::new();
        for dep in root_module_direct_dev_deps {
            if let Err(e) = RepositoryName::validate_user_provided_repo_name(&dep) {
                return Err(format!(
                    "in root_module_direct_dev_deps: {}",
                    e.get_message()
                ));
            }
            if explicit_root_module_direct_deps.contains(&dep) {
                return Err(format!(
                    "in root_module_direct_dev_deps: entry '{dep}' is also in \
                     root_module_direct_deps"
                ));
            }
            if explicit_root_module_direct_dev_deps.contains(&dep) {
                return Err(format!(
                    "in root_module_direct_dev_deps: duplicate entry '{dep}'"
                ));
            }
            explicit_root_module_direct_dev_deps.insert(dep);
        }

        Ok(Self::create_internal(
            Some(explicit_root_module_direct_deps),
            Some(explicit_root_module_direct_dev_deps),
            UseAllRepos::No,
            reproducible,
            facts,
        ))
    }
}

/// `LockfileModuleExtensionMetadata.of(moduleExtensionMetadata)`: `None` for the default
/// metadata.
// Java: com/google/devtools/build/lib/bazel/bzlmod/LockfileModuleExtensionMetadata.java:66 LockfileModuleExtensionMetadata.of
pub fn lockfile_module_extension_metadata_of(
    module_extension_metadata: &ModuleExtensionMetadata,
) -> Option<LockfileModuleExtensionMetadata> {
    if *module_extension_metadata == ModuleExtensionMetadata::default_metadata() {
        return None;
    }
    Some(LockfileModuleExtensionMetadata {
        explicit_root_module_direct_deps: module_extension_metadata
            .explicit_root_module_direct_deps
            .as_ref()
            .map(|s| s.iter().cloned().collect()),
        explicit_root_module_direct_dev_deps: module_extension_metadata
            .explicit_root_module_direct_dev_deps
            .as_ref()
            .map(|s| s.iter().cloned().collect()),
        use_all_repos: module_extension_metadata.use_all_repos,
        reproducible: module_extension_metadata.reproducible,
    })
}

/// Generated when incorrect use_repo calls are detected in the root module file and its
/// includes according to `ModuleExtensionMetadata`, and contains the buildozer commands required
/// to bring them into the expected state.
///
/// PORT: move to blaze_bzlmod/src/skyframe/keys.rs, replacing the `RootModuleFileFixup = ()`
/// placeholder there, so that `SingleExtensionValue::fixup` can hold it (`SingleExtensionFunction`
/// reports `warning`, `BazelModTidyFunction` applies the commands).
// Java: com/google/devtools/build/lib/bazel/bzlmod/RootModuleFileFixup.java:33 RootModuleFileFixup
#[derive(Debug, Clone)]
pub struct RootModuleFileFixup {
    /// The keys are the paths to the root MODULE.bazel file (or its includes); the values are
    /// the buildozer commands required to bring the keyed file into the expected state (Java
    /// `ImmutableListMultimap`: keys in first-insertion order).
    pub module_file_path_to_buildozer_commands: IndexMap<PathFragment, Vec<String>>,
    pub usage: ModuleExtensionUsage,
    pub warning: Event,
}

impl RootModuleFileFixup {
    /// A human-readable message describing the fixup after it has been applied.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RootModuleFileFixup.java:39 RootModuleFileFixup.getSuccessMessage
    pub fn get_success_message(&self) -> String {
        let extension_id = format!(
            "{}%{}",
            self.usage.extension_bzl_file, self.usage.extension_name
        );
        match &self.usage.isolation_key {
            Some(key) => format!(
                "Updated use_repo calls for isolated usage '{}' of {}",
                key.usage_exported_name(),
                extension_id
            ),
            None => format!("Updated use_repo calls for {extension_id}"),
        }
    }
}

/// An `ImmutableSortedSet<String>` (natural `String` order).
fn sorted_set<'a>(items: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let set: BTreeSet<JavaString> = items.into_iter().map(|s| JavaString(s.clone())).collect();
    set.into_iter().map(|s| s.0).collect()
}

/// A string ordered like Java's `String.compareTo`.
#[derive(PartialEq, Eq)]
struct JavaString(String);

impl Ord for JavaString {
    fn cmp(&self, other: &Self) -> Ordering {
        java_string_cmp(&self.0, &other.0)
    }
}

impl PartialOrd for JavaString {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `LockfileModuleExtensionMetadata.generateFixup(rootUsage, allRepos)`. The error is the
/// `EvalException` message.
// Java: com/google/devtools/build/lib/bazel/bzlmod/LockfileModuleExtensionMetadata.java:79 LockfileModuleExtensionMetadata.generateFixup
pub fn generate_fixup(
    metadata: &LockfileModuleExtensionMetadata,
    root_usage: &ModuleExtensionUsage,
    all_repos: &IndexSet<String>,
) -> Result<Option<RootModuleFileFixup>, String> {
    let root_module_direct_dev_deps = get_root_module_direct_dev_deps(metadata, all_repos)?;
    let root_module_direct_deps = get_root_module_direct_deps(metadata, all_repos)?;
    if root_module_direct_dev_deps.is_none() && root_module_direct_deps.is_none() {
        return Ok(None);
    }
    blaze_util::check_state!(
        root_module_direct_dev_deps.is_some() && root_module_direct_deps.is_some()
    );
    let root_module_direct_deps = root_module_direct_deps.unwrap();
    let root_module_direct_dev_deps = root_module_direct_dev_deps.unwrap();

    if !root_usage.get_has_non_dev_use_extension() && !root_module_direct_deps.is_empty() {
        return Err(
            "root_module_direct_deps must be empty if the root module contains no \
                    usages with dev_dependency = False"
                .to_owned(),
        );
    }
    if !root_usage.get_has_dev_use_extension() && !root_module_direct_dev_deps.is_empty() {
        return Err(
            "root_module_direct_dev_deps must be empty if the root module contains no \
                    usages with dev_dependency = True"
                .to_owned(),
        );
    }

    Ok(generate_fixup_internal(
        root_usage,
        all_repos,
        &root_module_direct_deps,
        &root_module_direct_dev_deps,
    ))
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/LockfileModuleExtensionMetadata.java:104 LockfileModuleExtensionMetadata.generateFixup
fn generate_fixup_internal(
    root_usage: &ModuleExtensionUsage,
    all_repos: &IndexSet<String>,
    expected_imports: &IndexSet<String>,
    expected_dev_imports: &IndexSet<String>,
) -> Option<RootModuleFileFixup> {
    let actual_dev_imports: IndexSet<String> = root_usage
        .proxies
        .iter()
        .filter(|p| p.dev_dependency)
        .flat_map(|p| p.imports.values().cloned())
        .collect();
    let actual_imports: IndexSet<String> = root_usage
        .proxies
        .iter()
        .filter(|p| !p.dev_dependency)
        .flat_map(|p| p.imports.values().cloned())
        .collect();

    let extension_bzl_file = &root_usage.extension_bzl_file;
    let extension_name = &root_usage.extension_name;

    let imports_to_add = sorted_set(expected_imports.difference(&actual_imports));
    let imports_to_remove = sorted_set(actual_imports.difference(expected_imports));
    let dev_imports_to_add = sorted_set(expected_dev_imports.difference(&actual_dev_imports));
    let dev_imports_to_remove = sorted_set(actual_dev_imports.difference(expected_dev_imports));

    if imports_to_add.is_empty()
        && imports_to_remove.is_empty()
        && dev_imports_to_add.is_empty()
        && dev_imports_to_remove.is_empty()
    {
        return None;
    }

    let mut message = format!(
        "The module extension {extension_name} defined in {extension_bzl_file} reported \
         incorrect imports of repositories via use_repo():\n\n"
    );

    let all_actual_imports = sorted_set(actual_imports.union(&actual_dev_imports));
    let all_expected_imports = sorted_set(expected_imports.union(expected_dev_imports));

    let invalid_imports: Vec<String> = sorted_set(
        all_actual_imports
            .iter()
            .filter(|i| !all_repos.contains(i.as_str())),
    );
    if !invalid_imports.is_empty() {
        message.push_str(&format!(
            "Imported, but not created by the extension (will cause the build to fail):\n    \
             {}\n\n",
            invalid_imports.join(", ")
        ));
    }

    let missing_imports = sorted_set(
        all_expected_imports
            .iter()
            .filter(|i| !all_actual_imports.contains(*i)),
    );
    if !missing_imports.is_empty() {
        message.push_str(&format!(
            "Not imported, but reported as direct dependencies by the extension (may cause the \
             build to fail):\n    {}\n\n",
            missing_imports.join(", ")
        ));
    }

    let non_dev_imports_of_dev_deps =
        sorted_set(expected_dev_imports.intersection(&actual_imports));
    if !non_dev_imports_of_dev_deps.is_empty() {
        message.push_str(&format!(
            "Imported as a regular dependency, but reported as a dev dependency by the \
             extension (may cause the build to fail when used by other modules):\n    {}\n\n",
            non_dev_imports_of_dev_deps.join(", ")
        ));
    }

    let dev_imports_of_non_dev_deps =
        sorted_set(expected_imports.intersection(&actual_dev_imports));
    if !dev_imports_of_non_dev_deps.is_empty() {
        message.push_str(&format!(
            "Imported as a dev dependency, but reported as a regular dependency by the \
             extension (may cause the build to fail when used by other modules):\n    {}\n\n",
            dev_imports_of_non_dev_deps.join(", ")
        ));
    }

    let indirect_dep_imports = sorted_set(
        all_actual_imports
            .iter()
            .filter(|i| all_repos.contains(i.as_str()) && !all_expected_imports.contains(*i)),
    );
    if !indirect_dep_imports.is_empty() {
        message.push_str(&format!(
            "Imported, but reported as indirect dependencies by the extension:\n    {}\n\n",
            indirect_dep_imports.join(", ")
        ));
    }

    message.push_str("Fix the use_repo calls by running 'bazel mod tidy'.");

    let mut module_file_path_to_commands: IndexMap<PathFragment, Vec<String>> = IndexMap::new();
    let mut put = |path: &PathFragment, command: String| {
        module_file_path_to_commands
            .entry(path.clone())
            .or_default()
            .push(command);
    };
    // Repos to add are easy: always add them to the first proxy of the correct type.
    if !imports_to_add.is_empty() {
        let first_non_dev_proxy = root_usage
            .proxies
            .iter()
            .find(|p| !p.dev_dependency)
            .expect("NoSuchElementException: No value present");
        put(
            &first_non_dev_proxy.containing_module_file_path,
            make_use_repo_command(
                "use_repo_add",
                &first_non_dev_proxy.proxy_name,
                &imports_to_add,
            ),
        );
    }
    if !dev_imports_to_add.is_empty() {
        let first_dev_proxy = root_usage
            .proxies
            .iter()
            .find(|p| p.dev_dependency)
            .expect("NoSuchElementException: No value present");
        put(
            &first_dev_proxy.containing_module_file_path,
            make_use_repo_command(
                "use_repo_add",
                &first_dev_proxy.proxy_name,
                &dev_imports_to_add,
            ),
        );
    }
    // Repos to remove are a bit trickier: remove them from the proxy that actually imported them.
    for proxy in &root_usage.proxies {
        let removals = if proxy.dev_dependency {
            &dev_imports_to_remove
        } else {
            &imports_to_remove
        };
        // Sets.intersection(proxy.getImports().values(), removals), sorted.
        let to_remove = sorted_set(proxy.imports.values().filter(|v| removals.contains(*v)));
        if !to_remove.is_empty() {
            put(
                &proxy.containing_module_file_path,
                make_use_repo_command("use_repo_remove", &proxy.proxy_name, &to_remove),
            );
        }
    }

    Some(RootModuleFileFixup {
        module_file_path_to_buildozer_commands: module_file_path_to_commands,
        usage: root_usage.clone(),
        warning: Event::warn_at(Some(root_usage.proxies[0].location.clone()), message),
    })
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/LockfileModuleExtensionMetadata.java:239 LockfileModuleExtensionMetadata.makeUseRepoCommand
fn make_use_repo_command(cmd: &str, proxy_name: &str, repos: &[String]) -> String {
    let mut command_parts = Vec::with_capacity(repos.len() + 2);
    command_parts.push(cmd.to_owned());
    command_parts.push(if proxy_name.is_empty() {
        "_unnamed_usage".to_owned()
    } else {
        proxy_name.to_owned()
    });
    command_parts.extend(repos.iter().cloned());
    command_parts.join(" ")
}

/// The explicit dependencies as a set (Java `ImmutableSet` from the lockfile form).
fn explicit_set(deps: &Option<Vec<String>>) -> Option<IndexSet<String>> {
    deps.as_ref().map(|d| d.iter().cloned().collect())
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/LockfileModuleExtensionMetadata.java:247 LockfileModuleExtensionMetadata.getRootModuleDirectDeps
fn get_root_module_direct_deps(
    metadata: &LockfileModuleExtensionMetadata,
    all_repos: &IndexSet<String>,
) -> Result<Option<IndexSet<String>>, String> {
    match metadata.use_all_repos {
        UseAllRepos::No => {
            let explicit = explicit_set(&metadata.explicit_root_module_direct_deps);
            if let Some(explicit) = &explicit {
                let invalid_repos: Vec<&str> = explicit
                    .iter()
                    .filter(|r| !all_repos.contains(*r))
                    .map(String::as_str)
                    .collect();
                if !invalid_repos.is_empty() {
                    return Err(format!(
                        "root_module_direct_deps contained the following repositories not \
                         generated by the extension: {}",
                        invalid_repos.join(", ")
                    ));
                }
            }
            Ok(explicit)
        }
        UseAllRepos::Regular => Ok(Some(all_repos.clone())),
        UseAllRepos::Dev => Ok(Some(IndexSet::new())),
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/LockfileModuleExtensionMetadata.java:267 LockfileModuleExtensionMetadata.getRootModuleDirectDevDeps
fn get_root_module_direct_dev_deps(
    metadata: &LockfileModuleExtensionMetadata,
    all_repos: &IndexSet<String>,
) -> Result<Option<IndexSet<String>>, String> {
    match metadata.use_all_repos {
        UseAllRepos::No => {
            let explicit = explicit_set(&metadata.explicit_root_module_direct_dev_deps);
            if let Some(explicit) = &explicit {
                let invalid_repos: Vec<&str> = explicit
                    .iter()
                    .filter(|r| !all_repos.contains(*r))
                    .map(String::as_str)
                    .collect();
                if !invalid_repos.is_empty() {
                    return Err(format!(
                        "root_module_direct_dev_deps contained the following repositories not \
                         generated by the extension: {}",
                        invalid_repos.join(", ")
                    ));
                }
            }
            Ok(explicit)
        }
        UseAllRepos::Regular => Ok(Some(IndexSet::new())),
        UseAllRepos::Dev => Ok(Some(all_repos.clone())),
    }
}
