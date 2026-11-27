//! `wasm_module` and `wasm_exec_result`: WebAssembly modules loaded by
//! `repository_ctx.load_wasm()` and executed by `repository_ctx.execute_wasm()`.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java and
//! StarlarkWasmExecutionResult.java.
//!
//! PORT: Bazel runs WebAssembly with Chicory, an interpreter in pure Java; Blaze uses the `wasmi`
//! crate, an interpreter in pure Rust, in the same role (ruling of 2026-11-26 in
//! docs/decisions.md). The messages of the engine's own errors (a module that does not parse, a
//! trap) are wasmi's, not Chicory's; everything around the engine follows the Java.

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, Instant};

use allocative::Allocative;
use blaze_starlark::structs::DeeplyImmutable;
use blaze_util::string_encoding::latin1_decode;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_complex_value;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{
    Demand, Freeze, FreezeResult, Freezer, FrozenValue, NoSerialize, StarlarkValue, Trace, Value,
    ValueLifetimeless, ValueLike, starlark_value,
};
use starlark_map::StarlarkHasher;

use blaze_util::profiler::{Profiler, ProfilerTask};

use super::base_external_context::eval_error;
use super::path::StarlarkPath;

/// `PAGE_SIZE` of a WebAssembly memory (64 KiB).
// Java (Chicory): com/dylibso/chicory/runtime/Memory.java Memory.PAGE_SIZE
const PAGE_SIZE: i64 = 65536;
/// `MAX_PAGES` of a 32-bit WebAssembly memory (4 GiB).
// Java (Chicory): com/dylibso/chicory/runtime/Memory.java Memory.RUNTIME_MAX_PAGES
const MAX_PAGES: i64 = 65536;

/// The fuel given to a call between two checks of the timeout.
// PORT: Java runs the call on a thread of its own and stops waiting after the timeout
// (`ExecutorService.invokeAny`); wasmi's fuel lets the calling thread check the elapsed time.
const FUEL_SLICE: u64 = 1_000_000;

/// The result of executing a WebAssembly function with `repository_ctx.execute_wasm()`. It
/// contains the function's return value and output buffer.
///
/// If execution failed before the function returned then the return code will be negative and
/// the `error_message` field will be set.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmExecutionResult.java:37 StarlarkWasmExecutionResult
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkWasmExecutionResult {
    return_code: i64,
    output: String,
    error_message: String,
}

starlark_simple_value!(StarlarkWasmExecutionResult);

impl StarlarkWasmExecutionResult {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmExecutionResult.java:48 StarlarkWasmExecutionResult.newOk
    pub fn new_ok(return_code: i64, output: String) -> Self {
        StarlarkWasmExecutionResult {
            return_code,
            output,
            error_message: String::new(),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmExecutionResult.java:52 StarlarkWasmExecutionResult.newErr
    pub fn new_err(error_message: String) -> Self {
        StarlarkWasmExecutionResult {
            return_code: -1,
            output: String::new(),
            error_message,
        }
    }
}

impl fmt::Display for StarlarkWasmExecutionResult {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmExecutionResult.java:62 StarlarkWasmExecutionResult.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<wasm_exec_result return_code={} output={} error_message={}>",
            self.return_code,
            repr_str(&self.output),
            repr_str(&self.error_message)
        )
    }
}

/// `Printer.repr` of a string.
fn repr_str(s: &str) -> String {
    starlark::values::Heap::temp(|heap| heap.alloc_str(s).to_value().to_repr())
}

#[starlark_value(type = "wasm_exec_result")]
impl<'v> StarlarkValue<'v> for StarlarkWasmExecutionResult {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("wasm_exec_result", wasm_exec_result_methods);
        Some(RES.methods())
    }

    // Java: isImmutable() is true, so the object is hashable (by identity, Object.hashCode).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        (self as *const Self as usize).hash(hasher);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmExecutionResult.java:57 StarlarkWasmExecutionResult.isImmutable
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn wasm_exec_result_methods(builder: &mut MethodsBuilder) {
    /// The return value of the WebAssembly function, or a negative value if execution was
    /// terminated before the function returned.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmExecutionResult.java:80 StarlarkWasmExecutionResult.getReturnCode
    #[starlark(attribute)]
    fn return_code(this: &StarlarkWasmExecutionResult) -> starlark::Result<i64> {
        Ok(this.return_code)
    }

    /// The content of the output buffer returned by the WebAssembly function.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmExecutionResult.java:88 StarlarkWasmExecutionResult.getOutput
    #[starlark(attribute)]
    fn output(this: &StarlarkWasmExecutionResult) -> starlark::Result<String> {
        Ok(this.output.clone())
    }

    /// Contains an error message if execution failed before the function returned.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmExecutionResult.java:96 StarlarkWasmExecutionResult.getErrorMessage
    #[starlark(attribute)]
    fn error_message(this: &StarlarkWasmExecutionResult) -> starlark::Result<String> {
        Ok(this.error_message.clone())
    }
}

