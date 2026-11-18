//! Minimal port of `lib/util/DetailedExitCode.java`, as far as the query engine needs it.
//!
//! PORT: move to blaze_util (lib/util/DetailedExitCode.java, lib/util/ExitCode.java) once
//! `blaze_util` ports them; the engine keeps this private copy until then. The exit code of a
//! `FailureDetail` comes from the `(metadata).exit_code` option of its category's code enum, which
//! prost does not keep; the codes of the query categories are transcribed below from
//! `failure_details.proto`, every other category is `unported!`.

use blaze_protos::failure_details::{
    FailureDetail, action_query, configurable_query, failure_detail::Category, query,
};

/// An `ExitCode` that has a `FailureDetail` unless it's `ExitCode.SUCCESS`.
///
/// PORT: Java's `ExitCode` is kept as its numeric code.
// Java: com/google/devtools/build/lib/util/DetailedExitCode.java:31 DetailedExitCode
#[derive(Debug, Clone, PartialEq)]
pub struct DetailedExitCode {
    exit_code: i32,
    failure_detail: Option<FailureDetail>,
}

impl DetailedExitCode {
    /// Java `getExitCode().getNumericExitCode()`.
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:40 DetailedExitCode.getExitCode
    pub fn get_exit_code(&self) -> i32 {
        self.exit_code
    }

