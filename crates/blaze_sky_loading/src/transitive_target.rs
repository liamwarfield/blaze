//! TRANSITIVE_TARGET: loading the transitive closure of a target (query's preloading).
//!
//! Java: com/google/devtools/build/lib/skyframe/{TransitiveTargetFunction,TransitiveTargetKey,
//! TransitiveTargetValue,TransitiveBaseTraversalFunction,TargetLoadingUtil}.java.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_events::{Event, EventHandler};
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::exceptions::{
    NoSuchPackageException, NoSuchPackageKind, NoSuchTargetException, NoSuchThingException,
};
use blaze_packages::target::label_visitation_utils;
use blaze_packages::target::{Target, target_utils};
use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionException, SkyFunctionName,
    SkyKey, SkyframeLookupResult,
};
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use crate::containing_package_lookup::ContainingPackageLookupKey;
use crate::io::IOException;
use crate::package_function::PackageKey;

/// Function name of TRANSITIVE_TARGET.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:72 SkyFunctions.TRANSITIVE_TARGET
pub const TRANSITIVE_TARGET: SkyFunctionName = SkyFunctionName("TRANSITIVE_TARGET");

/// The key of TRANSITIVE_TARGET. `Debug` prints `TransitiveTargetKey{label=<label>}`.
// Java: com/google/devtools/build/lib/skyframe/TransitiveTargetKey.java:28 TransitiveTargetKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TransitiveTargetKey(pub Label);

impl TransitiveTargetKey {
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTargetKey.java:35 TransitiveTargetKey.of
    pub fn of(label: Label) -> Self {
        TransitiveTargetKey(label)
    }

    // Java: com/google/devtools/build/lib/skyframe/TransitiveTargetKey.java:55 TransitiveTargetKey.getLabel
    pub fn get_label(&self) -> &Label {
        &self.0
    }
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveTargetKey.java:60 TransitiveTargetKey.toString
impl fmt::Debug for TransitiveTargetKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TransitiveTargetKey{{label={}}}", self.0)
    }
}

impl Key for TransitiveTargetKey {
    type Value = TransitiveTargetValue;
    const FUNCTION: SkyFunctionName = TRANSITIVE_TARGET;
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTargetKey.java:31 TransitiveTargetKey.NAME (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Default `values_equal` (never equal): Java's `TransitiveTargetValue` has no `equals`.
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTargetValue.java:34 TransitiveTargetValue
}

/// Java `NestedSet<Label>` in `STABLE_ORDER`: the target's label and its deps' sets.
#[derive(Debug)]
pub struct LabelNestedSet {
    direct: Label,
    transitive: Vec<Arc<LabelNestedSet>>,
}

impl LabelNestedSet {
    /// Java `NestedSet.toList()` (STABLE_ORDER flattening: transitive members first, in order,
    /// then the direct member; duplicates keep the first occurrence).
    pub fn to_list(&self) -> Vec<Label> {
        let mut out: IndexSet<Label> = IndexSet::new();
        let mut seen: std::collections::HashSet<*const LabelNestedSet> =
            std::collections::HashSet::new();
        fn walk(
            s: &LabelNestedSet,
            out: &mut IndexSet<Label>,
            seen: &mut std::collections::HashSet<*const LabelNestedSet>,
        ) {
            if !seen.insert(s as *const LabelNestedSet) {
                return;
            }
            for t in &s.transitive {
                walk(t, out, seen);
            }
            out.insert(s.direct.clone());
        }
        walk(self, &mut out, &mut seen);
        out.into_iter().collect()
    }
}

/// The result of TRANSITIVE_TARGET (Java `TransitiveTargetValue`).
// Java: com/google/devtools/build/lib/skyframe/TransitiveTargetValue.java:31 TransitiveTargetValue
#[derive(Debug)]
pub struct TransitiveTargetValue {
    transitive_targets: Arc<LabelNestedSet>,
    error_loading_target: Option<NoSuchTargetException>,
    encountered_loading_error: bool,
}

impl TransitiveTargetValue {
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTargetValue.java:62 TransitiveTargetValue.getTransitiveTargets
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTargetFunction.java:179 TransitiveTargetFunction.TransitiveTargetValueBuilder.getTransitiveTargets
    pub fn get_transitive_targets(&self) -> &Arc<LabelNestedSet> {
        &self.transitive_targets
    }

