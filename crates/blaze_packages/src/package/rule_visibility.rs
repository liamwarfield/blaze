//! Ports `packages/RuleVisibility.java` and `packages/PackageGroupsRuleVisibility.java`.

use std::fmt;
use std::sync::Arc;
use std::sync::LazyLock;

use blaze_cmdline::EvalException;
use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;

use crate::target::PackageGroupContents;
use crate::target::PackageSpecification;

// Java: com/google/devtools/build/lib/packages/RuleVisibility.java:54 RuleVisibility.PUBLIC_LABEL
static PUBLIC_LABEL: LazyLock<Label> =
    LazyLock::new(|| Label::parse_canonical_unchecked("//visibility:public"));

// Java: com/google/devtools/build/lib/packages/RuleVisibility.java:57 RuleVisibility.PRIVATE_LABEL
static PRIVATE_LABEL: LazyLock<Label> =
    LazyLock::new(|| Label::parse_canonical_unchecked("//visibility:private"));

/// A target's visibility (Java `RuleVisibility`: the `PUBLIC` and `PRIVATE` constants and
/// `PackageGroupsRuleVisibility`).
// Java: com/google/devtools/build/lib/packages/RuleVisibility.java:35 RuleVisibility
#[derive(Debug, Clone, PartialEq)]
pub enum RuleVisibility {
    /// Java `RuleVisibility.PUBLIC` (`//visibility:public`).
    Public,
    /// Java `RuleVisibility.PRIVATE` (`//visibility:private`).
    Private,
    /// Java `PackageGroupsRuleVisibility`.
    PackageGroups(Arc<PackageGroupsRuleVisibility>),
}

impl RuleVisibility {
    /// Java `RuleVisibility.PUBLIC_LABEL`.
    pub fn public_label() -> Label {
        PUBLIC_LABEL.clone()
    }

    /// Java `RuleVisibility.PRIVATE_LABEL`.
    pub fn private_label() -> Label {
        PRIVATE_LABEL.clone()
    }

    /// Java `RuleVisibility.PUBLIC_DECLARED_LABELS`.
    // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:61 RuleVisibility.PUBLIC_DECLARED_LABELS
    pub fn public_declared_labels() -> Vec<Label> {
        vec![RuleVisibility::public_label()]
    }

    /// Java `RuleVisibility.PRIVATE_DECLARED_LABELS`.
    // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:65 RuleVisibility.PRIVATE_DECLARED_LABELS
    pub fn private_declared_labels() -> Vec<Label> {
        vec![RuleVisibility::private_label()]
    }

    /// The labels as declared (before simplification).
    // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:42 RuleVisibility.getDeclaredLabels
    pub fn get_declared_labels(&self) -> Vec<Label> {
        match self {
            // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:72 RuleVisibility.PUBLIC.getDeclaredLabels
            RuleVisibility::Public => RuleVisibility::public_declared_labels(),
            // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:91 RuleVisibility.PRIVATE.getDeclaredLabels
            RuleVisibility::Private => RuleVisibility::private_declared_labels(),
            RuleVisibility::PackageGroups(p) => p.get_declared_labels().to_vec(),
        }
    }

    /// The labels of `package_group` targets this visibility depends on.
    // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:51 RuleVisibility.getDependencyLabels
    pub fn get_dependency_labels(&self) -> Vec<Label> {
        match self {
            RuleVisibility::Public | RuleVisibility::Private => Vec::new(),
            RuleVisibility::PackageGroups(p) => p.get_dependency_labels().to_vec(),
        }
    }

    /// Java `this == RuleVisibility.PUBLIC`.
    pub fn is_public(&self) -> bool {
        matches!(self, RuleVisibility::Public)
    }

    /// Java `this == RuleVisibility.PRIVATE`.
    pub fn is_private(&self) -> bool {
        matches!(self, RuleVisibility::Private)
    }

