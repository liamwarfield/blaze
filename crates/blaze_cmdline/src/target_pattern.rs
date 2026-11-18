//! Port of `TargetPattern` and `TargetPattern.Parser`.
//!
//! Evaluation (`TargetPattern.eval`, `evalAsync`, `TargetsInPackage.getWildcardConflict`,
//! `TargetsBelowDirectory.eval`) needs `TargetPatternResolver`, `IgnoredSubdirectories` and
//! `BatchCallback`, which are not ported yet; only parsing and the pure queries are ported.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use blaze_vfs::PathFragment;

use crate::label::Label;
use crate::label_parser::LabelParserParts;
use crate::package_identifier::PackageIdentifier;
use crate::repository_mapping::{EMPTY, RepositoryMapping};
use crate::repository_name::{MAIN, RepositoryName};
use crate::target_parsing_exception::{TargetParsingException, TargetPatternsCode};

/// The target pattern type (targets below package, in package, explicit target, etc.).
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:940 TargetPattern.Type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetPatternType {
    /// A path interpreted as a target, eg "foo/bar/baz"
    PathAsTarget,
    /// An explicit target, eg "//foo:bar."
    SingleTarget,
    /// Targets below a directory, eg "foo/...".
    TargetsBelowDirectory,
    /// Target in a package, eg "foo:all".
    TargetsInPackage,
}

/// A target pattern: a generalization of labels to include wildcards for all packages beneath a
/// directory and all targets within a package (Java `TargetPattern`, a closed class hierarchy).
///
/// Negative patterns ("-//foo/bar") are handled one level up, by
/// [`crate::SignedTargetPattern`]. `Display` is Java's `toString()`
/// (`MoreObjects.toStringHelper` format).
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:61 TargetPattern
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TargetPattern {
    SingleTarget(TargetPatternSingleTarget),
    InterpretPathAsTarget(TargetPatternInterpretPathAsTarget),
    TargetsInPackage(TargetPatternTargetsInPackage),
    TargetsBelowDirectory(TargetPatternTargetsBelowDirectory),
}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:66 TargetPattern.DEFAULT_PARSER
static DEFAULT_PARSER: LazyLock<TargetPatternParser> =
    LazyLock::new(|| TargetPattern::main_repo_parser(PathFragment::empty_fragment()));

