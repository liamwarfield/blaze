//! Port of `packages/PackageSpecification.java` (with `PackageGroupContents`).

use std::fmt;

use blaze_cmdline::EvalException;
use blaze_cmdline::Label;
use blaze_cmdline::LabelRepoContextImpl;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use indexmap::IndexSet;

// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:49 PackageSpecification.PUBLIC_VISIBILITY
const PUBLIC_VISIBILITY: &str = "public";
// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:50 PackageSpecification.PRIVATE_VISIBILITY
const PRIVATE_VISIBILITY: &str = "private";
// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:51 PackageSpecification.ALL_BENEATH_SUFFIX
const ALL_BENEATH_SUFFIX: &str = "/...";
// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:52 PackageSpecification.NEGATIVE_PREFIX
const NEGATIVE_PREFIX: &str = "-";

// Used for interpreting `visibility` labels.
// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:55 PackageSpecification.PACKAGE_LABEL
const PACKAGE_LABEL: &str = "__pkg__";
// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:56 PackageSpecification.SUBTREE_LABEL
const SUBTREE_LABEL: &str = "__subpackages__";

/// An invalid package specification (Java
/// `PackageSpecification.InvalidPackageSpecificationException`).
// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:445 PackageSpecification.InvalidPackageSpecificationException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidPackageSpecificationException(pub String);

/// A set of packages named in a `package_group` or visibility (Java `PackageSpecification` and
/// its subclasses).
// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:48 PackageSpecification
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PackageSpecification {
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:288 PackageSpecification.SinglePackage
    SinglePackage(PackageIdentifier),
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:323 PackageSpecification.AllPackagesBeneath
    AllPackagesBeneath(PackageIdentifier),
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:360 PackageSpecification.NegativePackageSpecification
    Negative(Box<PackageSpecification>),
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:393 PackageSpecification.AllPackages
    AllPackages,
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:419 PackageSpecification.NoPackages
    NoPackages,
}

impl PackageSpecification {
    /// Returns true if the package spec includes the provided `package_name`.
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:59 PackageSpecification.containsPackage
    pub fn contains_package(&self, package_name: &PackageIdentifier) -> bool {
        match self {
            // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:296 PackageSpecification.SinglePackage.containsPackage
            PackageSpecification::SinglePackage(single_package_name) => {
                single_package_name == package_name
            }
            // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:331 PackageSpecification.AllPackagesBeneath.containsPackage
            PackageSpecification::AllPackagesBeneath(prefix) => {
                package_name.get_repository() == prefix.get_repository()
                    && package_name
                        .get_package_fragment()
                        .starts_with(prefix.get_package_fragment())
            }
            // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:368 PackageSpecification.NegativePackageSpecification.containsPackage
            PackageSpecification::Negative(delegate) => delegate.contains_package(package_name),
            // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:398 PackageSpecification.AllPackages.containsPackage
            PackageSpecification::AllPackages => true,
            // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:424 PackageSpecification.NoPackages.containsPackage
            PackageSpecification::NoPackages => false,
        }
    }

    /// The string form; see Java's documentation of `includeDoubleSlash`.
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:76 PackageSpecification.asString
    pub fn as_string(&self, include_double_slash: bool) -> String {
        match self {
            // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:301 PackageSpecification.SinglePackage.asString
            PackageSpecification::SinglePackage(single_package_name) => {
                string_for_single_package(single_package_name, include_double_slash)
            }
            // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:337 PackageSpecification.AllPackagesBeneath.asString
            PackageSpecification::AllPackagesBeneath(prefix) => {
                string_for_all_packages_beneath(prefix, include_double_slash)
            }
            // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:373 PackageSpecification.NegativePackageSpecification.asString
            PackageSpecification::Negative(delegate) => {
                format!("-{}", delegate.as_string(include_double_slash))
            }
            // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:403 PackageSpecification.AllPackages.asString
            PackageSpecification::AllPackages => string_for_all_packages(include_double_slash),
            // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:429 PackageSpecification.NoPackages.asString
            PackageSpecification::NoPackages => "private".to_owned(),
        }
    }