    /// The `PackageGroupsRuleVisibility`, if this is one (Java `instanceof`).
    pub fn as_package_groups(&self) -> Option<&Arc<PackageGroupsRuleVisibility>> {
        match self {
            RuleVisibility::PackageGroups(p) => Some(p),
            _ => None,
        }
    }

    /// Validates and parses a visibility list.
    // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:107 RuleVisibility.parse
    pub fn parse(labels: &[Label]) -> Result<RuleVisibility, EvalException> {
        Ok(Self::parse_unchecked(&Self::validate_and_simplify(labels)?))
    }

    /// Like [`RuleVisibility::parse`] without validation or public/private simplification.
    // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:118 RuleVisibility.parseUnchecked
    pub fn parse_unchecked(labels: &[Label]) -> RuleVisibility {
        if let Some(result) = Self::parse_if_constant(labels) {
            return result;
        }
        RuleVisibility::PackageGroups(Arc::new(PackageGroupsRuleVisibility::create(labels)))
    }

    /// `Public` / `Private` for the constant lists, else `None`.
    // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:134 RuleVisibility.parseIfConstant
    pub fn parse_if_constant(labels: &[Label]) -> Option<RuleVisibility> {
        if labels.len() != 1 {
            return None;
        }
        let label = &labels[0];
        if *label == *PUBLIC_LABEL {
            return Some(RuleVisibility::Public);
        }
        if *label == *PRIVATE_LABEL {
            return Some(RuleVisibility::Private);
        }
        None
    }

    /// Fails if the label is in `//visibility` but is neither `__pkg__` nor `__subpackages__`.
    // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:162 RuleVisibility.checkForVisibilityMisspelling
    fn check_for_visibility_misspelling(label: &Label) -> Result<(), EvalException> {
        if label.get_package_identifier() == PUBLIC_LABEL.get_package_identifier()
            && PackageSpecification::from_label(label).is_none()
        {
            // Suggest just public/private, as that's way more common than //visibility:__pkg__ or
            // //visibility:__subpackages__.
            return Err(EvalException::new(format!(
                "Invalid visibility label '{label}'; did you mean //visibility:public or \
                 //visibility:private?"
            )));
        }
        Ok(())
    }

    /// Validates visibility labels, simplifies a list containing `//visibility:public` to it,
    /// drops `//visibility:private` among other labels, and canonicalizes `[]` to private.
    // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:185 RuleVisibility.validateAndSimplify
    pub fn validate_and_simplify(labels: &[Label]) -> Result<Vec<Label>, EvalException> {
        let mut has_public_label = false;
        let mut num_private_labels = 0usize;
        for label in labels {
            if *label == *PUBLIC_LABEL {
                // Do not short-circuit here; we want to validate all the labels.
                has_public_label = true;
            } else if *label == *PRIVATE_LABEL {
                num_private_labels += 1;
            } else {
                Self::check_for_visibility_misspelling(label)?;
            }
        }
        if has_public_label {
            return Ok(RuleVisibility::public_declared_labels());
        }
        if num_private_labels == labels.len() {
            return Ok(RuleVisibility::private_declared_labels());
        }
        if num_private_labels == 0 {
            return Ok(labels.to_vec());
        }
        Ok(labels
            .iter()
            .filter(|l| **l != *PRIVATE_LABEL)
            .cloned()
            .collect())
    }

    /// This visibility plus `package_identifier` (`//pkg:__pkg__`).
    // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:225 RuleVisibility.concatWithPackage
    pub fn concat_with_package(&self, package_identifier: &PackageIdentifier) -> RuleVisibility {
        let pkg_item = Label::create_unvalidated(package_identifier.clone(), "__pkg__");

        if *self == RuleVisibility::Private {
            // Left-side private is dropped.
            Self::parse_unchecked(&[pkg_item])
        } else if *self == RuleVisibility::Public {
            // Public is idempotent.
            RuleVisibility::Public
        } else {
            let items = self.get_declared_labels();
            if items.contains(&pkg_item) {
                self.clone()
            } else {
                let mut new_items = items;
                new_items.push(pkg_item);
                Self::parse_unchecked(&new_items)
            }
        }
    }
}

