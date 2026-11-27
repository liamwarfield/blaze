//! `lib/util/ExitCode` and `lib/util/DetailedExitCode`: the one shared copy.
//!
//! PORT: copies still to migrate to this module: blaze_runtime `exit_code`,
//! blaze_query_engine `detailed_exit_code`, blaze_packages `exceptions` (its DetailedExitCode),
//! blaze_options `rc::option_processor` (the client's `blaze_exit_code::ExitCode`, a different
//! C++ enum).

use std::fmt;

use blaze_protos::failure_details::FailureDetail;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/failure_detail_exit_codes.rs"));
}

/// An exit code for Blaze.
// Java: com/google/devtools/build/lib/util/ExitCode.java:38 ExitCode
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExitCode {
    numeric_exit_code: i32,
    name: &'static str,
    infrastructure_failure: bool,
}

impl ExitCode {
    const fn create(code: i32, name: &'static str) -> ExitCode {
        ExitCode {
            numeric_exit_code: code,
            name,
            infrastructure_failure: false,
        }
    }

    const fn create_infrastructure_failure(code: i32, name: &'static str) -> ExitCode {
        ExitCode {
            numeric_exit_code: code,
            name,
            infrastructure_failure: true,
        }
    }

    // Java: com/google/devtools/build/lib/util/ExitCode.java:42 ExitCode.SUCCESS
    pub const SUCCESS: ExitCode = ExitCode::create(0, "SUCCESS");
    pub const BUILD_FAILURE: ExitCode = ExitCode::create(1, "BUILD_FAILURE");
    /// Unregistered.
    pub const PARSING_FAILURE: ExitCode = ExitCode::create(1, "PARSING_FAILURE");
    pub const COMMAND_LINE_ERROR: ExitCode = ExitCode::create(2, "COMMAND_LINE_ERROR");
    pub const TESTS_FAILED: ExitCode = ExitCode::create(3, "TESTS_FAILED");
    /// Unregistered.
    pub const PARTIAL_ANALYSIS_FAILURE: ExitCode = ExitCode::create(3, "PARTIAL_ANALYSIS_FAILURE");
    pub const NO_TESTS_FOUND: ExitCode = ExitCode::create(4, "NO_TESTS_FOUND");
    pub const RUN_FAILURE: ExitCode = ExitCode::create(6, "RUN_FAILURE");
    pub const ANALYSIS_FAILURE: ExitCode = ExitCode::create(7, "ANALYSIS_FAILURE");
    pub const INTERRUPTED: ExitCode = ExitCode::create(8, "INTERRUPTED");
    pub const LOCK_HELD_NOBLOCK_FOR_LOCK: ExitCode =
        ExitCode::create(9, "LOCK_HELD_NOBLOCK_FOR_LOCK");
    pub const REMOTE_ENVIRONMENTAL_ERROR: ExitCode =
        ExitCode::create_infrastructure_failure(32, "REMOTE_ENVIRONMENTAL_ERROR");
    pub const OOM_ERROR: ExitCode = ExitCode::create_infrastructure_failure(33, "OOM_ERROR");
    pub const REMOTE_ERROR: ExitCode = ExitCode::create_infrastructure_failure(34, "REMOTE_ERROR");
    pub const LOCAL_ENVIRONMENTAL_ERROR: ExitCode =
        ExitCode::create_infrastructure_failure(36, "LOCAL_ENVIRONMENTAL_ERROR");
    pub const BLAZE_INTERNAL_ERROR: ExitCode =
        ExitCode::create_infrastructure_failure(37, "BLAZE_INTERNAL_ERROR");
    pub const TRANSIENT_BUILD_EVENT_SERVICE_UPLOAD_ERROR: ExitCode =
        ExitCode::create_infrastructure_failure(38, "PUBLISH_ERROR");
    pub const REMOTE_CACHE_EVICTED: ExitCode =
        ExitCode::create_infrastructure_failure(39, "REMOTE_CACHE_EVICTED");
    pub const PERSISTENT_BUILD_EVENT_SERVICE_UPLOAD_ERROR: ExitCode =
        ExitCode::create(45, "PERSISTENT_BUILD_EVENT_SERVICE_UPLOAD_ERROR");
    pub const EXTERNAL_DEPS_ERROR: ExitCode = ExitCode::create(48, "EXTERNAL_DEPS_ERROR");

