//! `CppIncludeExtractionContext` (the action context `CppCompileAction.execute` reads the
//! `ArtifactResolver` from) and Bazel's standalone implementation of it.
//!
//! Also the seam for the build-language option `CppCompileAction` reads during execution
//! ([`CppExecutionOptions`]), until the execution layer's `ActionExecutionContext` exposes
//! `getOptions()`.

use std::any::TypeId;
use std::sync::Arc;

use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_exec::actions::action_execution::ActionContextRef;
use blaze_exec::exec::action_execution_context::{ActionExecutionContext, get_context};

/// Context for actions that do include scanning.
// Java: com/google/devtools/build/lib/rules/cpp/CppIncludeExtractionContext.java:20 CppIncludeExtractionContext
pub trait CppIncludeExtractionContext: Send + Sync {
    /// Returns the artifact resolver.
    // Java: com/google/devtools/build/lib/rules/cpp/CppIncludeExtractionContext.java:22 CppIncludeExtractionContext.getArtifactResolver
    fn get_artifact_resolver(&self) -> &ArtifactFactory;
}

/// An IncludeExtractionContext that does nothing. Since local execution does not need to
/// discover inclusion in advance, we do not need include scanning.
///
/// PORT: the Java class lives in `lib/standalone` (registered by `StandaloneModule`); it is here
/// because the trait it implements is this crate's. `env.getSkyframeBuildView()
/// .getArtifactFactory()` is passed in.
// Java: com/google/devtools/build/lib/standalone/DummyCppIncludeExtractionContext.java:24 DummyCppIncludeExtractionContext
pub struct DummyCppIncludeExtractionContext {
    artifact_factory: Arc<ArtifactFactory>,
}

impl DummyCppIncludeExtractionContext {
    // Java: com/google/devtools/build/lib/standalone/DummyCppIncludeExtractionContext.java:27 DummyCppIncludeExtractionContext.<init>
    pub fn new(artifact_factory: Arc<ArtifactFactory>) -> DummyCppIncludeExtractionContext {
        DummyCppIncludeExtractionContext { artifact_factory }
    }
}

impl CppIncludeExtractionContext for DummyCppIncludeExtractionContext {
    // Java: com/google/devtools/build/lib/standalone/DummyCppIncludeExtractionContext.java:32 DummyCppIncludeExtractionContext.getArtifactResolver
    fn get_artifact_resolver(&self) -> &ArtifactFactory {
        &self.artifact_factory
    }
}

/// The command options `CppCompileAction` reads through `actionExecutionContext.getOptions()`:
/// `getOptions(BuildLanguageOptions.class).experimentalSiblingRepositoryLayout`.
///
/// PORT: seam. The execution layer's `ActionExecutionContext` has no `getOptions()`; the
/// command registers this value as an action context (see [`cpp_action_contexts`]).
#[derive(Clone, Copy, Debug)]
pub struct CppExecutionOptions {
    /// `--experimental_sibling_repository_layout`.
    pub experimental_sibling_repository_layout: bool,
}

/// `actionExecutionContext.getContext(CppIncludeExtractionContext.class)`; `None` where Java's
/// `getContext` returns null.
pub fn get_cpp_include_extraction_context(
    ctx: &dyn ActionExecutionContext,
) -> Option<Arc<dyn CppIncludeExtractionContext>> {
    get_context::<Arc<dyn CppIncludeExtractionContext>>(ctx).map(|c| (*c).clone())
}

/// `actionExecutionContext.getOptions().getOptions(BuildLanguageOptions.class)
/// .experimentalSiblingRepositoryLayout`.
pub fn experimental_sibling_repository_layout(ctx: &dyn ActionExecutionContext) -> bool {
    match get_context::<CppExecutionOptions>(ctx) {
        Some(o) => o.experimental_sibling_repository_layout,
        None => panic!(
            "NullPointerException: getOptions(BuildLanguageOptions.class) (CppExecutionOptions \
             context not registered)"
        ),
    }
}

/// The action contexts the C++ actions look up, keyed as
/// [`blaze_exec::exec::action_execution_context::get_context`] finds them:
///
/// - `CppIncludeExtractionContext` → `DummyCppIncludeExtractionContext(artifactFactory)`
///   (`StandaloneModule`), as `Arc<Arc<dyn CppIncludeExtractionContext>>`;
/// - [`CppExecutionOptions`] (the options seam), as `Arc<CppExecutionOptions>`.
pub fn cpp_action_contexts(
    artifact_factory: Arc<ArtifactFactory>,
    experimental_sibling_repository_layout: bool,
) -> Vec<(TypeId, ActionContextRef)> {
    let extraction: Arc<dyn CppIncludeExtractionContext> =
        Arc::new(DummyCppIncludeExtractionContext::new(artifact_factory));
    vec![
        (
            TypeId::of::<Arc<dyn CppIncludeExtractionContext>>(),
            Arc::new(extraction) as ActionContextRef,
        ),
        (
            TypeId::of::<CppExecutionOptions>(),
            Arc::new(CppExecutionOptions {
                experimental_sibling_repository_layout,
            }) as ActionContextRef,
        ),
    ]
}
