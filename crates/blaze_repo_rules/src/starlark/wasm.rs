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
    wasm_module: wasmi::Module,
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
        timeout: Duration,
        mem_limit_bytes: i64,
    ) -> starlark::Result<StarlarkWasmExecutionResult> {
        execute(&self.data, exec_fn_name, input, timeout, mem_limit_bytes)
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
    // PORT: skipped, profiling (WASM_LOAD "load", "parse").
    let mut config = wasmi::Config::default();
    config.consume_fuel(true);
    let engine = wasmi::Engine::new(&config);
    // `instance.memory()` of Chicory is the module's memory 0, exported or not; wasmi reaches a
    // memory by its export name, so the compiled module exports memory 0 as well, under a name
    // no module can have (see `with_memory_export`).
    let compiled_content = with_memory_export(module_content);
    let wasm_module = match wasmi::Module::new(&engine, &compiled_content[..]) {
        Ok(m) => m,
        // Java: throw new EvalException(e) (the ChicoryException's message).
        Err(e) => return Err(eval_error(e)),
    };
    let memory_section = memory_section(module_content);
    // The checks of the module's own exports read the module as given.
    let original_module = match wasmi::Module::new(&engine, module_content) {
        Ok(m) => m,
        Err(e) => return Err(eval_error(e)),
    };
    validate_module(&original_module, alloc_fn_name)?;
    let has_initialize_fn = has_initialize_fn(&original_module);
    Ok(heap.alloc(StarlarkWasmModuleGen {
        orig_path,
        data: Arc::new(WasmModuleData {
            path,
            engine,
            wasm_module,
            alloc_fn_name: alloc_fn_name.to_owned(),
            orig_path_repr: orig_path.to_repr(),
            has_initialize_fn,
            memory_section,
        }),
    }))
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
    timeout: Duration,
    mem_limit_bytes: i64,
) -> starlark::Result<StarlarkWasmExecutionResult> {
    // PORT: skipped, profiling (WASM_EXEC "execute <fn>").
    let mem_limits = get_mem_limits(data, mem_limit_bytes)?;
    // Perform initialization and execution so it can be stopped in case of timeout.
    let deadline = Instant::now() + timeout;
    let err_message = match run(data, exec_fn_name, input, mem_limits, deadline) {
        Ok(result) => return Ok(result),
        Err(RunOutcome::TimedOut) => format!("Error executing {exec_fn_name}: timed out"),
        Err(RunOutcome::Failed(RunError(message))) => {
            format!("Error executing {exec_fn_name}: {message}")
        }
    };
    Ok(StarlarkWasmExecutionResult::new_err(err_message))
}

