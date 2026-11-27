//! Port of `lib/includescanning`: the include scanner of `CppCompileAction`
//! (`--experimental_unsupported_and_brittle_include_scanning` with the `cc_include_scanning`
//! feature), and the parts of `IncludeScanningModule` that register it.
//!
//! PORT: the package's Java lives outside `lib/rules/cpp`, but its classes implement this
//! crate's `CppIncludeScanningContext` and read `CppCompileAction`, so the module is here.
//!
//! Not ported:
//! - `SpawnIncludeScanner` (grep-includes run as a spawn, remote extraction of inclusions):
//!   reached only with a `_grep_includes` tool in `CcToolchainInfo`, which rules_cc's
//!   `cc_toolchain` never sets (`IncludeParser::extract_inclusions_from_file` is `unported!`
//!   for it).
//! - `IncludeHintsFunction`, `Hints.getRules` and `IncludeScanningSkyFunctions`: Bazel's
//!   `IncludeScanningModule.getIncludeHintsFilename()` returns null, so no hints file is read
//!   and the function is never registered.
//! - `SwigIncludeScanner`, `SwigIncludeParser` and `SwigIncludeScanningContextImpl`: no rule
//!   of Bazel's creates a swig action.
//! - The include pool (`--experimental_include_scanning_parallelism`,
//!   `--experimental_reuse_include_scanning_threads`): the scan runs in the action's thread;
//!   see `LegacyIncludeScanner`.

// PORT: skipped, reached only with a C++ toolchain whose CcToolchainInfo has a grep-includes tool (rules_cc's cc_toolchain has none; IncludeParser::extract_inclusions_from_file is unported! for it): SpawnIncludeScanner.SpawnIncludeScanner, SpawnIncludeScanner.setOutputService, SpawnIncludeScanner.setInMemoryOutput, SpawnIncludeScanner.getIncludesOutput, SpawnIncludeScanner.execPath, SpawnIncludeScanner.shouldParseRemotely, SpawnIncludeScanner.extractInclusions, SpawnIncludeScanner.spawnGrep, SpawnIncludeScanner.dump
// PORT: skipped, reached only with a C++ toolchain whose CcToolchainInfo has a grep-includes tool: SpawnIncludeScanner.GrepIncludesAction, SpawnIncludeScanner.GrepIncludesSpawn, IncludeParser.processIncludes
// PORT: skipped, Bazel's IncludeScanningModule.getIncludeHintsFilename() returns null, so the function is never registered: IncludeHintsFunction.IncludeHintsFunction, IncludeHintsFunction.compute, IncludeHintsFunction.createFailureDetail, IncludeHintsFunction.IncludeHintsFunctionException, IncludeScanningModule.getIncludeHintsFilename, IncludeScanningModule.workspaceInit, IncludeScanningModule.getSkyFunctions
// PORT: skipped, no rule of Bazel's creates a swig action: SwigIncludeParser.SwigIncludeParser, SwigIncludeParser.skipParentheses, SwigIncludeParser.getFileType, SwigIncludeParser.expectIncludeKeyword, IncludeScanningModule.SwigIncludeScanningContextImpl, IncludeScanningModule.IncludeScannerLifecycleManager.getSwigActionContext
// PORT: skipped, the include pool is not ported (the scan runs in the action's thread, see LegacyIncludeScanner): IncludeScanningModule.IncludeScannerLifecycleManager.executionPhaseEnding, IncludeScanningModule.useAsyncExecution, IncludeScanningModule.shouldShuffle, LegacyIncludeScanner.ExecRuntimeException, LegacyIncludeScanner.InterruptedRuntimeException
// PORT: skipped, the standalone DummyCppIncludeExtractionContext (crate::cpp::cpp_include_extraction_context) answers the same artifact resolver: IncludeScanningModule.CppIncludeExtractionContextImpl

pub mod cpp_include_scanning_context_impl;
pub mod include_parser;
pub mod inclusion_with_context;
pub mod legacy_include_scanner;

use std::any::TypeId;
use std::sync::Arc;

use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_exec::actions::action_execution::ActionContextRef;
use blaze_vfs::Path;

use crate::cpp::cpp_include_scanning_context::CppIncludeScanningContext;
use cpp_include_scanning_context_impl::{CppIncludeScanningContextImpl, IncludeScannerSupplier};
use include_parser::{Hints, HintsRules, IncludeParser};

/// The `CppIncludeScanningContext` of one command: `IncludeScanningModule.registerActionContexts`
/// with the `IncludeScannerLifecycleManager`'s supplier, created as `executorCreated` creates it
/// and initialized as `executionPhaseStarting` initializes it (without include hints:
/// `HintsRules.EMPTY`). Keyed as `blaze_exec::exec::action_execution_context::get_context`
/// finds it (`Arc<dyn CppIncludeScanningContext>`).
///
/// `output_path` is `directories.getOutputPath(execRoot.getBaseName())` (`<execroot>/bazel-out`).
// Java: com/google/devtools/build/lib/includescanning/IncludeScanningModule.java:96 IncludeScanningModule.registerActionContexts
// Java: com/google/devtools/build/lib/includescanning/IncludeScanningModule.java:112 IncludeScanningModule.executorInit
// Java: com/google/devtools/build/lib/includescanning/IncludeScanningModule.java:252 IncludeScannerLifecycleManager.IncludeScannerLifecycleManager
// Java: com/google/devtools/build/lib/includescanning/IncludeScanningModule.java:270 IncludeScannerLifecycleManager.getCppActionContext
// Java: com/google/devtools/build/lib/includescanning/IncludeScanningModule.java:326 IncludeScannerLifecycleManager.executorCreated
// Java: com/google/devtools/build/lib/includescanning/IncludeScanningModule.java:279 IncludeScannerLifecycleManager.executionPhaseStarting
pub fn include_scanning_context(
    exec_root: Path,
    output_path: Path,
    artifact_factory: Arc<ArtifactFactory>,
) -> (TypeId, ActionContextRef) {
    let include_scanner_supplier = Arc::new(IncludeScannerSupplier::new(
        output_path,
        exec_root,
        artifact_factory,
    ));
    include_scanner_supplier.init(IncludeParser::new(Some(Hints::new(HintsRules::empty()))));
    let context: Arc<dyn CppIncludeScanningContext> =
        Arc::new(CppIncludeScanningContextImpl::new(include_scanner_supplier));
    (
        TypeId::of::<Arc<dyn CppIncludeScanningContext>>(),
        Arc::new(context) as ActionContextRef,
    )
}