    /// Returns the registered exit code associated with a `FailureDetail` message.
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:45 DetailedExitCode.getExitCode
    pub fn get_exit_code_for(failure_detail: &FailureDetail) -> i32 {
        get_numeric_exit_code(failure_detail)
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:53 DetailedExitCode.getFailureDetail
    pub fn get_failure_detail(&self) -> Option<&FailureDetail> {
        self.failure_detail.as_ref()
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:58 DetailedExitCode.isSuccess
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }

    /// Returns a `DetailedExitCode` specifying success (i.e. exit code 0).
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:63 DetailedExitCode.success
    pub fn success() -> DetailedExitCode {
        DetailedExitCode {
            exit_code: 0,
            failure_detail: None,
        }
    }

    /// Returns a `DetailedExitCode` combining the provided `FailureDetail` and exit code.
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:78 DetailedExitCode.of
    pub fn of_exit_code(exit_code: i32, failure_detail: FailureDetail) -> DetailedExitCode {
        DetailedExitCode {
            exit_code,
            failure_detail: Some(failure_detail),
        }
    }

    /// Returns a `DetailedExitCode` whose exit code is chosen referencing `FailureDetail`'s
    /// metadata.
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:86 DetailedExitCode.of
    pub fn of(failure_detail: FailureDetail) -> DetailedExitCode {
        DetailedExitCode {
            exit_code: get_numeric_exit_code(&failure_detail),
            failure_detail: Some(failure_detail),
        }
    }
}

/// Java `DetailedExitCode.getNumericExitCode` for the query categories.
fn get_numeric_exit_code(failure_detail: &FailureDetail) -> i32 {
    match &failure_detail.category {
        Some(Category::Query(q)) => query_code_exit_code(q.code()),
        Some(Category::ActionQuery(q)) => action_query_code_exit_code(q.code()),
        Some(Category::ConfigurableQuery(q)) => configurable_query_code_exit_code(q.code()),
        Some(Category::PackageLoading(p)) => package_loading_code_exit_code(p.code),
        Some(Category::TargetPatterns(t)) => target_patterns_code_exit_code(t.code),
        // The generated FailureDetails metadata of every other category.
        _ => blaze_util::DetailedExitCode::get_numeric_exit_code(failure_detail),
    }
}

/// `(metadata).exit_code` of `failure_details.Query.Code`.
pub fn query_code_exit_code(code: query::Code) -> i32 {
    use query::Code::*;
    match code {
        QueryUnknown => 37,
        QueryFileWithCommandLineExpression => 2,
        QueryFileReadFailure => 2,
        CommandLineExpressionMissing => 2,
        OutputFormatInvalid => 2,
        GraphlessPrereqUnmet => 2,
        QueryOutputWriteFailure => 36,
        QueryStdoutFlushFailure => 36,
        AnalysisQueryPrereqUnmet => 2,
        QueryResultsFlushFailure => 36,
        DeprecatedUnclosedQuotationExpressionError => 2,
        VariableNameInvalid => 7,
        VariableUndefined => 7,
        BuildfilesAndLoadfilesCannotUseOutputLocationError => 2,
        BuildFileError => 7,
        Cycle => 7,
        UniqueSkykeyThresholdExceeded => 7,
        TargetNotInUniverseScope => 2,
        InvalidFullUniverseExpression => 7,
        UniverseScopeLimitExceeded => 7,
        InvalidationLimitExceeded => 7,
        OutputFormatPrereqUnmet => 2,
        ArgumentsMissing => 7,
        RbuildfilesFunctionRequiresSkyquery => 7,
        FullTargetsNotSupported => 7,
        DeprecatedUnexpectedTokenError => 2,
        DeprecatedIntegerLiteralMissing => 2,
        DeprecatedInvalidStartingCharacterError => 2,
        DeprecatedPrematureEndOfInputError => 2,
        SyntaxError => 2,
        OutputFormatterIoException => 36,
        SkyqueryTransitiveTargetError => 7,
        SkyqueryTargetException => 7,
        InvalidLabelInTestSuite => 7,
        IllegalFlagCombination => 2,
        NonDetailedError => 1,
    }
}

/// `(metadata).exit_code` of `failure_details.ActionQuery.Code`.
pub fn action_query_code_exit_code(code: action_query::Code) -> i32 {
    use action_query::Code::*;
    match code {
        ActionQueryUnknown => 37,
        CommandLineExpansionFailure => 2,
        OutputFailure => 6,
        CommandLineExpressionMissing => 2,
        ExpressionParseFailure => 2,
        SkyframeStateWithCommandLineExpression => 2,
        InvalidAqueryExpression => 1,
        SkyframeStatePrereqUnmet => 2,
        AqueryOutputTooBig => 7,
        IllegalPatternSyntax => 2,
        IncorrectArguments => 2,
        TopLevelTargetsWithSkyframeStateNotSupported => 2,
        SkyframeStateAfterExecution => 1,
        LabelsFunctionNotSupported => 2,
        TemplateExpansionFailure => 2,
    }
}

/// `(metadata).exit_code` of `failure_details.ConfigurableQuery.Code`.
pub fn configurable_query_code_exit_code(code: configurable_query::Code) -> i32 {
    use configurable_query::Code::*;
    match code {
        ConfigurableQueryUnknown => 37,
        CommandLineExpressionMissing => 2,
        ExpressionParseFailure => 2,
        FiltersNotSupported => 2,
        BuildfilesFunctionNotSupported => 2,
        SiblingsFunctionNotSupported => 2,
        VisibleFunctionNotSupported => 2,
        AttributeMissing => 2,
        IncorrectConfigArgumentError => 2,
        TargetMissing => 2,
        StarlarkSyntaxError => 2,
        StarlarkEvalError => 2,
        FormatFunctionError => 2,
    }
}

/// `(metadata).exit_code` of `failure_details.PackageLoading.Code`, by numeric code.
pub fn package_loading_code_exit_code(code: i32) -> i32 {
    match code {
        0 => 37,
        1..=4 => 1,
        5 | 6 => 36,
        7 | 9..=13 => 1,
        14 => 36,
        15..=30 => 1,
        31 => 36,
        32 | 33 => 1,
        _ => blaze_util::unported!(
            "DetailedExitCode.getNumericExitCode",
            "PackageLoading code {code}"
        ),
    }
}

/// `(metadata).exit_code` of `failure_details.TargetPatterns.Code`, by numeric code.
pub fn target_patterns_code_exit_code(code: i32) -> i32 {
    match code {
        0 => 37,
        1 | 2 => 2,
        3..=19 => 1,
        _ => blaze_util::unported!(
            "DetailedExitCode.getNumericExitCode",
            "TargetPatterns code {code}"
        ),
    }
}

impl std::hash::Hash for DetailedExitCode {
    /// Java `Objects.hash(exitCode, failureDetail)`; the failure detail hashes by its encoding
    /// (prost messages do not implement `Hash`). Needed to attach it to events as a property.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use blaze_protos::JavaMessage;
        self.exit_code.hash(state);
        self.failure_detail
            .as_ref()
            .map(|f| f.to_byte_array())
            .hash(state);
    }
}
