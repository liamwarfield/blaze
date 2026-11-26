//! Execution of `lib/analysis/actions/BuildInfoFileWriteAction.java` (`TranslateBuildInfo`).

use std::io::Read;

use blaze_actions::actions::action::Action;
use blaze_actions::analysis::actions::build_info_file_write_action::{
    BuildInfoFileWriteAction, TranslateError,
};
use blaze_actions::analysis::actions::substitution::Substitution;
use blaze_actions::analysis::actions::template::Template;
use blaze_actions::analysis::actions::template_expansion_action::TemplateExpansionContextTemplateMetadata;
use blaze_actions::analysis::workspace_status_action::parse_values_in_insertion_order;
use blaze_protos::failure_details::execution::Code;
use blaze_util::profiler::java_hash_order::{Insertion, java_hash_map_order, string_hash_code};

use super::template_expansion_action::execute_template_expansion;
use super::{execution_category, failure_detail, get_input_path};
use crate::actions::{ActionExecutionException, ActionResult, ExecutableAction};
use crate::exec::action_execution_context::ActionExecutionContext;

impl ExecutableAction for BuildInfoFileWriteAction {
    /// Parses the workspace status file (`WorkspaceStatusAction.parseValues`), calls the Starlark
    /// translation callback with the values and expands the template with the returned dict
    /// (`TemplateExpansionAction.execute`).
    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:97 BuildInfoFileWriteAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        // Parse values from text file.
        let value_file = self
            .get_primary_input()
            .unwrap_or_else(|| panic!("NullPointerException: getPrimaryInput"));
        let parsed = (|| -> std::io::Result<Vec<(String, String)>> {
            let mut bytes = Vec::new();
            get_input_path(ctx, &value_file)
                .get_input_stream()?
                .read_to_end(&mut bytes)?;
            Ok(parse_values_in_insertion_order(&bytes))
        })();
        let parsed = match parsed {
            Ok(p) => p,
            Err(e) => {
                // Java: "Failed to parse workspace status: " + e.getMessage()
                let message = format!("Failed to parse workspace status: {e}");
                return Err(ActionExecutionException::with_cause(
                    message.clone(),
                    &super::io_exception_to_string(&e),
                    Some(self),
                    /* catastrophe= */ false,
                    Some(failure_detail(
                        &message,
                        execution_category(Code::SourceInputIoException),
                    )),
                ));
            }
        };
        let values = java_hash_map_iteration_order(parsed);

        // Call Starlark callback function which takes workspace status file's content as an
        // input and produces a dict which is written to the output.
        let substitution_dict = match self.call_translation_callback(&values) {
            Ok(d) => d,
            Err(TranslateError::Callback(e)) => {
                let message = format!(
                    "Error during translating {} status file : {}",
                    if self.is_volatile_status() {
                        "volatile"
                    } else {
                        "stable"
                    },
                    e
                );
                return Err(non_action_failure(self, message, &e));
            }
            Err(TranslateError::NotStringDict(e)) => {
                let message = format!(
                    "BuildInfo translation callback function is expected to return dict of \
                     strings to strings, could not convert return value to Java type: {e}"
                );
                return Err(non_action_failure(self, message, &e));
            }
            Err(TranslateError::Interrupted) => {
                return Err(ActionExecutionException::new(
                    "interrupted",
                    Some(self),
                    /* catastrophe= */ false,
                    Some(failure_detail(
                        "interrupted",
                        execution_category(Code::NonActionExecutionFailure),
                    )),
                ));
            }
        };
        let substitution_list: Vec<Substitution> = substitution_dict
            .into_iter()
            .map(|(k, v)| Substitution::of(k, v))
            .collect();
        execute_template_expansion(
            ctx,
            self,
            &TemplateExpansionContextTemplateMetadata {
                template: Template::for_artifact(self.get_template().clone()),
                primary_output: self.get_primary_output(),
                substitutions: substitution_list,
                make_executable: false,
            },
        )
    }
}

/// `new ActionExecutionException(message, e, this, false,
/// DetailedExitCode(Execution.NON_ACTION_EXECUTION_FAILURE))`.
fn non_action_failure(
    action: &BuildInfoFileWriteAction,
    message: String,
    cause: &str,
) -> ActionExecutionException {
    ActionExecutionException::with_cause(
        message.clone(),
        &cause,
        Some(action),
        /* catastrophe= */ false,
        Some(failure_detail(
            &message,
            execution_category(Code::NonActionExecutionFailure),
        )),
    )
}

/// The iteration order of `values` in Java: `parseValues` puts the entries into a `HashMap`
/// (`new HashMap<>()`, keys in file order), `ImmutableMap.copyOf` keeps that order, and
/// `execute` copies them with `putAll` into another `new HashMap<>()`, which presizes its table
/// to `tableSizeFor((int) (n / 0.75f + 1))` and so may iterate in another order; `Dict
/// .immutableCopyOf` keeps the second map's order.
///
/// PORT: java-hash-order. Treeified buckets (8 or more colliding keys) are not modelled.
fn java_hash_map_iteration_order(entries: Vec<(String, String)>) -> Vec<(String, String)> {
    let n = entries.len();
    if n == 0 {
        return entries;
    }
    let hash = |(k, _): &(String, String)| string_hash_code(k);
    let first = java_hash_map_order(&entries, hash, Insertion::Put);
    // HashMap.putAll on an empty map: threshold = tableSizeFor((int) (s / loadFactor + 1.0F)),
    // and the first put allocates a table of that size.
    let t = (n as f32 / 0.75f32 + 1.0f32) as usize;
    let capacity = t.max(1).next_power_of_two();
    let mut keyed: Vec<(usize, usize, (String, String))> = first
        .into_iter()
        .enumerate()
        .map(|(i, e)| {
            let h = string_hash_code(&e.0);
            let spread = h ^ ((h as u32) >> 16) as i32;
            ((spread as u32 as usize) & (capacity - 1), i, e)
        })
        .collect();
    keyed.sort_by_key(|(bucket, i, _)| (*bucket, *i));
    keyed.into_iter().map(|(_, _, e)| e).collect()
}