    /// Parses a package specification string (`//foo`, `//foo/...`, `-//foo`, `public`, ...).
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:124 PackageSpecification.fromString
    pub fn from_string(
        repository_mapping: &RepositoryMapping,
        repository_name: &RepositoryName,
        spec: &str,
        allow_public_private: bool,
        repo_root_means_current_repo: bool,
    ) -> Result<PackageSpecification, InvalidPackageSpecificationException> {
        if repo_root_means_current_repo && !allow_public_private {
            return Err(InvalidPackageSpecificationException(
                "Cannot use new \"//...\" meaning without allowing new \"public\" syntax. Try \
                 enabling --incompatible_package_group_has_public_syntax or disabling \
                 --incompatible_fix_package_group_reporoot_syntax."
                    .to_owned(),
            ));
        }
        if !allow_public_private && (spec == PUBLIC_VISIBILITY || spec == PRIVATE_VISIBILITY) {
            return Err(InvalidPackageSpecificationException(format!(
                "Use of \"{spec}\" package specification requires enabling \
                 --incompatible_package_group_has_public_syntax"
            )));
        }
        let mut spec = spec;
        let mut negative = false;
        if let Some(rest) = spec.strip_prefix(NEGATIVE_PREFIX) {
            negative = true;
            spec = rest;
            if spec == PUBLIC_VISIBILITY || spec == PRIVATE_VISIBILITY {
                return Err(InvalidPackageSpecificationException(format!(
                    "Cannot negate \"{spec}\" package specification"
                )));
            }
        }
        let package_specification = Self::from_string_positive(
            repository_mapping,
            repository_name,
            spec,
            repo_root_means_current_repo,
        )?;
        Ok(if negative {
            PackageSpecification::Negative(Box::new(package_specification))
        } else {
            package_specification
        })
    }

    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:159 PackageSpecification.fromStringPositive
    fn from_string_positive(
        repository_mapping: &RepositoryMapping,
        repository_name: &RepositoryName,
        spec: &str,
        repo_root_means_current_repo: bool,
    ) -> Result<PackageSpecification, InvalidPackageSpecificationException> {
        if spec == PUBLIC_VISIBILITY {
            return Ok(PackageSpecification::AllPackages);
        } else if spec == PRIVATE_VISIBILITY {
            return Ok(PackageSpecification::NoPackages);
        }

        if !spec.starts_with("//") && !spec.starts_with('@') {
            return Err(InvalidPackageSpecificationException(format!(
                "invalid package name '{spec}': must start with '//', '@', or be 'public' or \
                 'private'"
            )));
        }

        let repo_context =
            LabelRepoContextImpl::of(repository_name.clone(), repository_mapping.clone());
        // Fall through to parse as a package path (e.g. //foo/...) on a LabelSyntaxException.
        if let Ok(label) = Label::parse_with_repo_context(spec, &repo_context)
            && let Some(mapped_spec) = Self::from_label(&label)
        {
            return Ok(mapped_spec);
        }

        let mut pkg_path: String;
        let mut all_beneath = false;
        if let Some(stripped) = spec.strip_suffix(ALL_BENEATH_SUFFIX) {
            all_beneath = true;
            pkg_path = stripped.to_owned();
            if pkg_path == "/" {
                // spec was "//...".
                if repo_root_means_current_repo {
                    pkg_path = "//".to_owned();
                } else {
                    // Legacy behavior: //... is "public".
                    return Ok(PackageSpecification::AllPackages);
                }
            } else if spec.ends_with("//...") {
                // spec was "@repo//..."
                pkg_path.push('/');
            }
        } else {
            pkg_path = spec.to_owned();
        }

        let pkg_id =
            match Label::parse_with_repo_context(&format!("{pkg_path}:__pkg__"), &repo_context) {
                Ok(label) => label.get_package_identifier().clone(),
                Err(e) => {
                    return Err(InvalidPackageSpecificationException(format!(
                        "invalid package name '{spec}': {}",
                        e.get_message()
                    )));
                }
            };
        Ok(if all_beneath {
            PackageSpecification::AllPackagesBeneath(pkg_id)
        } else {
            PackageSpecification::SinglePackage(pkg_id)
        })
    }

