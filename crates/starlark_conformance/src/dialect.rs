//! Bazel-like dialect and core globals on starlark-rust 0.14.2.
//!
//! The Java side is `net.starlark.java.syntax.FileOptions` (per file kind) and
//! `net.starlark.java.eval.Starlark.UNIVERSE` (core builtins, from `MethodLibrary`).

use starlark::environment::Globals;
use starlark::environment::GlobalsBuilder;
use starlark::environment::LibraryExtension;
use starlark::syntax::Dialect;
use starlark::syntax::DialectTypes;

/// Dialect for `.bzl` files (and the ScriptTest chunks, which use `FileOptions.DEFAULT`).
///
/// Java: net/starlark/java/syntax/FileOptions.java (DEFAULT) and
/// com/google/devtools/build/lib/skyframe/BzlCompileFunction.java (bzl options).
pub fn bzl_dialect() -> Dialect {
    Dialect {
        enable_def: true,
        enable_lambda: true,
        enable_load: true,
        // Bazel accepts `def f(a, *, b)`.
        enable_keyword_only_arguments: true,
        // Bazel has no `/` parameter marker.
        enable_positional_only_arguments: false,
        // `--experimental_starlark_type_syntax` is off by default in 9.3.2.
        enable_types: DialectTypes::Disable,
        // Bazel: load() binds file-locally; loaded names are not re-exported.
        enable_load_reexport: false,
        // Bazel Resolver: "for loops / if statements are not allowed at the top level".
        enable_top_level_stmt: false,
        enable_f_strings: false,
        ..Dialect::Standard
    }
}

/// Dialect for BUILD files. Parsing is more permissive than `.bzl`; Bazel's BUILD restrictions
/// (no def/lambda/for/if, no `*args`/`**kwargs`) are an AST pass (`build_syntax_check`), which
/// lets Blaze emit Bazel's exact messages. Top-level rebinding is allowed in BUILD files; that is
/// starlark-rust's default behaviour (see the dialect diff in docs/research/05-starlark-rust.md).
pub fn build_dialect() -> Dialect {
    Dialect {
        // Let `for`/`if` parse so build_check reports Bazel's BUILD-specific message (Java runs
        // checkBuildSyntax before the resolver would reject them).
        enable_top_level_stmt: true,
        ..bzl_dialect()
    }
}

/// Names in Bazel's `Starlark.UNIVERSE` (MethodLibrary + True/False/None), with `set` enabled by
/// `--experimental_enable_starlark_set` (default true in 9.3.2).
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

/// starlark-rust globals (standard + `set` extension), restricted to Bazel's universe names.
/// Returns (globals builder, names Bazel has that starlark-rust lacks, names starlark-rust has that
/// Bazel lacks).
pub fn bazel_universe_builder() -> (GlobalsBuilder, Vec<String>, Vec<String>) {
    let full = GlobalsBuilder::standard()
        .with(|b| LibraryExtension::SetType.add(b))
        .with(|b| LibraryExtension::Print.add(b))
        .build();
    let mut builder = GlobalsBuilder::new();
    builder.frozen_heap().add_reference(full.heap());
    let mut extra = Vec::new();
    for (name, value) in full.iter() {
        if BAZEL_UNIVERSE.contains(&name) {
            builder.set(name, value);
        } else {
            extra.push(name.to_owned());
        }
    }
    let have: Vec<&str> = full.iter().map(|(n, _)| n).collect();
    let missing = BAZEL_UNIVERSE
        .iter()
        .filter(|n| !have.contains(n) && !["True", "False", "None"].contains(n))
        .map(|s| s.to_string())
        .collect();
    (builder, missing, extra)
}

/// Universe-only globals (what a bare Bazel `Module.withPredeclared(semantics, {})` sees).
pub fn bazel_universe() -> Globals {
    bazel_universe_builder().0.build()
}
