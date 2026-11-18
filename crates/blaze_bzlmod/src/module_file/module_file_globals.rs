//! Port of `ModuleFileGlobals` (module(), bazel_dep(), use_extension(), ...), with
//! `ModuleExtensionProxy`, its `TagCallable` and `RepoRuleProxy`. OWNER: agent A.
//!
//! Parameters follow the Java `@Param`s: `named = true, positional = false` is
//! `require = named`; Java's defaults (`named = false, positional = true`) are positional-only
//! (`require = pos`). Parameters are taken as raw values and type-checked here so that the error
//! text is Java's (`BuiltinFunction.checkParamValue`: "in call to f(), parameter 'p' got value of
//! type 't', want 'w'").
// PORT: Java checks parameter types while binding arguments, in call-site order; here they are
// checked in declaration order. Argument-count and unknown-keyword errors are starlark-rust's.

use std::cell::RefMut;
use std::fmt;
use std::sync::LazyLock;

use allocative::Allocative;
use anyhow::anyhow;
use blaze_bzlmod_model::repo_spec::{GIT_REPOSITORY, HTTP_ARCHIVE};
use blaze_bzlmod_model::{
    AttributeValue, AttributeValues, DepSpec, ModuleKey, ModuleOverride, MultipleVersionOverride,
    NonRegistryOverride, SingleVersionOverride, Version, local_path_repo_spec,
};
use blaze_cmdline::package_identifier::EMPTY_PACKAGE_ID;
use blaze_cmdline::repository_name::{MAIN, is_valid_module_name};
use blaze_cmdline::{
    Label, LabelPackageContext, PackageIdentifier, RepositoryMapping, RepositoryName,
};
use blaze_events::Event;
use blaze_starlark::env::{NativeRegistry, caller_location, universe_builder};
use blaze_starlark::error::{EvalErrorKind, StackEntry};
use blaze_starlark::label::LabelValue;
use blaze_starlark::{EvalError, FileKind, StarlarkSemantics, ThreadContext};
use blaze_vfs::PathFragment;
use parking_lot::Mutex;
use starlark::any::ProvidesStaticType;
use starlark::codemap::FileSpan;
use starlark::collections::SmallMap;
use starlark::environment::{Globals, GlobalsBuilder, Module};
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::none::NoneType;
use starlark::values::tuple::{AllocTuple, TupleRef, UnpackTuple};
use starlark::values::{
    Heap, NoSerialize, StarlarkValue, UnpackValue, Value, ValueLike, starlark_value,
};

use super::module_thread_context::{
    ModuleThreadContext, ProxyBuilder, ProxyRef, validate_user_provided_repo_name,
};
use super::{EvalException, entry_location};

// ---------------------------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------------------------

static MODULE_BAZEL_ENVS: LazyLock<Mutex<Vec<(StarlarkSemantics, Globals)>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// The predeclared environment of MODULE.bazel files: the universe plus `ModuleFileGlobals`.
/// Cached per semantics.
// Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:231 BazelStarlarkEnvironment.getModuleBazelEnv
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:129 StarlarkGlobalsImpl.getModuleToplevels
pub fn module_file_globals(semantics: &StarlarkSemantics) -> Globals {
    let mut envs = MODULE_BAZEL_ENVS.lock();
    if let Some((_, globals)) = envs.iter().find(|(s, _)| s == semantics) {
        return globals.clone();
    }
    let globals = universe_builder(semantics)
        .with(register_module_file_globals)
        .build();
    envs.push((semantics.clone(), globals.clone()));
    globals
}

/// Registers `ModuleFileGlobals` for [`FileKind::ModuleFile`] in a [`NativeRegistry`].
pub fn register_natives(registry: &mut NativeRegistry) {
    registry.register(&[FileKind::ModuleFile], register_module_file_globals);
}

// ---------------------------------------------------------------------------------------------
// Thread helpers
// ---------------------------------------------------------------------------------------------

/// Java `ModuleThreadContext.fromOrFail`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleThreadContext.java:69 ModuleThreadContext.fromOrFail
fn from_or_fail<'a>(
    eval: &Evaluator<'_, 'a, '_>,
    what: &str,
) -> anyhow::Result<RefMut<'a, ModuleThreadContext>> {
    ThreadContext::get(eval)
        .ok()
        .and_then(|thread| thread.payload::<ModuleThreadContext>())
        .ok_or_else(|| anyhow!("{what} can only be called from MODULE.bazel and files it includes"))
}

/// The context a proxy belongs to (Java's proxies hold their usage builder directly).
fn proxy_context<'a>(
    eval: &Evaluator<'_, 'a, '_>,
) -> starlark::Result<RefMut<'a, ModuleThreadContext>> {
    ThreadContext::get(eval)
        .ok()
        .and_then(|thread| thread.payload::<ModuleThreadContext>())
        .ok_or_else(|| native_error(anyhow!("internal error: no ModuleThreadContext")))
}

fn native_error(e: impl Into<anyhow::Error>) -> starlark::Error {
    starlark::Error::new_native(e)
}

/// Java `StarlarkThread.getCallerLocation()` as an event location.
fn caller_location_of(eval: &Evaluator) -> blaze_events::Location {
    entry_location(caller_location(eval).as_ref())
}

fn stack_entry(name: &str, span: Option<&FileSpan>) -> StackEntry {
    match span {
        Some(span) => {
            let location = blaze_starlark::Location::bazel_position(span);
            let source_line = if location.line == 0 {
                None
            } else {
                Some(span.file.source_line(location.line - 1).trim().to_owned())
            };
            StackEntry {
                name: name.to_owned(),
                location: Some(location),
                source_line,
            }
        }
        None => StackEntry {
            name: name.to_owned(),
            location: None,
            source_line: None,
        },
    }
}

/// Java `StarlarkThread.getCallStack()` from inside a builtin: the including files' frames
/// (`prefix`), `<toplevel>` at the current call, then the called builtin (location BUILTIN).
// Java: net/starlark/java/eval/StarlarkThread.java:603 StarlarkThread.getCallStack
fn call_stack(eval: &Evaluator, prefix: &[StackEntry]) -> Vec<StackEntry> {
    let frames = eval.call_stack().frames;
    let mut stack = prefix.to_vec();
    let mut caller = "<toplevel>".to_owned();
    for frame in &frames {
        stack.push(stack_entry(&caller, frame.location.as_ref()));
        caller = frame.name.clone();
    }
    if !frames.is_empty() {
        stack.push(stack_entry(&caller, None));
    }
    stack
}

/// An error raised while executing an included file, already carrying its full Bazel call
/// stack. It travels through the including file's evaluation as a native error.
#[derive(Debug)]
struct IncludedFileError(EvalError);

