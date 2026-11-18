//! CONTAINING_PACKAGE_LOOKUP: the package that owns a directory.
//!
//! Java: com/google/devtools/build/lib/skyframe/{ContainingPackageLookupFunction,
//! ContainingPackageLookupValue}.java.

use std::fmt;

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_skyframe::{ComputeError, Environment, Key, SkyFunctionName};
use blaze_vfs::{PathFragment, Root};

use crate::package_lookup::{ErrorReason, PackageLookupKey, PackageLookupValue};
use crate::sky_functions;

/// A value that represents the result of looking for the existence of a package that owns a
/// specific directory path. Compare with [`PackageLookupValue`], which deals with existence of a
/// specific package.
// Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:36 ContainingPackageLookupValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ContainingPackageLookupValue {
    /// Value indicating there is no containing package; `reason` is why (usually the reason the
    /// outer-most directory isn't a package). `None` for Java's `NONE`.
    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:155 ContainingPackageLookupValue.NoContainingPackage
    NoContainingPackage { reason: Option<String> },
    /// A successful lookup value.
    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:196 ContainingPackageLookupValue.ContainingPackage
    ContainingPackage {
        containing_package: PackageIdentifier,
        containing_package_root: Root,
    },
}

impl ContainingPackageLookupValue {
    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:38 ContainingPackageLookupValue.NONE
    pub const NONE: ContainingPackageLookupValue =
        ContainingPackageLookupValue::NoContainingPackage { reason: None };

    /// Returns whether there is a containing package.
    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:41 ContainingPackageLookupValue.hasContainingPackage
    pub fn has_containing_package(&self) -> bool {
        matches!(self, ContainingPackageLookupValue::ContainingPackage { .. })
    }

    /// If there is a containing package, returns its name.
    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:44 ContainingPackageLookupValue.getContainingPackageName
    pub fn get_containing_package_name(&self) -> &PackageIdentifier {
        match self {
            ContainingPackageLookupValue::ContainingPackage {
                containing_package, ..
            } => containing_package,
            _ => panic!("IllegalStateException"),
        }
    }

    /// If there is a containing package, returns its package root.
    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:47 ContainingPackageLookupValue.getContainingPackageRoot
    pub fn get_containing_package_root(&self) -> &Root {
        match self {
            ContainingPackageLookupValue::ContainingPackage {
                containing_package_root,
                ..
            } => containing_package_root,
            _ => panic!("IllegalStateException"),
        }
    }