impl TargetPattern {
    /// A parser defaulting to the main repo, with no offset or repo mapping.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:74 TargetPattern.defaultParser
    pub fn default_parser() -> &'static TargetPatternParser {
        &DEFAULT_PARSER
    }

    /// A parser defaulting to the main repo, with an empty repo mapping, using the given offset.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:84 TargetPattern.mainRepoParser
    pub fn main_repo_parser(offset: PathFragment) -> TargetPatternParser {
        TargetPatternParser::new(offset, MAIN.clone(), EMPTY.clone())
    }

    /// Normalizes a relative path by resolving `//`, `/./` and `x/../`. Leading ".." segments are
    /// kept. Panics (Java `IllegalArgumentException`) if the path starts with "/" or "@".
    /// Visible for testing.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:96 TargetPattern.normalize
    pub fn normalize(path: &str) -> String {
        assert!(!path.starts_with('/'), "{}", path);
        assert!(!path.starts_with('@'), "{}", path);
        let mut pieces: Vec<&str> = Vec::new();
        for piece in path.split('/') {
            if piece == "." || piece.is_empty() {
                continue;
            }
            if piece == ".." {
                if pieces.is_empty() {
                    pieces.push(piece);
                    continue;
                }
                let predecessor = pieces.pop().expect("non-empty");
                if predecessor == ".." {
                    pieces.push(piece);
                    pieces.push(piece);
                }
                continue;
            }
            pieces.push(piece);
        }
        pieces.join("/")
    }

    /// The type of the pattern.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:132 TargetPattern.getType
    pub fn get_type(&self) -> TargetPatternType {
        match self {
            TargetPattern::SingleTarget(_) => TargetPatternType::SingleTarget,
            TargetPattern::InterpretPathAsTarget(_) => TargetPatternType::PathAsTarget,
            TargetPattern::TargetsInPackage(_) => TargetPatternType::TargetsInPackage,
            TargetPattern::TargetsBelowDirectory(_) => TargetPatternType::TargetsBelowDirectory,
        }
    }

    /// The string that was parsed into this pattern.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:135 TargetPattern.getOriginalPattern
    pub fn get_original_pattern(&self) -> &str {
        match self {
            TargetPattern::SingleTarget(p) => &p.original_pattern,
            TargetPattern::InterpretPathAsTarget(p) => &p.original_pattern,
            TargetPattern::TargetsInPackage(p) => &p.original_pattern,
            TargetPattern::TargetsBelowDirectory(p) => &p.original_pattern,
        }
    }

    /// For `PATH_AS_TARGET` patterns, the path. Panics (Java `IllegalStateException`) otherwise.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:227 TargetPattern.getPathForPathAsTarget
    pub fn get_path_for_path_as_target(&self) -> &str {
        match self {
            // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:386 TargetPattern.InterpretPathAsTarget.getPathForPathAsTarget
            TargetPattern::InterpretPathAsTarget(p) => &p.path,
            _ => panic!("IllegalStateException"),
        }
    }

    /// For `SINGLE_TARGET` patterns, the label. Panics (Java `IllegalStateException`) otherwise.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:232 TargetPattern.getSingleTargetLabel
    pub fn get_single_target_label(&self) -> &Label {
        match self {
            // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:306 TargetPattern.SingleTarget.getSingleTargetLabel
            TargetPattern::SingleTarget(p) => &p.target,
            _ => panic!("IllegalStateException"),
        }
    }

    /// For `SINGLE_TARGET`, `TARGETS_IN_PACKAGE` and `TARGETS_BELOW_DIRECTORY` patterns, the
    /// package identifier of the pattern (`//foo` for `//foo:bar`, `//foo:all`, `//foo/...`).
    /// Panics (Java `IllegalStateException`) for `PATH_AS_TARGET`.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:251 TargetPattern.getDirectory
    pub fn get_directory(&self) -> &PackageIdentifier {
        match self {
            // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:291 TargetPattern.SingleTarget.getDirectory
            TargetPattern::SingleTarget(p) => p.target.get_package_identifier(),
            // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:470 TargetPattern.TargetsInPackage.getDirectory
            TargetPattern::TargetsInPackage(p) => &p.package_identifier,
            // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:723 TargetPattern.TargetsBelowDirectory.getDirectory
            TargetPattern::TargetsBelowDirectory(p) => &p.directory,
            TargetPattern::InterpretPathAsTarget(_) => panic!("IllegalStateException"),
        }
    }

    /// The repository name of the target pattern.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:256 TargetPattern.getRepository
    pub fn get_repository(&self) -> RepositoryName {
        match self {
            // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:296 TargetPattern.SingleTarget.getRepository
            TargetPattern::SingleTarget(p) => p.target.get_repository().clone(),
            // InterpretPathAsTarget is validated by PackageIdentifier.createInMainRepo,
            // therefore it must belong to the main repository.
            // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:391 TargetPattern.InterpretPathAsTarget.getRepository
            TargetPattern::InterpretPathAsTarget(_) => MAIN.clone(),
            // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:475 TargetPattern.TargetsInPackage.getRepository
            TargetPattern::TargetsInPackage(p) => p.package_identifier.get_repository().clone(),
            // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:728 TargetPattern.TargetsBelowDirectory.getRepository
            TargetPattern::TargetsBelowDirectory(p) => p.directory.get_repository().clone(),
        }
    }

    /// Whether this is a `TARGETS_BELOW_DIRECTORY` or `TARGETS_IN_PACKAGE` pattern matching
    /// rules only.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:262 TargetPattern.getRulesOnly
    pub fn get_rules_only(&self) -> bool {
        match self {
            TargetPattern::SingleTarget(_) | TargetPattern::InterpretPathAsTarget(_) => false,
            TargetPattern::TargetsInPackage(p) => p.rules_only,
            TargetPattern::TargetsBelowDirectory(p) => p.rules_only,
        }
    }

    /// The `TargetsBelowDirectory` view if this pattern has that type (Java cast).
    pub fn as_targets_below_directory(&self) -> Option<&TargetPatternTargetsBelowDirectory> {
        match self {
            TargetPattern::TargetsBelowDirectory(p) => Some(p),
            _ => None,
        }
    }

    // TargetPattern.eval / evalAdaptedForAsync / evalAsync: not ported (see module docs).
}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:264 TargetPattern.toStringHelper
impl fmt::Display for TargetPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetPattern::SingleTarget(p) => fmt::Display::fmt(p, f),
            TargetPattern::InterpretPathAsTarget(p) => fmt::Display::fmt(p, f),
            TargetPattern::TargetsInPackage(p) => fmt::Display::fmt(p, f),
            TargetPattern::TargetsBelowDirectory(p) => fmt::Display::fmt(p, f),
        }
    }
}

