//! Port of Bazel's `com.google.devtools.build.lib.cmdline`: repository names, package
//! identifiers, labels and target patterns.
//!
//! - [`RepositoryName`] (canonical repo names, `@@`/`@` forms, not-visible names)
//! - [`RepositoryMapping`] (apparent to canonical repo names, from a context repo)
//! - [`PackageIdentifier`], [`Label`] (parsing in canonical, repo and package contexts; display
//!   forms under a main repo mapping)
//! - [`label_parser`], [`label_validator`] (syntax and exact error messages)
//! - [`TargetPattern`], [`TargetPatternParser`], [`SignedTargetPattern`], [`ResolvedTargets`]
//! - [`label_constants`]
//!
//! Values are immutable, cheap to clone (`Arc`, interned where Java interns), `Send + Sync`, and
//! their `Eq`/`Hash`/`Ord` follow Java's `equals`/`compareTo`.
//!
//! Not ported: serialization codecs, `TargetPattern` evaluation (needs `TargetPatternResolver`,
//! `IgnoredSubdirectories`), `ParallelVisitor`, `BazelModuleContext`, Starlark integration
//! (a later crate wraps `Label` as a Starlark value).

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

mod interner;
pub mod label;
pub mod label_constants;
pub mod label_parser;
pub mod label_syntax_exception;
pub mod label_validator;
pub mod package_identifier;
pub mod repository_mapping;
pub mod repository_name;
pub mod resolved_targets;
pub mod signed_target_pattern;
mod string_utilities;
pub mod target_parsing_exception;
pub mod target_pattern;

pub use label::{
    Label, LabelPackageContext, LabelRepoContext, LabelRepoContextImpl, LabelRepoMappingRecorder,
    LabelSimpleRepoMappingRecorder,
};
pub use label_parser::LabelParserParts;
pub use label_syntax_exception::{EvalException, LabelSyntaxException};
pub use label_validator::{LabelValidatorBadLabelException, LabelValidatorPackageAndTarget};
pub use package_identifier::PackageIdentifier;
pub use repository_mapping::RepositoryMapping;
pub use repository_name::RepositoryName;
pub use resolved_targets::{ResolvedTargets, ResolvedTargetsBuilder};
pub use signed_target_pattern::{SignedTargetPattern, SignedTargetPatternSign};
pub use target_parsing_exception::{TargetParsingException, TargetPatternsCode};
pub use target_pattern::{
    TargetPattern, TargetPatternInterpretPathAsTarget, TargetPatternParser,
    TargetPatternSingleTarget, TargetPatternTargetsBelowDirectory,
    TargetPatternTargetsBelowDirectoryContainsResult, TargetPatternTargetsInPackage,
    TargetPatternType,
};