    /// If there is not a containing package, returns a reason why (this is usually the reason the
    /// outer-most directory isn't a package).
    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:53 ContainingPackageLookupValue.getReasonForNoContainingPackage
    pub fn get_reason_for_no_containing_package(&self) -> Option<&str> {
        match self {
            ContainingPackageLookupValue::NoContainingPackage { reason } => reason.as_deref(),
            _ => panic!("IllegalStateException"),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:62 ContainingPackageLookupValue.getErrorMessageForLabelCrossingPackageBoundary
    pub fn get_error_message_for_label_crossing_package_boundary(
        pkg_root: &Root,
        label: &Label,
        containing_pkg_lookup_value: &ContainingPackageLookupValue,
    ) -> String {
        let containing_pkg = containing_pkg_lookup_value.get_containing_package_name();
        let crosses_package_boundary_below = containing_pkg
            .get_source_root()
            .starts_with(label.get_package_identifier().get_source_root());
        let label_name_fragment = PathFragment::create(label.get_name());
        let mut message = if crosses_package_boundary_below {
            format!(
                "Label '{}' is invalid because '{}' is a subpackage",
                label, containing_pkg
            )
        } else {
            format!(
                "Label '{}' is invalid because '{}' is not a package",
                label,
                label.get_package_name()
            )
        };

        let containing_root = containing_pkg_lookup_value.get_containing_package_root();
        if pkg_root == containing_root {
            let containing_pkg_fragment = containing_pkg.get_package_fragment();
            let label_name_in_containing_package = if crosses_package_boundary_below {
                label_name_fragment.sub_fragment(
                    containing_pkg_fragment.segment_count()
                        - label.get_package_fragment().segment_count(),
                    label_name_fragment.segment_count(),
                )
            } else {
                label
                    .to_path_fragment()
                    .relative_to(containing_pkg_fragment)
            };
            message.push_str("; perhaps you meant to put the colon here: '");
            if containing_pkg.get_repository().is_main() {
                message.push_str("//");
            }
            message.push_str(&format!(
                "{}:{}'?",
                containing_pkg, label_name_in_containing_package
            ));
        } else {
            message.push_str(&format!(
                "; have you deleted {}/BUILD? If so, use the --deleted_packages={} option",
                containing_pkg, containing_pkg
            ));
        }
        message
    }

    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:143 ContainingPackageLookupValue.withContainingPackage
    pub fn with_containing_package(pkg_id: PackageIdentifier, root: Root) -> Self {
        ContainingPackageLookupValue::ContainingPackage {
            containing_package: pkg_id,
            containing_package_root: root,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:147 ContainingPackageLookupValue.noContainingPackage
    pub fn no_containing_package(reason: impl Into<String>) -> Self {
        ContainingPackageLookupValue::NoContainingPackage {
            reason: Some(reason.into()),
        }
    }
}

/// SkyKey for [`ContainingPackageLookupValue`]. `Debug` prints `CONTAINING_PACKAGE_LOOKUP:<pkg>`.
// Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:117 ContainingPackageLookupValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ContainingPackageLookupKey(pub PackageIdentifier);

impl ContainingPackageLookupKey {
    // Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:57 ContainingPackageLookupValue.key
    pub fn key(id: PackageIdentifier) -> Self {
        blaze_util::check_argument!(!id.get_package_fragment().is_absolute(), "{}", id);
        ContainingPackageLookupKey(id)
    }
}

impl fmt::Debug for ContainingPackageLookupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

impl Key for ContainingPackageLookupKey {
    type Value = ContainingPackageLookupValue;
    const FUNCTION: SkyFunctionName = sky_functions::CONTAINING_PACKAGE_LOOKUP;
}

/// SkyFunction for [`ContainingPackageLookupValue`]s.
// Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupFunction.java:31 ContainingPackageLookupFunction.compute
pub fn containing_package_lookup_function(
    sky_key: &ContainingPackageLookupKey,
    env: &mut Environment<'_>,
) -> Result<Option<ContainingPackageLookupValue>, ComputeError> {
    let dir = &sky_key.0;
    let Some(pkg_lookup_value) = env.get_value(PackageLookupKey::key(dir.clone()))? else {
        return Ok(None);
    };

    if pkg_lookup_value.package_exists() {
        return Ok(Some(ContainingPackageLookupValue::with_containing_package(
            dir.clone(),
            pkg_lookup_value.get_root().clone(),
        )));
    }

    // Does the requested package cross into a sub-repository, which we should report via the
    // correct package identifier?
    if let PackageLookupValue::IncorrectRepositoryReference {
        corrected_package_identifier,
        ..
    } = &*pkg_lookup_value
    {
        return Ok(env
            .get_value(ContainingPackageLookupKey::key(
                corrected_package_identifier.clone(),
            ))?
            .map(|v| (*v).clone()));
    }

    if pkg_lookup_value.get_error_reason() == ErrorReason::RepositoryNotFound {
        return Ok(Some(ContainingPackageLookupValue::no_containing_package(
            pkg_lookup_value.get_error_msg(),
        )));
    }
    let Some(parent_dir) = dir.get_package_fragment().get_parent_directory() else {
        return Ok(Some(ContainingPackageLookupValue::NONE));
    };
    let parent_id = PackageIdentifier::create(dir.get_repository().clone(), parent_dir);
    Ok(env
        .get_value(ContainingPackageLookupKey::key(parent_id))?
        .map(|v| (*v).clone()))
}
