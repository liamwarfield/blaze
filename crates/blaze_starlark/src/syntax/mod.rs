//! Bazel's static checks that starlark-rust's parser and resolver do not perform.
//!
//! - [`dot_bazel`]: `DotBazelFileSyntaxChecker` (BUILD, MODULE.bazel, REPO.bazel, VENDOR.bazel),
//!   plus `PackageFactory.checkBuildSyntax`'s glob/subpackages prefetch and generator names.
//! - [`resolver`]: the Java Resolver rules for top-level bindings and load placement
//!   (`'x' redeclared at top level`, conflicting load/global, load-first) and .scl's ASCII-only
//!   string literals.
//!
//! [`check_file`] runs whichever of these Bazel runs for a [`FileKind`](crate::FileKind).

pub mod dot_bazel;
pub mod resolver;

use starlark::syntax::AstModule;

use crate::error::Location;
use crate::error::SyntaxError;
use crate::file_kind::FileKind;

pub(crate) fn location(ast: &AstModule, span: starlark::codemap::Span) -> Location {
    Location::of_span_start(&ast.file_span(span))
}

/// The extra results of checking a BUILD file (Java `checkBuildSyntax` out-parameters).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildFileFacts {
    /// Literal `glob()` include patterns with `exclude_directories` != 0.
    pub globs: Vec<String>,
    /// Literal `glob()` include patterns with `exclude_directories = 0`.
    pub globs_with_dirs: Vec<String>,
    /// Literal `subpackages()` include patterns.
    pub subpackages: Vec<String>,
    /// `(location of the call's '(', name)` for every call `f(name = "literal", ...)`.
    pub generator_name_by_location: Vec<(Location, String)>,
}

/// Runs Bazel's static checks for `kind` on a parsed file. Returns the errors (empty = ok) and,
/// for BUILD files, the extracted facts.
pub fn check_file(kind: FileKind, ast: &AstModule) -> (Vec<SyntaxError>, BuildFileFacts) {
    check_file_with_options(kind, kind.file_options(), ast)
}

/// [`check_file`] with explicit file options.
pub fn check_file_with_options(
    kind: FileKind,
    options: crate::file_kind::FileOptions,
    ast: &AstModule,
) -> (Vec<SyntaxError>, BuildFileFacts) {
    let mut errors = Vec::new();
    let mut facts = BuildFileFacts::default();
    if let Some((where_, can_load_bzl, allow_literal_star_star)) = kind.dot_bazel_checker_config() {
        let checker = dot_bazel::Checker {
            where_,
            can_load_bzl,
            allow_literal_star_star_args: allow_literal_star_star,
        };
        let r = checker.check(ast, kind == FileKind::Build);
        errors.extend(r.0);
        facts = r.1;
        // Java: PackageFunction / module-file code return the checker's errors before compiling.
        if !errors.is_empty() {
            return (errors, facts);
        }
    }
    errors.extend(resolver::check(ast, options));
    (errors, facts)
}
