//! Port of `com.google.devtools.build.lib.rules.cpp.IncludeProblems`.

use crate::actions::{ActionExecutionException, CppCompileCode};

/// Accumulator for problems encountered while reading or validating inclusion results.
// Java: com/google/devtools/build/lib/rules/cpp/IncludeProblems.java:28 IncludeProblems
#[derive(Debug, Default)]
pub struct IncludeProblems {
    problems: Option<String>, // null when no problems
}

impl IncludeProblems {
    pub fn new() -> IncludeProblems {
        IncludeProblems::default()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeProblems.java:32 IncludeProblems.add
    pub fn add(&mut self, included: &str) {
        let problems = self.problems.get_or_insert_with(String::new);
        problems.push_str("\n  '");
        problems.push_str(included);
        problems.push('\'');
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeProblems.java:39 IncludeProblems.hasProblems
    pub fn has_problems(&self) -> bool {
        self.problems.is_some()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeProblems.java:43 IncludeProblems.assertProblemFree
    pub fn assert_problem_free(&self, message: &str) -> Result<(), ActionExecutionException> {
        if let Some(problems) = &self.problems {
            let full_message = format!("{message}{problems}");
            return Err(ActionExecutionException {
                message: full_message,
                catastrophe: false,
                code: CppCompileCode::UNDECLARED_INCLUSIONS,
            });
        }
        Ok(())
    }
}
