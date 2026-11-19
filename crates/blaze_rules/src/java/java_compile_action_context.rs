//! `rules/java/JavaCompileActionContext.java` (context for compiling Java files: a cache of
//! parsed `.jdeps` files) and the `.jdeps` message of `src/main/protobuf/deps.proto`
//! (`com.google.devtools.build.lib.view.proto.Deps`).

use std::any::TypeId;
use std::io::Read;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_exec::actions::action_execution::ActionContextRef;
use blaze_exec::exec::action_execution_context::ActionExecutionContext;
use parking_lot::Mutex;
use prost::Message;

/// `blaze_deps.SourceLocation`.
// Java: src/main/protobuf/deps.proto SourceLocation
#[derive(Clone, PartialEq, prost::Message)]
pub struct SourceLocation {
    #[prost(bytes = "vec", required, tag = "1")]
    pub path: Vec<u8>,
    #[prost(int32, optional, tag = "2")]
    pub line: Option<i32>,
    #[prost(int32, optional, tag = "3")]
    pub column: Option<i32>,
}

/// `blaze_deps.Dependency`.
///
/// PORT: proto `string` fields are kept as bytes: Java decodes them as UTF-8 with replacement
/// (`getPath()`), where prost would reject invalid UTF-8; [`Dependency::get_path`] decodes as
/// Java does.
// Java: src/main/protobuf/deps.proto Dependency
#[derive(Clone, PartialEq, prost::Message)]
pub struct Dependency {
    #[prost(bytes = "vec", required, tag = "1")]
    pub path: Vec<u8>,
    #[prost(int32, required, tag = "2")]
    pub kind: i32,
    #[prost(message, repeated, tag = "3")]
    pub location: Vec<SourceLocation>,
}

impl Dependency {
    /// Java `getPath()`: the field decoded as UTF-8 (a Unicode string).
    pub fn get_path(&self) -> String {
        String::from_utf8_lossy(&self.path).into_owned()
    }
}

/// `blaze_deps.Dependencies`.
// Java: src/main/protobuf/deps.proto Dependencies
#[derive(Clone, PartialEq, prost::Message)]
pub struct Dependencies {
    #[prost(message, repeated, tag = "1")]
    pub dependency: Vec<Dependency>,
    #[prost(bytes = "vec", optional, tag = "2")]
    pub rule_label: Option<Vec<u8>>,
    #[prost(bool, optional, tag = "3")]
    pub success: Option<bool>,
    #[prost(bytes = "vec", repeated, tag = "4")]
    pub contained_package: Vec<Vec<u8>>,
    #[prost(bool, optional, tag = "5")]
    pub requires_reduced_classpath_fallback: Option<bool>,
}

impl Dependencies {
    /// Java `getRequiresReducedClasspathFallback()`.
    pub fn get_requires_reduced_classpath_fallback(&self) -> bool {
        self.requires_reduced_classpath_fallback.unwrap_or(false)
    }

    /// Java `Deps.Dependencies.parseFrom(bytes)`.
    pub fn parse_from(bytes: &[u8]) -> std::io::Result<Dependencies> {
        Dependencies::decode(bytes).map_err(|e| {
            // Java: InvalidProtocolBufferException extends IOException.
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })
    }
}

/// Context for compiling Java files.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionContext.java:28 JavaCompileActionContext
#[derive(Default)]
pub struct JavaCompileActionContext {
    cache: Mutex<std::collections::HashMap<Artifact, Arc<Dependencies>>>,
}

impl JavaCompileActionContext {
    pub fn new() -> JavaCompileActionContext {
        JavaCompileActionContext::default()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionContext.java:31 JavaCompileActionContext.getDependencies
    pub fn get_dependencies(
        &self,
        jdeps_file: &Artifact,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> std::io::Result<Arc<Dependencies>> {
        // TODO(djasper): Investigate caching across builds.
        // The cache value computation is potentially expensive, e.g. when we have to download an
        // input with actionFS, so we are explicitly not using computeIfAbsent here. The downside
        // is that potentially we parse the same jdepsFile twice, but at least we are not
        // blocking all other threads on the lock for the cache.
        if let Some(deps) = self.cache.lock().get(jdeps_file) {
            return Ok(deps.clone());
        }
        let path =
            blaze_exec::analysis::actions::get_input_path(action_execution_context, jdeps_file);
        let mut bytes = Vec::new();
        path.get_input_stream()?.read_to_end(&mut bytes)?;
        let deps = Arc::new(Dependencies::parse_from(&bytes)?);
        self.cache
            .lock()
            .entry(jdeps_file.clone())
            .or_insert_with(|| deps.clone());
        Ok(deps)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileActionContext.java:46 JavaCompileActionContext.insertDependencies
    pub fn insert_dependencies(&self, jdeps_file: &Artifact, dependencies: Arc<Dependencies>) {
        self.cache.lock().insert(jdeps_file.clone(), dependencies);
    }
}

/// `registryBuilder.register(JavaCompileActionContext.class, new JavaCompileActionContext())`,
/// keyed as `blaze_exec::exec::action_execution_context::get_context` finds it.
// Java: com/google/devtools/build/lib/bazel/rules/BazelRulesModule.java:979 BazelRulesModule.BazelRulesModule.registerActionContexts
pub fn java_action_context() -> (TypeId, ActionContextRef) {
    (
        TypeId::of::<JavaCompileActionContext>(),
        Arc::new(JavaCompileActionContext::new()) as ActionContextRef,
    )
}
