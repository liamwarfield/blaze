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

#[macro_use]
mod macros;

pub mod converter;
pub mod converters;
pub mod data;
pub mod definition;
pub mod exception;
pub mod foreign;
pub mod global_rc;
pub mod html_utils;
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