    /// Parses a string to a `PackageSpecification` for use with .bzl load visibility.
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:233 PackageSpecification.fromStringForBzlVisibility
    pub fn from_string_for_bzl_visibility(
        repository_mapping: &RepositoryMapping,
        repository_name: &RepositoryName,
        spec: &str,
    ) -> Result<PackageSpecification, EvalException> {
        let result = match Self::from_string(
            repository_mapping,
            repository_name,
            spec,
            /* allow_public_private= */ true,
            /* repo_root_means_current_repo= */ true,
        ) {
            Ok(r) => r,
            Err(e) => return Err(EvalException::new(e.0)),
        };
        if matches!(result, PackageSpecification::Negative(_)) {
            return Err(EvalException::new(
                "Cannot use negative package patterns here",
            ));
        }
        Ok(result)
    }

    /// The specification a visibility label denotes (`__pkg__`, `__subpackages__`), if any.
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:270 PackageSpecification.fromLabel
    pub fn from_label(label: &Label) -> Option<PackageSpecification> {
        if label.get_name() == PACKAGE_LABEL {
            Some(PackageSpecification::SinglePackage(
                label.get_package_identifier().clone(),
            ))
        } else if label.get_name() == SUBTREE_LABEL {
            Some(PackageSpecification::AllPackagesBeneath(
                label.get_package_identifier().clone(),
            ))
        } else {
            None
        }
    }

    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:280 PackageSpecification.everything
    pub fn everything() -> PackageSpecification {
        PackageSpecification::AllPackages
    }

    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:284 PackageSpecification.nothing
    pub fn nothing() -> PackageSpecification {
        PackageSpecification::NoPackages
    }
}

impl fmt::Display for PackageSpecification {
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:79 PackageSpecification.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_string(false))
    }
}

/// The unrolled contents of a list of package specifications (Java
/// `PackageSpecification.PackageGroupContents`).
// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:464 PackageSpecification.PackageGroupContents
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageGroupContents {
    single_package_positives: IndexSet<PackageIdentifier>,
    all_packages_beneath_positives: Vec<PackageIdentifier>,
    has_positive_all_packages: bool,
    single_package_negatives: IndexSet<PackageIdentifier>,
    all_packages_beneath_negatives: Vec<PackageIdentifier>,
    has_private: bool,
    has_negative_all_packages: bool,
}

impl PackageGroupContents {
    /// Creates a `PackageGroupContents` representing a collection of package specifications.
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:496 PackageSpecification.PackageGroupContents.create
    pub fn create(package_specifications: &[PackageSpecification]) -> PackageGroupContents {
        let mut single_package_positives = IndexSet::new();
        let mut all_packages_beneath_positives = Vec::new();
        let mut has_positive_all_packages = false;
        let mut single_package_negatives = IndexSet::new();
        let mut all_packages_beneath_negatives = Vec::new();
        let mut has_private = false;
        let mut has_negative_all_packages = false;

        for spec in package_specifications {
            match spec {
                PackageSpecification::SinglePackage(p) => {
                    single_package_positives.insert(p.clone());
                }
                PackageSpecification::AllPackagesBeneath(p) => {
                    all_packages_beneath_positives.push(p.clone());
                }
                PackageSpecification::AllPackages => has_positive_all_packages = true,
                // We can't drop NoPackages because it still needs to be serialized, e.g. in
                // bazel query output.
                PackageSpecification::NoPackages => has_private = true,
                PackageSpecification::Negative(delegate) => match &**delegate {
                    PackageSpecification::SinglePackage(p) => {
                        single_package_negatives.insert(p.clone());
                    }
                    PackageSpecification::AllPackagesBeneath(p) => {
                        all_packages_beneath_negatives.push(p.clone());
                    }
                    PackageSpecification::AllPackages => has_negative_all_packages = true,
                    _ => panic!("IllegalStateException: {spec}"),
                },
            }
        }

        PackageGroupContents {
            single_package_positives,
            all_packages_beneath_positives,
            has_positive_all_packages,
            single_package_negatives,
            all_packages_beneath_negatives,
            has_private,
            has_negative_all_packages,
        }
    }

    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:478 PackageSpecification.PackageGroupContents.singlePackagePositives
    pub fn single_package_positives(&self) -> &IndexSet<PackageIdentifier> {
        &self.single_package_positives
    }

    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:480 PackageSpecification.PackageGroupContents.allPackagesBeneathPositives
    pub fn all_packages_beneath_positives(&self) -> &[PackageIdentifier] {
        &self.all_packages_beneath_positives
    }

    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:482 PackageSpecification.PackageGroupContents.hasPositiveAllPackages
    pub fn has_positive_all_packages(&self) -> bool {
        self.has_positive_all_packages
    }

    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:484 PackageSpecification.PackageGroupContents.singlePackageNegatives
    pub fn single_package_negatives(&self) -> &IndexSet<PackageIdentifier> {
        &self.single_package_negatives
    }

    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:486 PackageSpecification.PackageGroupContents.allPackagesBeneathNegatives
    pub fn all_packages_beneath_negatives(&self) -> &[PackageIdentifier] {
        &self.all_packages_beneath_negatives
    }

    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:488 PackageSpecification.PackageGroupContents.hasPrivate
    pub fn has_private(&self) -> bool {
        self.has_private
    }

    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:490 PackageSpecification.PackageGroupContents.hasNegativeAllPackages
    pub fn has_negative_all_packages(&self) -> bool {
        self.has_negative_all_packages
    }