impl From<TargetPatternSingleTarget> for TargetPattern {
    fn from(p: TargetPatternSingleTarget) -> Self {
        TargetPattern::SingleTarget(p)
    }
}

impl From<TargetPatternInterpretPathAsTarget> for TargetPattern {
    fn from(p: TargetPatternInterpretPathAsTarget) -> Self {
        TargetPattern::InterpretPathAsTarget(p)
    }
}

impl From<TargetPatternTargetsInPackage> for TargetPattern {
    fn from(p: TargetPatternTargetsInPackage) -> Self {
        TargetPattern::TargetsInPackage(p)
    }
}

impl From<TargetPatternTargetsBelowDirectory> for TargetPattern {
    fn from(p: TargetPatternTargetsBelowDirectory) -> Self {
        TargetPattern::TargetsBelowDirectory(p)
    }
}

/// An explicit target, e.g. `//foo:bar` (Java `TargetPattern.SingleTarget`).
/// Equality compares only the target label, as in Java.
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:269 TargetPattern.SingleTarget
#[derive(Debug, Clone)]
pub struct TargetPatternSingleTarget {
    original_pattern: String,
    target: Label,
}

impl TargetPatternSingleTarget {
    /// Visible for testing.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:274 TargetPattern.SingleTarget.<init>
    pub fn new(original_pattern: &str, target: Label) -> Self {
        TargetPatternSingleTarget {
            original_pattern: original_pattern.to_owned(),
            target,
        }
    }
}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:316 TargetPattern.SingleTarget.equals
impl PartialEq for TargetPatternSingleTarget {
    fn eq(&self, other: &Self) -> bool {
        self.target == other.target
    }
}

impl Eq for TargetPatternSingleTarget {}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:328 TargetPattern.SingleTarget.hashCode
impl Hash for TargetPatternSingleTarget {
    fn hash<H: Hasher>(&self, state: &mut H) {
        TargetPatternType::SingleTarget.hash(state);
        self.target.hash(state);
    }
}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:333 TargetPattern.SingleTarget.toString
impl fmt::Display for TargetPatternSingleTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SingleTarget{{originalPattern={}, target={}}}",
            self.original_pattern, self.target
        )
    }
}

/// A path interpreted as a target, e.g. `foo/bar/baz` (Java
/// `TargetPattern.InterpretPathAsTarget`). Equality compares only the normalized path.
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:339 TargetPattern.InterpretPathAsTarget
#[derive(Debug, Clone)]
pub struct TargetPatternInterpretPathAsTarget {
    original_pattern: String,
    path: String,
}

impl TargetPatternInterpretPathAsTarget {
    /// Visible for testing. The path is normalized with [`TargetPattern::normalize`].
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:343 TargetPattern.InterpretPathAsTarget.<init>
    pub fn new(original_pattern: &str, path: &str) -> Self {
        TargetPatternInterpretPathAsTarget {
            original_pattern: original_pattern.to_owned(),
            path: TargetPattern::normalize(path),
        }
    }
}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:408 TargetPattern.InterpretPathAsTarget.equals
impl PartialEq for TargetPatternInterpretPathAsTarget {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for TargetPatternInterpretPathAsTarget {}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:420 TargetPattern.InterpretPathAsTarget.hashCode
impl Hash for TargetPatternInterpretPathAsTarget {
    fn hash<H: Hasher>(&self, state: &mut H) {
        TargetPatternType::PathAsTarget.hash(state);
        self.path.hash(state);
    }
}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:425 TargetPattern.InterpretPathAsTarget.toString
impl fmt::Display for TargetPatternInterpretPathAsTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "InterpretPathAsTarget{{originalPattern={}, path={}}}",
            self.original_pattern, self.path
        )
    }
}

