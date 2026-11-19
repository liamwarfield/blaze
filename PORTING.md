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

Blaze's strings mean what Bazel's strings mean (docs/decisions.md 2026-11-18;
docs/research/07-byte-strings.md, rules in 07-byte-strings-rules.md). As in
`lib/util/StringEncoding.java` there are three kinds, and a value never changes kind without a
call that says so:

- **Internal strings**: almost every string Blaze holds (Starlark strings, labels, paths, option
  values, environment, file names, action arguments). A Rust `String` whose chars are all
  `<= U+00FF`, one char per byte of the original byte sequence (Java: a Latin-1 `String` holding
  raw bytes). `len("é") == 2`; a byte that is not UTF-8 is an ordinary char.
- **Unicode strings**: ordinary text (received proto `string` fields, parsed JSON, URLs, Rust
  literals).
- **Platform strings**: bytes for the operating system (`OsStr`): file names, argv, environment.

Rules:

- Starlark sources and everything Java reads as `ISO_8859_1` (or with the server's default
  charset, which is ISO-8859-1) are decoded one char per byte:
  `blaze_util::string_encoding::latin1_decode`. Source files go through
  `string_encoding::parser_input_from_latin1` only.
- Bytes leave through the encoder Java uses at that site: `latin1_encode`
  (`StringUnsafe.getInternalStringBytes`), `latin1_encode_lossy` (`getBytes(ISO_8859_1)`, the
  console, any default-charset `PrintStream`/`Writer`), `internal_to_platform` /
  `platform_to_internal` (system calls). Where Java applies a UTF-8 encoder to an internal
  string (`Fingerprint.addString`, `getBytes(UTF_8)`, a proto string field) the Rust is
  `as_bytes()`: the "double encoding" that results is Bazel's behaviour.
- Convert between internal and Unicode exactly where Java calls `StringEncoding.internalToUnicode`
  / `unicodeToInternal`, with the functions of the same name. Text handed to a crate that
  interprets characters (regex, JSON, URL) is whatever Java hands to its counterpart.
- Never use `String::from_utf8*`, `to_string_lossy`, `std::env::var`/`args` or a `&str` path
  with `std::fs` on data that is an internal or platform string. File system access goes through
  `blaze_vfs` (which converts) or `internal_to_platform`.
- Java `length()`, `charAt`, `substring` and columns count chars: use `chars()`, not byte
  offsets, when the number is visible or compared with a constant. Java `Character` predicates
  and case mappings are applied to the Latin-1 chars; port the predicate, do not assume Rust's
  `char` method agrees.
- Every conversion is the identity on ASCII.

## Options

Options classes are transcribed with the `blaze_options` declaration macro (see
`crates/blaze_options/TRANSCRIBE.md`). One Rust options struct per Java `OptionsBase` subclass,
same option names, defaults, converters, tags and help text.

## Starlark

- Use `blaze_starlark` (docs/research/05-starlark-rust.md sections 6 and 8): `FileKind`,
  `eval::compile` / `exec_module` / `call_frozen`, `ThreadContext` with a per-phase payload,
  `NativeRegistry` to contribute natives per file kind, `EvalError` for Bazel tracebacks.
- Each Java `@StarlarkMethod` group becomes one `#[starlark_module]` in the crate that ports the
  Java class, with Bazel's exact method and parameter names, positional/named rules and defaults.
- Error text: match Bazel's where Blaze raises the error; starlark-rust's own core-language error
  text is accepted (decisions.md). Fixes to core types go in the vendored crate's patch queue
  (`third_party/starlark-rust/PATCHES.md`), never as workarounds.

## Skyframe

- One Rust struct per Java `SkyKey` class, with `impl blaze_skyframe::Key` (`type Value`,
  `const FUNCTION: SkyFunctionName("JAVA_NAME")`); its `Debug` output must match Java's `toString`.
  Put it in the crate that owns the Java key class.
- Register with `EvaluatorBuilder::typed_function::<K, _>(|k, env| -> Result<Option<K::Value>, ComputeError>)`.
  Implement `SkyFunction` directly only when you need `extract_tag`.
- Dependencies: `env.get_value(k)?`, `env.get_value_or_throw::<K, E>(k)?` (gives
  `Result<Option<_>, E>`), and `env.get_values_and_exceptions(&keys)?` then `.get` / `.get_or_throw`.
  Return `Ok(None)` where Java returns null, and keep `if env.values_missing() { return Ok(None) }`
  exactly where Java has it. Throw with `Err(SkyFunctionException::new(cause, transience).into())`.
- `?` propagates `Interrupted`. Long work that requests no dependencies polls `env.check_interrupted()?`.
- Report events through `env.listener()`.
- Never hold a lock (Mutex, RwLock, RefCell borrow) across a dependency request: the call can
  compute other nodes on the same thread or block on other threads.
- Only immutable `Send + Sync` values cross nodes; Starlark values must be frozen. Never keep a live
  Starlark module or heap across a dependency request (starlark-rust cannot suspend evaluation):
  request every dependency first, then evaluate.
- Errors follow Java's protocol, so port the Java function as it is written:
  - keep_going: `get_value_or_throw` / `get_or_throw` return the dependency's exception.
  - no keep_going, normal evaluation: a dependency in error is `None` and sets
    `values_missing()`, whatever exception type was asked for. Return `Ok(None)`. The first
    exception *thrown* stops the evaluation; later ones are dropped.
  - Error bubbling (`ParallelEvaluator.bubbleErrorUp`): after the first failure (or a catastrophe
    with keep_going) the engine re-runs, once each, the functions on one path from the failed node
    up to a top-level node, with `env.in_error_bubbling()` true. Only in that run do the
    exceptions reach `get_value_or_throw` without keep_going. Requests compute nothing new there
    (a dependency that is not done is missing), `env.check_interrupted()` fails once a dependency
    in error was requested, and the function's stored events are reported. Keep every Java
    `env.inErrorBubbling()` branch.
  - So a function can run twice for one node. Request dependencies first, as the Java function
    does; do not rely on "computed once". A value returned from the bubbling run is ignored.
  - `get_value_or_throw::<K, E>` matches the concrete type `E` only (Java also matches
    subclasses); use `get_value_or_throw_with` / `get_or_throw_with` for several types.
- Requests are evaluated one at a time, in order (Java collects all requests of a run and
  evaluates none before the function returns). Where a Java function relies on that, port it as
  written and mark the spot `// PORT: request order`.
- Not ported, fails with `unported!`: `env.get_state` (SkyKeyComputeState),
  `env.register_dependencies`, `env.depend_on_future`, `SkyframeLookupResult::query_dep`; also
  absent: `EvaluationProgressReceiver`, `SkyFunction.Reset`, partial reevaluation, the per-key
  keep_going predicate, `EventFilter` (see `crates/blaze_skyframe/src/lib.rs`, "Not ported").

## Tests

- Port the JUnit tests of each class you port, keeping test names (snake_case). A test needing
  missing infrastructure is `#[ignore = "needs <what>"]`, never deleted.
- Expected values come from the Java test or from the oracle, never from running Blaze.

## Style

- Edition 2024, `rustfmt --edition 2024`, `clippy -D warnings` (allow `clippy::too_many_arguments`
  and `clippy::new_without_default` crate-wide; ported signatures win).
- Doc comment on each public type naming its Java class.