    /// True if the package matches at least one positive specification and no negative one.
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:562 PackageSpecification.PackageGroupContents.containsPackage
    pub fn contains_package(&self, package_identifier: &PackageIdentifier) -> bool {
        if self.has_negative_all_packages {
            return false;
        }
        if self.single_package_negatives.contains(package_identifier) {
            return false;
        }
        for prefix in &self.all_packages_beneath_negatives {
            if matches_all_packages_beneath(package_identifier, prefix) {
                return false;
            }
        }
        if self.has_positive_all_packages {
            return true;
        }
        if self.single_package_positives.contains(package_identifier) {
            return true;
        }
        for prefix in &self.all_packages_beneath_positives {
            if matches_all_packages_beneath(package_identifier, prefix) {
                return true;
            }
        }
        false
    }

    /// The package strings of the contents (query output of `package_group`s).
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:610 PackageSpecification.PackageGroupContents.packageStrings
    pub fn package_strings(&self, include_double_slash: bool) -> Vec<String> {
        let mut result = Vec::new();
        for pkg_id in &self.single_package_positives {
            result.push(string_for_single_package(pkg_id, include_double_slash));
        }
        for pkg_id in &self.all_packages_beneath_positives {
            result.push(string_for_all_packages_beneath(
                pkg_id,
                include_double_slash,
            ));
        }
        for pkg_id in &self.single_package_negatives {
            result.push(format!(
                "-{}",
                string_for_single_package(pkg_id, include_double_slash)
            ));
        }
        for pkg_id in &self.all_packages_beneath_negatives {
            result.push(format!(
                "-{}",
                string_for_all_packages_beneath(pkg_id, include_double_slash)
            ));
        }
        if self.has_positive_all_packages {
            result.push(string_for_all_packages(include_double_slash));
        }
        if self.has_private {
            result.push("private".to_owned());
        }
        if self.has_negative_all_packages {
            result.push(format!(
                "-{}",
                string_for_all_packages(include_double_slash)
            ));
        }
        result
    }

    /// Like [`PackageGroupContents::package_strings`] with `//` and without the repository.
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:689 PackageSpecification.PackageGroupContents.packageStringsWithDoubleSlashAndWithoutRepository
    pub fn package_strings_with_double_slash_and_without_repository(&self) -> Vec<String> {
        let mut result = Vec::new();
        for pkg_id in &self.single_package_positives {
            result.push(string_for_single_package_with_double_slash_and_without_repository(pkg_id));
        }
        for pkg_id in &self.all_packages_beneath_positives {
            result.push(
                string_for_all_packages_beneath_with_double_slash_and_without_repository(pkg_id),
            );
        }
        for pkg_id in &self.single_package_negatives {
            result.push(format!(
                "-{}",
                string_for_single_package_with_double_slash_and_without_repository(pkg_id)
            ));
        }
        for pkg_id in &self.all_packages_beneath_negatives {
            result.push(format!(
                "-{}",
                string_for_all_packages_beneath_with_double_slash_and_without_repository(pkg_id)
            ));
        }
        if self.has_positive_all_packages {
            result.push("public".to_owned());
        }
        if self.has_private {
            result.push("private".to_owned());
        }
        result
    }
}