/// All (rule) targets in a package, e.g. `foo:all`, `foo:*` (Java
/// `TargetPattern.TargetsInPackage`).
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:431 TargetPattern.TargetsInPackage
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetPatternTargetsInPackage {
    original_pattern: String,
    package_identifier: PackageIdentifier,
    suffix: String,
    was_originally_absolute: bool,
    rules_only: bool,
}

impl TargetPatternTargetsInPackage {
    /// Visible for testing.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:438 TargetPattern.TargetsInPackage.<init>
    pub fn new(
        original_pattern: &str,
        package_identifier: PackageIdentifier,
        suffix: &str,
        was_originally_absolute: bool,
        rules_only: bool,
    ) -> Self {
        TargetPatternTargetsInPackage {
            original_pattern: original_pattern.to_owned(),
            package_identifier,
            suffix: suffix.to_owned(),
            was_originally_absolute,
            rules_only,
        }
    }
}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:517 TargetPattern.TargetsInPackage.toString
impl fmt::Display for TargetPatternTargetsInPackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TargetsInPackage{{originalPattern={}, packageIdentifier={}, suffix={}, \
             wasOriginallyAbsolute={}, rulesOnly={}}}",
            self.original_pattern,
            self.package_identifier,
            self.suffix,
            self.was_originally_absolute,
            self.rules_only
        )
    }
}

/// All (rule) targets beneath a directory, e.g. `foo/...` (Java
/// `TargetPattern.TargetsBelowDirectory`).
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:573 TargetPattern.TargetsBelowDirectory
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetPatternTargetsBelowDirectory {
    original_pattern: String,
    directory: PackageIdentifier,
    rules_only: bool,
}

/// How a directory exclusion relates to subtracting one pattern from another (Java
/// `TargetPattern.TargetsBelowDirectory.ContainsResult`).
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:704 TargetPattern.TargetsBelowDirectory.ContainsResult
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetPatternTargetsBelowDirectoryContainsResult {
    /// Evaluating this pattern with a directory exclusion of the other pattern's directory would
    /// result in exactly the same set of targets as evaluating the subtraction of the other
    /// pattern from this one.
    DirectoryExclusionWouldBeExact,
    /// A directory exclusion of the other pattern's directory would be too broad because this
    /// pattern is not "rules only" and the other one is.
    DirectoryExclusionWouldBeTooBroad,
    /// None of the above. The other pattern isn't contained by this pattern.
    NotContained,
}

impl TargetPatternTargetsBelowDirectory {
    /// Visible for testing.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:578 TargetPattern.TargetsBelowDirectory.<init>
    pub fn new(original_pattern: &str, directory: PackageIdentifier, rules_only: bool) -> Self {
        TargetPatternTargetsBelowDirectory {
            original_pattern: original_pattern.to_owned(),
            directory,
            rules_only,
        }
    }

    pub fn get_original_pattern(&self) -> &str {
        &self.original_pattern
    }

    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:723 TargetPattern.TargetsBelowDirectory.getDirectory
    pub fn get_directory(&self) -> &PackageIdentifier {
        &self.directory
    }

    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:733 TargetPattern.TargetsBelowDirectory.getRulesOnly
    pub fn get_rules_only(&self) -> bool {
        self.rules_only
    }

    /// Is `containing_directory` an ancestor of or equal to this pattern's directory?
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:669 TargetPattern.TargetsBelowDirectory.containedIn
    pub fn contained_in(&self, containing_directory: &PathFragment) -> bool {
        self.directory
            .get_package_fragment()
            .starts_with(containing_directory)
    }

