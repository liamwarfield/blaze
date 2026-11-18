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

/// Java `System.getProperty("os.name").toLowerCase(Locale.ROOT)` (also Guava's
/// `StandardSystemProperty.OS_NAME.value()`, which reads the same property).
///
/// The one place that knows the JVM's `os.name` for the system Blaze runs on: `repository_os.name`
/// and the predeclared input hash of repo marker files (`DigestWriter`) must agree.
// PORT: Blaze supports Linux only (AGENTS.md), where the JVM reports "Linux"; any other system
// fails loudly instead of guessing the JVM's value.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:68 StarlarkOS.getName
// Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:193 DigestWriter.computePredeclaredInputHash
pub fn java_os_name_lower_case() -> String {
    match std::env::consts::OS {
        "linux" => "linux".to_owned(),
        other => blaze_util::unported!("System.getProperty", "os.name for {other}"),
    }
}

/// Java `System.getProperty("os.arch").toLowerCase(Locale.ROOT)`.
///
/// The one place that knows the JVM's `os.arch` for the system Blaze runs on: `repository_os.arch`
/// and the predeclared input hash of repo marker files (`DigestWriter`) must agree.
// PORT: Blaze supports Linux x86-64 only (AGENTS.md), where the JVM reports "amd64". The JVM's
// value for 64-bit ARM Linux ("aarch64") is kept from the earlier `repository_os.arch` port so
// that both users see the same value there too; any other architecture fails loudly instead of
// guessing the JVM's value.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:80 StarlarkOS.getArch
// Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:194 DigestWriter.computePredeclaredInputHash
pub fn java_os_arch_lower_case() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "amd64".to_owned(),
        "aarch64" => "aarch64".to_owned(),
        other => blaze_util::unported!("System.getProperty", "os.arch for {other}"),
    }
}

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
        java_os_name_lower_case()
    }

    /// Java `System.getProperty("os.arch").toLowerCase(Locale.ROOT)`.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:80 StarlarkOS.getArch
    pub fn get_arch(&self) -> String {
        java_os_arch_lower_case()
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
    ///
    /// Each access gives a new dict that the calling thread may mutate. Java's getter returns an
    /// `ImmutableMap`, which is not a Starlark value: the interpreter converts the result of a
    /// `structField` method with `Starlark.fromJava(result, mu)`, where `mu` is the mutability of
    /// the thread reading the field (`Starlark.getattr(fr.thread.mutability(), ...)`), and
    /// `fromJava` copies a `Map` with `Dict.copyOf(mu, map)`. So `os.environ["X"] = "y"` succeeds
    /// in Bazel and changes only that copy.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkOS.java:56 StarlarkOS.getEnvironmentVariables
    // Java: net/starlark/java/eval/MethodDescriptor.java:256 MethodDescriptor.call
    // Java: net/starlark/java/eval/Starlark.java:198 Starlark.fromJava
    // Java: net/starlark/java/eval/Eval.java:656 Eval.evalDot
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

#[cfg(test)]
mod tests {
    use starlark::environment::Globals;
    use starlark::environment::Module;
    use starlark::eval::Evaluator;
    use starlark::syntax::AstModule;
    use starlark::syntax::Dialect;

    use super::*;

    /// Evaluates `program` with `os` bound to a `repository_os` over `FOO=BAR` and returns the
    /// repr of its value.
    fn eval_with_os(program: &str) -> String {
        let repo_env: IndexMap<String, String> =
            [("FOO".to_owned(), "BAR".to_owned())].into_iter().collect();
        Module::with_temp_heap(|module| {
            module.set(
                "os",
                module.heap().alloc(StarlarkOS::new(Arc::new(repo_env))),
            );
            let ast = AstModule::parse("test.bzl", program.to_owned(), &Dialect::Standard).unwrap();
            let globals = Globals::standard();
            let mut eval = Evaluator::new(&module);
            let v = eval.eval_module(ast, &globals).unwrap();
            v.to_repr()
        })
    }

    /// Java converts the getter's `ImmutableMap` with `Starlark.fromJava(result, mu)`, `mu` being
    /// the mutability of the reading thread: `Dict.copyOf(mu, map)` is a new dict that thread may
    /// mutate, so a write succeeds and is not seen by the next `os.environ`.
    // Java: net/starlark/java/eval/MethodDescriptor.java:256 MethodDescriptor.call
    // Java: net/starlark/java/eval/Dict.java:456 Dict.copyOf
    #[test]
    fn environ_is_a_new_mutable_dict_on_each_access() {
        assert_eq!(
            eval_with_os(
                "e = os.environ\ne[\"NEW\"] = \"1\"\n[e, \"NEW\" in os.environ, os.environ]\n"
            ),
            "[{\"FOO\": \"BAR\", \"NEW\": \"1\"}, False, {\"FOO\": \"BAR\"}]"
        );
    }

    /// `repository_os.arch` and the marker file hash read the same function.
    #[test]
    fn name_and_arch_come_from_the_shared_functions() {
        assert_eq!(
            eval_with_os("os.name"),
            format!("{:?}", java_os_name_lower_case())
        );
        assert_eq!(
            eval_with_os("os.arch"),
            format!("{:?}", java_os_arch_lower_case())
        );
    }

    /// Blaze's only platform (AGENTS.md): the JVM reports `os.name` "Linux" and `os.arch`
    /// "amd64" there.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn jvm_system_properties_on_linux_x86_64() {
        assert_eq!(java_os_name_lower_case(), "linux");
        assert_eq!(java_os_arch_lower_case(), "amd64");
    }
}
