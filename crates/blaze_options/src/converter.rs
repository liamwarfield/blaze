//! The converter interface.

use std::any::Any;

use crate::exception::OptionsParsingException;
use crate::value::Value;

/// Java's `conversionContext` (`@Nullable Object`).
pub type ConversionContext = dyn Any + Send + Sync;

// Java: com/google/devtools/common/options/Converter.java:22 Converter
/// Converts the string form of an option value into a [`Value`].
///
/// `input` is `None` for Java `null` (e.g. a `Void` option given without a value). The result is
/// `None` for Java `null`.
pub trait Converter: Send + Sync {
    // Java: com/google/devtools/common/options/Converter.java:29 Converter.convert
    fn convert(
        &self,
        input: Option<&str>,
        conversion_context: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException>;

    // Java: com/google/devtools/common/options/Converter.java:35 Converter.getTypeDescription
    fn get_type_description(&self) -> String;

    // Java: com/google/devtools/common/options/Converter.java:43 Converter.starlarkConvertible
    fn starlark_convertible(&self) -> bool {
        false
    }

    // Java: com/google/devtools/common/options/Converter.java:59 Converter.reverseForStarlark
    fn reverse_for_starlark(&self, _converted: &Value) -> String {
        panic!("UnsupportedOperationException: This converter doesn't support Starlark reversal.")
    }

    /// `converter instanceof BoolOrEnumConverter` (used by `usesBooleanValueSyntax`).
    fn is_bool_or_enum(&self) -> bool {
        false
    }
}

/// Java dereferences the (non-null) input of contextless converters; a `null` input is a
/// `NullPointerException`.
pub fn non_null(input: Option<&str>) -> &str {
    match input {
        Some(s) => s,
        None => panic!("NullPointerException: converter input is null"),
    }
}
