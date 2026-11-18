//! Port of `skyframe/BuildOptionsScopeFunction.java`: resolves the `Scope.ScopeType` of the
//! Starlark flags of a `BuildOptions` whose scope is not known yet (flags a transition set, flags
//! propagated into the exec configuration) and, for project-scoped flags, their scope definition.
//!
//! The key and value are `blaze_build_config::build_options_scope_value`; the caller is
//! `BuildConfigurationKeyProducer.findBuildOptionsScopes`
//! (`blaze_build_config::skyframe`).

use std::sync::Arc;

use blaze_build_config::build_options::{Scope, ScopeType};
use blaze_build_config::build_options_scope_value::{BuildOptionsScopeKey, BuildOptionsScopeValue};
use blaze_cmdline::{Label, TargetParsingException, TargetPatternsCode};
use blaze_packages::attribute::{AttributeMap, RawAttributeMapper};
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::target::Target;
use blaze_packages::types::{AttrValue, Type};
use blaze_sky_loading::package_function::PackageKey;
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, Interrupted, SkyFunctionException, Transience,
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
    // Java: com/google/devtools/build/lib/skyframe/BuildOptionsScopeFunction.java:131 BuildOptionsScopeFunction.findProjectFiles
    for (starlark_option, scope) in &scopes {
        if scope.get_scope_type() == ScopeType::PROJECT {
            // PORT: ProjectFilesLookupValue / ProjectValue (PROJECT.scl) are not ported.
            blaze_util::unported!(
                "BuildOptionsScopeFunction.findProjectFiles",
                "project-scoped flag {}",
                starlark_option
            );
        }
    }
    // No flag has a PROJECT.scl file: projectValueSkyKeysMap is empty, so no scope gets a
    // ScopeDefinition and the list of flags with project files is empty.
    Ok(Some(BuildOptionsScopeValue::create(
        Arc::new(fully_resolved_build_options_builder.build()),
        Vec::new(),
        scopes,
    )))
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
