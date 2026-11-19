//! Port of `skyframe/BuildOptionsScopeFunction.java`: resolves the `Scope.ScopeType` of the
//! Starlark flags of a `BuildOptions` whose scope is not known yet (flags a transition set, flags
//! propagated into the exec configuration) and, for project-scoped flags, their scope definition.
//!
//! The key and value are `blaze_build_config::build_options_scope_value`; the caller is
//! `BuildConfigurationKeyProducer.findBuildOptionsScopes`
//! (`blaze_build_config::skyframe`).

use std::sync::Arc;

use blaze_build_config::build_options::{Scope, ScopeDefinition, ScopeType};
use blaze_build_config::build_options_scope_value::{BuildOptionsScopeKey, BuildOptionsScopeValue};
use blaze_cmdline::{Label, TargetParsingException, TargetPatternsCode};
use blaze_packages::attribute::{AttributeMap, RawAttributeMapper};
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::target::Target;
use blaze_packages::types::{AttrValue, Type};
use blaze_sky_loading::package_function::PackageKey;
use blaze_sky_loading::project_function::{
    ProjectFilesLookupKey, ProjectFilesLookupValue, ProjectKey,
};
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, Interrupted, SkyFunctionException, SkyKey,
    Transience,
};
use indexmap::IndexMap;

/// Registers BUILD_OPTIONS_SCOPE.
pub fn register(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder.typed_function::<BuildOptionsScopeKey, _>(compute)
}

// Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeFunction.java:60 BuildOptionsScopeFunction.compute
fn compute(
    key: &BuildOptionsScopeKey,
    env: &mut Environment<'_>,
) -> Result<Option<BuildOptionsScopeValue>, ComputeError> {
    let mut fully_resolved_build_options_builder = key.get_build_options().to_builder();
    // LinkedHashMap<Label, Scope>
    let mut scopes: IndexMap<Label, Scope> = IndexMap::new();

    for scoped_flag in key.get_flags_with_incomplete_scope_info() {
        let scope_type = match key
            .get_build_options()
            .get_scope_type_map()
            .get(scoped_flag)
            .copied()
        {
            Some(scope_type) => scope_type,
            None => match get_scope_type(env, scoped_flag)? {
                Some(scope_type) => scope_type,
                None => return Ok(None),
            },
        };
        scopes.insert(scoped_flag.clone(), Scope::new(scope_type, None));
        // this is needed because the final BuildOptions used to create the BuildConfigurationKey
        // needs to have the scopeType set for all starlark flags.
        fully_resolved_build_options_builder.add_scope_type(scoped_flag.clone(), scope_type);
    }

    // get PROJECT.scl files for each scoped flag that is not universal
    let Some(project_files) = find_project_files(&scopes, env)? else {
        return Ok(None);
    };

    // look up ProjectValue for scoped flags that have PROJECT.scl files to get the list of
    // active directories that define the scope of the flag.
    // PORT: java-hash-order. Java's HashMap<Label, SkyKey> is iterated below and its key set
    // becomes the value's list of flags with project files; the entries are kept in the order of
    // `scopes` here.
    let mut project_value_sky_keys_map: IndexMap<Label, ProjectKey> = IndexMap::new();
    for (project_scoped_flag, files) in &project_files {
        if !files.is_empty() {
            let project_key = ProjectKey::new(files[0].clone());
            project_value_sky_keys_map.insert(project_scoped_flag.clone(), project_key);
        }
    }

    let sky_keys: Vec<SkyKey> = project_value_sky_keys_map
        .values()
        .map(|k| SkyKey::new(k.clone()))
        .collect();
    let project_values_look_up_result = env.get_values_and_exceptions(&sky_keys)?;

    if env.values_missing() {
        return Ok(None);
    }

    for (project_scoped_flag, project_key) in &project_value_sky_keys_map {
        let project_value = project_values_look_up_result
            .get_typed(project_key)
            .expect("NullPointerException: projectValue");
        let default_project_directories = project_value.get_default_project_directories();
        let scope_type = scopes[project_scoped_flag].get_scope_type();
        scopes.insert(
            project_scoped_flag.clone(),
            Scope::new(
                scope_type,
                if default_project_directories.is_empty() {
                    None
                } else {
                    Some(ScopeDefinition::new(default_project_directories))
                },
            ),
        );
    }

    Ok(Some(BuildOptionsScopeValue::create(
        Arc::new(fully_resolved_build_options_builder.build()),
        project_value_sky_keys_map.keys().cloned().collect(),
        scopes,
    )))
}

// Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeFunction.java:131 BuildOptionsScopeFunction.findProjectFiles
/// TODO: b/384057043 - deduplicate this method in several places in a follow up CL.
///
/// `None` where Java returns null. The multimap is a list of (flag, project files) in the order
/// of `scopes`.
fn find_project_files(
    scopes: &IndexMap<Label, Scope>,
    env: &mut Environment<'_>,
) -> Result<Option<Vec<(Label, Vec<Label>)>>, ComputeError> {
    // PORT: java-hash-order. Java collects the keys in a HashMap<Label, Key> and visits it in
    // its order; the flags are visited in the order of `scopes` here (the lookups do not report
    // events, and the result is a multimap keyed by flag).
    let mut targets_to_sky_keys: Vec<(Label, ProjectFilesLookupKey)> = Vec::new();
    for (starlark_option, scope) in scopes {
        if scope.get_scope_type() == ScopeType::PROJECT {
            targets_to_sky_keys.push((
                starlark_option.clone(),
                ProjectFilesLookupKey::key(starlark_option.get_package_identifier().clone()),
            ));
        }
    }

    let mut project_files_lookup_values: Vec<(Label, Arc<ProjectFilesLookupValue>)> = Vec::new();
    for (flag, sky_key) in targets_to_sky_keys {
        // Java: env.getValueOrThrow(key, ProjectResolutionException.class): PROJECT_FILES_LOOKUP
        // throws ProjectFilesLookupException, which is not a ProjectResolutionException, so an
        // error of the lookup is this node's error.
        let Some(project_files_lookup_value) = env.get_value(sky_key)? else {
            return Ok(None);
        };
        project_files_lookup_values.push((flag, project_files_lookup_value));
    }

    let mut project_files: Vec<(Label, Vec<Label>)> = Vec::new();
    for (flag, value) in project_files_lookup_values {
        project_files.push((flag, value.get_project_files().to_vec()));
    }
    Ok(Some(project_files))
}

// Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeFunction.java:164 BuildOptionsScopeFunction.getScopeType
/// `None` where Java returns null (the package of the flag is not available yet).
fn get_scope_type(
    env: &mut Environment<'_>,
    label: &Label,
) -> Result<Option<ScopeType>, ComputeError> {
    // Java builds PackageContext.of(packageIdentifier, RepositoryMapping.EMPTY) and has
    // SkyframeTargetLoader parse label.getUnambiguousCanonicalForm() in it, which gives back
    // `label`.
    let target = match load_build_setting(env, label)? {
        Ok(Some(target)) => target,
        Ok(None) => return Ok(None),
        // Java: new BuildOptionsScopeFunctionException(e) (Transience.PERSISTENT)
        Err(e) => return Err(SkyFunctionException::new(e, Transience::Persistent).into()),
    };
    let rule = target.get_associated_rule().unwrap_or_else(|| {
        panic!(
            "NullPointerException: RawAttributeMapper.of(null): {} is not a rule",
            label
        )
    });
    let attrs = RawAttributeMapper::of(&**rule);
    if !attrs.has_typed("scope", Type::String)
        // TODO: https://github.com/bazelbuild/bazel/issues/26909 - Honor the rule's actual
        // value when --incompatible_exclude_starlark_flags_from_exec_config is stably enabled
        // and existing rules like skylib's have updated to TARGET.
        || !attrs.is_attribute_value_explicitly_specified("scope")
    {
        return Ok(Some(ScopeType::DEFAULT));
    }
    let scope = match attrs.get("scope", Type::String) {
        Some(AttrValue::String(scope)) => scope,
        other => panic!(
            "ClassCastException: scope attribute of {} is {:?}",
            label, other
        ),
    };
    match ScopeType::value_of_ignore_case(&scope) {
        Some(scope_type) => Ok(Some(scope_type)),
        // Scope.ScopeType.valueOfIgnoreCase throws an unchecked IllegalArgumentException.
        None => panic!(
            "IllegalArgumentException: No enum constant com.google.devtools.build.lib.analysis.config.Scope.ScopeType.{}",
            scope.to_uppercase()
        ),
    }
}

// Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeFunction.java:207 BuildOptionsScopeFunction.SkyframeTargetLoader.loadBuildSetting
/// Same as `ParsedFlagsFunction.SkyframeTargetLoader`. `Ok(Ok(None))` where Java returns null.
fn load_build_setting(
    env: &mut Environment<'_>,
    as_label: &Label,
) -> Result<Result<Option<Target>, TargetParsingException>, Interrupted> {
    let failed_to_load = || {
        // Java: new TargetParsingException(String.format("Failed to load %s", name), e,
        // DEPENDENCY_NOT_FOUND), where name is the label's unambiguous canonical form.
        TargetParsingException::new(
            format!(
                "Failed to load {}",
                as_label.get_unambiguous_canonical_form()
            ),
            TargetPatternsCode::DependencyNotFound,
        )
    };
    let pkg_key = PackageKey::key(as_label.get_package_identifier().clone());
    let pkg = match env.get_value_or_throw::<PackageKey, NoSuchPackageException>(pkg_key)? {
        Ok(Some(pkg)) => pkg,
        Ok(None) => return Ok(Ok(None)),
        Err(_) => return Ok(Err(failed_to_load())),
    };
    match pkg.get_package().get_target(as_label.get_name()) {
        Ok(target) => Ok(Ok(Some(target.clone()))),
        Err(_) => Ok(Err(failed_to_load())),
    }
}
