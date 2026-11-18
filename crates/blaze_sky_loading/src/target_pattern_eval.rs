//! `TargetPattern.eval` and `TargetPatternResolver`.
//!
//! PORT: move to blaze_cmdline (`target_pattern.rs`). blaze_cmdline has the patterns and their
//! parser but not their evaluation; it is ported here, next to its only resolver.
//!
//! Java: com/google/devtools/build/lib/cmdline/TargetPattern.java (the `eval` methods) and
//! com/google/devtools/build/lib/cmdline/TargetPatternResolver.java.

use blaze_cmdline::{
    Label, LabelParserParts, PackageIdentifier, RepositoryName, TargetParsingException,
    TargetPattern, TargetPatternsCode,
};
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use crate::ignored_subdirectories::IgnoredSubdirectories;
use crate::io::IOException;

/// What evaluating a target pattern can throw besides a result (Java: `TargetParsingException`,
/// `InterruptedException`, `InconsistentFilesystemException`, `ProcessPackageDirectoryException`
/// and, inside Skyframe, `MissingDepException`).
#[derive(Debug, Clone)]
pub enum TargetPatternEvalError {
    TargetParsing(TargetParsingException),
    Interrupted,
    /// An `InconsistentFilesystemException` (an [`IOException`] of that kind).
    InconsistentFilesystem(IOException),
    /// A `ProcessPackageDirectoryException` with its message.
    ProcessPackageDirectory(String),
    /// Java `MissingDepException`: a Skyframe dependency is unavailable (in error).
    MissingDep,
}

impl From<TargetParsingException> for TargetPatternEvalError {
    fn from(e: TargetParsingException) -> Self {
        TargetPatternEvalError::TargetParsing(e)
    }
}

impl From<blaze_skyframe::Interrupted> for TargetPatternEvalError {
    fn from(_: blaze_skyframe::Interrupted) -> Self {
        TargetPatternEvalError::Interrupted
    }
}

/// A callback that is used during the process of converting target patterns (such as `//foo:all`)
/// into one or more lists of targets (Java `TargetPatternResolver`).
// Java: com/google/devtools/build/lib/cmdline/TargetPatternResolver.java:35 TargetPatternResolver
pub trait TargetPatternResolver<T> {
    /// Reports the given warning.
    // Java: com/google/devtools/build/lib/cmdline/TargetPatternResolver.java:39 TargetPatternResolver.warn
    fn warn(&self, msg: &str);

    /// Returns a single target corresponding to the given label, or `None`.
    // Java: com/google/devtools/build/lib/cmdline/TargetPatternResolver.java:45 TargetPatternResolver.getTargetOrNull
    fn get_target_or_null(&self, label: &Label) -> Result<Option<T>, TargetPatternEvalError>;

    /// Returns a single target corresponding to the given label, or an empty or failed result.
    // Java: com/google/devtools/build/lib/cmdline/TargetPatternResolver.java:51 TargetPatternResolver.getExplicitTarget
    fn get_explicit_target(&self, label: &Label) -> Result<Vec<T>, TargetPatternEvalError>;

    /// Returns the set containing the targets found in the given package.
    // Java: com/google/devtools/build/lib/cmdline/TargetPatternResolver.java:66 TargetPatternResolver.getTargetsInPackage
    fn get_targets_in_package(
        &self,
        original_pattern: &str,
        package_identifier: &PackageIdentifier,
        rules_only: bool,
    ) -> Result<Vec<T>, TargetPatternEvalError>;

    /// Computes the set containing the targets found below the given `directory`, passing it in
    /// batches to `callback`.
    // Java: com/google/devtools/build/lib/cmdline/TargetPatternResolver.java:89 TargetPatternResolver.findTargetsBeneathDirectory
    fn find_targets_beneath_directory(
        &self,
        repository: &RepositoryName,
        original_pattern: &str,
        directory: &str,
        rules_only: bool,
        forbidden_subdirectories: &IgnoredSubdirectories,
        excluded_subdirectories: &[PathFragment],
        callback: &mut dyn FnMut(Vec<T>),
    ) -> Result<(), TargetPatternEvalError>;

    /// Returns true if, and only if, the given package identifier corresponds to a package.
    // Java: com/google/devtools/build/lib/cmdline/TargetPatternResolver.java:149 TargetPatternResolver.isPackage
    fn is_package(
        &self,
        package_identifier: &PackageIdentifier,
    ) -> Result<bool, TargetPatternEvalError>;

    /// Returns the target kind of the given target, for example `cc_library rule`.
    // Java: com/google/devtools/build/lib/cmdline/TargetPatternResolver.java:157 TargetPatternResolver.getTargetKind
    fn get_target_kind(&self, target: &T) -> String;
}

// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:845 TargetPattern.label
fn label(pkg: &PackageIdentifier, name: &str) -> Result<Label, TargetParsingException> {
    Label::create_in_package(pkg, name).map_err(|e| {
        TargetParsingException::new(e.get_message(), TargetPatternsCode::LabelSyntaxError)
    })
}

