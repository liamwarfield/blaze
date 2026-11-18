# Blaze porting contract

Blaze ports Bazel 9.3.2 (`UPSTREAM.json`) from Java to Rust. Many agents write separate files at
the same time without seeing each other's work. These rules make the files agree. When a rule
does not cover a case, choose the most literal port and add a `// PORT:` comment explaining the
choice.

The Java source is `~/.cache/blaze/src/bazel-9.3.2/src/main/java`. Paths below are relative to it.

## Fidelity

- Same classes, same methods, same algorithm, same order of effects. Do not simplify, merge or
  "improve". Ordering of anything that reaches output (lists, maps iterated into output, events,
  error order) must match Java.
- Same error and warning text, character for character, including punctuation and quoting.
- Keep Java comments that explain behaviour.
- Mark every ported item: `// Java: com/google/devtools/build/lib/cmdline/Label.java:123 Label.parseCanonical`.
- Missing dependency: call `unported!("Class.method")` at that point (from `blaze_util`; crates
  without it use `unimplemented!("Java: Class.method")`). Never return a guessed value.
- Skip only: Windows/macOS-only code paths (comment `// PORT: skipped, <os> only`), Google-internal
  code, profiling/tracing, memory instrumentation, and incremental-state plumbing that a
  single-build-per-process evaluator does not need (comment `// PORT: skipped, incremental`).

## Names

- Classes, interfaces, enums keep their Java names in PascalCase. Nested `Outer.Inner` becomes
  `OuterInner` unless it is only used inside one file (then a private `Inner` in that module).
- Methods and fields: Java camelCase to Rust snake_case. Insert `_` before each capital that
  follows a lowercase letter or digit, and before the last capital of an acronym run followed by
  a lowercase letter; then lowercase. `getRepositoryName` → `get_repository_name`,
  `toPathFragment` → `to_path_fragment`, `getHTTPClient` → `get_http_client`. Rust keywords get a
  trailing underscore (`type_`, `ref_`, `impl_`).
- Constants: SCREAMING_SNAKE as in Java.
- Java package `com.google.devtools.build.lib.X.Y` maps to crate per `docs/crates.md` and module
  `x::y` inside it.
- Starlark-visible names (methods, parameters, providers, fields) are exactly Bazel's
  `@StarlarkMethod`/`@Param` names, never renamed.

## Types

| Java | Rust |
| --- | --- |
| `@Nullable T` field, param or return | `Option<T>` |
| non-null reference to an immutable value | owned value or `Arc<T>` when shared |
| `String` | `String` / `&str`; see "Strings" below |
| `int` / `long` / `boolean` / `double` | `i32` / `i64` / `bool` / `f64` (keep Java widths; Java `int` overflow wraps, so use `wrapping_*` where Java can overflow) |
| `ImmutableList<T>`, `List<T>` | `Vec<T>` (or `Arc<[T]>` when shared and immutable) |
| `ImmutableMap`, `LinkedHashMap`, `ImmutableSet`, `LinkedHashSet` | `IndexMap` / `IndexSet` (insertion order) |
| `ImmutableSortedMap`, `TreeMap`, `TreeSet` | `BTreeMap` / `BTreeSet`, with an `Ord` that matches the Java comparator |
| `HashMap` / `HashSet` whose iteration order is never observable | `HashMap` / `HashSet` |
| `HashMap` / `HashSet` whose iteration order reaches output | `// PORT: java-hash-order` and replicate Java's order (ask root) |
| `Optional<T>` | `Option<T>` |
| checked exception | `Result<T, E>` where `E` is a Rust error type named after the Java exception (`thiserror`), carrying the same message |
| unchecked exceptions Bazel treats as crashes (`IllegalStateException`, `Preconditions`) | `panic!` with the same text (`check_state!`, `check_argument!` in `blaze_util`) |
| `InterruptedException` | `Result<_, Interrupted>` variant; propagate |
| `@AutoValue` class | plain struct with `#[derive(Clone, Debug, PartialEq, Eq, Hash)]` as Java's equals/hashCode imply |
| interface with open implementations | trait (object-safe, `Send + Sync`) |
| closed class hierarchy or `enum` with behaviour | Rust `enum` |
| `Interner` / `interned` values | `Arc<T>` from a global intern table; equality may then use `Arc::ptr_eq` only where Java uses `==` |
| `synchronized`, `ConcurrentHashMap` | `parking_lot::Mutex` / `RwLock`, `dashmap`-style sharded maps |
| `NestedSet<T>` | `blaze_collect::NestedSet<T>` (one shared port; keeps Java's order semantics) |

All shared values are `Send + Sync`. Anything Skyframe stores is immutable after construction.

## Strings

Bazel stores many strings in an "internal" form: Latin-1 Java strings holding UTF-8 bytes
(`lib/util/StringEncoding.java`). Rust `String` is already UTF-8, so the internal form is the
natural form in Rust. Where Java converts between internal and Unicode forms
(`StringEncoding.internalToUnicode` etc.), the Rust port is usually the identity; mark it
`// PORT: StringEncoding identity`. File contents and paths that may not be valid UTF-8 use
`Vec<u8>` / `OsString`, not lossy conversion.

## Options

Options classes are transcribed with the `blaze_options` declaration macro (see
`crates/blaze_options/TRANSCRIBE.md`). One Rust options struct per Java `OptionsBase` subclass,
same option names, defaults, converters, tags and help text.

## Starlark

To be written after `docs/research/05-starlark-rust.md`.

## Skyframe

To be written after `docs/research/06-bazel-architecture-map.md`.

## Tests

- Port the JUnit tests of each class you port, keeping test names (snake_case). A test needing
  missing infrastructure is `#[ignore = "needs <what>"]`, never deleted.
- Expected values come from the Java test or from the oracle, never from running Blaze.

## Style

- Edition 2024, `rustfmt --edition 2024`, `clippy -D warnings` (allow `clippy::too_many_arguments`
  and `clippy::new_without_default` crate-wide; ported signatures win).
- Doc comment on each public type naming its Java class.