impl fmt::Display for IncludedFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.message)
    }
}

impl std::error::Error for IncludedFileError {}

/// Converts a starlark-rust error of a MODULE.bazel evaluation to Bazel's `EvalException` form,
/// with the frames of the including files (`prefix`) in front.
pub(crate) fn java_eval_error(e: &starlark::Error, prefix: &[StackEntry]) -> EvalError {
    if let starlark::ErrorKind::Native(cause) | starlark::ErrorKind::Other(cause) = e.kind()
        && let Some(included) = cause.downcast_ref::<IncludedFileError>()
    {
        return included.0.clone();
    }
    let mut error = EvalError::from_starlark(e);
    if !prefix.is_empty() && !matches!(error.kind, EvalErrorKind::Syntax(_)) {
        let mut callstack = prefix.to_vec();
        callstack.append(&mut error.callstack);
        error.callstack = callstack;
    }
    error
}

// ---------------------------------------------------------------------------------------------
// Parameter checks (Java BuiltinFunction.checkParamValue, Sequence.cast, Dict.cast)
// ---------------------------------------------------------------------------------------------

// Java: net/starlark/java/eval/BuiltinFunction.java:393 ArgumentProcessor.checkParamValue
fn type_error(method: &str, param: &str, value: Value, want: &str) -> anyhow::Error {
    anyhow!(
        "in call to {}(), parameter '{}' got value of type '{}', want '{}'",
        method,
        param,
        value.get_type(),
        want
    )
}

fn param_str<'v>(method: &str, param: &str, value: Value<'v>) -> anyhow::Result<&'v str> {
    value
        .unpack_str()
        .ok_or_else(|| type_error(method, param, value, "string"))
}

fn opt_str<'v>(
    method: &str,
    param: &str,
    value: Option<Value<'v>>,
    default: &'v str,
) -> anyhow::Result<&'v str> {
    match value {
        None => Ok(default),
        Some(v) => param_str(method, param, v),
    }
}

fn param_bool(method: &str, param: &str, value: Value) -> anyhow::Result<bool> {
    value
        .unpack_bool()
        .ok_or_else(|| type_error(method, param, value, "bool"))
}

fn opt_bool(
    method: &str,
    param: &str,
    value: Option<Value>,
    default: bool,
) -> anyhow::Result<bool> {
    match value {
        None => Ok(default),
        Some(v) => param_bool(method, param, v),
    }
}

/// A `StarlarkInt` parameter: returns `Ok(None)` for the default.
fn opt_int<'v>(
    method: &str,
    param: &str,
    value: Option<Value<'v>>,
) -> anyhow::Result<Option<Value<'v>>> {
    match value {
        None => Ok(None),
        Some(v) if v.get_type() == "int" => Ok(Some(v)),
        Some(v) => Err(type_error(method, param, v, "int")),
    }
}

/// `StarlarkInt.toInt(what)`.
// Java: net/starlark/java/eval/StarlarkInt.java:338 StarlarkInt.toInt
fn to_int(value: Option<Value>, default: i32, what: &str) -> anyhow::Result<i32> {
    match value {
        None => Ok(default),
        Some(v) => v.unpack_i32().ok_or_else(|| {
            anyhow!(
                "got {} for {}, want value in signed 32-bit range",
                v.to_str(),
                what
            )
        }),
    }
}

/// `allowedTypes = {@ParamType(type = Iterable.class, ...)}`: Java `Iterable` Starlark values.
fn opt_iterable<'v>(
    method: &str,
    param: &str,
    value: Option<Value<'v>>,
) -> anyhow::Result<Option<Value<'v>>> {
    match value {
        None => Ok(None),
        Some(v) if matches!(v.get_type(), "list" | "tuple" | "dict" | "set" | "range") => {
            Ok(Some(v))
        }
        Some(v) => Err(type_error(method, param, v, "Iterable")),
    }
}

/// The elements of a `Sequence` (list, tuple or range).
// Java: net/starlark/java/eval/Sequence.java:131 Sequence.cast
fn sequence_elements<'v>(
    x: Value<'v>,
    what: &str,
    heap: Heap<'v>,
) -> anyhow::Result<Vec<Value<'v>>> {
    if let Some(list) = ListRef::from_value(x) {
        Ok(list.content().to_vec())
    } else if let Some(tuple) = TupleRef::from_value(x) {
        Ok(tuple.content().to_vec())
    } else if x.get_type() == "range" {
        Ok(x.iterate(heap).map_err(|e| e.into_anyhow())?.collect())
    } else {
        Err(anyhow!("for {}, got {}, want sequence", what, x.get_type()))
    }
}

/// `Sequence.cast(x, String.class, what)` on already-extracted elements.
// Java: net/starlark/java/eval/Sequence.java:131 Sequence.cast
fn cast_strings(elements: &[Value], what: &str) -> anyhow::Result<Vec<String>> {
    for (i, elem) in elements.iter().enumerate() {
        if elem.unpack_str().is_none() {
            return Err(anyhow!(
                "at index {} of {}, got element of type {}, want string",
                i,
                what,
                elem.get_type()
            ));
        }
    }
    Ok(elements
        .iter()
        .map(|e| e.unpack_str().expect("checked").to_owned())
        .collect())
}

/// `Sequence.cast(x, String.class, what)`; `None` (the parameter default `[]`) is empty.
fn sequence_cast_strings<'v>(
    x: Option<Value<'v>>,
    what: &str,
    heap: Heap<'v>,
) -> anyhow::Result<Vec<String>> {
    match x {
        None => Ok(Vec::new()),
        Some(x) => cast_strings(&sequence_elements(x, what, heap)?, what),
    }
}

/// `Dict.cast(kwargs, String.class, String.class, what)`.
// Java: net/starlark/java/eval/Dict.java:659 Dict.cast
fn dict_cast_strings(
    kwargs: &SmallMap<String, Value>,
    what: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    for (_, v) in kwargs.iter() {
        if v.unpack_str().is_none() {
            return Err(anyhow!(
                "got dict<string, {}> for '{}', want dict<string, string>",
                v.get_type(),
                what
            ));
        }
    }
    Ok(kwargs
        .iter()
        .map(|(k, v)| (k.clone(), v.unpack_str().expect("checked").to_owned()))
        .collect())
}

/// A `ModuleExtensionProxy` parameter.
fn param_proxy(method: &str, param: &str, value: Value) -> anyhow::Result<ProxyRef> {
    value
        .downcast_ref::<ModuleExtensionProxy>()
        .map(|p| p.proxy)
        .ok_or_else(|| type_error(method, param, value, "module_extension_proxy"))
}