    // Java: com/google/devtools/build/lib/skyframe/TransitiveTargetValue.java:56 TransitiveTargetValue.encounteredLoadingError
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTargetValue.java:70 TransitiveTargetValue.UnsuccessfulTransitiveTargetValue.encounteredLoadingError
    pub fn encountered_loading_error(&self) -> bool {
        self.encountered_loading_error
    }

    // Java: com/google/devtools/build/lib/skyframe/TransitiveTargetValue.java:51 TransitiveTargetValue.getErrorLoadingTarget
    // Java: com/google/devtools/build/lib/skyframe/TransitiveTargetValue.java:64 TransitiveTargetValue.UnsuccessfulTransitiveTargetValue.getErrorLoadingTarget
    pub fn get_error_loading_target(&self) -> Option<&NoSuchTargetException> {
        self.error_loading_target.as_ref()
    }
}

/// A loaded target and whether its package was in error (Java
/// `TargetLoadingUtil.TargetAndErrorIfAny`).
// Java: com/google/devtools/build/lib/skyframe/TargetLoadingUtil.java:117 TargetLoadingUtil.TargetAndErrorIfAny
pub struct TargetAndErrorIfAny {
    pub package_loaded_successfully: bool,
    pub error_loading_target: Option<NoSuchTargetException>,
    pub target: Target,
    /// Java `getPackage()`.
    pub package: Arc<blaze_packages::package::Package>,
}

/// What `TargetLoadingUtil.loadTarget` throws.
pub enum LoadTargetError {
    NoSuchTarget(NoSuchTargetException),
    NoSuchPackage(NoSuchPackageException),
}

// Java: com/google/devtools/build/lib/skyframe/TargetLoadingUtil.java:107 TargetLoadingUtil.getContainingDirectory
fn get_containing_directory(label: &Label) -> PathFragment {
    let pkg = label.get_package_fragment();
    let name = label.get_name();
    if name == "." {
        pkg.clone()
    } else {
        pkg.get_relative_str(name)
            .get_parent_directory()
            .expect("a label path has a parent")
    }
}

/// Loads the target of `label`; `Ok(None)` if a dependency is missing.
// Java: com/google/devtools/build/lib/skyframe/TargetLoadingUtil.java:43 TargetLoadingUtil.loadTarget
pub fn load_target(
    env: &mut Environment<'_>,
    label: &Label,
) -> Result<Result<Option<TargetAndErrorIfAny>, LoadTargetError>, blaze_skyframe::Interrupted> {
    if label.get_name().contains('/') {
        // This target is in a subdirectory, therefore it could potentially be invalidated by a
        // new BUILD file appearing in the hierarchy.
        let containing_directory = get_containing_directory(label);
        let new_pkg_id =
            PackageIdentifier::create(label.get_repository().clone(), containing_directory);
        let containing_package_key = ContainingPackageLookupKey::key(new_pkg_id);
        enum Caught {
            BuildFileNotFound(NoSuchPackageException),
            InconsistentFilesystem(IOException),
        }
        let containing_package_lookup_value =
            match env.get_value_or_throw_with(containing_package_key, |e| {
                if let Some(e) = e
                    .downcast_ref::<NoSuchPackageException>()
                    .filter(|e| e.kind() == NoSuchPackageKind::BuildFileNotFound)
                {
                    Some(Caught::BuildFileNotFound(e.clone()))
                } else {
                    e.downcast_ref::<IOException>()
                        .filter(|e| e.is_inconsistent_filesystem())
                        .map(|e| Caught::InconsistentFilesystem(e.clone()))
                }
            })? {
                Err(Caught::BuildFileNotFound(e)) => {
                    return Ok(Err(LoadTargetError::NoSuchPackage(e)));
                }
                Err(Caught::InconsistentFilesystem(e)) => {
                    return Ok(Err(LoadTargetError::NoSuchTarget(
                        NoSuchTargetException::with_label(label.clone(), e.get_message()),
                    )));
                }
                Ok(None) => return Ok(Ok(None)),
                Ok(Some(v)) => v,
            };
        if !containing_package_lookup_value.has_containing_package() {
            // This means the label's package doesn't exist. E.g. there is no package 'a' and we
            // are trying to build the target for label 'a:b/foo'.
            return Ok(Err(LoadTargetError::NoSuchPackage(
                NoSuchPackageException::build_file_not_found(
                    label.get_package_identifier().clone(),
                    format!(
                        "BUILD file not found on package path for '{}'",
                        label.get_package_fragment().get_path_string()
                    ),
                ),
            )));
        }
        if containing_package_lookup_value.get_containing_package_name()
            != label.get_package_identifier()
        {
            return Ok(Err(LoadTargetError::NoSuchTarget(
                NoSuchTargetException::with_label(
                    label.clone(),
                    &format!(
                        "Label '{}' crosses boundary of subpackage '{}'",
                        label,
                        containing_package_lookup_value.get_containing_package_name()
                    ),
                ),
            )));
        }
    }

    let package_value = match env.get_value_or_throw::<PackageKey, NoSuchPackageException>(
        PackageKey::key(label.get_package_identifier().clone()),
    )? {
        Err(e) => return Ok(Err(LoadTargetError::NoSuchPackage(e))),
        Ok(None) => return Ok(Ok(None)),
        Ok(Some(v)) => v,
    };
    let pkg = package_value.get_package();
    let target = match pkg.get_target(label.get_name()) {
        Ok(t) => t.clone(),
        Err(e) => return Ok(Err(LoadTargetError::NoSuchTarget(e))),
    };
    let error = pkg
        .contains_errors()
        .then(|| NoSuchTargetException::for_target_in_error(target.get_label().clone()));
    Ok(Ok(Some(TargetAndErrorIfAny {
        package_loaded_successfully: !pkg.contains_errors(),
        error_loading_target: error,
        target,
        package: pkg.clone(),
    })))
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveTargetFunction.java:142 TransitiveTargetFunction.maybeReportErrorAboutMissingEdge
fn maybe_report_error_about_missing_edge(
    target: &Target,
    dep_label: &Label,
    e: &NoSuchThingException,
    event_handler: &dyn EventHandler,
) {
    let report = match e {
        NoSuchThingException::Target(nste) => nste.get_label() == Some(dep_label),
        NoSuchThingException::Package(nspe) => {
            nspe.get_package_id() == dep_label.get_package_identifier()
        }
        _ => false,
    };
    if report {
        event_handler.handle(&Event::error_at(
            target_utils::get_location_maybe(target)
                .map(|l| crate::starlark_util::event_location(&l)),
            target_utils::format_missing_edge(Some(target), dep_label, e),
        ));
    }
}

/// TRANSITIVE_TARGET (Java `TransitiveTargetFunction` over `TransitiveBaseTraversalFunction`).
// Java: com/google/devtools/build/lib/skyframe/TransitiveBaseTraversalFunction.java:91 TransitiveBaseTraversalFunction.compute
pub fn transitive_target_function(
    key: &TransitiveTargetKey,
    env: &mut Environment<'_>,
) -> Result<Option<TransitiveTargetValue>, ComputeError> {
    let label = key.get_label();
    let target_and_error_if_any = match load_target(env, label)? {
        Err(LoadTargetError::NoSuchTarget(e)) => {
            return Err(SkyFunctionException::persistent(e).into());
        }
        Err(LoadTargetError::NoSuchPackage(e)) => {
            return Err(SkyFunctionException::persistent(e).into());
        }
        Ok(None) => return Ok(None),
        Ok(Some(t)) => t,
    };

    // Process deps from attributes.
    let label_dep_keys = get_label_dep_keys(&target_and_error_if_any);
    let sky_keys: Vec<SkyKey> = label_dep_keys.iter().cloned().map(SkyKey::new).collect();
    let dep_map = env.get_values_and_exceptions(&sky_keys)?;
    if env.values_missing() {
        return Ok(None);
    }

    // Process deps from aspects.
    // PORT: Java also visits the label deps of the aspects of the rule's attributes
    // (getStrictLabelAspectDepKeys: Attribute.getAspects, AspectDefinition.satisfies,
    // forEachLabelDepFromAllAttributesOfAspect); those APIs are not ported in blaze_packages
    // yet, so aspect deps are not preloaded.

    let target = &target_and_error_if_any.target;
    let mut transitive: Vec<Arc<LabelNestedSet>> = Vec::new();
    let mut successful_transitive_loading = target_and_error_if_any.package_loaded_successfully;
    process_deps(
        target,
        env.listener(),
        &dep_map,
        &label_dep_keys,
        &mut transitive,
        &mut successful_transitive_loading,
    );

    // Java's getValuesAndExceptions marks missing deps eagerly (checked above); Blaze's lookup
    // result marks them when read, so check again after reading every entry.
    if env.values_missing() {
        return Ok(None);
    }

    // Java TransitiveTargetValueBuilder.build.
    let loaded_targets = Arc::new(LabelNestedSet {
        direct: target.get_label().clone(),
        transitive,
    });
    Ok(Some(if successful_transitive_loading {
        TransitiveTargetValue {
            transitive_targets: loaded_targets,
            error_loading_target: None,
            encountered_loading_error: false,
        }
    } else {
        TransitiveTargetValue {
            transitive_targets: loaded_targets,
            error_loading_target: target_and_error_if_any.error_loading_target,
            encountered_loading_error: true,
        }
    }))
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveBaseTraversalFunction.java:142 TransitiveBaseTraversalFunction.getLabelDepKeys
fn get_label_dep_keys(target_and_error_if_any: &TargetAndErrorIfAny) -> Vec<TransitiveTargetKey> {
    let mut deps: IndexSet<TransitiveTargetKey> = IndexSet::new();
    label_visitation_utils::visit_target(
        &target_and_error_if_any.target,
        &DependencyFilter::no_nodep_attributes_except_visibility(),
        &mut |_from_target, _attribute, to_label| {
            deps.insert(TransitiveTargetKey::of(to_label.clone()));
        },
    );
    deps.into_iter().collect()
}

// Java: com/google/devtools/build/lib/skyframe/TransitiveTargetFunction.java:58 TransitiveTargetFunction.processDeps
fn process_deps(
    target: &Target,
    event_handler: &dyn EventHandler,
    dep_entries: &SkyframeLookupResult,
    dep_keys: &[TransitiveTargetKey],
    transitive: &mut Vec<Arc<LabelNestedSet>>,
    successful_transitive_loading: &mut bool,
) {
    for sky_key in dep_keys {
        let dep_label = sky_key.get_label();
        let transitive_target_value = match dep_entries.get_or_throw_with(sky_key, |e| {
            if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
                Some(NoSuchThingException::Package(e.clone()))
            } else {
                e.downcast_ref::<NoSuchTargetException>()
                    .map(|e| NoSuchThingException::Target(e.clone()))
            }
        }) {
            Err(e) => {
                *successful_transitive_loading = false;
                maybe_report_error_about_missing_edge(target, dep_label, &e, event_handler);
                continue;
            }
            Ok(v) => v,
        };
        let Some(transitive_target_value) = transitive_target_value else {
            // Java: BugReport.sendNonFatalBugReport("TransitiveTargetValue ... was missing").
            continue;
        };
        transitive.push(transitive_target_value.get_transitive_targets().clone());
        if transitive_target_value.encountered_loading_error() {
            *successful_transitive_loading = false;
            if let Some(e) = transitive_target_value.get_error_loading_target() {
                maybe_report_error_about_missing_edge(
                    target,
                    dep_label,
                    &NoSuchThingException::Target(e.clone()),
                    event_handler,
                );
            }
        }
    }
}