/// The parsed module and what `load_wasm` validated about it.
struct WasmModuleData {
    path: StarlarkPath,
    engine: wasmi::Engine,
    /// The module as loaded. Each execution compiles it with its memory 0 given the limits of
    /// that execution (Java: `Instance.builder(wasmModule).withMemoryLimits(memLimits)`).
    module_content: Vec<u8>,
    alloc_fn_name: String,
    /// `Printer.repr(origPath)`, for `repr`.
    orig_path_repr: String,
    has_initialize_fn: bool,
    /// `wasmModule.memorySection()`: the number of memories the module defines and the initial
    /// pages of the first one.
    memory_section: Option<(u32, u64)>,
}

/// A WebAssembly module loaded by `repository_ctx.load_wasm()`.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:51 StarlarkWasmModule
#[derive(Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkWasmModuleGen<V: ValueLifetimeless> {
    /// `origPath`: the `path` argument as given (a string, `Label` or `path`).
    orig_path: V,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    data: Arc<WasmModuleData>,
}

starlark_complex_value!(pub StarlarkWasmModule);

// SAFETY: `repr(C)`, and `FrozenValue`/`Value` have the same layout.
unsafe impl<'v> Coerce<StarlarkWasmModuleGen<Value<'v>>> for StarlarkWasmModuleGen<FrozenValue> {}

impl<'v> Freeze for StarlarkWasmModule<'v> {
    type Frozen = FrozenStarlarkWasmModule;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenStarlarkWasmModule> {
        Ok(StarlarkWasmModuleGen {
            orig_path: self.orig_path.freeze(freezer)?,
            data: self.data,
        })
    }
}

impl<V: ValueLifetimeless> fmt::Debug for StarlarkWasmModuleGen<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl<V: ValueLifetimeless> fmt::Display for StarlarkWasmModuleGen<V> {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:98 StarlarkWasmModule.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<wasm_module path={} allocate_fn={}>",
            self.data.orig_path_repr,
            repr_str(&self.data.alloc_fn_name)
        )
    }
}

#[starlark_value(type = "wasm_module")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkWasmModuleGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("wasm_module", wasm_module_methods);
        Some(RES.methods())
    }

    // Java: isImmutable() is true, so the object is hashable (by identity, Object.hashCode).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        (Arc::as_ptr(&self.data) as usize).hash(hasher);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:93 StarlarkWasmModule.isImmutable
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn wasm_module_methods(builder: &mut MethodsBuilder) {
    /// The path this WebAssembly module was loaded from.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:114 StarlarkWasmModule.getOrigPath
    #[starlark(attribute)]
    fn path<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(wasm_module_of(this)
            .expect("the receiver is a wasm_module")
            .orig_path)
    }
}

/// The [`StarlarkWasmModule`] of a value, frozen or not.
pub fn wasm_module_of<'v>(value: Value<'v>) -> Option<StarlarkWasmModuleView<'v>> {
    if let Some(m) = value.downcast_ref::<StarlarkWasmModule<'v>>() {
        return Some(StarlarkWasmModuleView {
            orig_path: m.orig_path,
            data: m.data.clone(),
        });
    }
    value
        .downcast_ref::<FrozenStarlarkWasmModule>()
        .map(|m| StarlarkWasmModuleView {
            orig_path: m.orig_path.to_value(),
            data: m.data.clone(),
        })
}

/// A loaded module as the context methods use it.
pub struct StarlarkWasmModuleView<'v> {
    orig_path: Value<'v>,
    data: Arc<WasmModuleData>,
}