/// A Starlark value as stored in `AttributeValues` (Java keeps the Starlark object itself).
// PORT: Java stores the kwargs `Dict` itself; the model crate stores plain data, so values are
// copied, once the module file that passed them has been executed (see
// `defer_attribute_values`). Values of kinds `AttributeValue` cannot hold (floats, structs,
// functions, ...) are not ported.
// Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValues.java:34 AttributeValues.create
fn to_attribute_value(v: Value) -> AttributeValue {
    if v.is_none() {
        AttributeValue::None
    } else if let Some(b) = v.unpack_bool() {
        AttributeValue::Bool(b)
    } else if v.get_type() == "int" {
        match i64::unpack_value(v) {
            Ok(Some(i)) => AttributeValue::Int(i),
            _ => blaze_util::unported!("AttributeValues.create", "int {} beyond 64 bits", v),
        }
    } else if let Some(s) = v.unpack_str() {
        AttributeValue::String(s.to_owned())
    } else if let Some(label) = v.downcast_ref::<LabelValue>() {
        AttributeValue::Label(label.label().clone())
    } else if let Some(list) = ListRef::from_value(v) {
        AttributeValue::List(list.iter().map(to_attribute_value).collect())
    } else if let Some(tuple) = TupleRef::from_value(v) {
        AttributeValue::List(tuple.iter().map(to_attribute_value).collect())
    } else if let Some(dict) = DictRef::from_value(v) {
        AttributeValue::Dict(
            dict.iter()
                .map(|(k, v)| (to_attribute_value(k), to_attribute_value(v)))
                .collect(),
        )
    } else {
        blaze_util::unported!(
            "AttributeValues.create",
            "attribute value of type {}",
            v.get_type()
        )
    }
}

fn to_attribute_values<'a, 'v: 'a>(
    kwargs: impl IntoIterator<Item = (&'a str, Value<'v>)>,
) -> AttributeValues {
    AttributeValues::create(
        kwargs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), to_attribute_value(v))),
    )
}

/// Keeps the kwargs of a tag or override call alive, as live Starlark values, until the module
/// file that made the call has been executed; `deferred` is the id the context gave the call
/// (`ModuleThreadContext::tag_call_deferred`, `add_override_deferred`).
///
/// PORT: Java's `AttributeValues.create(kwargs)` keeps the kwargs `Dict` itself: the entries are
/// those of the call, but a list or dict among the values can still be mutated by the rest of
/// the file (`l = ["a"]; ext.tag(x = l); l.append("b")` gives the tag `["a", "b"]`), until the
/// `Mutability` of the module file is closed. starlark-rust values cannot be stored in the
/// context (they do not outlive their heap), so the call is recorded on the heap of the file's
/// module: the module's extra value is a chain of tuples
/// `(previous record or None, deferred id, name, value, name, value, ...)`, which the garbage
/// collector traces. [`resolve_deferred_attribute_values`] converts them when the file is done.
// Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValues.java:34 AttributeValues.create
fn defer_attribute_values<'a, 'v: 'a>(
    eval: &Evaluator<'v, '_, '_>,
    deferred: usize,
    kwargs: impl IntoIterator<Item = (&'a str, Value<'v>)>,
) {
    let heap = eval.heap();
    let module = eval.module();
    let mut record: Vec<Value<'v>> = vec![
        module.extra_value().unwrap_or_else(Value::new_none),
        heap.alloc(deferred as i32),
    ];
    for (name, value) in kwargs {
        record.push(heap.alloc(name));
        record.push(value);
    }
    module.set_extra_value(heap.alloc(AllocTuple(record)));
}

/// Converts the kwargs that the tag and override calls of the file executed in `module` left on
/// its heap ([`defer_attribute_values`]) to `AttributeValues`, in call order, and stores them in
/// `context`. Must run after the file has been executed and before its heap is dropped; this is
/// the moment the values stop changing, as Java's do when the module file's `Mutability` closes.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:662 ModuleFileFunction.execModuleFile
pub(crate) fn resolve_deferred_attribute_values<'v>(
    module: &Module<'v>,
    context: &mut ModuleThreadContext,
) {
    let mut records: Vec<&'v [Value<'v>]> = Vec::new();
    let mut next = module.extra_value();
    while let Some(record) = next {
        // The chain ends with None.
        let Some(tuple) = TupleRef::from_value(record) else {
            break;
        };
        let content = tuple.content();
        next = Some(content[0]);
        records.push(content);
    }
    for content in records.into_iter().rev() {
        let deferred = content[1].unpack_i32().expect("deferred id") as usize;
        let attribute_values = to_attribute_values(
            content[2..]
                .chunks_exact(2)
                .map(|entry| (entry[0].unpack_str().expect("attribute name"), entry[1])),
        );
        context.set_deferred_attribute_values(deferred, attribute_values);
    }
    module.set_extra_value(Value::new_none());
}

// ---------------------------------------------------------------------------------------------
// ModuleFileGlobals helpers
// ---------------------------------------------------------------------------------------------

