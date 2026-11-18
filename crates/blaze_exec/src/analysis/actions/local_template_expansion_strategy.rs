//! Ports of `lib/analysis/actions/TemplateExpansionContext.java` and
//! `lib/analysis/actions/LocalTemplateExpansionStrategy.java`.

use std::io::{self, Write};

use blaze_actions::actions::action::Action;
use blaze_actions::analysis::actions::abstract_file_write_action::DeterministicWriter;
use blaze_actions::analysis::actions::template_expansion_action::{
    TemplateExpansionContextTemplateMetadata, TemplateExpansionError, get_expanded_template_unsafe,
};
use blaze_protos::failure_details::execution::Code;

use super::file_write_action_context::FileWriteActionContext;
use super::{execution_category, failure_detail};
use crate::actions::{EnvironmentalExecException, ExecException, SpawnResult};
use crate::exec::action_execution_context::ActionExecutionContext;

/// The checked exceptions of `TemplateExpansionContext.expandTemplate`: `EvalException`
/// (`InterruptedException` is [`ExecException::Interrupted`]).
#[derive(Debug)]
pub enum ExpandTemplateError {
    /// `EvalException` (its message).
    Eval(String),
    Exec(ExecException),
}

impl From<ExecException> for ExpandTemplateError {
    fn from(e: ExecException) -> Self {
        ExpandTemplateError::Exec(e)
    }
}

/// The action context for `TemplateExpansionAction` instances.
///
/// Register as `Arc<dyn TemplateExpansionContext>` (see [`super::default_action_contexts`]).
// Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionContext.java:27 TemplateExpansionContext
pub trait TemplateExpansionContext: Send + Sync {
    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionContext.java:62 TemplateExpansionContext.expandTemplate
    fn expand_template(
        &self,
        action: &dyn Action,
        ctx: &dyn ActionExecutionContext,
        template_metadata: &TemplateExpansionContextTemplateMetadata,
    ) -> Result<Vec<SpawnResult>, ExpandTemplateError>;
}

/// Strategy to perform template expansion locally.
// Java: com/google/devtools/build/lib/analysis/actions/LocalTemplateExpansionStrategy.java:35 LocalTemplateExpansionStrategy
#[derive(Debug, Default)]
pub struct LocalTemplateExpansionStrategy;

impl LocalTemplateExpansionStrategy {
    pub fn new() -> LocalTemplateExpansionStrategy {
        LocalTemplateExpansionStrategy
    }
}

struct BytesWriter(Vec<u8>);

impl DeterministicWriter for BytesWriter {
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&self.0)
    }
}

impl TemplateExpansionContext for LocalTemplateExpansionStrategy {
    // Java: com/google/devtools/build/lib/analysis/actions/LocalTemplateExpansionStrategy.java:42 LocalTemplateExpansionStrategy.expandTemplate
    fn expand_template(
        &self,
        action: &dyn Action,
        ctx: &dyn ActionExecutionContext,
        template_metadata: &TemplateExpansionContextTemplateMetadata,
    ) -> Result<Vec<SpawnResult>, ExpandTemplateError> {
        // PORT: Java passes `ctx.getPathResolver()`; without an action file system it is the
        // identity resolver (`Artifact.getPath()`), which is what `Template::get_content` reads.
        if ctx.get_action_file_system().is_some() {
            blaze_util::unported!(
                "LocalTemplateExpansionStrategy.expandTemplate (action file system)"
            );
        }
        let expanded_template = match get_expanded_template_unsafe(
            &template_metadata.template,
            &template_metadata.substitutions,
        ) {
            Ok(s) => s,
            Err(e) => {
                let cause = match &e {
                    TemplateExpansionError::Io(io) => io.to_string(),
                    TemplateExpansionError::Eval(m) => m.clone(),
                };
                // new EnvironmentalExecException(e, FailureDetail(Execution.LOCAL_TEMPLATE_EXPANSION_FAILURE))
                return Err(ExecException::from(EnvironmentalExecException::with_cause(
                    &cause,
                    failure_detail("", execution_category(Code::LocalTemplateExpansionFailure)),
                ))
                .into());
            }
        };
        // PORT: StringEncoding identity (`expandedTemplate.getBytes(ISO_8859_1)` of the internal
        // string is its UTF-8 bytes).
        let deterministic_writer = BytesWriter(expanded_template.into_bytes());
        let context = super::get_required_context::<dyn FileWriteActionContext>(
            ctx,
            "FileWriteActionContext",
        );
        Ok(context.write_output_to_file(
            action,
            ctx,
            &deterministic_writer,
            template_metadata.make_executable,
            /* is_remotable= */ true,
        )?)
    }
}