    /// The registered exit codes (`exitCodeRegistry`).
    const REGISTERED: [ExitCode; 18] = [
        ExitCode::SUCCESS,
        ExitCode::BUILD_FAILURE,
        ExitCode::COMMAND_LINE_ERROR,
        ExitCode::TESTS_FAILED,
        ExitCode::NO_TESTS_FOUND,
        ExitCode::RUN_FAILURE,
        ExitCode::ANALYSIS_FAILURE,
        ExitCode::INTERRUPTED,
        ExitCode::LOCK_HELD_NOBLOCK_FOR_LOCK,
        ExitCode::REMOTE_ENVIRONMENTAL_ERROR,
        ExitCode::OOM_ERROR,
        ExitCode::REMOTE_ERROR,
        ExitCode::LOCAL_ENVIRONMENTAL_ERROR,
        ExitCode::BLAZE_INTERNAL_ERROR,
        ExitCode::TRANSIENT_BUILD_EVENT_SERVICE_UPLOAD_ERROR,
        ExitCode::REMOTE_CACHE_EVICTED,
        ExitCode::PERSISTENT_BUILD_EVENT_SERVICE_UPLOAD_ERROR,
        ExitCode::EXTERNAL_DEPS_ERROR,
    ];

    // Java: com/google/devtools/build/lib/util/ExitCode.java:138 ExitCode.forCode
    pub fn for_code(code: i32) -> Option<ExitCode> {
        ExitCode::REGISTERED
            .iter()
            .copied()
            .find(|e| e.numeric_exit_code == code)
    }

    // Java: com/google/devtools/build/lib/util/ExitCode.java:190 ExitCode.getNumericExitCode
    pub fn get_numeric_exit_code(&self) -> i32 {
        self.numeric_exit_code
    }

    // Java: com/google/devtools/build/lib/util/ExitCode.java:197 ExitCode.name
    pub fn name(&self) -> &'static str {
        self.name
    }

    // Java: com/google/devtools/build/lib/util/ExitCode.java:205 ExitCode.isInfrastructureFailure
    pub fn is_infrastructure_failure(&self) -> bool {
        self.infrastructure_failure
    }
}

// PORT: skipped, the registry is the constant table above (no other module registers a code): ExitCode.create, ExitCode.createInfrastructureFailure, ExitCode.createUnregistered
// PORT: skipped, the registry is the constant table above (no other module registers a code): ExitCode.register, ExitCode.values
// PORT: skipped, derived (PartialEq, Eq, Hash over the same three fields): ExitCode.hashCode, ExitCode.equals
// Java: com/google/devtools/build/lib/util/ExitCode.java:182 ExitCode.toString
impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

/// An [`ExitCode`] and its optional [`FailureDetail`].
// Java: com/google/devtools/build/lib/util/DetailedExitCode.java:31 DetailedExitCode
#[derive(Clone, Debug, PartialEq)]
pub struct DetailedExitCode {
    exit_code: ExitCode,
    failure_detail: Option<FailureDetail>,
}