/// Java `Identifier.isValid`.
// Java: net/starlark/java/syntax/Identifier.java:82 Identifier.isValid
fn identifier_is_valid(name: &str) -> bool {
    // Keep consistent with Lexer.scanIdentifier.
    for (i, c) in name.encode_utf16().enumerate() {
        let ok = (u16::from(b'a')..=u16::from(b'z')).contains(&c)
            || (u16::from(b'A')..=u16::from(b'Z')).contains(&c)
            || (i > 0 && (u16::from(b'0')..=u16::from(b'9')).contains(&c))
            || c == u16::from(b'_');
        if !ok {
            return false;
        }
    }
    !name.is_empty()
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:70 ModuleFileGlobals.validateModuleName
pub fn validate_module_name(module_name: &str) -> Result<(), EvalException> {
    if !is_valid_module_name(module_name) {
        return Err(EvalException::new(format!(
            "invalid module name '{module_name}': valid names must 1) only contain lowercase letters (a-z), digits (0-9), dots (.), hyphens (-), and underscores (_); 2) begin with a lowercase letter; 3) end with a lowercase letter or digit."
        )));
    }
    Ok(())
}

/// Java `VALID_BAZEL_COMPATIBILITY_VERSION.matcher(version).matches()` for
/// `(>|<|-|<=|>=)(\d+\.){2}\d+`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:66 ModuleFileGlobals.VALID_BAZEL_COMPATIBILITY_VERSION
fn is_valid_bazel_compatibility_version(version: &str) -> bool {
    let version_matches = |rest: &str| {
        let parts: Vec<&str> = rest.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    [">", "<", "-", "<=", ">="]
        .iter()
        .any(|prefix| version.strip_prefix(prefix).is_some_and(version_matches))
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:198 ModuleFileGlobals.checkAllAbsolutePatterns
fn check_all_absolute_patterns(list: Vec<String>, where_: &str) -> anyhow::Result<Vec<String>> {
    for item in &list {
        if !item.starts_with("//") && !item.starts_with('@') {
            return Err(anyhow!(
                "Expected absolute target patterns (must begin with '//' or '@') for '{}' argument, but got '{}' as an argument",
                where_,
                item
            ));
        }
    }
    Ok(list)
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:212 ModuleFileGlobals.checkAllCompatibilityVersions
fn check_all_compatibility_versions(list: Vec<String>) -> anyhow::Result<Vec<String>> {
    for version in &list {
        if !is_valid_bazel_compatibility_version(version) {
            return Err(anyhow!(
                "invalid version argument '{}': valid argument must 1) start with (<,<=,>,>=,-); 2) contain a version number in form of X.X.X where X is a number",
                version
            ));
        }
    }
    Ok(list)
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:486 ModuleFileGlobals.normalizeLabelString
fn normalize_label_string(
    module: &ModuleThreadContext,
    raw_extension_bzl_file: &str,
) -> Result<String, EvalException> {
    // Normalize the label by parsing and stringifying it with a repo mapping that preserves the
    // apparent repository name, except that a reference to the main repository via the empty
    // repo name is translated to using the module repo name. This is necessary as
    // ModuleExtensionUsages are grouped by the string value of this label, but later mapped to
    // their Label representation. If multiple strings map to the same Label, this would result in a
    // crash.
    // ownName can't change anymore as calling module() after this results in an error.
    let own_name = module
        .module_repo_name()
        .unwrap_or(module.module_name())
        .to_owned();
    let own_repo_name = RepositoryName::create_unvalidated(&own_name);
    let mut repo_mapping: Vec<(String, RepositoryName)> = Vec::new();
    if *module.module_key() == ModuleKey::root() {
        repo_mapping.push((String::new(), own_repo_name.clone()));
    }
    let label = Label::parse_with_package_context(
        raw_extension_bzl_file,
        &LabelPackageContext::of(
            &PackageIdentifier::create(own_repo_name.clone(), PathFragment::empty_fragment()),
            RepositoryMapping::create(repo_mapping, own_repo_name),
        ),
    )
    .map_err(|e| {
        EvalException::new(format!(
            "invalid label \"{}\": {}",
            raw_extension_bzl_file,
            e.get_message()
        ))
    })?;
    let apparent_repo_name = label.get_repository().get_name().to_owned();
    let fabricated_label = Label::create_unvalidated(
        PackageIdentifier::create(
            RepositoryName::create_unvalidated(&apparent_repo_name),
            label.get_package_fragment().clone(),
        ),
        label.get_name(),
    );
    // Skip over the leading "@" of the unambiguous form.
    Ok(fabricated_label.get_unambiguous_canonical_form()[1..].to_owned())
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:522 ModuleFileGlobals.convertAndValidatePatchLabel
fn convert_and_validate_patch_label(
    module: &ModuleThreadContext,
    raw_label: &str,
) -> Result<Label, EvalException> {
    let own_name = module
        .module_repo_name()
        .unwrap_or(module.module_name())
        .to_owned();
    // ImmutableMap.Builder.buildKeepingLast
    let repo_mapping = RepositoryMapping::create(
        [(String::new(), MAIN.clone()), (own_name, MAIN.clone())],
        MAIN.clone(),
    );
    let label = Label::parse_with_package_context(
        raw_label,
        &LabelPackageContext::of(&EMPTY_PACKAGE_ID, repo_mapping),
    )
    .map_err(|e| {
        EvalException::new(format!(
            "invalid label \"{}\" in 'patches': {}",
            raw_label,
            e.get_message()
        ))
    })?;
    if !label.get_repository().is_visible() {
        return Err(EvalException::new(format!(
            "invalid label in 'patches': only patches in the main repository can be applied, not from '@{}'",
            label.get_repository().get_name()
        )));
    }
    Ok(label)
}

// ---------------------------------------------------------------------------------------------
// Proxy values
// ---------------------------------------------------------------------------------------------

/// Java `ModuleFileGlobals.ModuleExtensionProxy` (`module_extension_proxy`): a `Structure` whose
/// every field is a [`TagCallable`], and a `StarlarkExportable` named after the top-level
/// variable it is first assigned to.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:548 ModuleFileGlobals.ModuleExtensionProxy
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ModuleExtensionProxy {
    #[allocative(skip)]
    proxy: ProxyRef,
}

starlark_simple_value!(ModuleExtensionProxy);

impl fmt::Display for ModuleExtensionProxy {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (default)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.bazel.bzlmod.ModuleFileGlobals$ModuleExtensionProxy>",
        )
    }
}

#[starlark_value(type = "module_extension_proxy")]
impl<'v> StarlarkValue<'v> for ModuleExtensionProxy {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:604 ModuleExtensionProxy.getValue
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        Some(heap.alloc(TagCallable {
            proxy: self.proxy,
            tag_name: attribute.to_owned(),
        }))
    }

    fn has_attr(&self, _attribute: &str, _heap: Heap<'v>) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:609 ModuleExtensionProxy.getFieldNames
    fn dir_attr(&self) -> Vec<String> {
        Vec::new()
    }

    /// The post-assign hook of `ModuleFileFunction.execModuleFile`: an unexported exportable
    /// value assigned to a top-level variable is exported under that name.
    // PORT: starlark-rust calls `export_as` on every assignment to a module-level variable, which
    // is where Java's postAssignHook runs.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:675 ModuleFileFunction.execModuleFile
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:625 ModuleExtensionProxy.export
    fn export_as(
        &self,
        variable_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        if let Ok(thread) = ThreadContext::get(eval)
            && let Some(mut context) = thread.payload::<ModuleThreadContext>()
            && !context.proxy_is_exported(self.proxy)
        {
            context.proxy_export(self.proxy, variable_name);
        }
        Ok(())
    }
}

/// Java `ModuleExtensionProxy.TagCallable`: calling it adds a tag.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:579 ModuleExtensionProxy.TagCallable
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TagCallable {
    #[allocative(skip)]
    proxy: ProxyRef,
    tag_name: String,
}

starlark_simple_value!(TagCallable);

impl fmt::Display for TagCallable {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (default)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.bazel.bzlmod.ModuleFileGlobals$ModuleExtensionProxy$TagCallable>",
        )
    }
}

#[starlark_value(type = "TagCallable")]
impl<'v> StarlarkValue<'v> for TagCallable {
    /// Java names a `selfCall` method's frame after the method: `call`.
    fn name_for_call_stack(&self, _me: Value<'v>) -> String {
        "call".to_owned()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:592 TagCallable.call
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        args.no_positional_args(eval.heap())?;
        let kwargs = args.names_map()?;
        let location = caller_location_of(eval);
        // AttributeValues.create(kwargs) keeps the live kwargs; see defer_attribute_values.
        let deferred = {
            let mut context = proxy_context(eval)?;
            context.tag_call_deferred(self.proxy, &self.tag_name, location)
        };
        defer_attribute_values(eval, deferred, kwargs.iter().map(|(k, v)| (k.as_str(), *v)));
        Ok(Value::new_none())
    }
}

/// Java `ModuleFileGlobals.RepoRuleProxy` (`repo_rule_proxy`), returned by `use_repo_rule`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:832 ModuleFileGlobals.RepoRuleProxy
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct RepoRuleProxy {
    usage_builder: usize,
}

starlark_simple_value!(RepoRuleProxy);

impl fmt::Display for RepoRuleProxy {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (default)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.bazel.bzlmod.ModuleFileGlobals$RepoRuleProxy>",
        )
    }
}