enum RunOutcome {
    TimedOut,
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

/// The store data: the memory limiter (Java's `withMemoryLimits`).
struct StoreData {
    limits: wasmi::StoreLimits,
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkWasmModule.java:144 StarlarkWasmModule.run
fn run(
    data: &WasmModuleData,
    exec_fn_name: &str,
    input: &[u8],
    mem_limits: MemoryLimits,
    deadline: Instant,
) -> Result<StarlarkWasmExecutionResult, RunOutcome> {
    // PORT: Chicory builds the memory with `MemoryLimits(initialPages, maximumPages)`; wasmi
    // keeps the module's own limits and fails a growth beyond the limiter's maximum. The
    // initial pages are the module's (`mem_limits.initial_pages` only matters when Chicory
    // raises a zero-page memory to one page).
    let _ = mem_limits.initial_pages;
    let limits = wasmi::StoreLimitsBuilder::new()
        .memory_size((mem_limits.maximum_pages * PAGE_SIZE) as usize)
        .build();
    let mut store = wasmi::Store::new(&data.engine, StoreData { limits });
    store.limiter(|d| &mut d.limits);
    store.set_fuel(FUEL_SLICE)?;
    // Disable calling `_start()`, which is the entry point for WASI-style command modules
    // (Chicory's `withStart(false)`; wasmi never calls it).
    let linker = wasmi::Linker::<StoreData>::new(&data.engine);
    let instance = linker
        .instantiate_and_start(&mut store, &data.wasm_module)
        .map_err(RunError::from)?;
    // If `_initialize()` is present then call it to perform early setup.
    //
    // Note: The WebAssembly spec describes a "start function", named in a "start section", that
    // is to be called as part of module initialization. Actual implementations such as LLVM have
    // instead used the start function as the equivalent of a native binary's entry point, and
    // expect (or emit) a function named `_initialize` to be used for early initialization.
    if data.has_initialize_fn {
        let initialize = instance
            .get_func(&store, "_initialize")
            .ok_or_else(|| RunError("export _initialize is not a function".to_owned()))?;
        call(&mut store, &initialize, &[], &mut [], deadline)?;
    }

    // instance.memory(): the module's memory 0.
    let memory = instance
        .get_memory(&store, MEMORY_0_EXPORT)
        .ok_or_else(|| RunError("WebAssembly module has no memory".to_owned()))?;
    // Chicory's `instance.export(name)` throws `ChicoryException("Unknown export with name " +
    // name)` for a missing export, so the null checks after it (and their "doesn't export"
    // messages) are not reached.
    // Java (Chicory): com/dylibso/chicory/runtime/Instance.java Instance.export
    let alloc_fn = instance
        .get_func(&store, &data.alloc_fn_name)
        .ok_or_else(|| RunError(format!("Unknown export with name {}", data.alloc_fn_name)))?;
    // TODO: #26092 - Validate execFn has the expected signature?
    let exec_fn = instance
        .get_func(&store, exec_fn_name)
        .ok_or_else(|| RunError(format!("Unknown export with name {exec_fn_name}")))?;

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
    memory
        .write(&mut store, input_ptr as u32 as usize, input)
        .map_err(|e| RunError(e.to_string()))?;

    // struct { output_ptr_ptr: **u8, output_len_ptr: *u32 }
    let params_ptr = alloc(&mut store, &data.alloc_fn_name, &alloc_fn, 8, 4, deadline)?;
    let output_ptr_ptr = params_ptr;
    let output_len_ptr = params_ptr.wrapping_add(4);
    write_i32(&mut store, &memory, output_ptr_ptr, 0)?;
    write_i32(&mut store, &memory, output_len_ptr, 0)?;

    let mut exec_result = [wasmi::Val::I32(0)];
    call(
        &mut store,
        &exec_fn,
        &[
            wasmi::Val::I32(input_ptr),
            wasmi::Val::I32(input_len),
            wasmi::Val::I32(output_ptr_ptr),
            wasmi::Val::I32(output_len_ptr),
        ],
        &mut exec_result,
        deadline,
    )?;

    // TODO: #26092 - Not 100% sure this check is necessary, but the ambiguity between
    // signed/unsigned in Java vs WebAssembly makes me nervous.
    //
    // Chicory returns the i32 result as a long holding the signed value.
    let mut return_code: i64 = match exec_result[0] {
        wasmi::Val::I32(v) => v as i64,
        wasmi::Val::I64(v) => v,
        _ => return Err(RunError("unexpected result type".to_owned()).into()),
    };
    if !(0..=0xFFFF_FFFF).contains(&return_code) {
        return_code = 0xFFFF_FFFF;
    }
    let output_ptr = read_i32(&store, &memory, output_ptr_ptr)?;
    let output_len = read_i32(&store, &memory, output_len_ptr)?;

    let mut output = String::new();
    if output_len > 0 {
        let mut output_bytes = vec![0u8; output_len as usize];
        memory
            .read(&store, output_ptr as u32 as usize, &mut output_bytes)
            .map_err(|e| RunError(e.to_string()))?;
        // new String(outputBytes, ISO_8859_1)
        output = latin1_decode(&output_bytes).into_owned();
    }
    Ok(StarlarkWasmExecutionResult::new_ok(return_code, output))
}

/// `ExportFunction.apply`, with the timeout checked whenever the call runs out of fuel.
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
    let mut alloc_result = [wasmi::Val::I32(0)];
    call(
        store,
        alloc_fn,
        &[wasmi::Val::I32(size), wasmi::Val::I32(align)],
        &mut alloc_result,
        deadline,
    )?;
    // Chicory: the i32 result as a long holding the signed value.
    let ptr: i64 = match alloc_result[0] {
        wasmi::Val::I32(v) => v as i64,
        wasmi::Val::I64(v) => v,
        _ => return Err(RunError("unexpected result type".to_owned()).into()),
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