impl DetailedExitCode {
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:40 DetailedExitCode.getExitCode
    pub fn get_exit_code(&self) -> ExitCode {
        self.exit_code
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:45 DetailedExitCode.getExitCode
    pub fn get_exit_code_of(failure_detail: &FailureDetail) -> ExitCode {
        let numeric_exit_code = Self::get_numeric_exit_code(failure_detail);
        ExitCode::for_code(numeric_exit_code)
            .unwrap_or_else(|| panic!("No ExitCode for numericExitCode {}", numeric_exit_code))
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:54 DetailedExitCode.getFailureDetail
    pub fn get_failure_detail(&self) -> Option<&FailureDetail> {
        self.failure_detail.as_ref()
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:58 DetailedExitCode.isSuccess
    pub fn is_success(&self) -> bool {
        self.exit_code == ExitCode::SUCCESS
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:63 DetailedExitCode.success
    pub fn success() -> DetailedExitCode {
        DetailedExitCode {
            exit_code: ExitCode::SUCCESS,
            failure_detail: None,
        }
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:78 DetailedExitCode.of
    pub fn of_exit_code(exit_code: ExitCode, failure_detail: FailureDetail) -> DetailedExitCode {
        DetailedExitCode {
            exit_code,
            failure_detail: Some(failure_detail),
        }
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:86 DetailedExitCode.of
    pub fn of(failure_detail: FailureDetail) -> DetailedExitCode {
        DetailedExitCode {
            exit_code: Self::get_exit_code_of(&failure_detail),
            failure_detail: Some(failure_detail),
        }
    }

    /// `DetailedExitCode.of(FailureDetail.newBuilder().setMessage(message).set<Category>(
    /// <Category>.newBuilder().setCode(<Category>.Code.<code>)))`, with the category named by its
    /// proto message type (`"ExternalRepository"`) and the code by its enum value name; `None` for
    /// `code` is the category's default instance (code 0). `None` when either name is unknown.
    ///
    /// PORT: crates that keep a reduced `DetailedExitCode` (blaze_packages: message plus
    /// `<Category>.<CODE>`) rebuild Java's object with this; the table is generated from
    /// failure_details.proto, so every category is known.
    pub fn of_category_name(
        message: impl Into<String>,
        category: &str,
        code: Option<&str>,
    ) -> Option<DetailedExitCode> {
        let category = generated::category_of_name(category, code)?;
        Some(Self::of(FailureDetail {
            message: message.into(),
            category: Some(category),
        }))
    }

    /// [`DetailedExitCode::of_category_name`] with the code given by its number.
    pub fn of_category_value(
        message: impl Into<String>,
        category: &str,
        code: i32,
    ) -> Option<DetailedExitCode> {
        let category = generated::category_of_value(category, code)?;
        Some(Self::of(FailureDetail {
            message: message.into(),
            category: Some(category),
        }))
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:115 DetailedExitCode.getNumericExitCode
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:126 DetailedExitCode.getNumericExitCode
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:138 DetailedExitCode.getCategorySubmessage
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:156 DetailedExitCode.isCategoryField
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:165 DetailedExitCode.getSubcategoryDescriptor
    // PORT: the category submessage and its enum field #1 come from the generated table
    // (`generated::category_number_and_code`), not from proto reflection.
    pub fn get_numeric_exit_code(failure_detail: &FailureDetail) -> i32 {
        let category = failure_detail.category.as_ref().unwrap_or_else(|| {
            panic!(
                "FailureDetail missing category submessage: {:?}",
                failure_detail
            )
        });
        let (number, code) = generated::category_number_and_code(category);
        match generated::metadata_exit_code(number, code) {
            Some(e) => e as i32,
            None => panic!(
                "IllegalArgumentException: Enum value {} of category {} has no FailureDetails.metadata",
                code, number
            ),
        }
    }

    /// `DetailedExitCodeComparator.chooseMoreImportantWithFirstIfTie` (Java accepts nulls).
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:190 DetailedExitCodeComparator.chooseMoreImportantWithFirstIfTie
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:195 DetailedExitCode.DetailedExitCodeComparator.compare
    pub fn choose_more_important_with_first_if_tie(
        first: Option<DetailedExitCode>,
        second: Option<DetailedExitCode>,
    ) -> Option<DetailedExitCode> {
        if Self::get_priority(first.as_ref()) - Self::get_priority(second.as_ref()) >= 0 {
            first
        } else {
            second
        }
    }

    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:201 DetailedExitCodeComparator.getPriority
    fn get_priority(code: Option<&DetailedExitCode>) -> i32 {
        match code {
            None => 0,
            Some(code) => {
                let code_class = if code.get_exit_code().is_infrastructure_failure() {
                    4
                } else {
                    2
                };
                code_class
                    + if code.get_failure_detail().is_some() {
                        1
                    } else {
                        0
                    }
            }
        }
    }
}

// PORT: skipped, derived PartialEq over exit code and failure detail (no Rust caller hashes one): DetailedExitCode.hashCode, DetailedExitCode.equals
impl fmt::Display for DetailedExitCode {
    // Java: com/google/devtools/build/lib/util/DetailedExitCode.java:109 DetailedExitCode.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DetailedExitCode{{exitCode={}, failureDetail={:?}}}",
            self.exit_code, self.failure_detail
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_protos::failure_details::{self, failure_detail::Category};

    fn info_failure(code: failure_details::info_command::Code) -> FailureDetail {
        FailureDetail {
            message: "m".to_string(),
            category: Some(Category::InfoCommand(failure_details::InfoCommand {
                code: code as i32,
            })),
        }
    }

    /// Values from failure_details.proto's `(metadata) = { exit_code: N }` options.
    #[test]
    fn numeric_exit_code_from_proto_metadata() {
        use failure_details::info_command::Code;
        assert_eq!(
            DetailedExitCode::get_numeric_exit_code(&info_failure(Code::KeyNotRecognized)),
            2
        );
        assert_eq!(
            DetailedExitCode::get_numeric_exit_code(&info_failure(Code::AllInfoWriteFailure)),
            36
        );
        let interrupted = FailureDetail {
            message: String::new(),
            category: Some(Category::Interrupted(failure_details::Interrupted {
                code: failure_details::interrupted::Code::Interrupted as i32,
            })),
        };
        assert_eq!(DetailedExitCode::get_numeric_exit_code(&interrupted), 8);
        assert_eq!(
            DetailedExitCode::of(info_failure(Code::KeyNotRecognized)).get_exit_code(),
            ExitCode::COMMAND_LINE_ERROR
        );
    }

    #[test]
    fn choose_more_important_with_first_if_tie() {
        use failure_details::info_command::Code;
        let a = DetailedExitCode::of(info_failure(Code::KeyNotRecognized));
        let b = DetailedExitCode::of(info_failure(Code::TooManyKeys));
        let c = DetailedExitCode::of(info_failure(Code::AllInfoWriteFailure));
        assert_eq!(
            DetailedExitCode::choose_more_important_with_first_if_tie(Some(a.clone()), Some(b)),
            Some(a.clone())
        );
        assert_eq!(
            DetailedExitCode::choose_more_important_with_first_if_tie(Some(a), Some(c.clone())),
            Some(c)
        );
        assert_eq!(
            DetailedExitCode::choose_more_important_with_first_if_tie(
                None,
                Some(DetailedExitCode::success())
            ),
            Some(DetailedExitCode::success())
        );
    }
}
