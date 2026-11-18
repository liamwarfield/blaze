//! Predeclared environments.
//!
//! Java: net/starlark/java/eval/Starlark.java:112 makeUniverse (core builtins),
//! net/starlark/java/eval/MethodLibrary.java (print, fail), and
//! com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java +
//! analysis/starlark/StarlarkGlobalsImpl.java (which names each file kind sees). The per-kind
//! composition is ported by `blaze_packages`; this module supplies the universe, the
//! registry that crates fill with their natives, and blaze_starlark's own builtins.

use std::sync::Arc;

use starlark::environment::Globals;
use starlark::environment::GlobalsBuilder;
use starlark::environment::LibraryExtension;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Value;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::tuple::UnpackTuple;

use crate::error::Location;
use crate::file_kind::FileKind;
use crate::printer::debug_print;
use crate::semantics::StarlarkSemantics;
use crate::thread::ThreadContext;

/// Names in Bazel's `Starlark.UNIVERSE` (MethodLibrary + True/False/None). Under
/// `--noexperimental_enable_starlark_set` (default: enabled) `set` is still defined and fails
/// when called.
pub const BAZEL_UNIVERSE: &[&str] = &[
    "False",
    "True",
    "None",
    "min",
    "max",
    "abs",
    "all",
    "any",
    "sorted",
    "reversed",
    "tuple",
    "list",
    "len",
    "str",
    "repr",
    "bool",
    "float",
    "int",
    "dict",
    "set",
    "enumerate",
    "hash",
    "range",
    "hasattr",
    "getattr",
    "dir",
    "fail",
    "print",
    "type",
    "zip",
];

/// A builder holding Bazel's universe: starlark-rust's standard globals restricted to
/// [`BAZEL_UNIVERSE`] (dropping `bytes`, `chr`, `ord`), plus Bazel's `print` and `fail`.
pub fn universe_builder(semantics: &StarlarkSemantics) -> GlobalsBuilder {
    let full = GlobalsBuilder::standard()
        .with(|b| LibraryExtension::SetType.add(b))
        .build();
    let mut builder = GlobalsBuilder::new();
    builder.frozen_heap().add_reference(full.heap());
    for (name, value) in full.iter() {
        if !BAZEL_UNIVERSE.contains(&name) {
            continue;
        }
        if name == "set" && !semantics.experimental_enable_starlark_set {
            // Java keeps `set` defined and fails when it is called (below).
            continue;
        }
        builder.set(name, value);
    }
    let builder = builder.with(register_print_fail);
    if semantics.experimental_enable_starlark_set {
        builder
    } else {
        builder.with(register_disabled_set)
    }
}

/// `set` under `--noexperimental_enable_starlark_set`: the name stays defined (so a file that
/// mentions `set()` in a function it never calls loads) and a call fails.
#[starlark_module]
fn register_disabled_set(builder: &mut GlobalsBuilder) {
    /// Java: "Ordinarily we would use StarlarkMethod#enableOnlyWithFlag, but this doesn't work
    /// for top-level symbols, so enforce it here instead."
    // Java: net/starlark/java/eval/MethodLibrary.java:686 MethodLibrary.set
    fn set<'v>(
        #[starlark(require = pos)] elements: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        // Java converts the argument to `StarlarkIterable` before the method runs.
        // PORT: a non-iterable fails with starlark-rust's text (core-language text,
        // decisions.md 2026-11-17).
        if let Some(elements) = elements {
            elements.iterate(eval.heap())?;
        }
        // Java checks the thread's semantics; the Globals are built for the same semantics.
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "Use of set() requires --experimental_enable_starlark_set"
        )))
    }
}

/// Marker in a `fail()` error for `stack_trace = False` (Java `EvalException` with
/// includeStackTrace = false): render the message alone.
#[derive(Debug)]
pub struct FailWithoutStack(pub String);

impl std::fmt::Display for FailWithoutStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FailWithoutStack {}

/// The `(` of the innermost call: Java `StarlarkThread.getCallerLocation()` for a builtin.
pub fn caller_location(eval: &Evaluator) -> Option<Location> {
    eval.call_stack_top_location()
        .map(|l| Location::bazel_position(&l))
}

