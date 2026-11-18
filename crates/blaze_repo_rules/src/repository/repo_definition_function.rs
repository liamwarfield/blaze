//! REPO_DEFINITION: looks up the definition of a repo with the given name.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/RepoDefinitionFunction.java.

use std::sync::Arc;

use blaze_bzlmod::skyframe::keys::BazelDepGraphKey;
use blaze_bzlmod::skyframe::keys::ModuleFileKey;
use blaze_bzlmod::skyframe::keys::SingleExtensionValue;
use blaze_bzlmod_model::BazelDepGraphValue;
use blaze_bzlmod_model::ExternalDepsCode;
use blaze_bzlmod_model::ExternalDepsException;
use blaze_bzlmod_model::RepoRuleId;
use blaze_bzlmod_model::RepoSpec;
use blaze_bzlmod_model::discovery::RootModuleFileValue;
use blaze_bzlmod_model::module::BOOTSTRAP_REPO_RULES;
use blaze_bzlmod_model::module::ModuleOverride;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use blaze_cmdline::package_identifier::EMPTY_PACKAGE_ID;
use blaze_cmdline::repository_name::BAZEL_TOOLS;
use blaze_cmdline::repository_name::MAIN;
use blaze_options::spelling::SpellChecker;
use blaze_packages::types::label_converter::LabelConverter;
use blaze_sky_loading::precomputed::REPOSITORY_OVERRIDES;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_skyframe::ComputeError;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_skyframe::SkyFunctionException;
use blaze_starlark::error::StackEntry;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use starlark::environment::Module;
use starlark::values::Value;

use super::repo_definition::RepoDefinition;
use super::repo_definition::RepoDefinitionKey;
use super::repo_definition::RepoDefinitionValue;
use super::repo_rule::RepoRule;
use crate::attr_values::attribute_value_to_starlark;
use crate::bzl_load::load_bzl_for_bzlmod;
use crate::services::RepoServices;
use crate::starlark::repository_module::repo_rule_from_global;

/// Java `RepoDefinitionFunctionException(e, Transience.PERSISTENT)`: the cause is the
/// `BzlLoadFailedException` or `ExternalDepsException` itself.
// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinitionFunction.java:322 RepoDefinitionFunction.RepoDefinitionFunctionException
fn repo_definition_function_exception<E>(e: E) -> ComputeError
where
    E: std::error::Error + Send + Sync + 'static,
{
    SkyFunctionException::persistent(e).into()
}

/// `REPOSITORY_OVERRIDES.get(env)` followed by `Preconditions.checkNotNull`.
fn repository_overrides(
    env: &mut Environment<'_>,
) -> Result<Arc<IndexMap<String, PathFragment>>, Interrupted> {
    Ok(REPOSITORY_OVERRIDES
        .get(env)?
        .expect("NullPointerException: REPOSITORY_OVERRIDES"))
}

// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinitionFunction.java:68 RepoDefinitionFunction.compute
pub fn compute(
    services: &RepoServices,
    key: &RepoDefinitionKey,
    env: &mut Environment<'_>,
) -> Result<Option<RepoDefinitionValue>, ComputeError> {
    let root = env.get_value(ModuleFileKey::key_for_root_module())?;
    if env.values_missing() {
        return Ok(None);
    }
    let root = root.expect("values are not missing");
    let root = root.as_root();

    // Sometimes, the attributes in the repo specs contain label strings instead of label objects.
    // This really only happens for attributes like `patches` that come from an `archive_override`
    // or `git_override` in MODULE.bazel. (This is because those overrides just store all
    // attributes unparsed and unvalidated in the repo spec.) In other words, this can only happen
    // for Step 1 below -- but for consistency's sake, we pass this mapping for all 3 cases.
    //
    // In such cases, we need to provide a "basic repo mapping", so that we can properly turn
    // those label strings into label objects. Since we only accept patches from the main repo
    // anyway, we only need the two simple entries pointing into the main repo itself.
    // Java: ImmutableMap.builder().put("", MAIN).put(repoName, MAIN).buildKeepingLast()
    let basic_main_repo_mapping = RepositoryMapping::create(
        [
            (String::new(), MAIN.clone()),
            (root.module.repo_name.clone(), MAIN.clone()),
        ],
        MAIN.clone(),
    );

    let repository_name = key.argument();

    // Step 0: Look for repository overrides via canonical names. Requesting the main repo mapping
    // at this point would result in a cycle, so any lookup by apparent name requires special
    // handling.
    let overrides = repository_overrides(env)?;
    if let Some(path) = overrides.get(repository_name.get_name()) {
        return Ok(Some(RepoDefinitionValue::RepoOverride(path.clone())));
    }
    if *repository_name == *BAZEL_TOOLS {
        return Ok(Some(RepoDefinitionValue::RepoOverride(
            services
                .embedded_binaries_root()
                .get_relative_str("embedded_tools")
                .as_fragment()
                .clone(),
        )));
    }

    // Step 1: Look for repositories defined by non-registry overrides.
    let non_registry_override = check_repo_from_non_registry_overrides(
        root,
        repository_name,
        &overrides,
        &basic_main_repo_mapping,
        env,
    )?;
    if env.values_missing() {
        return Ok(None);
    }
    if let Some(value) = non_registry_override {
        return Ok(Some(value));
    }

    // BazelDepGraphValue is affected by repos found in Step 1, therefore it should NOT
    // be requested in Step 1 to avoid cycle dependency.
    let bazel_dep_graph_value = env.get_value(BazelDepGraphKey)?;
    if env.values_missing() {
        return Ok(None);
    }
    let bazel_dep_graph_value = bazel_dep_graph_value.expect("values are not missing");

    // Step 2: Look for repository overrides via apparent names.
    let apparent_name_overrides = get_apparent_name_overrides(env)?;
    if env.values_missing() {
        return Ok(None);
    }
    let apparent_name_overrides = apparent_name_overrides.expect("values are not missing");
    if let Some(path) = apparent_name_overrides.get(repository_name) {
        return Ok(Some(RepoDefinitionValue::RepoOverride(path.clone())));
    }

    // Step 3: Look for repositories derived from Bazel Modules.
    if let Some(repo_spec) = check_repo_from_bazel_modules(&bazel_dep_graph_value, repository_name)
    {
        return create_repo_definition_from_spec(
            &repo_spec,
            repository_name,
            /* original_name= */ None,
            &basic_main_repo_mapping,
            env,
        );
    }

    // Step 4: look for the repo from module extension evaluation results.
    let extension_id = bazel_dep_graph_value
        .extension_unique_names
        .iter()
        .find(|(_, unique_name)| {
            repository_name
                .get_name()
                .starts_with(&format!("{}+", unique_name))
        })
        .map(|(id, _)| id.clone());

    let Some(extension_id) = extension_id else {
        return Ok(Some(RepoDefinitionValue::NotFound));
    };

    let Some(extension_value) = env.get_value(SingleExtensionValue::key(extension_id))? else {
        return Ok(None);
    };

    let Some(internal_repo) = extension_value
        .canonical_repo_name_to_internal_names
        .get(repository_name)
    else {
        return Ok(Some(RepoDefinitionValue::NotFound));
    };
    let ext_repo_spec = extension_value
        .generated_repo_specs
        .get(internal_repo)
        .expect("NullPointerException: generated repo spec");
    create_repo_definition_from_spec(
        ext_repo_spec,
        repository_name,
        Some(internal_repo.clone()),
        &basic_main_repo_mapping,
        env,
    )
}