    /// Whether `contained_directory` is contained by or equals this pattern's directory.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:680 TargetPattern.TargetsBelowDirectory.containsAllTransitiveSubdirectories
    pub fn contains_all_transitive_subdirectories(
        &self,
        contained_directory: &PackageIdentifier,
    ) -> bool {
        // Note that merely checking to see if the directory startsWith the TargetsBelowDirectory's
        // directory is insufficient. "food" begins with "foo", but "//foo/..." does not contain
        // "//food/...".
        contained_directory.get_repository() == self.directory.get_repository()
            && contained_directory
                .get_package_fragment()
                .starts_with(self.directory.get_package_fragment())
    }

    /// How a directory exclusion of `contained_pattern`'s directory relates to subtracting
    /// `contained_pattern` from this pattern.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:693 TargetPattern.TargetsBelowDirectory.contains
    pub fn contains(
        &self,
        contained_pattern: &TargetPatternTargetsBelowDirectory,
    ) -> TargetPatternTargetsBelowDirectoryContainsResult {
        if self.contains_all_transitive_subdirectories(&contained_pattern.directory) {
            if !self.rules_only && contained_pattern.rules_only {
                TargetPatternTargetsBelowDirectoryContainsResult::DirectoryExclusionWouldBeTooBroad
            } else {
                TargetPatternTargetsBelowDirectoryContainsResult::DirectoryExclusionWouldBeExact
            }
        } else {
            TargetPatternTargetsBelowDirectoryContainsResult::NotContained
        }
    }
}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:762 TargetPattern.TargetsBelowDirectory.toString
impl fmt::Display for TargetPatternTargetsBelowDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TargetsBelowDirectory{{originalPattern={}, directory={}, rulesOnly={}}}",
            self.original_pattern, self.directory, self.rules_only
        )
    }
}

/// The set of target-pattern suffixes which indicate wildcards over all *rules* in a single
/// package.
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:773 TargetPattern.Parser.ALL_RULES_IN_SUFFIXES
const ALL_RULES_IN_SUFFIXES: [&str; 1] = ["all"];

/// The set of target-pattern suffixes which indicate wildcards over all *targets* in a single
/// package.
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:779 TargetPattern.Parser.ALL_TARGETS_IN_SUFFIXES
const ALL_TARGETS_IN_SUFFIXES: [&str; 2] = ["*", "all-targets"];

/// Parses target patterns relative to a directory, a current repo and a repo mapping (Java
/// `TargetPattern.Parser`).
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:768 TargetPattern.Parser
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetPatternParser {
    /// Directory prefix to use when resolving relative labels: if the working directory is
    /// "<workspace root>/foo", this is "foo", so "bar:bar" resolves to "//foo/bar:bar".
    relative_directory: PathFragment,
    /// The repo to use for repo-relative target patterns (so "//foo" becomes
    /// "@currentRepo//foo").
    current_repo: RepositoryName,
    /// The repo mapping to use for the @repo part of target patterns.
    repo_mapping: RepositoryMapping,
}

impl TargetPatternParser {
    /// Creates a parser. Panics (Java `IllegalArgumentException`) for a non-main repo with a
    /// non-empty relative directory.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:798 TargetPattern.Parser.<init>
    pub fn new(
        relative_directory: PathFragment,
        current_repo: RepositoryName,
        repo_mapping: RepositoryMapping,
    ) -> Self {
        assert!(
            current_repo.is_main() || relative_directory.is_empty(),
            "parsing target patterns in a non-main repo with a relative directory is unsupported"
        );
        TargetPatternParser {
            relative_directory,
            current_repo,
            repo_mapping,
        }
    }

