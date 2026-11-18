//! Port of `skyframe/StarlarkBuildSettingsDetailsFunction.java`: details (defaults, types,
//! aliases) of the Starlark build settings a transition reads or writes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use blaze_analysis::config::starlark_transition::{
    StarlarkBuildSettingsDetailsKey, StarlarkBuildSettingsDetailsValue, TransitionException,
    attr_value_to_starlark_option,
};
use blaze_build_config::build_options::StarlarkOptionValue;
use blaze_cmdline::{Label, PackageIdentifier};
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::package::Package;
use blaze_packages::rule_class::rule_class::STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME;
use blaze_packages::target::{Rule, RuleOrMacroInstance, Target};
use blaze_packages::types::AttrValue;
use blaze_sky_loading::package_function::PackageKey;
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, SkyFunctionException, SkyKey, Transience,
};
use indexmap::{IndexMap, IndexSet};

// Use the plain strings rather than reaching into the Alias class and adding a dependency edge.
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsFunction.java:63 StarlarkBuildSettingsDetailsFunction.ALIAS_RULE_NAME
const ALIAS_RULE_NAME: &str = "alias";
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsFunction.java:64 StarlarkBuildSettingsDetailsFunction.ALIAS_ACTUAL_ATTRIBUTE_NAME
const ALIAS_ACTUAL_ATTRIBUTE_NAME: &str = "actual";

pub fn register(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder.typed_function::<StarlarkBuildSettingsDetailsKey, _>(compute)
}

fn throw(e: TransitionException) -> ComputeError {
    // Java: StarlarkBuildSettingsDetailsException(e) with Transience.PERSISTENT
    SkyFunctionException::new(e, Transience::Persistent).into()
}

// Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsFunction.java:68 StarlarkBuildSettingsDetailsFunction.compute
fn compute(
    key: &StarlarkBuildSettingsDetailsKey,
    env: &mut Environment<'_>,
) -> Result<Option<StarlarkBuildSettingsDetailsValue>, ComputeError> {
    // Ideally, callers would bypass StarlarkBuildSettingsDetailsFunction entirely when the key
    // is empty but provide a fast escape here just in case.
    if key.build_settings().is_empty() {
        return Ok(Some(StarlarkBuildSettingsDetailsValue::empty()));
    }

    let build_setting_packages = match get_build_setting_packages(env, key.build_settings())? {
        Ok(Some(p)) => p,
        Ok(None) => return Ok(None),
        Err(e) => return Err(throw(e)),
    };

    // Each setting is unique so don't need a merge function.
    let raw_setting_to_actual_rule: IndexMap<Label, Arc<Rule>> = key
        .build_settings()
        .iter()
        .map(|setting| {
            let rule = get_actual(&build_setting_packages, setting)
                .get_associated_rule()
                .expect("build setting rule")
                .clone();
            (setting.clone(), rule)
        })
        .collect();
    // ImmutableSet.copyOf: distinct rules, first occurrence order.
    let mut actual_rules: Vec<Arc<Rule>> = Vec::new();
    for rule in raw_setting_to_actual_rule.values() {
        if !actual_rules.iter().any(|r| Arc::ptr_eq(r, rule)) {
            actual_rules.push(rule.clone());
        }
    }

    // Calculate info based on the actual rules.
    let mut value = StarlarkBuildSettingsDetailsValue::empty();
    for rule in &actual_rules {
        let build_setting = rule
            .get_rule_class_object()
            .get_build_setting()
            .expect("build setting");
        let default = rule
            .get_attr(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME)
            .unwrap_or_else(|| {
                panic!(
                    "NullPointerException: build_setting_default of {}",
                    rule.get_label()
                )
            });
        let default = attr_value_to_starlark_option(&default);
        value.build_setting_to_default.insert(
            rule.get_label().clone(),
            if build_setting.allows_multiple() {
                // ImmutableList.of(rule.getAttr(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME))
                StarlarkOptionValue::list(vec![default])
            } else {
                default
            },
        );
    }
    for rule in &actual_rules {
        value.build_setting_to_type.insert(
            rule.get_label().clone(),
            rule.get_rule_class_object()
                .get_build_setting()
                .expect("build setting")
                .get_type(),
        );
    }
    for rule in &actual_rules {
        if rule
            .get_rule_class_object()
            .get_build_setting()
            .expect("build setting")
            .allows_multiple()
        {
            value
                .build_setting_is_allows_multiple
                .insert(rule.get_label().clone());
        }
    }

    // Calculate the alias table (filtering out non-aliases!)
    for (setting, rule) in &raw_setting_to_actual_rule {
        if setting != rule.get_label() {
            value
                .alias_to_actual
                .insert(setting.clone(), rule.get_label().clone());
        }
    }
    Ok(Some(value))
}

// Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsFunction.java:139 StarlarkBuildSettingsDetailsFunction.getBuildSettingPackages
fn get_build_setting_packages(
    env: &mut Environment<'_>,
    build_settings: &[Label],
) -> Result<
    Result<Option<HashMap<PackageIdentifier, Arc<Package>>>, TransitionException>,
    blaze_skyframe::Interrupted,