/// Callers must check `env.values_missing()` and ignore the result if true.
// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinitionFunction.java:170 RepoDefinitionFunction.checkRepoFromNonRegistryOverrides
fn check_repo_from_non_registry_overrides(
    root: &RootModuleFileValue,
    repository_name: &RepositoryName,
    overrides: &IndexMap<String, PathFragment>,
    basic_main_repo_mapping: &RepositoryMapping,
    env: &mut Environment<'_>,
) -> Result<Option<RepoDefinitionValue>, ComputeError> {
    let Some(module_name) = root
        .non_registry_override_canonical_repo_to_module_name
        .get(repository_name)
    else {
        return Ok(None);
    };
    // If this module is a direct dep of the root module, its apparent name may be named by the
    // --override_repository flag, which takes precedence over the non-registry override.
    if let Some(module_repo_name) = root
        .non_registry_override_module_to_repo_name
        .get(module_name)
        && let Some(command_line_override) = overrides.get(module_repo_name)
    {
        return Ok(Some(RepoDefinitionValue::RepoOverride(
            command_line_override.clone(),
        )));
    }
    let override_ = match root.overrides.get(module_name) {
        Some(ModuleOverride::NonRegistry(o)) => o,
        _ => panic!("ClassCastException: override of {module_name} is not a NonRegistryOverride"),
    };
    create_repo_definition_from_spec(
        override_.repo_spec(),
        repository_name,
        /* original_name= */ None,
        basic_main_repo_mapping,
        env,
    )
}

/// `None` if a Skyframe value is missing.
// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinitionFunction.java:201 RepoDefinitionFunction.getApparentNameOverrides
fn get_apparent_name_overrides(
    env: &mut Environment<'_>,
) -> Result<Option<IndexMap<RepositoryName, PathFragment>>, ComputeError> {
    let Some(main_repo_mapping) = env.get_value(RepositoryMappingKey::key(MAIN.clone()))? else {
        return Ok(None);
    };
    let overrides = repository_overrides(env)?;
    let mapping = main_repo_mapping
        .repository_mapping()
        .expect("NullPointerException: main repository mapping");
    let mut apparent_name_overrides: IndexMap<RepositoryName, PathFragment> = IndexMap::new();
    for (apparent_name, path) in overrides.iter() {
        if !RepositoryName::is_apparent(apparent_name) {
            continue;
        }
        let canonical_name = mapping.get(apparent_name);
        if !canonical_name.is_visible() {
            return Err(repo_definition_function_exception(
                ExternalDepsException::with_message(
                    ExternalDepsCode::BadModule,
                    format!(
                        "no repository visible as '@{}'{} from the main repository, but \
                         overridden with --override_repository. Use --inject_repository to add \
                         new repositories.",
                        apparent_name,
                        SpellChecker::did_you_mean(
                            apparent_name,
                            mapping.entries().keys().map(|name| format!("@{}", name))
                        )
                    ),
                ),
            ));
        }
        // Java: ImmutableMap.Builder.buildOrThrow
        if apparent_name_overrides.contains_key(&canonical_name) {
            panic!(
                "IllegalArgumentException: Multiple entries with same key: {}",
                canonical_name
            );
        }
        apparent_name_overrides.insert(canonical_name, path.clone());
    }
    Ok(Some(apparent_name_overrides))
}

// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinitionFunction.java:235 RepoDefinitionFunction.checkRepoFromBazelModules
fn check_repo_from_bazel_modules(
    bazel_dep_graph_value: &BazelDepGraphValue,
    repository_name: &RepositoryName,
) -> Option<RepoSpec> {
    let module_key = bazel_dep_graph_value
        .canonical_repo_name_lookup
        .get(repository_name)?;
    bazel_dep_graph_value
        .dep_graph
        .get(module_key)
        .expect("NullPointerException: module of the canonical repo name lookup")
        .repo_spec
        .clone()
}

// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinitionFunction.java:244 RepoDefinitionFunction.createRepoDefinitionFromSpec
fn create_repo_definition_from_spec(
    repo_spec: &RepoSpec,
    repository_name: &RepositoryName,
    original_name: Option<String>,
    basic_main_repo_mapping: &RepositoryMapping,
    env: &mut Environment<'_>,
) -> Result<Option<RepoDefinitionValue>, ComputeError> {
    let repo_rule_id = repo_spec
        .repo_rule_id()
        .expect("NullPointerException: RepoSpec.repoRuleId");
    let Some(repo_rule) = load_repo_rule(repo_rule_id, env)? else {
        return Ok(None);
    };

    // Use a completely fake call stack. This should never be user-visible anyway, since the repo
    // spec here should have already been typechecked (or generated directly by a
    // Registry#getRepoSpec call -- in which case it better be correct already...).
    // Java: StarlarkThread.callStackEntry("<toplevel>", Location.BUILTIN)
    let call_stack = [StackEntry {
        name: "<toplevel>".to_owned(),
        location: None,
        source_line: None,
    }];
    let label_converter = LabelConverter::new(&EMPTY_PACKAGE_ID, basic_main_repo_mapping.clone());
    let listener = env.listener();
    // PORT: the stored attribute values become Starlark values on a temporary heap so that they
    // go through the same type check as values from a Starlark call.
    let type_checked_repo_spec = Module::with_temp_heap(|module| {
        let kwargs: Vec<(String, Value)> = repo_spec
            .attributes()
            .attributes()
            .iter()
            .map(|(k, v)| (k.clone(), attribute_value_to_starlark(v, module.heap())))
            .collect();
        repo_rule.instantiate(
            &kwargs,
            &call_stack,
            &label_converter,
            listener,
            "to the root module",
        )
    });
    match type_checked_repo_spec {
        Ok(type_checked_repo_spec) => {
            let repo_definition = RepoDefinition {
                repo_rule,
                attr_values: type_checked_repo_spec.attributes().clone(),
                name: repository_name.get_name().to_owned(),
                original_name,
            };
            Ok(Some(RepoDefinitionValue::Found(Arc::new(repo_definition))))
        }
        Err(e) => Err(repo_definition_function_exception(e)),
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinitionFunction.java:278 RepoDefinitionFunction.loadRepoRule
fn load_repo_rule(
    repo_rule_id: &RepoRuleId,
    env: &mut Environment<'_>,
) -> Result<Option<Arc<RepoRule>>, ComputeError> {
    let bootstrap = BOOTSTRAP_REPO_RULES.contains(repo_rule_id);
    let bzl_file_label = repo_rule_id
        .bzl_file_label()
        .expect("NullPointerException: RepoRuleId.bzlFileLabel");

    // Load the .bzl file pointed to by the label.
    let bzl_load_value = match load_bzl_for_bzlmod(env, bzl_file_label, bootstrap)? {
        None => return Ok(None),
        // No need for a super detailed error message, since errors here can basically only happen
        // when something is horribly wrong. (The labels to load are either hardcoded or already
        // sanity-checked somewhere else.)
        Some(Err(e)) => return Err(repo_definition_function_exception(e)),
        Some(Ok(v)) => v,
    };

    // PORT: Java `Module.getGlobal` returns any global; starlark-rust also marks loaded symbols
    // private, so a private (`_name`) global is found the same way.
    let object = bzl_load_value
        .get_module()
        .get_any_visibility(repo_rule_id.rule_name())
        .ok()
        .map(|(value, _)| value);
    let Some(object) = object else {
        return Err(repo_definition_function_exception(
            ExternalDepsException::with_message(
                ExternalDepsCode::ExtensionEvalError,
                format!(
                    "repository rule {} does not exist (no such symbol in that file)",
                    repo_rule_id
                ),
            ),
        ));
    };
    match repo_rule_from_global(&object, env)? {
        None => Ok(None),
        Some(Some(repo_rule)) => Ok(Some(repo_rule)),
        Some(None) => Err(repo_definition_function_exception(
            ExternalDepsException::with_message(
                ExternalDepsCode::ExtensionEvalError,
                format!(
                    "invalid repository rule: {}, expected type repository_rule, got type {}",
                    repo_rule_id,
                    object.value().get_type()
                ),
            ),
        )),
    }
}
