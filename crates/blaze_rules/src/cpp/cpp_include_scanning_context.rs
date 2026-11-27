//! `CppIncludeScanningContext`, the action context that does include scanning for
//! `CppCompileAction` (implemented by `includescanning::CppIncludeScanningContextImpl`).

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_exec::actions::ExecException;
use blaze_exec::exec::action_execution_context::{ActionExecutionContext, get_context};

use super::cpp_compile_action::CppCompileAction;
use super::include_scanner::IncludeScanningHeaderData;

/// Context for include scanning.
// Java: com/google/devtools/build/lib/rules/cpp/CppIncludeScanningContext.java:30 CppIncludeScanningContext
pub trait CppIncludeScanningContext: Send + Sync {
    /// Does include scanning to find the list of files needed to execute the action.
    ///
    /// Returns `None` if a skyframe restart is necessary.
    // Java: com/google/devtools/build/lib/rules/cpp/CppIncludeScanningContext.java:37 CppIncludeScanningContext.findAdditionalInputs
    fn find_additional_inputs(
        &self,
        action: &CppCompileAction,
        action_execution_context: &dyn ActionExecutionContext,
        include_scanning_header_data: &IncludeScanningHeaderData,
    ) -> Result<Option<Vec<Artifact>>, ExecException>;
}

/// `actionExecutionContext.getContext(CppIncludeScanningContext.class)`; `None` where Java's
/// `getContext` returns null.
pub fn get_cpp_include_scanning_context(
    ctx: &dyn ActionExecutionContext,
) -> Option<Arc<dyn CppIncludeScanningContext>> {
    get_context::<Arc<dyn CppIncludeScanningContext>>(ctx).map(|c| (*c).clone())
}
