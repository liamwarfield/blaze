//! Port of Bazel 9.3.2's command-line options framework
//! (`com.google.devtools.common.options`), the C++ client's rc-file reader
//! (`src/main/cpp/rc_file.cc`, parts of `option_processor.cc`) and the server-side rc/`--config`
//! handling of `BlazeOptionHandler` / `ConfigExpander`.
//!
//! Options classes are declared with the [`options!`] macro; see `TRANSCRIBE.md` for the rules
//! that map a Java `@Option` field onto it.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]
// OptionDefinition's OnceLock caches do not take part in Hash/Eq.
#![allow(clippy::mutable_key_type)]

// Java classes with no Rust counterpart (each method on its own line, see docs/coverage):
// PORT: skipped, no user in Bazel at the pin (params files of other tools): ParamsFilePreProcessor.preProcess
// PORT: skipped, no user in Bazel at the pin (params files of other tools): ParamsFilePreProcessor.parse
// PORT: skipped, no user in Bazel at the pin (params files of other tools): ShellQuotedParamsFilePreProcessor.parse
// PORT: skipped, no user in Bazel at the pin (params files of other tools): UnquotedParamsFilePreProcessor.parse
// PORT: skipped, no user in Bazel at the pin (params files of other tools): ShellQuotedReader.read
// PORT: skipped, no user in Bazel at the pin (params files of other tools): ShellQuotedReader.unread
// PORT: skipped, no user in Bazel at the pin (params files of other tools): ShellQuotedReader.hasNext
// PORT: skipped, no user in Bazel at the pin (params files of other tools): ShellQuotedReader.close
// PORT: skipped, no user in Bazel at the pin (params files of other tools): ShellQuotedReader.readArg
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.name
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.abbrev
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.help
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.valueHelp
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.defaultValue
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.category
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.documentationCategory
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.effectTags
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.metadataTags
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.converter
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.allowMultiple
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.expansion
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.implicitRequirements
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.deprecationWarning
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.oldName
// PORT: skipped, an annotation attribute; the options! macro attribute of the same name carries it: Option.oldNameWarning
// PORT: skipped, JVM reflection on converter type parameters (the options! macro types each field): GenericTypeHelper.getRawType
// PORT: skipped, JVM reflection on converter type parameters (the options! macro types each field): GenericTypeHelper.matchTypeVariable
// PORT: skipped, JVM reflection on converter type parameters (the options! macro types each field): GenericTypeHelper.getActualReturnType
// PORT: skipped, JVM reflection on converter type parameters (the options! macro types each field): GenericTypeHelper.isAssignableFrom
// PORT: skipped, the opaque holder of OptionsData for other packages (Rust shares OptionsData itself): OpaqueOptionsData.<init>
// PORT: skipped, the exception's constructor (its class name is exception::DUPLICATE_OPTION_DECLARATION_EXCEPTION): DuplicateOptionDeclarationException.<init>

#[macro_use]
mod macros;

pub mod caffeine_spec;
pub mod converter;
pub mod converters;
pub mod data;
pub mod definition;
pub mod exception;
pub mod foreign;
pub mod global_rc;
pub mod html_utils;
pub mod invocation_policy;
mod line_break_iterator;
pub mod option_filter_descriptions;
pub mod options;
pub mod options_usage;
pub mod parsed;
pub mod parser;
mod parser_impl;
pub mod priority;
pub mod rc;
pub mod spelling;
pub mod tags;
pub mod value;
pub mod value_description;

pub use converter::{ConversionContext, Converter};
pub use converters::*;
pub use data::OptionsData;
pub use definition::{
    FieldInfo, OptionAnnotation, OptionDefinition, OptionsBase, OptionsClass, OptionsClassInfo,
};
pub use exception::{ConstructionException, OptionsParsingException};
pub use foreign::Foreign;
pub use options::Options;
pub use options_usage::OptionsUsage;
pub use parsed::{OptionInstanceOrigin, ParsedOptionDescription};
pub use parser::{ArgAndFallbackData, ArgsPreProcessor, OptionDescription, OptionsParser};
pub use priority::{OptionPriority, PriorityCategory};
pub use tags::{OptionDocumentationCategory, OptionEffectTag, OptionMetadataTag, TriState};
pub use value::{
    FieldKind, FieldType, ListValue, OptionScalar, OptionValue, OptionalInt, Value, Void,
};
pub use value_description::OptionValueDescription;

pub use blaze_util::unported;

#[doc(hidden)]
pub mod __private {
    pub use std::sync::{Arc, LazyLock};
}
