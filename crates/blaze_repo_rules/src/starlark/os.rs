//! `repository_os`: information about the system Blaze runs on (`repository_ctx.os`,
//! `module_ctx.os`).
//!
//! Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use allocative::Allocative;
use blaze_starlark::structs::DeeplyImmutable;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Demand;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::AllocDict;
use starlark::values::starlark_value;
use starlark_map::StarlarkHasher;

/// A Starlark structure to deliver information about the system we are running on.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:31 StarlarkOS
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkOS {
    #[allocative(skip)]
    repo_env: Arc<IndexMap<String, String>>,
}

starlark_simple_value!(StarlarkOS);

impl StarlarkOS {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:35 StarlarkOS.<init>
    pub fn new(repo_env: Arc<IndexMap<String, String>>) -> Self {
        StarlarkOS { repo_env }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:56 StarlarkOS.getEnvironmentVariables
    pub fn get_environment_variables(&self) -> &Arc<IndexMap<String, String>> {
        &self.repo_env
    }

    /// Java `System.getProperty("os.name").toLowerCase(Locale.ROOT)`.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:68 StarlarkOS.getName
    pub fn get_name(&self) -> String {
        // PORT: the JVM's "os.name" is "Linux" on Linux; Blaze targets Linux only.
        match std::env::consts::OS {
            "linux" => "linux".to_owned(),
            other => blaze_util::unported!("StarlarkOS.getName", "os.name for {other}"),
        }
    }

    /// Java `System.getProperty("os.arch").toLowerCase(Locale.ROOT)`.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:80 StarlarkOS.getArch
    pub fn get_arch(&self) -> String {
        // PORT: the JVM reports "amd64" on x86-64 Linux and "aarch64" on 64-bit ARM Linux.
        match std::env::consts::ARCH {
            "x86_64" => "amd64".to_owned(),
            "aarch64" => "aarch64".to_owned(),
            other => blaze_util::unported!("StarlarkOS.getArch", "os.arch for {other}"),
        }
    }
}

impl fmt::Display for StarlarkOS {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (default)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.bazel.repository.starlark.StarlarkOS>",
        )
    }
}

#[starlark_value(type = "repository_os")]
impl<'v> StarlarkValue<'v> for StarlarkOS {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("repository_os", starlark_os_methods);
        Some(RES.methods())
    }

    /// Java: one `StarlarkOS` object per context, compared by identity; Blaze allocates a new
    /// value on each `ctx.os` access, so values of the same context compare equal.
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<StarlarkOS>()
            .is_some_and(|o| Arc::ptr_eq(&o.repo_env, &self.repo_env)))
    }

    // Java: isImmutable() is true, so the object is hashable (by identity).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        (Arc::as_ptr(&self.repo_env) as usize).hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn starlark_os_methods(builder: &mut MethodsBuilder) {
    /// The dictionary of environment variables.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:56 StarlarkOS.getEnvironmentVariables
    #[starlark(attribute)]
    fn environ<'v>(this: &StarlarkOS, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(heap.alloc(AllocDict(
            this.repo_env.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        )))
    }

    /// A string identifying the operating system Bazel is running on.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:68 StarlarkOS.getName
    #[starlark(attribute)]
    fn name(this: &StarlarkOS) -> starlark::Result<String> {
        Ok(this.get_name())
    }

    /// A string identifying the architecture Bazel is running on.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:80 StarlarkOS.getArch
    #[starlark(attribute)]
    fn arch(this: &StarlarkOS) -> starlark::Result<String> {
        Ok(this.get_arch())
    }
}