impl fmt::Display for PackageGroupContents {
    /// Java's AutoValue-generated `toString`.
    // Java: com/google/devtools/build/lib/packages/PackageSpecification.java:463 PackageSpecification.PackageGroupContents.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PackageGroupContents{{singlePackagePositives={}, allPackagesBeneathPositives={}, \
             hasPositiveAllPackages={}, singlePackageNegatives={}, \
             allPackagesBeneathNegatives={}, hasPrivate={}, hasNegativeAllPackages={}}}",
            java_list(self.single_package_positives.iter()),
            java_list(self.all_packages_beneath_positives.iter()),
            self.has_positive_all_packages,
            java_list(self.single_package_negatives.iter()),
            java_list(self.all_packages_beneath_negatives.iter()),
            self.has_private,
            self.has_negative_all_packages,
        )
    }
}

/// Java `AbstractCollection.toString`: `[a, b, c]`.
fn java_list<T: fmt::Display>(items: impl Iterator<Item = T>) -> String {
    let items: Vec<String> = items.map(|i| i.to_string()).collect();
    format!("[{}]", items.join(", "))
}

// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:593 PackageSpecification.PackageGroupContents.matchesAllPackagesBeneath
fn matches_all_packages_beneath(pkg_id: &PackageIdentifier, prefix: &PackageIdentifier) -> bool {
    pkg_id.get_repository() == prefix.get_repository()
        && pkg_id
            .get_package_fragment()
            .starts_with(prefix.get_package_fragment())
}

// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:636 PackageSpecification.PackageGroupContents.stringForAllPackages
fn string_for_all_packages(include_double_slash: bool) -> String {
    // Under legacy formatting rules, use legacy syntax. This avoids ambiguity between "public"
    // and "//public", and ensures that AllPackages is round-trippable when the value of
    // includeDoubleSlash matches allowPublicPrivate.
    if include_double_slash {
        "public".to_owned()
    } else {
        "//...".to_owned()
    }
}

// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:643 PackageSpecification.PackageGroupContents.stringForSinglePackage
fn string_for_single_package(pkg_id: &PackageIdentifier, include_double_slash: bool) -> String {
    if include_double_slash {
        pkg_id.get_canonical_form()
    } else {
        // PackageIdentifier#toString implements the legacy behavior of omitting the double slash
        // for the main repo.
        pkg_id.to_string()
    }
}

// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:654 PackageSpecification.PackageGroupContents.stringForAllPackagesBeneath
fn string_for_all_packages_beneath(
    pkg_id: &PackageIdentifier,
    include_double_slash: bool,
) -> String {
    if pkg_id.get_package_fragment().is_empty() {
        // Special case: Emit "//..." rather than suffixing "/...", which would yield "/...".
        // Make sure not to strip the repo in the case of "@repo//...".
        //
        // Note that "//..." is the desired result, not "...", even under the legacy behavior of
        // includeDoubleSlash=false.
        return format!("{}...", pkg_id.get_canonical_form());
    }
    if include_double_slash {
        format!("{}{}", pkg_id.get_canonical_form(), ALL_BENEATH_SUFFIX)
    } else {
        // PackageIdentifier#toString implements the legacy behavior of omitting the double slash
        // for the main repo.
        format!("{pkg_id}{ALL_BENEATH_SUFFIX}")
    }
}

// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:713 PackageSpecification.PackageGroupContents.stringForSinglePackageWithDoubleSlashAndWithoutRepository
fn string_for_single_package_with_double_slash_and_without_repository(
    pkg_id: &PackageIdentifier,
) -> String {
    format!("//{}", pkg_id.get_package_fragment().get_path_string())
}

// Java: com/google/devtools/build/lib/packages/PackageSpecification.java:718 PackageSpecification.PackageGroupContents.stringForAllPackagesBeneathWithDoubleSlashAndWithoutRepository
fn string_for_all_packages_beneath_with_double_slash_and_without_repository(
    pkg_id: &PackageIdentifier,
) -> String {
    let path_fragment = pkg_id.get_package_fragment();
    if path_fragment.is_empty() {
        "//...".to_owned()
    } else {
        format!(
            "//{}{}",
            path_fragment.get_path_string(),
            ALL_BENEATH_SUFFIX
        )
    }
}