> {
    let mut build_setting_packages: HashMap<PackageIdentifier, Arc<Package>> = HashMap::new();
    // This happens before cycle detection so keep track of all seen build settings to ensure we
    // don't get stuck in endless loops (e.g. //alias1->//alias2 && //alias2->alias1)
    let mut all_seen_build_settings: HashSet<Label> = HashSet::new();
    let mut unverified_build_settings: IndexSet<Label> = build_settings.iter().cloned().collect();
    while !unverified_build_settings.is_empty() {
        for build_setting in &unverified_build_settings {
            if !all_seen_build_settings.insert(build_setting.clone()) {
                return Ok(Err(TransitionException(format!(
                    "Dependency cycle involving '{}' detected in aliased build settings",
                    build_setting
                ))));
            }
        }
        let package_keys: IndexSet<PackageIdentifier> = unverified_build_settings
            .iter()
            .map(|s| s.get_package_identifier().clone())
            .collect();
        let sky_keys: Vec<SkyKey> = package_keys
            .iter()
            .map(|p| SkyKey::new(PackageKey(p.clone())))
            .collect();
        let newly_loaded = env.get_values_and_exceptions(&sky_keys)?;
        if env.values_missing() {
            return Ok(Ok(None));
        }
        for package_key in &package_keys {
            match newly_loaded
                .get_or_throw_typed::<_, NoSuchPackageException>(&PackageKey(package_key.clone()))
            {
                Ok(Some(v)) => {
                    build_setting_packages.insert(package_key.clone(), v.get_package().clone());
                }
                Ok(None) => return Ok(Ok(None)),
                // Java: new TransitionException(e): the cause's toString as the message.
                Err(e) => {
                    return Ok(Err(TransitionException(format!(
                        "com.google.devtools.build.lib.packages.NoSuchPackageException: {}",
                        e.get_message()
                    ))));
                }
            }
        }
        unverified_build_settings = match verify_build_settings_and_get_aliases(
            &build_setting_packages,
            &unverified_build_settings,
        ) {
            Ok(a) => a,
            Err(e) => return Ok(Err(e)),
        };
    }
    Ok(Ok(Some(build_setting_packages)))
}

// Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsFunction.java:191 StarlarkBuildSettingsDetailsFunction.verifyBuildSettingsAndGetAliases
fn verify_build_settings_and_get_aliases(
    build_setting_packages: &HashMap<PackageIdentifier, Arc<Package>>,
    build_settings_to_verify: &IndexSet<Label>,
) -> Result<IndexSet<Label>, TransitionException> {
    let mut actual_setting_builder: IndexSet<Label> = IndexSet::new();
    for alleged_build_setting in build_settings_to_verify {
        let build_setting_package = build_setting_packages
            .get(alleged_build_setting.get_package_identifier())
            .expect("Reading build setting for which we don't have a package");
        let build_setting_target =
            match build_setting_package.get_target(alleged_build_setting.get_name()) {
                Ok(t) => t,
                // Java: new TransitionException(e)
                Err(e) => {
                    return Err(TransitionException(format!(
                        "com.google.devtools.build.lib.packages.NoSuchTargetException: {}",
                        e.get_message()
                    )));
                }
            };
        let Some(rule) = build_setting_target.get_associated_rule() else {
            return Err(TransitionException(format!(
                "attempting to transition on '{}' which is not a build setting",
                alleged_build_setting
            )));
        };
        if rule.get_rule_class() == ALIAS_RULE_NAME {
            match rule.get_attr(ALIAS_ACTUAL_ATTRIBUTE_NAME) {
                Some(AttrValue::Label(label)) => {
                    actual_setting_builder.insert(label);
                    continue;
                }
                Some(AttrValue::SelectorList(_)) => {
                    // configured "actual" value
                    return Err(TransitionException(format!(
                        "attempting to transition on aliased build setting '{}', the actual value of which uses select(). Aliased build settings with configured actual values is not supported.",
                        alleged_build_setting
                    )));
                }
                _ => panic!(
                    "IllegalStateException: Alias target '{}' with 'actual' attr value not equals to a label or a selectorlist",
                    alleged_build_setting
                ),
            }
        }
        if !rule.is_build_setting() {
            return Err(TransitionException(format!(
                "attempting to transition on '{}' which is not a build setting",
                alleged_build_setting
            )));
        }
    }
    Ok(actual_setting_builder)
}

// Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsFunction.java:245 StarlarkBuildSettingsDetailsFunction.getActual
fn get_actual(
    build_setting_packages: &HashMap<PackageIdentifier, Arc<Package>>,
    setting: &Label,
) -> Target {
    let mut target = get_target(build_setting_packages, setting);
    while target.get_associated_rule().expect("rule").get_rule_class() == ALIAS_RULE_NAME {
        let Some(AttrValue::Label(actual)) = target
            .get_associated_rule()
            .expect("rule")
            .get_attr(ALIAS_ACTUAL_ATTRIBUTE_NAME)
        else {
            panic!("ClassCastException: alias actual is not a Label");
        };
        target = get_target(build_setting_packages, &actual);
    }
    target
}

// Java: com/google/devtools/build/lib/skyframe/StarlarkBuildSettingsDetailsFunction.java:257 StarlarkBuildSettingsDetailsFunction.getTarget
fn get_target(
    build_setting_packages: &HashMap<PackageIdentifier, Arc<Package>>,
    setting: &Label,
) -> Target {
    let build_setting_package = build_setting_packages
        .get(setting.get_package_identifier())
        .expect("Reading build setting for which we don't have a package");
    match build_setting_package.get_target(setting.get_name()) {
        Ok(t) => t.clone(),
        // This should never happen, see javadoc.
        Err(e) => panic!("IllegalStateException: {e}"),
    }
}
