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
        // getExpandedTemplateUnsafe(template, substitutions, ctx.getPathResolver()): an
        // artifact template is read at `resolver.toPath(templateArtifact)`, on the action file
        // system when there is one, whose `getInputStream` downloads a remote-only template
        // first and records it as a lost input when its blob is gone
        // (RemoteActionFileSystem.java:380-393).
        // PORT: `Template::get_content` (blaze_actions) takes no resolver; the artifact template
        // is read here through the resolved path and expanded as a string template.
        // Java: com/google/devtools/build/lib/analysis/actions/LocalTemplateExpansionStrategy.java:77 LocalTemplateExpansionStrategy.getExpandedTemplateUnsafe
        let resolved_template = match &template_metadata.template {
            blaze_actions::analysis::actions::template::Template::Artifact(template_artifact) => {
                match get_artifact_template_content(&super::get_input_path(ctx, template_artifact))
                {
                    Ok(content) => {
                        Some(blaze_actions::analysis::actions::template::Template::String(content))
                    }
                    Err(e) => {
                        // catch (IOException | EvalException e): new
                        // EnvironmentalExecException(e, FailureDetail(Execution
                        // .LOCAL_TEMPLATE_EXPANSION_FAILURE))
                        return Err(ExecException::from(EnvironmentalExecException::with_cause(
                            &e.to_string(),
                            failure_detail(
                                "",
                                execution_category(Code::LocalTemplateExpansionFailure),
                            ),
                        ))
                        .into());
                    }
                }
            }
            _ => None,
        };
        let expanded_template = match get_expanded_template_unsafe(
            resolved_template
                .as_ref()
                .unwrap_or(&template_metadata.template),
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
        // `expandedTemplate.getBytes(ISO_8859_1)`: the bytes of the internal string.
        // Seam (decisions.md 2026-11-18 "Strings"): `internal_string_bytes` gives them whether
        // or not Blaze's strings are internal yet (`STRINGS_ARE_INTERNAL`).
        let deterministic_writer = BytesWriter(
            blaze_util::string_encoding::internal_string_bytes(&expanded_template).into_owned(),
        );
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

/// `ArtifactTemplate.getContent(resolver)` with the template's resolved path: its content read as
/// Latin-1 (`FileSystemUtils.readContent(templatePath, ISO_8859_1)`, through the path's
/// `getInputStream`), or the `IOException` "failed to load template file '<path>' due to I/O
/// error: <message>".
// Java: com/google/devtools/build/lib/analysis/actions/Template.java:103 ArtifactTemplate.getContent
fn get_artifact_template_content(template_path: &blaze_vfs::Path) -> io::Result<String> {
    let read = || -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut template_path.get_input_stream()?, &mut bytes)?;
        Ok(bytes)
    };
    match read() {
        // Bazel's internal encoding for strings is raw bytes as Latin-1.
        // PORT: as `Template::get_content`: until the decoding switch
        // (`string_encoding::STRINGS_ARE_INTERNAL`) a template that is not valid UTF-8 has no
        // string.
        Ok(bytes) => match blaze_util::string_encoding::internal_string_from_bytes(&bytes) {
            Ok(s) => Ok(s.into_owned()),
            Err(_) => blaze_util::unported!(
                "ArtifactTemplate.getContent",
                "template '{}' is not valid UTF-8",
                template_path.get_path_string()
            ),
        },
        Err(e) => Err(io::Error::new(
            e.kind(),
            format!(
                "failed to load template file '{}' due to I/O error: {}",
                template_path.get_path_string(),
                e
            ),
        )),
    }
}