impl fmt::Display for RuleVisibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:82 RuleVisibility.PUBLIC.toString
            RuleVisibility::Public => write!(f, "{}", *PUBLIC_LABEL),
            // Java: com/google/devtools/build/lib/packages/RuleVisibility.java:101 RuleVisibility.PRIVATE.toString
            RuleVisibility::Private => write!(f, "{}", *PRIVATE_LABEL),
            RuleVisibility::PackageGroups(p) => fmt::Display::fmt(&**p, f),
        }
    }
}

/// A visibility given by package specifications and `package_group` labels (Java
/// `PackageGroupsRuleVisibility`).
// Java: com/google/devtools/build/lib/packages/PackageGroupsRuleVisibility.java:26 PackageGroupsRuleVisibility
#[derive(Debug, Clone, PartialEq)]
pub struct PackageGroupsRuleVisibility {
    package_groups: Vec<Label>,
    direct_packages: PackageGroupContents,
    declared_labels: Vec<Label>,
}

impl PackageGroupsRuleVisibility {
    /// Creates a visibility from a non-empty list of validated and simplified labels that is not
    /// `["//visibility:public"]` or `["//visibility:private"]`.
    // Java: com/google/devtools/build/lib/packages/PackageGroupsRuleVisibility.java:41 PackageGroupsRuleVisibility.create
    pub fn create(labels: &[Label]) -> PackageGroupsRuleVisibility {
        let mut direct_packages = Vec::new();
        let mut package_groups = Vec::new();

        blaze_util::check_argument!(!labels.is_empty(), "labels must not be empty");
        for label in labels {
            match PackageSpecification::from_label(label) {
                Some(specification) => direct_packages.push(specification),
                None => {
                    blaze_util::check_argument!(
                        *label != *PUBLIC_LABEL && *label != *PRIVATE_LABEL,
                        "labels list {} must {}",
                        java_label_list(labels),
                        if labels.len() == 1 {
                            "not equal [\"//visibility:public\"] or [\"//visibility:private\"]"
                        } else {
                            "be validated and simplified"
                        }
                    );
                    package_groups.push(label.clone());
                }
            }
        }

        PackageGroupsRuleVisibility {
            package_groups,
            direct_packages: PackageGroupContents::create(&direct_packages),
            declared_labels: labels.to_vec(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroupsRuleVisibility.java:27 PackageGroupsRuleVisibility.getPackageGroups
    pub fn get_package_groups(&self) -> &[Label] {
        &self.package_groups
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroupsRuleVisibility.java:29 PackageGroupsRuleVisibility.getDirectPackages
    pub fn get_direct_packages(&self) -> &PackageGroupContents {
        &self.direct_packages
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroupsRuleVisibility.java:32 PackageGroupsRuleVisibility.getDeclaredLabels
    pub fn get_declared_labels(&self) -> &[Label] {
        &self.declared_labels
    }

    // Java: com/google/devtools/build/lib/packages/PackageGroupsRuleVisibility.java:70 PackageGroupsRuleVisibility.getDependencyLabels
    pub fn get_dependency_labels(&self) -> &[Label] {
        &self.package_groups
    }
}

impl fmt::Display for PackageGroupsRuleVisibility {
    /// Java's AutoValue-generated `toString`.
    // Java: com/google/devtools/build/lib/packages/PackageGroupsRuleVisibility.java:25 PackageGroupsRuleVisibility.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PackageGroupsRuleVisibility{{packageGroups={}, directPackages={}, declaredLabels={}}}",
            java_label_list(&self.package_groups),
            self.direct_packages,
            java_label_list(&self.declared_labels),
        )
    }
}

/// Java `List.toString` of labels: `[//a:b, //c:d]`.
fn java_label_list(labels: &[Label]) -> String {
    let items: Vec<String> = labels.iter().map(|l| l.to_string()).collect();
    format!("[{}]", items.join(", "))
}