/// Evaluates `pattern` with `resolver`, passing the targets to `callback`.
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:153 TargetPattern.eval
pub fn eval<T>(
    pattern: &TargetPattern,
    resolver: &dyn TargetPatternResolver<T>,
    ignored_subdirectories: &dyn Fn() -> Result<IgnoredSubdirectories, TargetPatternEvalError>,
    excluded_subdirectories: &[PathFragment],
    callback: &mut dyn FnMut(Vec<T>),
) -> Result<(), TargetPatternEvalError> {
    match pattern {
        // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:280 TargetPattern.SingleTarget.eval
        TargetPattern::SingleTarget(_) => {
            callback(resolver.get_explicit_target(pattern.get_single_target_label())?);
            Ok(())
        }
        // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:349 TargetPattern.InterpretPathAsTarget.eval
        TargetPattern::InterpretPathAsTarget(_) => {
            let path = pattern.get_path_for_path_as_target();
            let path_as_package = PackageIdentifier::create_in_main_repo_str(path);
            if resolver.is_package(&path_as_package)? {
                // User has specified a package name. lookout for default target.
                callback(resolver.get_explicit_target(&label(
                    &path_as_package,
                    path_as_package.get_package_fragment().get_base_name(),
                )?)?);
                return Ok(());
            }
            let pieces: Vec<&str> = path.split('/').collect();

            // Interprets the label as a file target. This loop stops as soon as the first BUILD
            // file is found (i.e. longest prefix match).
            for i in (0..pieces.len()).rev() {
                let pkg = PackageIdentifier::create_in_main_repo_str(&pieces[..i].join("/"));
                if resolver.is_package(&pkg)? {
                    let target_name = pieces[i..].join("/");
                    callback(resolver.get_explicit_target(&label(&pkg, &target_name)?)?);
                    return Ok(());
                }
            }

            Err(TargetParsingException::new(
                format!("couldn't determine target from filename '{}'", path),
                TargetPatternsCode::CannotDetermineTargetFromFilename,
            )
            .into())
        }
        // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:452 TargetPattern.TargetsInPackage.eval
        TargetPattern::TargetsInPackage(_) => {
            if let Some(targets) = get_wildcard_conflict(pattern, resolver)? {
                callback(targets);
                return Ok(());
            }
            callback(resolver.get_targets_in_package(
                pattern.get_original_pattern(),
                pattern.get_directory(),
                pattern.get_rules_only(),
            )?);
            Ok(())
        }
        // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:585 TargetPattern.TargetsBelowDirectory.eval
        TargetPattern::TargetsBelowDirectory(_) => {
            let directory = pattern.get_directory();
            assert!(
                !excluded_subdirectories.contains(directory.get_package_fragment()),
                "Fully excluded target pattern {} should have already been filtered out ({:?})",
                pattern,
                excluded_subdirectories
            );
            let ignored_subdirectories = ignored_subdirectories()?;
            let matching_entry =
                ignored_subdirectories.matching_entry(directory.get_package_fragment());
            // Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:655 TargetPattern.TargetsBelowDirectory.warnIfFiltered
            if let Some(matching_entry) = matching_entry {
                resolver.warn(&format!(
                    "Pattern '{}' was filtered out by ignored directory '{}'",
                    pattern.get_original_pattern(),
                    matching_entry
                ));
                return Ok(());
            }

            let filtered_ignored_subdirectories =
                ignored_subdirectories.filter_for_directory(directory.get_package_fragment());

            resolver.find_targets_beneath_directory(
                directory.get_repository(),
                pattern.get_original_pattern(),
                directory.get_package_fragment().get_path_string(),
                pattern.get_rules_only(),
                &filtered_ignored_subdirectories,
                excluded_subdirectories,
                callback,
            )
        }
    }
}

/// The `suffix` and `wasOriginallyAbsolute` of a `TargetsInPackage` pattern, as Java's
/// `TargetPattern.Parser.parse` computes them from the original pattern.
// PORT: replace with TargetPatternTargetsInPackage getters once blaze_cmdline exposes them.
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:796 TargetPattern.Parser.parse
fn targets_in_package_suffix_and_absolute(pattern: &TargetPattern) -> (String, bool) {
    let parts = LabelParserParts::parse(pattern.get_original_pattern())
        .expect("the pattern was parsed before");
    (parts.target().to_owned(), parts.pkg_is_absolute())
}

/// There's a potential ambiguity if `//foo/bar:all` refers to an actual target. In this case, we
/// use the target but print a warning. Returns the target if the pattern is absolute and there is
/// such a target.
// Java: com/google/devtools/build/lib/cmdline/TargetPattern.java:534 TargetPattern.TargetsInPackage.getWildcardConflict
fn get_wildcard_conflict<T>(
    pattern: &TargetPattern,
    resolver: &dyn TargetPatternResolver<T>,
) -> Result<Option<Vec<T>>, TargetPatternEvalError> {
    let (suffix, was_originally_absolute) = targets_in_package_suffix_and_absolute(pattern);
    if !was_originally_absolute {
        return Ok(None);
    }

    let Ok(label) = Label::create_in_package(pattern.get_directory(), &suffix) else {
        return Ok(None);
    };
    let target = resolver.get_target_or_null(&label)?;

    if let Some(target) = target {
        resolver.warn(&format!(
            "The target pattern '{}' is ambiguous: ':{}' is both a wildcard, and the name of an \
             existing {}; using the latter interpretation",
            pattern.get_original_pattern(),
            suffix,
            resolver.get_target_kind(&target)
        ));
        return match resolver.get_explicit_target(&label) {
            Ok(targets) => Ok(Some(targets)),
            Err(TargetPatternEvalError::TargetParsing(e)) => panic!(
                "IllegalStateException: getTargetOrNull() returned non-null, so target should \
                 exist: {}",
                e
            ),
            Err(e) => Err(e),
        };
    }
    Ok(None)
}

/// Java `ResolvedTargets.Builder<T>` for the callers here: an insertion-ordered set plus the
/// error bit.
#[derive(Debug, Clone)]
pub struct ResolvedTargetSet<T: std::hash::Hash + Eq> {
    pub targets: IndexSet<T>,
    pub has_error: bool,
}

impl<T: std::hash::Hash + Eq> Default for ResolvedTargetSet<T> {
    fn default() -> Self {
        ResolvedTargetSet {
            targets: IndexSet::new(),
            has_error: false,
        }
    }
}