#[starlark_value(type = "repo_rule_proxy")]
impl<'v> StarlarkValue<'v> for RepoRuleProxy {
    /// Java names a `selfCall` method's frame after the method: `call`.
    fn name_for_call_stack(&self, _me: Value<'v>) -> String {
        "call".to_owned()
    }

    /// `call(*, name, dev_dependency = False, **kwargs)`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:850 RepoRuleProxy.call
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        args.no_positional_args(eval.heap())?;
        let named = args.names_map()?;
        let mut name: Option<&str> = None;
        let mut dev_dependency: Option<bool> = None;
        let mut kwargs: Vec<(&str, Value<'v>)> = Vec::new();
        for (k, v) in named.iter() {
            match k.as_str() {
                "name" => name = Some(param_str("call", "name", *v).map_err(native_error)?),
                "dev_dependency" => {
                    dev_dependency =
                        Some(param_bool("call", "dev_dependency", *v).map_err(native_error)?)
                }
                other => kwargs.push((other, *v)),
            }
        }
        // Java: net/starlark/java/eval/BuiltinFunction.java:486 (missing named parameters)
        let Some(name) = name else {
            return Err(native_error(anyhow!(
                "call() missing 1 required named argument: name"
            )));
        };
        let dev_dependency = dev_dependency.unwrap_or(false);
        let location = caller_location_of(eval);

        let mut context = proxy_context(eval)?;
        validate_user_provided_repo_name(name).map_err(native_error)?;
        if context.should_ignore_dev_deps() && dev_dependency {
            return Ok(Value::new_none());
        }
        kwargs.push(("name", eval.heap().alloc(name)));
        let containing_module_file_path = context.get_current_module_file_path().clone();
        let extension_proxy = context.new_module_extension_proxy(
            self.usage_builder,
            ProxyBuilder::new(
                dev_dependency,
                location.clone(),
                containing_module_file_path,
            ),
        );
        // extensionProxy.getValue("repo").call(kwargs, thread): AttributeValues.create(kwargs)
        // keeps the live kwargs; see defer_attribute_values.
        let deferred = context.tag_call_deferred(extension_proxy, "repo", location);
        defer_attribute_values(eval, deferred, kwargs);
        let stack = call_stack(eval, &context.call_stack_prefix);
        context
            .proxy_add_import(extension_proxy, name, name, "by a repo rule", stack)
            .map_err(native_error)?;
        Ok(Value::new_none())
    }
}

// ---------------------------------------------------------------------------------------------
// The globals
// ---------------------------------------------------------------------------------------------