impl<'v> StarlarkWasmModuleView<'v> {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:106 StarlarkWasmModule.getPath
    pub fn get_path(&self) -> &StarlarkPath {
        &self.data.path
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:118 StarlarkWasmModule.execute
    pub fn execute(
        &self,
        exec_fn_name: &str,
        input: &[u8],
        timeout_millis: i64,
        mem_limit_bytes: i64,
    ) -> Result<StarlarkWasmExecutionResult, ExecuteError> {
        execute(
            &self.data,
            exec_fn_name,
            input,
            timeout_millis,
            mem_limit_bytes,
        )
    }
}

/// How [`StarlarkWasmModuleView::execute`] fails: Java's `EvalException` or
/// `InterruptedException`.
pub enum ExecuteError {
    Eval(starlark::Error),
    Interrupted,
}

impl From<starlark::Error> for ExecuteError {
    fn from(e: starlark::Error) -> Self {
        ExecuteError::Eval(e)
    }
}

/// `new StarlarkWasmModule(path, origPath, moduleContent, allocFnName)`.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:58 StarlarkWasmModule.<init>
pub fn new_wasm_module<'v>(
    heap: starlark::values::Heap<'v>,
    path: StarlarkPath,
    orig_path: Value<'v>,
    module_content: &[u8],
    alloc_fn_name: &str,
) -> starlark::Result<Value<'v>> {
    let prof = Profiler::instance();
    let _c1 = prof.profile_with(ProfilerTask::WASM_LOAD, || format!("load {path}"));
    let mut config = wasmi::Config::default();
    config.consume_fuel(true);
    let engine = wasmi::Engine::new(&config);
    let (memory_section, original_module) = {
        let _c2 = prof.profile(ProfilerTask::WASM_LOAD, "parse");
        // Java: throw new EvalException(e) (the ChicoryException's message). Chicory reads the
        // header itself; wasmi's own messages for the rest of the module are wasmi's.
        check_header(module_content).map_err(eval_error)?;
        let memory_section = memory_section(module_content);
        // The checks of the module's own exports read the module as given.
        let original_module = match wasmi::Module::new(&engine, module_content) {
            Ok(m) => m,
            Err(e) => return Err(eval_error(e)),
        };
        (memory_section, original_module)
    };
    validate_module(&original_module, alloc_fn_name)?;
    let has_initialize_fn = has_initialize_fn(&original_module);
    Ok(heap.alloc(StarlarkWasmModuleGen {
        orig_path,
        data: Arc::new(WasmModuleData {
            path,
            engine,
            module_content: module_content.to_vec(),
            alloc_fn_name: alloc_fn_name.to_owned(),
            orig_path_repr: orig_path.to_repr(),
            has_initialize_fn,
            memory_section,
        }),
    }))
}

