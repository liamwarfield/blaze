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

    /// `getClass().getSimpleName()`. Blaze's converter types keep the Java class names
    /// (PORTING.md "Names"), so this is the Rust type name without its path and generic
    /// arguments; a converter whose Java class is anonymous overrides it with `""`.
    fn java_simple_class_name(&self) -> &'static str {
        let full = std::any::type_name::<Self>();
        let without_generics = full.split('<').next().unwrap_or(full);
        let rust_name = without_generics
            .rsplit("::")
            .next()
            .unwrap_or(without_generics);
        nested_java_converter_simple_name(rust_name).unwrap_or(rust_name)
    }

    /// For `converter instanceof EnumConverter`: `getEnumType().getEnumConstants()`, each
    /// constant's `toString()`; `None` for other converters.
    fn enum_type_constants(&self) -> Option<Vec<String>> {
        None
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

/// Converters whose Java class is a nested class: PORTING.md names them `OuterInner`, while
/// Java's `getSimpleName()` is `Inner`. Each entry is (Rust name, Java simple name), from the
/// Java sources at the pin (e.g. `CompilationMode.Converter`, `PackageOptions.OptionConverter`
/// of `LazyMacroExpansionPackages`, `ResourceConverter.AssignmentConverter`).
const NESTED_JAVA_CONVERTERS: &[(&str, &str)] = &[
    // com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java AutoOutputFilter.Converter
    ("AutoOutputFilterConverter", "Converter"),
    // com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java
    ("BazelCompatibilityModeConverter", "Converter"),
    ("CheckDirectDepsModeConverter", "Converter"),
    ("LockfileModeConverter", "Converter"),
    ("RequireRepoExtensionMetadataModeConverter", "Converter"),
    // com/google/devtools/build/lib/analysis/test/TestConfiguration.java CancelConcurrentTests.Converter
    ("CancelConcurrentTestsConverter", "Converter"),
    // com/google/devtools/build/lib/remote/options/RemoteOptions.java
    ("ChunkingFunctionValueConverter", "Converter"),
    ("CircuitBreakerStrategyConverter", "Converter"),
    ("ConcurrentChangesCheckLevelConverter", "Converter"),
    ("ExecutionMessagePrintModeConverter", "Converter"),
    // com/google/devtools/build/lib/analysis/config/CompilationMode.java CompilationMode.Converter
    ("CompilationModeConverter", "Converter"),
    // com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java ExecutionInfoModifier.Converter
    ("ExecutionInfoModifierConverter", "Converter"),
    // com/google/devtools/build/lib/profiler/MemoryProfiler.java MemoryProfileStableHeapParameters.Converter
    ("MemoryProfileStableHeapParametersConverter", "Converter"),
    // com/google/devtools/build/lib/exec/ExecutionOptions.java
    ("TestOutputFormatConverter", "Converter"),
    ("TestSummaryFormatConverter", "Converter"),
    // com/google/devtools/build/lib/runtime/CommandLineEvent.java ToolCommandLineEvent.Converter
    ("ToolCommandLineEventConverter", "Converter"),
    // com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java Utf8EnforcementMode.Converter
    ("Utf8EnforcementModeConverter", "Converter"),
    // com/google/devtools/build/lib/pkgcache/PackageOptions.java:281 LazyMacroExpansionPackages.OptionConverter
    (
        "LazyMacroExpansionPackagesOptionConverter",
        "OptionConverter",
    ),
    // com/google/devtools/build/lib/util/ResourceConverter.java:57 ResourceConverter.AssignmentConverter
    (
        "ResourceConverterAssignmentConverter",
        "AssignmentConverter",
    ),
];

/// The Java simple name of a converter whose Java class is nested (see `NESTED_JAVA_CONVERTERS`).
pub fn nested_java_converter_simple_name(rust_name: &str) -> Option<&'static str> {
    NESTED_JAVA_CONVERTERS
        .iter()
        .find(|(rust, _)| *rust == rust_name)
        .map(|(_, java)| *java)
}