/// A collection of global Starlark build API functions that apply to MODULE.bazel files.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:61 ModuleFileGlobals
#[starlark_module]
pub fn register_module_file_globals(builder: &mut GlobalsBuilder) {
    /// Declares certain properties of the Bazel module represented by the current Bazel repo.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:148 ModuleFileGlobals.module
    fn module<'v>(
        #[starlark(require = named)] name: Option<Value<'v>>,
        #[starlark(require = named)] version: Option<Value<'v>>,
        #[starlark(require = named)] compatibility_level: Option<Value<'v>>,
        #[starlark(require = named)] repo_name: Option<Value<'v>>,
        #[starlark(require = named)] bazel_compatibility: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let name = opt_str("module", "name", name, "")?;
        let version = opt_str("module", "version", version, "")?;
        let compatibility_level = opt_int("module", "compatibility_level", compatibility_level)?;
        let repo_name = opt_str("module", "repo_name", repo_name, "")?;
        let bazel_compatibility =
            opt_iterable("module", "bazel_compatibility", bazel_compatibility)?;

        let mut context = from_or_fail(eval, "module()")?;
        if context.is_module_called() {
            return Err(anyhow!("the module() directive can only be called once"));
        }
        if to_int(compatibility_level, -1, "compatibility_level")? != -1
            && *context.module_key() == ModuleKey::root()
        {
            context.add_warning(Event::warn_at(
                Some(caller_location_of(eval)),
                "The attribute 'compatibility_level' in module() is a no-op and will be removed in a future Bazel release. Please remove it from your MODULE.bazel file.",
            ));
        }
        if context.had_non_module_call() {
            return Err(anyhow!(
                "if module() is called, it must be called before any other functions"
            ));
        }
        context.set_module_called();
        if !name.is_empty() {
            validate_module_name(name)?;
        }
        let repo_name = if repo_name.is_empty() {
            let stack = call_stack(eval, &context.call_stack_prefix);
            context.add_repo_name_usage(name, "as the current module name", stack)?;
            name
        } else {
            validate_user_provided_repo_name(repo_name)?;
            let stack = call_stack(eval, &context.call_stack_prefix);
            context.add_repo_name_usage(repo_name, "as the module's own repo name", stack)?;
            repo_name
        };
        // Java: new EvalException("Invalid version in module()", e)
        let parsed_version =
            Version::parse(version).map_err(|_| anyhow!("Invalid version in module()"))?;
        let bazel_compatibility = check_all_compatibility_versions(sequence_cast_strings(
            bazel_compatibility,
            "bazel_compatibility",
            eval.heap(),
        )?)?;
        context.set_module_properties(name, parsed_version, 0, bazel_compatibility, repo_name);
        Ok(NoneType)
    }

    /// Declares a direct dependency on another Bazel module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:274 ModuleFileGlobals.bazelDep
    fn bazel_dep<'v>(
        #[starlark(require = named)] name: Value<'v>,
        #[starlark(require = named)] version: Option<Value<'v>>,
        #[starlark(require = named)] max_compatibility_level: Option<Value<'v>>,
        #[starlark(require = named)] repo_name: Option<Value<'v>>,
        #[starlark(require = named)] dev_dependency: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let name = param_str("bazel_dep", "name", name)?;
        let version = opt_str("bazel_dep", "version", version, "")?;
        let max_compatibility_level = opt_int(
            "bazel_dep",
            "max_compatibility_level",
            max_compatibility_level,
        )?;
        // allowedTypes = {String, NoneType}, default ''.
        let repo_name_arg: Option<&str> = match repo_name {
            None => Some(""),
            Some(v) if v.is_none() => None,
            Some(v) => Some(
                v.unpack_str()
                    .ok_or_else(|| type_error("bazel_dep", "repo_name", v, "string or NoneType"))?,
            ),
        };
        let dev_dependency = opt_bool("bazel_dep", "dev_dependency", dev_dependency, false)?;

        let mut context = from_or_fail(eval, "bazel_dep()")?;
        context.set_non_module_called();
        validate_module_name(name)?;
        let parsed_version =
            Version::parse(version).map_err(|_| anyhow!("Invalid version in bazel_dep()"))?;
        if to_int(max_compatibility_level, -1, "max_compatibility_level")? != -1
            && *context.module_key() == ModuleKey::root()
        {
            context.add_warning(Event::warn_at(
                Some(caller_location_of(eval)),
                "The attribute 'max_compatibility_level' in bazel_dep() is a no-op and will be removed in a future Bazel release. Please remove it from your MODULE.bazel file.",
            ));
        }

        let repo_name: Option<&str> = match repo_name_arg {
            None => None,
            Some("") => Some(name),
            Some(s) => {
                validate_user_provided_repo_name(s)?;
                Some(s)
            }
        };

        if !(context.should_ignore_dev_deps() && dev_dependency) {
            context.add_dep(repo_name, DepSpec::new(name, parsed_version, -1));
        }

        if let Some(repo_name) = repo_name {
            let stack = call_stack(eval, &context.call_stack_prefix);
            context.add_repo_name_usage(repo_name, "by a bazel_dep", stack)?;
        }
        Ok(NoneType)
    }

    /// Specifies already-defined execution platforms to be registered when this module is
    /// selected.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:347 ModuleFileGlobals.registerExecutionPlatforms
    fn register_execution_platforms<'v>(
        #[starlark(args)] platform_labels: UnpackTuple<Value<'v>>,
        #[starlark(require = named)] dev_dependency: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let dev_dependency = opt_bool(
            "register_execution_platforms",
            "dev_dependency",
            dev_dependency,
            false,
        )?;
        let mut context = from_or_fail(eval, "register_execution_platforms()")?;
        context.set_non_module_called();
        if context.should_ignore_dev_deps() && dev_dependency {
            return Ok(NoneType);
        }
        let patterns = check_all_absolute_patterns(
            cast_strings(&platform_labels.items, "register_execution_platforms")?,
            "register_execution_platforms",
        )?;
        context.add_execution_platforms_to_register(patterns);
        Ok(NoneType)
    }

    /// Specifies already-defined toolchains to be registered when this module is selected.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:389 ModuleFileGlobals.registerToolchains
    fn register_toolchains<'v>(
        #[starlark(args)] toolchain_labels: UnpackTuple<Value<'v>>,
        #[starlark(require = named)] dev_dependency: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let dev_dependency = opt_bool(
            "register_toolchains",
            "dev_dependency",
            dev_dependency,
            false,
        )?;
        let single_package_toolchain_binding = ThreadContext::get(eval)
            .map(|t| t.semantics.experimental_single_package_toolchain_binding)
            .unwrap_or(false);
        let mut context = from_or_fail(eval, "register_toolchains()")?;
        context.set_non_module_called();
        if context.should_ignore_dev_deps() && dev_dependency {
            return Ok(NoneType);
        }
        let checked_toolchain_labels = check_all_absolute_patterns(
            cast_strings(&toolchain_labels.items, "register_toolchains")?,
            "register_toolchains",
        )?;
        if single_package_toolchain_binding {
            for label in &checked_toolchain_labels {
                if label.contains("...") {
                    return Err(anyhow!(
                        "invalid target pattern \"{}\": register_toolchain target patterns may only refer to targets within a single package",
                        label
                    ));
                }
            }
        }
        context.add_toolchains_to_register(checked_toolchain_labels);
        Ok(NoneType)
    }

    /// Returns a proxy object representing a module extension; its methods can be invoked to
    /// create module extension tags.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:452 ModuleFileGlobals.useExtension
    fn use_extension<'v>(
        #[starlark(require = pos)] extension_bzl_file: Value<'v>,
        #[starlark(require = pos)] extension_name: Value<'v>,
        #[starlark(require = named)] dev_dependency: Option<Value<'v>>,
        #[starlark(require = named)] isolate: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<ModuleExtensionProxy> {
        let raw_extension_bzl_file =
            param_str("use_extension", "extension_bzl_file", extension_bzl_file)?;
        let extension_name = param_str("use_extension", "extension_name", extension_name)?;
        let dev_dependency = opt_bool("use_extension", "dev_dependency", dev_dependency, false)?;
        // enableOnlyWithFlag = "-experimental_isolated_extension_usages"
        // Java: net/starlark/java/eval/BuiltinFunction.java:370 (disabled parameter)
        let isolated_extension_usages = ThreadContext::get(eval)
            .map(|t| t.semantics.experimental_isolated_extension_usages)
            .unwrap_or(false);
        if isolate.is_some() && !isolated_extension_usages {
            return Err(anyhow!(
                "in call to use_extension(), parameter 'isolate' is experimental and thus unavailable with the current flags. It may be enabled by setting --experimental_isolated_extension_usages"
            ));
        }
        let isolate = opt_bool("use_extension", "isolate", isolate, false)?;

        let mut context = from_or_fail(eval, "use_extension()")?;
        context.set_non_module_called();

        if !identifier_is_valid(extension_name) {
            return Err(anyhow!(
                "extension name is not a valid identifier: {}",
                extension_name
            ));
        }

        let proxy_builder = ProxyBuilder::new(
            dev_dependency,
            caller_location_of(eval),
            context.get_current_module_file_path().clone(),
        );

        let extension_bzl_file = normalize_label_string(&context, raw_extension_bzl_file)?;

        if context.should_ignore_dev_deps() && dev_dependency {
            // This is a no-op proxy.
            let usage_builder =
                context.new_detached_usage_builder(&extension_bzl_file, extension_name, isolate);
            let proxy = context.new_module_extension_proxy(usage_builder, proxy_builder);
            return Ok(ModuleExtensionProxy { proxy });
        }

        let usage_builder = context.get_or_create_extension_usage_builder(
            &extension_bzl_file,
            extension_name,
            isolate,
        );
        let proxy = context.new_module_extension_proxy(usage_builder, proxy_builder);
        Ok(ModuleExtensionProxy { proxy })
    }

    /// Imports one or more repos generated by the given module extension into the scope of the
    /// current module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:656 ModuleFileGlobals.useRepo
    fn use_repo<'v>(
        #[starlark(require = pos)] extension_proxy: Value<'v>,
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        #[starlark(kwargs)] kwargs: SmallMap<String, Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let extension_proxy = param_proxy("use_repo", "extension_proxy", extension_proxy)?;
        let mut context = from_or_fail(eval, "use_repo()")?;
        context.set_non_module_called();
        let stack = call_stack(eval, &context.call_stack_prefix);
        for arg in cast_strings(&args.items, "args")? {
            context.proxy_add_import(
                extension_proxy,
                &arg,
                &arg,
                "by a use_repo() call",
                stack.clone(),
            )?;
        }
        let module_name = context.module_name().to_owned();
        let module_version = context.module_version().get_normalized().to_owned();
        for (key, value) in dict_cast_strings(&kwargs, "kwargs")? {
            let exported_name = value
                .replace("{name}", &module_name)
                .replace("{version}", &module_version);
            context.proxy_add_import(
                extension_proxy,
                &key,
                &exported_name,
                "by a use_repo() call",
                stack.clone(),
            )?;
        }
        Ok(NoneType)
    }

    /// Overrides one or more repos defined by the given module extension with the given repos
    /// visible to the current module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:716 ModuleFileGlobals.overrideRepo
    fn override_repo<'v>(
        #[starlark(require = pos)] extension_proxy: Value<'v>,
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        #[starlark(kwargs)] kwargs: SmallMap<String, Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let extension_proxy = param_proxy("override_repo", "extension_proxy", extension_proxy)?;
        let mut context = from_or_fail(eval, "override_repo()")?;
        context.set_non_module_called();
        if context.should_ignore_dev_deps() {
            // Ignore calls early as they may refer to repos that are dev dependencies (or this is not the
            // root module).
            return Ok(NoneType);
        }
        let stack = call_stack(eval, &context.call_stack_prefix);
        for arg in cast_strings(&args.items, "args")? {
            context.proxy_add_override(
                extension_proxy,
                &arg,
                &arg,
                /* must_exist= */ true,
                stack.clone(),
            )?;
        }
        for (key, value) in dict_cast_strings(&kwargs, "kwargs")? {
            context.proxy_add_override(
                extension_proxy,
                &key,
                &value,
                /* must_exist= */ true,
                stack.clone(),
            )?;
        }
        Ok(NoneType)
    }

    /// Injects one or more new repos into the given module extension.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:776 ModuleFileGlobals.injectRepo
    fn inject_repo<'v>(
        #[starlark(require = pos)] extension_proxy: Value<'v>,
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        #[starlark(kwargs)] kwargs: SmallMap<String, Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let extension_proxy = param_proxy("inject_repo", "extension_proxy", extension_proxy)?;
        let mut context = from_or_fail(eval, "inject_repo()")?;
        context.set_non_module_called();
        if context.should_ignore_dev_deps() {
            // Ignore calls early as they may refer to repos that are dev dependencies (or this is not the
            // root module).
            return Ok(NoneType);
        }
        let stack = call_stack(eval, &context.call_stack_prefix);
        for arg in cast_strings(&args.items, "args")? {
            context.proxy_add_override(
                extension_proxy,
                &arg,
                &arg,
                /* must_exist= */ false,
                stack.clone(),
            )?;
        }
        for (key, value) in dict_cast_strings(&kwargs, "kwargs")? {
            context.proxy_add_override(
                extension_proxy,
                &key,
                &value,
                /* must_exist= */ false,
                stack.clone(),
            )?;
        }
        Ok(NoneType)
    }

    /// Returns a proxy value that can be directly invoked in the MODULE.bazel file as a
    /// repository rule, one or more times.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:819 ModuleFileGlobals.useRepoRule
    fn use_repo_rule<'v>(
        #[starlark(require = pos)] repo_rule_bzl_file: Value<'v>,
        #[starlark(require = pos)] repo_rule_name: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<RepoRuleProxy> {
        let bzl_file = param_str("use_repo_rule", "repo_rule_bzl_file", repo_rule_bzl_file)?;
        let rule_name = param_str("use_repo_rule", "repo_rule_name", repo_rule_name)?;
        let mut context = from_or_fail(eval, "use_repo_rule()")?;
        context.set_non_module_called();
        // Not a valid Starlark identifier so that it can't collide with a real extension.
        let extension_name = format!("{bzl_file} {rule_name}");
        // Find or create the builder for the singular "innate" extension of this repo rule for this
        // module.
        let usage_builder = context.get_or_create_extension_usage_builder(
            "//:MODULE.bazel",
            &extension_name,
            /* isolate= */ false,
        );
        Ok(RepoRuleProxy { usage_builder })
    }

    /// Includes the contents of another MODULE.bazel-like file.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:903 ModuleFileGlobals.include
    fn include<'v>(
        #[starlark(require = pos)] label: Value<'v>,
        #[starlark(require = named)] dev_dependency: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let label = param_str("include", "label", label)?;
        let dev_dependency = opt_bool("include", "dev_dependency", dev_dependency, false)?;
        // Java: ModuleThreadContext.include, split around the execution of the included file so
        // that the context is not borrowed while it runs.
        let (frame, including_prefix) = {
            let mut context = from_or_fail(eval, "include()")?;
            context.set_non_module_called();
            let Some(frame) = context.include_begin(label, dev_dependency)? else {
                return Ok(NoneType);
            };
            let stack = call_stack(eval, &context.call_stack_prefix);
            let including_prefix = std::mem::replace(&mut context.call_stack_prefix, stack);
            (frame, including_prefix)
        };
        let thread = ThreadContext::get(eval)?;
        let result = frame.compiled_module_file.run_on_thread(thread);
        {
            let mut context = from_or_fail(eval, "include()")?;
            context.include_end(&frame);
            context.call_stack_prefix = including_prefix;
        }
        result.map_err(|e| anyhow::Error::new(IncludedFileError(e)))?;
        Ok(NoneType)
    }

    /// Specifies that a dependency should still come from a registry, but its version should be
    /// pinned, or its registry overridden, or a list of patches applied.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:973 ModuleFileGlobals.singleVersionOverride
    fn single_version_override<'v>(
        #[starlark(require = named)] module_name: Value<'v>,
        #[starlark(require = named)] version: Option<Value<'v>>,
        #[starlark(require = named)] registry: Option<Value<'v>>,
        #[starlark(require = named)] patches: Option<Value<'v>>,
        #[starlark(require = named)] patch_cmds: Option<Value<'v>>,
        #[starlark(require = named)] patch_strip: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let method = "single_version_override";
        let module_name = param_str(method, "module_name", module_name)?;
        let version = opt_str(method, "version", version, "")?;
        let registry = opt_str(method, "registry", registry, "")?;
        let patches = opt_iterable(method, "patches", patches)?;
        let patch_cmds = opt_iterable(method, "patch_cmds", patch_cmds)?;
        let patch_strip = opt_int(method, "patch_strip", patch_strip)?;

        let mut context = from_or_fail(eval, "single_version_override()")?;
        context.set_non_module_called();
        validate_module_name(module_name)?;
        let parsed_version = Version::parse(version)
            .map_err(|_| anyhow!("Invalid version in single_version_override(): {}", version))?;
        let mut patches_builder = Vec::new();
        for patch in sequence_cast_strings(patches, "patches", eval.heap())? {
            patches_builder.push(convert_and_validate_patch_label(&context, &patch)?);
        }
        let patch_cmds = sequence_cast_strings(patch_cmds, "patchCmds", eval.heap())?;
        let patch_strip = to_int(patch_strip, 0, "single_version_override.patch_strip")?;
        context.add_override(
            module_name,
            ModuleOverride::SingleVersion(SingleVersionOverride {
                version: parsed_version,
                registry: registry.to_owned(),
                patches: patches_builder,
                patch_cmds,
                patch_strip,
            }),
        )?;
        Ok(NoneType)
    }

    /// Specifies that a dependency should still come from a registry, but multiple versions of it
    /// should be allowed to coexist.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:1043 ModuleFileGlobals.multipleVersionOverride
    fn multiple_version_override<'v>(
        #[starlark(require = named)] module_name: Value<'v>,
        #[starlark(require = named)] versions: Value<'v>,
        #[starlark(require = named)] registry: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let method = "multiple_version_override";
        let module_name = param_str(method, "module_name", module_name)?;
        let versions = opt_iterable(method, "versions", Some(versions))?;
        let registry = opt_str(method, "registry", registry, "")?;

        let mut context = from_or_fail(eval, "multiple_version_override()")?;
        context.set_non_module_called();
        validate_module_name(module_name)?;
        let mut parsed_versions = Vec::new();
        for version in sequence_cast_strings(versions, "versions", eval.heap())? {
            parsed_versions.push(
                Version::parse(&version)
                    .map_err(|_| anyhow!("Invalid version in multiple_version_override()"))?,
            );
        }
        if parsed_versions.len() < 2 {
            return Err(anyhow!(
                "multiple_version_override() must specify at least 2 versions"
            ));
        }
        context.add_override(
            module_name,
            ModuleOverride::MultipleVersion(MultipleVersionOverride {
                versions: parsed_versions,
                registry: registry.to_owned(),
            }),
        )?;
        Ok(NoneType)
    }

    /// Specifies that this dependency should come from an archive file (zip, gzip, etc) at a
    /// certain location, instead of from a registry.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:1093 ModuleFileGlobals.archiveOverride
    fn archive_override<'v>(
        #[starlark(require = named)] module_name: Value<'v>,
        #[starlark(kwargs)] kwargs: SmallMap<String, Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let module_name = param_str("archive_override", "module_name", module_name)?;
        let mut context = from_or_fail(eval, "archive_override()")?;
        context.set_non_module_called();
        validate_module_name(module_name)?;
        let patches = match kwargs.get("patches") {
            None => Vec::new(),
            Some(v) => cast_strings(&sequence_elements(*v, "patches", eval.heap())?, "patches")?,
        };
        for patch in &patches {
            let _unused = convert_and_validate_patch_label(&context, patch)?;
        }
        // new NonRegistryOverride(new RepoSpec(HTTP_ARCHIVE, AttributeValues.create(kwargs))):
        // AttributeValues.create keeps the live kwargs; see defer_attribute_values.
        if let Some(deferred) = context.add_override_deferred(module_name, HTTP_ARCHIVE.clone())? {
            defer_attribute_values(eval, deferred, kwargs.iter().map(|(k, v)| (k.as_str(), *v)));
        }
        Ok(NoneType)
    }

    /// Specifies that this dependency should come from a certain commit in a Git repository,
    /// instead of from a registry.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:1138 ModuleFileGlobals.gitOverride
    fn git_override<'v>(
        #[starlark(require = named)] module_name: Value<'v>,
        #[starlark(kwargs)] kwargs: SmallMap<String, Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let module_name = param_str("git_override", "module_name", module_name)?;
        let mut context = from_or_fail(eval, "git_override()")?;
        context.set_non_module_called();
        validate_module_name(module_name)?;
        let patches = match kwargs.get("patches") {
            None => Vec::new(),
            Some(v) => cast_strings(&sequence_elements(*v, "patches", eval.heap())?, "patches")?,
        };
        for patch in &patches {
            let _unused = convert_and_validate_patch_label(&context, patch)?;
        }
        // new NonRegistryOverride(new RepoSpec(GIT_REPOSITORY, AttributeValues.create(kwargs))):
        // AttributeValues.create keeps the live kwargs; see defer_attribute_values.
        if let Some(deferred) =
            context.add_override_deferred(module_name, GIT_REPOSITORY.clone())?
        {
            defer_attribute_values(eval, deferred, kwargs.iter().map(|(k, v)| (k.as_str(), *v)));
        }
        Ok(NoneType)
    }

    /// Specifies that this dependency should come from a certain directory on local disk,
    /// instead of from a registry.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:1179 ModuleFileGlobals.localPathOverride
    fn local_path_override<'v>(
        #[starlark(require = named)] module_name: Value<'v>,
        #[starlark(require = named)] path: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let module_name = param_str("local_path_override", "module_name", module_name)?;
        let path = param_str("local_path_override", "path", path)?;
        let mut context = from_or_fail(eval, "local_path_override()")?;
        context.set_non_module_called();
        validate_module_name(module_name)?;
        context.add_override(
            module_name,
            ModuleOverride::NonRegistry(NonRegistryOverride::new(local_path_repo_spec(path))),
        )?;
        Ok(NoneType)
    }

    /// Maps a command-line flag --foo to a Starlark flag --@repo//defs:foo.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileGlobals.java:1203 ModuleFileGlobals.flagAlias
    fn flag_alias<'v>(
        #[starlark(require = named)] name: Value<'v>,
        #[starlark(require = named)] starlark_flag: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let native_name = param_str("flag_alias", "name", name)?;
        let starlark_label = param_str("flag_alias", "starlark_flag", starlark_flag)?;
        let mut context = from_or_fail(eval, "flag_alias()")?;
        let normalized_starlark_label = normalize_label_string(&context, starlark_label)?;

        // TODO: add input validation for stalark flag label
        context.set_non_module_called();
        context.add_flag_alias(native_name, &normalized_starlark_label);
        Ok(NoneType)
    }
}