/// The magic and version checks of Chicory's parser, with its messages: a `MalformedException`
/// thrown while parsing is rethrown with the prefix "section size mismatch, unexpected end of
/// section or function, ".
// Java (Chicory): com/dylibso/chicory/wasm/Parser.java Parser.parse
// Java (Chicory): com/dylibso/chicory/wasm/Encoding.java Encoding.readBytes
fn check_header(content: &[u8]) -> Result<(), String> {
    const MAGIC_BYTES: [u8; 4] = [0x00, 0x61, 0x73, 0x6d];
    const VERSION_BYTES: [u8; 4] = [0x01, 0x00, 0x00, 0x00];
    let malformed =
        |m: String| format!("section size mismatch, unexpected end of section or function, {m}");
    // Arrays.toString(byte[]): signed bytes.
    let bytes = |b: &[u8]| {
        let v: Vec<String> = b.iter().map(|x| (*x as i8).to_string()).collect();
        format!("[{}]", v.join(", "))
    };
    let Some(magic) = content.get(..4) else {
        return Err(malformed("length out of bounds".to_owned()));
    };
    if magic != MAGIC_BYTES {
        return Err(malformed(format!(
            "magic header not detected, found: {} expected: {}",
            bytes(magic),
            bytes(&MAGIC_BYTES)
        )));
    }
    let Some(version) = content.get(4..8) else {
        return Err(malformed("length out of bounds".to_owned()));
    };
    if version != VERSION_BYTES {
        return Err(malformed(format!(
            "unknown binary version, found: {} expected: {}",
            bytes(version),
            bytes(&VERSION_BYTES)
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:81 StarlarkWasmModule.hasInitializeFn
fn has_initialize_fn(wasm_module: &wasmi::Module) -> bool {
    wasm_module
        .exports()
        .any(|export| export.name() == "_initialize")
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:231 StarlarkWasmModule.validateModule
fn validate_module(wasm_module: &wasmi::Module, alloc_fn_name: &str) -> starlark::Result<()> {
    if wasm_module
        .exports()
        .any(|export| export.name() == alloc_fn_name)
    {
        // TODO: #26092 - Validate exported type is a function and has the expected signature?
        return Ok(());
    }
    Err(eval_error(format!(
        "WebAssembly module doesn't contain an export named \"{alloc_fn_name}\""
    )))
}

/// `MemoryLimits(initialPages, maximumPages)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemoryLimits {
    initial_pages: i64,
    maximum_pages: i64,
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:245 StarlarkWasmModule.getMemLimits
fn get_mem_limits(data: &WasmModuleData, mem_limit_bytes: i64) -> starlark::Result<MemoryLimits> {
    let mut initial_pages: i64 = 1;
    let mem_limit_pages = get_mem_limit_pages(mem_limit_bytes);

    if let Some((memory_count, first_initial_pages)) = data.memory_section {
        if memory_count > 1 {
            // TODO: #26092 - Figure out what memory limits mean when applied to a WebAssembly
            // module with multiple memories.
            return Err(eval_error(
                "WebAssembly modules with multiple memories not yet supported",
            ));
        }
        if memory_count != 0 && first_initial_pages as i64 > initial_pages {
            initial_pages = first_initial_pages as i64;
        }
    }
    if initial_pages > mem_limit_pages {
        // TODO: #26092 - Should probably throw an exception. The execution will likely fail
        // anyway, and throwing an exception from this point would provide more relevant details.
        initial_pages = mem_limit_pages;
    }
    Ok(MemoryLimits {
        initial_pages,
        maximum_pages: mem_limit_pages,
    })
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:273 StarlarkWasmModule.getMemLimitPages
fn get_mem_limit_pages(mem_limit_bytes: i64) -> i64 {
    if mem_limit_bytes == 0 {
        return 1;
    }
    // Math.ceilDiv(memLimitBytes, PAGE_SIZE)
    let pages = mem_limit_bytes.div_euclid(PAGE_SIZE)
        + if mem_limit_bytes.rem_euclid(PAGE_SIZE) != 0 {
            1
        } else {
            0
        };
    MAX_PAGES.min(pages)
}

/// Why `run` failed: Java's `ExecutionException` cause (`e.getCause().getMessage()`).
struct RunError(String);

impl From<wasmi::Error> for RunError {
    fn from(e: wasmi::Error) -> Self {
        RunError(e.to_string())
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:118 StarlarkWasmModule.execute
fn execute(
    data: &WasmModuleData,
    exec_fn_name: &str,
    input: &[u8],
    timeout_millis: i64,
    mem_limit_bytes: i64,
) -> Result<StarlarkWasmExecutionResult, ExecuteError> {
    let _c = Profiler::instance().profile_with(ProfilerTask::WASM_EXEC, || {
        format!("execute {exec_fn_name}")
    });
    let mem_limits = get_mem_limits(data, mem_limit_bytes)?;
    // Perform initialization and execution so it can be stopped in case of timeout.
    // `executor.invokeAny(tasks, timeout.toMillis(), MILLISECONDS)` with a timeout of zero or
    // less waits for nothing and throws TimeoutException.
    if timeout_millis <= 0 {
        return Ok(StarlarkWasmExecutionResult::new_err(format!(
            "Error executing {exec_fn_name}: timed out"
        )));
    }
    let deadline = Instant::now() + Duration::from_millis(timeout_millis as u64);
    let err_message = match run(data, exec_fn_name, input, mem_limits, deadline) {
        Ok(result) => return Ok(result),
        Err(RunOutcome::TimedOut) => format!("Error executing {exec_fn_name}: timed out"),
        // invokeAny throws InterruptedException.
        Err(RunOutcome::Interrupted) => return Err(ExecuteError::Interrupted),
        Err(RunOutcome::Failed(RunError(message))) => {
            format!("Error executing {exec_fn_name}: {message}")
        }
    };
    Ok(StarlarkWasmExecutionResult::new_err(err_message))
}

enum RunOutcome {
    TimedOut,
    /// The command was interrupted while the call ran (Java: `invokeAny` throws
    /// `InterruptedException`).
    Interrupted,
    Failed(RunError),
}

impl From<RunError> for RunOutcome {
    fn from(e: RunError) -> Self {
        RunOutcome::Failed(e)
    }
}

impl From<wasmi::Error> for RunOutcome {
    fn from(e: wasmi::Error) -> Self {
        RunOutcome::Failed(e.into())
    }
}

/// The store data (none: the memory limits are in the compiled module, see
/// [`with_memory_limits`]).
type StoreData = ();

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:144 StarlarkWasmModule.run
fn run(
    data: &WasmModuleData,
    exec_fn_name: &str,
    input: &[u8],
    mem_limits: MemoryLimits,
    deadline: Instant,
) -> Result<StarlarkWasmExecutionResult, RunOutcome> {
    // Chicory builds memory 0 with `MemoryLimits(initialPages, maximumPages)` in place of the
    // module's own limits; the module is compiled with those limits. `instance.memory()` of
    // Chicory is the module's memory 0, exported or not; wasmi reaches a memory by its export
    // name, so the compiled module exports memory 0 as well, under a name no module can have
    // (see `with_memory_export`).
    let compiled_content =
        with_memory_limits(&with_memory_export(&data.module_content), mem_limits);
    let wasm_module =
        wasmi::Module::new(&data.engine, &compiled_content[..]).map_err(RunError::from)?;
    let mut store = wasmi::Store::new(&data.engine, ());
    store.set_fuel(FUEL_SLICE)?;
    // Disable calling `_start()`, which is the entry point for WASI-style command modules
    // (Chicory's `withStart(false)`; wasmi never calls it).
    let linker = wasmi::Linker::<StoreData>::new(&data.engine);
    let instance = linker
        .instantiate_and_start(&mut store, &wasm_module)
        .map_err(RunError::from)?;
    // If `_initialize()` is present then call it to perform early setup.
    //
    // Note: The WebAssembly spec describes a "start function", named in a "start section", that
    // is to be called as part of module initialization. Actual implementations such as LLVM have
    // instead used the start function as the equivalent of a native binary's entry point, and
    // expect (or emit) a function named `_initialize` to be used for early initialization.
    if data.has_initialize_fn {
        let _c = Profiler::instance().profile(ProfilerTask::WASM_EXEC, "initialize");
        let initialize = export(&instance, &store, "_initialize")?;
        apply(&mut store, &initialize, &[], deadline)?;
    }

    // instance.memory(): the module's memory 0 (null without one).
    let memory = instance.get_memory(&store, MEMORY_0_EXPORT);
    // Chicory's `instance.export(name)` throws for a missing export or one that is not a
    // function, so the null checks after it (and their "doesn't export" messages) are not
    // reached.
    let alloc_fn = export(&instance, &store, &data.alloc_fn_name)?;
    // TODO: #26092 - Validate execFn has the expected signature?
    let exec_fn = export(&instance, &store, exec_fn_name)?;

    let input_len =
        i32::try_from(input.len()).map_err(|_| RunError("integer overflow".to_owned()))?;
    let input_ptr = alloc(
        &mut store,
        &data.alloc_fn_name,
        &alloc_fn,
        input_len,
        1,
        deadline,
    )?;
    // memory.write(inputPtr, input) on a null memory: the NullPointerException's (helpful)
    // message.
    let Some(memory) = memory else {
        return Err(RunError(
            "Cannot invoke \"com.dylibso.chicory.runtime.Memory.write(int, byte[])\" because \
             \"memory\" is null"
                .to_owned(),
        )
        .into());
    };
    {
        let _c = Profiler::instance().profile(ProfilerTask::WASM_EXEC, "copy input");
        memory
            .write(&mut store, input_ptr as u32 as usize, input)
            .map_err(|e| RunError(e.to_string()))?;
    }

    // struct { output_ptr_ptr: **u8, output_len_ptr: *u32 }
    let params_ptr = alloc(&mut store, &data.alloc_fn_name, &alloc_fn, 8, 4, deadline)?;
    let output_ptr_ptr = params_ptr;
    let output_len_ptr = params_ptr.wrapping_add(4);
    write_i32(&mut store, &memory, output_ptr_ptr, 0)?;
    write_i32(&mut store, &memory, output_len_ptr, 0)?;

    let exec_result = {
        let _c = Profiler::instance().profile(ProfilerTask::WASM_EXEC, "execute");
        apply(
            &mut store,
            &exec_fn,
            &[
                input_ptr as i64,
                input_len as i64,
                output_ptr_ptr as i64,
                output_len_ptr as i64,
            ],
            deadline,
        )?
    };

    // TODO: #26092 - Not 100% sure this check is necessary, but the ambiguity between
    // signed/unsigned in Java vs WebAssembly makes me nervous.
    //
    // Might be unnecessary if the function signature is verified before execution?
    // `execResult[0]` of a function without results (Chicory returns null).
    let Some(&first) = exec_result.as_ref().and_then(|r| r.first()) else {
        return Err(RunError(
            "Cannot load from long array because \"execResult\" is null".to_owned(),
        )
        .into());
    };
    let mut return_code: i64 = first;
    if !(0..=0xFFFF_FFFF).contains(&return_code) {
        return_code = 0xFFFF_FFFF;
    }
    let output_ptr = read_i32(&store, &memory, output_ptr_ptr)?;
    let output_len = read_i32(&store, &memory, output_len_ptr)?;

    let mut output = String::new();
    if output_len > 0 {
        let _c = Profiler::instance().profile(ProfilerTask::WASM_EXEC, "copy output");
        let mut output_bytes = vec![0u8; output_len as usize];
        memory
            .read(&store, output_ptr as u32 as usize, &mut output_bytes)
            .map_err(|e| RunError(e.to_string()))?;
        // new String(outputBytes, ISO_8859_1)
        output = latin1_decode(&output_bytes).into_owned();
    }
    Ok(StarlarkWasmExecutionResult::new_ok(return_code, output))
}

/// `instance.export(name)`: the exported function, or Chicory's error for a missing export
/// or one of another kind.
// Java (Chicory): com/dylibso/chicory/runtime/Instance.java Instance.Exports.function
// Java (Chicory): com/dylibso/chicory/runtime/Instance.java Instance.Exports.getExport
fn export(
    instance: &wasmi::Instance,
    store: &wasmi::Store<StoreData>,
    name: &str,
) -> Result<wasmi::Func, RunError> {
    match instance.get_export(store, name) {
        None => Err(RunError(format!("Unknown export with name {name}"))),
        Some(wasmi::Extern::Func(f)) => Ok(f),
        Some(other) => {
            // Chicory's ExternalType names.
            let kind = match other {
                wasmi::Extern::Global(_) => "GLOBAL",
                wasmi::Extern::Table(_) => "TABLE",
                wasmi::Extern::Memory(_) => "MEMORY",
                wasmi::Extern::Func(_) => unreachable!(),
            };
            Err(RunError(format!(
                "The export {name} is of type {kind} and cannot be converted to FUNCTION"
            )))
        }
    }
}

/// `ExportFunction.apply(args)`: Chicory does not check the function's signature. The frame's
/// locals are `Arrays.copyOf(args, ...)` (missing arguments are zero, extra ones are dropped)
/// and the results are the values the function returns, `null` when it returns none.
// Java (Chicory): com/dylibso/chicory/runtime/InterpreterMachine.java InterpreterMachine.call
// Java (Chicory): com/dylibso/chicory/runtime/StackFrame.java StackFrame.<init>
fn apply(
    store: &mut wasmi::Store<StoreData>,
    func: &wasmi::Func,
    args: &[i64],
    deadline: Instant,
) -> Result<Option<Vec<i64>>, RunOutcome> {
    let ty = func.ty(&*store);
    let params: Vec<wasmi::Val> = ty
        .params()
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let arg = args.get(i).copied().unwrap_or(0);
            match t {
                // An i32 local holds the low 32 bits of the long.
                wasmi::ValType::I32 => wasmi::Val::I32(arg as i32),
                wasmi::ValType::I64 => wasmi::Val::I64(arg),
                other => blaze_util::unported!(
                    "InterpreterMachine.call",
                    "a parameter of type {other:?} given as a long"
                ),
            }
        })
        .collect();
    let mut results: Vec<wasmi::Val> = ty
        .results()
        .iter()
        .map(|t| wasmi::Val::default(*t))
        .collect();
    call(store, func, &params, &mut results, deadline)?;
    if results.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        results
            .iter()
            .map(|v| match v {
                // The i32 result as a long holding the signed value.
                wasmi::Val::I32(v) => *v as i64,
                wasmi::Val::I64(v) => *v,
                other => blaze_util::unported!(
                    "InterpreterMachine.call",
                    "a result of type {:?} as a long",
                    other.ty()
                ),
            })
            .collect(),
    ))
}

/// A call of `func`, with the timeout checked whenever the call runs out of fuel.
fn call(
    store: &mut wasmi::Store<StoreData>,
    func: &wasmi::Func,
    params: &[wasmi::Val],
    results: &mut [wasmi::Val],
    deadline: Instant,
) -> Result<(), RunOutcome> {
    let mut call = func.call_resumable(&mut *store, params, results)?;
    loop {
        match call {
            wasmi::ResumableCall::Finished => return Ok(()),
            wasmi::ResumableCall::OutOfFuel(out_of_fuel) => {
                // PORT: Java runs the call on a thread of its own and waits for it in
                // `invokeAny`, which an interrupt of the command thread ends; the call here
                // runs on the command's thread and looks at the interrupt with the deadline.
                if blaze_shell::thread_interrupt::interrupted() {
                    return Err(RunOutcome::Interrupted);
                }
                if Instant::now() >= deadline {
                    return Err(RunOutcome::TimedOut);
                }
                let required = out_of_fuel.required_fuel();
                store.set_fuel(FUEL_SLICE.max(required))?;
                call = out_of_fuel.resume(&mut *store, results)?;
            }
            wasmi::ResumableCall::HostTrap(_) => {
                // There are no host functions.
                unreachable!("a WebAssembly module without imports called a host function")
            }
        }
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:280 StarlarkWasmModule.alloc
fn alloc(
    store: &mut wasmi::Store<StoreData>,
    alloc_fn_name: &str,
    alloc_fn: &wasmi::Func,
    size: i32,
    align: i32,
    deadline: Instant,
) -> Result<i32, RunOutcome> {
    let alloc_result = apply(store, alloc_fn, &[size as i64, align as i64], deadline)?;
    // `allocResult[0]` of a function without results (Chicory returns null).
    let Some(&ptr) = alloc_result.as_ref().and_then(|r| r.first()) else {
        return Err(RunError(
            "Cannot load from long array because \"allocResult\" is null".to_owned(),
        )
        .into());
    };
    if ptr == 0 {
        return Err(RunError(format!(
            "allocation failed: {alloc_fn_name}({size}, {align}) returned NULL"
        ))
        .into());
    }
    match i32::try_from(ptr) {
        Ok(ptr) => Ok(ptr),
        Err(_) => Err(RunError(format!(
            "allocation failed: {alloc_fn_name}({size}, {align}) returned invalid pointer 0x{:08X} \
             (out of range)",
            ptr
        ))
        .into()),
    }
}

/// `memory.writeI32(addr, value)`: little-endian.
fn write_i32(
    store: &mut wasmi::Store<StoreData>,
    memory: &wasmi::Memory,
    addr: i32,
    value: i32,
) -> Result<(), RunOutcome> {
    memory
        .write(&mut *store, addr as u32 as usize, &value.to_le_bytes())
        .map_err(|e| RunError(e.to_string()).into())
}

/// `memory.readInt(addr)`: little-endian.
fn read_i32(
    store: &wasmi::Store<StoreData>,
    memory: &wasmi::Memory,
    addr: i32,
) -> Result<i32, RunOutcome> {
    let mut bytes = [0u8; 4];
    memory
        .read(store, addr as u32 as usize, &mut bytes)
        .map_err(|e| RunError(e.to_string()))?;
    Ok(i32::from_le_bytes(bytes))
}

/// `wasmModule.memorySection()`: the number of memories the module defines in its memory
/// section and the initial pages of the first one; `None` without a memory section.
///
/// PORT: Chicory's parsed `WasmModule` has the section; wasmi does not expose a module's own
/// memories, so the section is read from the (already validated) binary.
fn memory_section(wasm: &[u8]) -> Option<(u32, u64)> {
    // magic and version
    let mut pos = 8;
    while pos < wasm.len() {
        let id = wasm[pos];
        pos += 1;
        let size = read_leb_u64(wasm, &mut pos)? as usize;
        let end = pos.checked_add(size)?;
        if id == 5 {
            let mut p = pos;
            let count = read_leb_u64(wasm, &mut p)? as u32;
            if count == 0 {
                return Some((0, 0));
            }
            // limits: flags, then the minimum (an index type of 64 bits when flag bit 2 is set)
            let _flags = *wasm.get(p)?;
            p += 1;
            let minimum = read_leb_u64(wasm, &mut p)?;
            return Some((count, minimum));
        }
        pos = end;
    }
    None
}

/// The name under which the compiled module exports its memory 0: export names are UTF-8, and
/// a name with a NUL byte and this prefix is not one a toolchain emits for a module's own
/// exports.
const MEMORY_0_EXPORT: &str = "\u{0}blaze:chicory-memory-0";

/// The module with an export of its memory 0 named [`MEMORY_0_EXPORT`] added (the module
/// unchanged when it has no memory section or does not parse as sections).
///
/// PORT: an adapter for wasmi (see `run`); Chicory reads `instance.memory()` directly.
fn with_memory_export(wasm: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    match memory_section(wasm) {
        Some((count, _)) if count > 0 => {}
        _ => return std::borrow::Cow::Borrowed(wasm),
    }
    let mut entry = Vec::new();
    write_leb_u64(&mut entry, MEMORY_0_EXPORT.len() as u64);
    entry.extend_from_slice(MEMORY_0_EXPORT.as_bytes());
    entry.push(0x02); // memory
    write_leb_u64(&mut entry, 0);

    // The sections: (id, start of the id byte, start of the content, end).
    let mut sections = Vec::new();
    let mut pos = 8;
    while pos < wasm.len() {
        let start = pos;
        let id = wasm[pos];
        pos += 1;
        let Some(size) = read_leb_u64(wasm, &mut pos) else {
            return std::borrow::Cow::Borrowed(wasm);
        };
        let content = pos;
        let Some(end) = pos.checked_add(size as usize).filter(|e| *e <= wasm.len()) else {
            return std::borrow::Cow::Borrowed(wasm);
        };
        sections.push((id, start, content, end));
        pos = end;
    }

    let mut out = wasm[..8].to_vec();
    let mut done = false;
    for &(id, start, content, end) in &sections {
        if !done && id == 7 {
            let mut p = content;
            let Some(count) = read_leb_u64(wasm, &mut p) else {
                return std::borrow::Cow::Borrowed(wasm);
            };
            let mut body = Vec::new();
            write_leb_u64(&mut body, count + 1);
            body.extend_from_slice(&wasm[p..end]);
            body.extend_from_slice(&entry);
            out.push(7);
            write_leb_u64(&mut out, body.len() as u64);
            out.extend_from_slice(&body);
            done = true;
            continue;
        }
        // A new export section goes before the start, element, data count, code and data
        // sections (the order of the binary format).
        if !done && matches!(id, 8 | 9 | 10 | 11 | 12) {
            push_export_section(&mut out, &entry);
            done = true;
        }
        out.extend_from_slice(&wasm[start..end]);
    }
    if !done {
        push_export_section(&mut out, &entry);
    }
    std::borrow::Cow::Owned(out)
}

/// The module with the limits of its memory 0 replaced by `limits` (the module unchanged when
/// it has no memory or does not parse as sections).
///
/// PORT: an adapter for wasmi; Chicory builds the memory from `MemoryLimits` directly
/// (`Instance.Builder.withMemoryLimits`).
fn with_memory_limits(wasm: &[u8], limits: MemoryLimits) -> Vec<u8> {
    let mut out = wasm[..8.min(wasm.len())].to_vec();
    let mut pos = 8;
    while pos < wasm.len() {
        let start = pos;
        let id = wasm[pos];
        pos += 1;
        let Some(size) = read_leb_u64(wasm, &mut pos) else {
            return wasm.to_vec();
        };
        let content = pos;
        let Some(end) = pos.checked_add(size as usize).filter(|e| *e <= wasm.len()) else {
            return wasm.to_vec();
        };
        pos = end;
        if id != 5 {
            out.extend_from_slice(&wasm[start..end]);
            continue;
        }
        let mut p = content;
        let Some(count) = read_leb_u64(wasm, &mut p) else {
            return wasm.to_vec();
        };
        if count == 0 {
            out.extend_from_slice(&wasm[start..end]);
            continue;
        }
        // limits: flags (bit 0: a maximum follows), the minimum, the maximum.
        let Some(&flags) = wasm.get(p) else {
            return wasm.to_vec();
        };
        p += 1;
        if read_leb_u64(wasm, &mut p).is_none() {
            return wasm.to_vec();
        }
        if flags & 1 != 0 && read_leb_u64(wasm, &mut p).is_none() {
            return wasm.to_vec();
        }
        let mut body = Vec::new();
        write_leb_u64(&mut body, count);
        body.push(flags | 1);
        write_leb_u64(&mut body, limits.initial_pages as u64);
        write_leb_u64(&mut body, limits.maximum_pages as u64);
        body.extend_from_slice(&wasm[p..end]);
        out.push(5);
        write_leb_u64(&mut out, body.len() as u64);
        out.extend_from_slice(&body);
    }
    out
}

/// Appends an export section with the one export `entry`.
fn push_export_section(out: &mut Vec<u8>, entry: &[u8]) {
    let mut body = Vec::new();
    write_leb_u64(&mut body, 1);
    body.extend_from_slice(entry);
    out.push(7);
    write_leb_u64(out, body.len() as u64);
    out.extend_from_slice(&body);
}

/// Writes an unsigned LEB128 number.
fn write_leb_u64(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// An unsigned LEB128 number.
fn read_leb_u64(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let byte = *bytes.get(*pos)?;
        *pos += 1;
        if shift < 64 {
            result |= ((byte & 0x7f) as u64) << shift;
        }
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No JUnit test exists at the pin; these cover the ported arithmetic.
    #[test]
    fn mem_limit_pages_round_up_and_cap() {
        assert_eq!(get_mem_limit_pages(0), 1);
        assert_eq!(get_mem_limit_pages(1), 1);
        assert_eq!(get_mem_limit_pages(PAGE_SIZE), 1);
        assert_eq!(get_mem_limit_pages(PAGE_SIZE + 1), 2);
        assert_eq!(get_mem_limit_pages(64 * 1024 * 1024), 1024);
        assert_eq!(get_mem_limit_pages(i64::MAX), MAX_PAGES);
    }

    // Expectations: the oracle's messages (review probe `bad`, and `badver` of the response
    // probe, docs/reviews/2026-11-27-small-ports-response.md).
    #[test]
    fn header_errors_have_chicorys_text() {
        assert_eq!(
            check_header(b"not wasm"),
            Err(
                "section size mismatch, unexpected end of section or function, magic header not \
                 detected, found: [110, 111, 116, 32] expected: [0, 97, 115, 109]"
                    .to_owned()
            )
        );
        assert_eq!(
            check_header(&[0x00, 0x61, 0x73, 0x6d, 0x02, 0x00, 0x00, 0x00]),
            Err(
                "section size mismatch, unexpected end of section or function, unknown binary \
                 version, found: [2, 0, 0, 0] expected: [1, 0, 0, 0]"
                    .to_owned()
            )
        );
        assert_eq!(
            check_header(&[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]),
            Ok(())
        );
    }

    #[test]
    fn memory_limits_replace_the_modules_own() {
        // (module (memory 1 2)) gets (memory 1 1024); (module (memory 3)) gets (memory 1 1).
        let max2 = [
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x05, 0x04, 0x01, 0x01, 0x01, 0x02,
        ];
        let out = with_memory_limits(
            &max2,
            MemoryLimits {
                initial_pages: 1,
                maximum_pages: 1024,
            },
        );
        assert_eq!(
            out,
            [
                0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x05, 0x05, 0x01, 0x01, 0x01, 0x80,
                0x08
            ]
        );
        let mem3 = [
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x05, 0x03, 0x01, 0x00, 0x03,
        ];
        let out = with_memory_limits(
            &mem3,
            MemoryLimits {
                initial_pages: 1,
                maximum_pages: 1,
            },
        );
        assert_eq!(memory_section(&out), Some((1, 1)));
        assert_eq!(
            out,
            [
                0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x05, 0x04, 0x01, 0x01, 0x01, 0x01
            ]
        );
    }

    #[test]
    fn memory_section_of_a_module_with_one_memory() {
        // (module (memory 3))
        let wasm = [
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x05, 0x03, 0x01, 0x00, 0x03,
        ];
        assert_eq!(memory_section(&wasm), Some((1, 3)));
        assert_eq!(
            memory_section(&[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]),
            None
        );
    }
}