#[starlark_module]
fn register_print_fail(builder: &mut GlobalsBuilder) {
    /// Java: net/starlark/java/eval/MethodLibrary.java:918 MethodLibrary.print
    fn print<'v>(
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        #[starlark(require = named, default = " ")] sep: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let cx = ThreadContext::get(eval).ok();
        let mut msg = args
            .items
            .iter()
            .map(|x| debug_print(*x, cx))
            .collect::<Vec<_>>()
            .join(sep);
        if cx.is_some_and(|c| c.semantics.internal_starlark_flag_test_canary) {
            msg.push_str("<== Starlark flag test ==>");
        }
        let loc = caller_location(eval);
        match cx {
            Some(c) => c.events.debug(loc.as_ref(), &msg),
            None => eprintln!(
                "DEBUG: {}: {msg}",
                loc.map(|l| l.to_string()).unwrap_or_default()
            ),
        }
        Ok(NoneType)
    }

    /// Java: net/starlark/java/eval/MethodLibrary.java fail
    fn fail<'v>(
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] msg: NoneOr<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] attr: NoneOr<&str>,
        #[starlark(require = named, default = " ")] sep: &str,
        #[starlark(require = named, default = true)] stack_trace: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let cx = ThreadContext::get(eval).ok();
        let include_stack =
            stack_trace || cx.is_some_and(|c| c.semantics.force_starlark_stack_trace);
        let mut out = String::new();
        let mut need_sep = false;
        if let NoneOr::Other(a) = attr {
            out.push_str("attribute ");
            out.push_str(a);
            out.push(':');
            need_sep = true;
        }
        if let NoneOr::Other(m) = msg {
            if need_sep {
                out.push_str(sep);
            }
            out.push_str(&debug_print(m, cx));
            need_sep = true;
        }
        for a in &args.items {
            if need_sep {
                out.push_str(sep);
            }
            out.push_str(&debug_print(*a, cx));
            need_sep = true;
        }
        let e = if include_stack {
            anyhow::anyhow!("{out}")
        } else {
            anyhow::Error::new(FailWithoutStack(out))
        };
        Err(starlark::Error::new_kind(starlark::ErrorKind::Fail(e)))
    }
}

/// A function adding natives to a builder (a `#[starlark_module]` function).
pub type NativeFn = fn(&mut GlobalsBuilder);

#[derive(Clone)]
struct Registration {
    kinds: Vec<FileKind>,
    namespace: Option<&'static str>,
    f: NativeFn,
}

/// Natives contributed by crates, per file kind. Crates register their `#[starlark_module]`
/// functions once; [`NativeRegistry::build`] composes the `Globals` for a kind.
#[derive(Clone, Default)]
pub struct NativeRegistry {
    entries: Vec<Registration>,
}

impl NativeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The builtins this crate owns, in the environments Java puts them in:
    /// `depset` and `json` (BUILD, .bzl: StarlarkGlobalsImpl.addCommonUtilToplevels), `struct`
    /// (.bzl, .scl: getFixedBzlToplevels/getSclToplevels), `Label` (.bzl:
    /// StarlarkRuleClassFunctions).
    pub fn with_blaze_starlark_builtins() -> Self {
        let mut r = Self::new();
        let bzl = [FileKind::Bzl, FileKind::BuiltinsBzl, FileKind::BuildPrelude];
        let build_and_bzl = [
            FileKind::Build,
            FileKind::Bzl,
            FileKind::BuiltinsBzl,
            FileKind::BuildPrelude,
        ];
        r.register(&build_and_bzl, crate::depset::register_depset);
        r.register(&build_and_bzl, crate::json::register_json);
        r.register(
            &[
                FileKind::Bzl,
                FileKind::BuiltinsBzl,
                FileKind::BuildPrelude,
                FileKind::Scl,
            ],
            crate::structs::register_struct,
        );
        r.register(&bzl, crate::label::register_label);
        r
    }

    /// Adds `f`'s natives as globals of `kinds`.
    pub fn register(&mut self, kinds: &[FileKind], f: NativeFn) {
        self.entries.push(Registration {
            kinds: kinds.to_vec(),
            namespace: None,
            f,
        });
    }

    /// Adds `f`'s natives to the namespace `name` (e.g. `native`) of `kinds`. Several
    /// registrations of one namespace are merged.
    pub fn register_namespace(&mut self, kinds: &[FileKind], name: &'static str, f: NativeFn) {
        self.entries.push(Registration {
            kinds: kinds.to_vec(),
            namespace: Some(name),
            f,
        });
    }

    /// Universe + every registration for `kind`. Later registrations override earlier names.
    pub fn build(&self, kind: FileKind, semantics: &StarlarkSemantics) -> Globals {
        let mut b = universe_builder(semantics);
        let mut namespaces: Vec<(&'static str, Vec<NativeFn>)> = Vec::new();
        for e in &self.entries {
            if !e.kinds.contains(&kind) {
                continue;
            }
            match e.namespace {
                None => (e.f)(&mut b),
                Some(ns) => match namespaces.iter_mut().find(|(n, _)| *n == ns) {
                    Some((_, fs)) => fs.push(e.f),
                    None => namespaces.push((ns, vec![e.f])),
                },
            }
        }
        for (ns, fs) in namespaces {
            b.namespace(ns, |nb| {
                for f in fs {
                    f(nb);
                }
            });
        }
        b.build()
    }
}

/// Shared `Globals` per (kind, semantics) are cheap to clone (`Arc` inside); callers cache them.
pub type SharedGlobals = Arc<Globals>;