    /// Parses the given pattern.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:814 TargetPattern.Parser.parse
    pub fn parse(&self, pattern: &str) -> Result<TargetPattern, TargetParsingException> {
        let parts = LabelParserParts::parse(pattern).map_err(|e| {
            TargetParsingException::new(e.get_message(), TargetPatternsCode::LabelSyntaxError)
        })?;

        // Special case: For a target pattern that just looks like `foo/bar/baz`, we treat this as
        // a file path. LabelParser parses it as `:foo/bar/baz`, so we need to distinguish this
        // case by checking if the original pattern contains a colon.
        if !parts.pkg_is_absolute()
            && self.current_repo.is_main()
            && parts.pkg().is_empty()
            && !parts.pkg_ends_with_triple_dots()
            && !pattern.contains(':')
        {
            return Ok(TargetPatternInterpretPathAsTarget::new(
                pattern,
                self.relative_directory
                    .get_relative_str(parts.target())
                    .get_path_string(),
            )
            .into());
        }

        let package_identifier = self.create_package_identifier_from_parts(&parts);
        if parts.pkg_ends_with_triple_dots() {
            if parts.target().is_empty() || ALL_RULES_IN_SUFFIXES.contains(&parts.target()) {
                return Ok(TargetPatternTargetsBelowDirectory::new(
                    pattern,
                    package_identifier,
                    true,
                )
                .into());
            } else if ALL_TARGETS_IN_SUFFIXES.contains(&parts.target()) {
                return Ok(TargetPatternTargetsBelowDirectory::new(
                    pattern,
                    package_identifier,
                    false,
                )
                .into());
            }
            return Err(TargetParsingException::new(
                format!(
                    "Invalid target pattern {}: '...' can only be used with wildcard targets",
                    pattern
                ),
                TargetPatternsCode::LabelSyntaxError,
            ));
        }

        if pattern.contains(':') && ALL_RULES_IN_SUFFIXES.contains(&parts.target()) {
            return Ok(TargetPatternTargetsInPackage::new(
                pattern,
                package_identifier,
                parts.target(),
                parts.pkg_is_absolute(),
                true,
            )
            .into());
        }

        if pattern.contains(':') && ALL_TARGETS_IN_SUFFIXES.contains(&parts.target()) {
            return Ok(TargetPatternTargetsInPackage::new(
                pattern,
                package_identifier,
                parts.target(),
                parts.pkg_is_absolute(),
                false,
            )
            .into());
        }

        Ok(TargetPatternSingleTarget::new(
            pattern,
            Label::create_unvalidated(package_identifier, parts.target()),
        )
        .into())
    }

    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:859 TargetPattern.Parser.createPackageIdentifierFromParts
    fn create_package_identifier_from_parts(&self, parts: &LabelParserParts) -> PackageIdentifier {
        let repo = match parts.repo() {
            None => self.current_repo.clone(),
            Some(repo) if parts.repo_is_canonical() => RepositoryName::create_unvalidated(repo),
            Some(repo) => self.repo_mapping.get(repo),
        };

        let package_path_fragment = if parts.pkg_is_absolute() {
            PathFragment::create(parts.pkg())
        } else {
            self.relative_directory.get_relative_str(parts.pkg())
        };
        PackageIdentifier::create(repo, package_path_fragment)
    }

    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:876 TargetPattern.Parser.getRepoMapping
    pub fn get_repo_mapping(&self) -> &RepositoryMapping {
        &self.repo_mapping
    }

    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:880 TargetPattern.Parser.getCurrentRepo
    pub fn get_current_repo(&self) -> &RepositoryName {
        &self.current_repo
    }

    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:884 TargetPattern.Parser.getRelativeDirectory
    pub fn get_relative_directory(&self) -> &PathFragment {
        &self.relative_directory
    }

    /// Parses a constant pattern, panicking (Java `IllegalStateException`) if it is invalid.
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:892 TargetPattern.Parser.parseConstantUnchecked
    pub fn parse_constant_unchecked(&self, pattern: &str) -> TargetPattern {
        match self.parse(pattern) {
            Ok(p) => p,
            Err(e) => panic!("IllegalStateException: {}", e),
        }
    }

    /// Absolutizes the pattern to the offset: patterns starting with "//" are returned as is.
    /// With offset "foo": ":bar" gives "//foo:bar", "bar" gives "//foo/bar", "biz:bar" gives
    /// "//foo/biz:bar".
    // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:911 TargetPattern.Parser.absolutize
    pub fn absolutize(&self, pattern: &str) -> String {
        if pattern.starts_with("//") {
            return pattern.to_owned();
        }

        // PathFragment#getRelative doesn't work when the pattern starts with ":".
        // "foo".getRelative(":all") would return "foo/:all", where we really want "foo:all".
        if pattern.starts_with(':') || self.relative_directory.is_empty() {
            format!("//{}{}", self.relative_directory, pattern)
        } else {
            format!("//{}/{}", self.relative_directory, pattern)
        }
    }
}
