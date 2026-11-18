# 07 — Byte strings: what Bazel 9.3.2 does with non-ASCII text, and how Blaze should match it

Status: design study with a spike (2026-11-18). Question from the workerd stress project:
aspect_tools_telemetry's `sha1.bzl` hashes the first lines of a README that contains an emoji and
gets a different result in Blaze. Decisions.md (2026-11-17) accepted starlark-rust's code-point
strings "until a non-ASCII case shows up". This is that case.

Sources: Bazel 9.3.2 Java at the pin (paths relative to `src/main/java`), the oracle binary on a
scratch workspace (`scripts/bytes/run`, ground truth below), Blaze at `0388d76` plus the working
tree, and the spike `crates/starlark_conformance/tests/byte_strings.rs`.

## 1. Summary

- Bazel stores almost every string in an "internal" form: a Java `String` with one Latin-1 char per
  UTF-8 byte (`lib/util/StringEncoding.java`). BUILD, .bzl, MODULE.bazel and REPO.bazel files are
  decoded as Latin-1 (`ParserInput.fromLatin1`), `repository_ctx.read` and `execute` output as
  `ISO_8859_1`, and `--internal_starlark_utf_8_byte_strings` defaults to **true**. To a Starlark
  program every string is a byte string: `len("é") == 2`, `"🐷"[1:3]` is two continuation bytes,
  `elems()` yields bytes, `"\303\251" == "é"`.
- Bazel converts to Unicode only at explicit sites (249 `StringEncoding` calls in 50 files, mostly
  protos and logs). Everywhere else the internal form leaks out as-is, which produces oracle
  quirks Blaze must reproduce: action keys hash the UTF-8 of the Latin-1 chars (each byte >= 0x80
  is two bytes), `aquery --include_file_write_contents` shows base64 of that double encoding,
  `cquery --output=jsonproto` writes `é` as the single byte E9 and emoji as `?`.
- **Recommendation: option A, mirror Bazel.** Use the internal form everywhere in Blaze: a Rust
  `String` whose chars are all <= U+00FF, one per byte. Decode sources and file contents as Latin-1;
  encode at the platform boundary (paths, argv, env, file contents, console). Convert to and from
  real Unicode exactly where Java calls `StringEncoding`. starlark-rust then works on bytes without
  any change to its storage. Four small starlark-rust patches (ASCII-only case mapping and
  predicates, ASCII `strip`, Bazel's `repr` escaping, lexer escapes) and one Blaze json change close
  the rest.
- Spike (69 oracle cases): Blaze today matches **12**. Latin-1 decoding alone, with no interpreter
  patch, matches **57**, including all four `sha1.bzl` hashes and the telemetry repository-rule flow
  (`read`, first 4 lines, sha1, a byte-splitting slice written back). The 12 left are all fixed by
  the planned patches.
- Boundary estimate for Blaze under A: about 300 sites, all identities for ASCII, so brotli and re2
  stay byte-identical at every step. About 80 of them are deletions: Blaze already re-creates Java's
  double encoding by hand (`Fingerprint::add_internal_string` and friends), and under A those
  emulations become plain `add_string` / `as_bytes()`.

## 2. What Bazel 9.3.2 does

### 2.1 The semantics flag

| item | value | source |
|---|---|---|
| `--internal_starlark_utf_8_byte_strings` | default **true**, hidden, `BUILD_FILE_SEMANTICS` | `packages/semantics/BuildLanguageOptions.java:801` |
| semantics key | `-internal_bazel_only_utf_8_byte_strings` ("Whether the Starlark interpreter uses UTF-8 byte strings instead of UTF-16 strings") | `net/starlark/java/eval/StarlarkSemantics.java:267` |
| wired | `BuildLanguageOptions.setFlags` :989 | |
| also forced true | `FactsAdapter` (lockfile `facts` JSON) :41 | `bazel/bzlmod/FactsAdapter.java` |
| readers of the flag | only two: `StringModule.matcher` :332 (`strip` family: ASCII whitespace instead of Latin-1 whitespace) and `Json.decode` :353 (`\uXXXX` escapes appended as UTF-8 bytes) | |

Everything else about byte strings is not a flag: it follows from how the input was decoded. The
interpreter is UTF-16 code-unit based; fed Latin-1 text, its code units are bytes.

The JVM setup matters too. The client starts the server with `-Dfile.encoding=ISO-8859-1`
(`src/main/cpp/blaze.cc:431`) and the `en_US.ISO-8859-1` locale when available (:1388; this host has
it, so `sun.jnu.encoding` is ISO-8859-1). Default-charset writers and file names, argv and
environment through the JDK are therefore Latin-1, so internal strings reach the OS as raw bytes.

### 2.2 Where text enters

| input | decoding | source |
|---|---|---|
| BUILD, .bzl, MODULE.bazel, REPO.bazel, VENDOR.bazel | `ParserInput.fromLatin1` (one char per byte), after the `--incompatible_enforce_starlark_utf8` check (default `warning`: invalid UTF-8 warns and still parses) | `skyframe/StarlarkUtil.java:43`; callers `BzlCompileFunction:181`, `PackageFunction:1335`, `RepoFileFunction:120`, `CompiledModuleFile:69`, `VendorFileFunction:165` |
| string literals | Latin-1 chars copied through; octal escapes `\0`–`\377` give one char (`> \377` is an error); `\x`, `\u`, `\U`, `\N` are **errors** ("invalid escape sequence") | `net/starlark/java/syntax/Lexer.java:373-404` |
| `repository_ctx.read`, `module_ctx.read` | `readContent(path, ISO_8859_1)` | `bazel/repository/starlark/StarlarkBaseExternalContext.java:1643` |
| `repository_ctx.execute` stdout/stderr | `toString(ISO_8859_1)` | `StarlarkExecutionResult.java:185` |
| `repository_ctx.template` | template read and written as ISO_8859_1 | `StarlarkRepositoryContext.java:339-348` |
| WASM module output | ISO_8859_1 | `StarlarkWasmModule.java:225` |
| file names (readdir, glob), argv, env | JVM platform strings = internal (Latin-1 locale), else `platformToInternal` | `StringEncoding` javadoc |
| protos, JSON from Gson, registry files | Unicode; converted with `unicodeToInternal` where Bazel cares (lockfile adapters) | `GsonTypeAdapterUtil.java` |

### 2.3 Starlark operations (oracle, `scripts/bytes/run`)

All values are byte lists printed through a lookup table built from octal escapes
(`scripts/bytes/workspace/bytes_util.bzl`), so the log is ASCII. `e = "é"` (C3 A9), `pig = "🐷"`
(F0 9F 90 B7).

| operation | Bazel 9.3.2 | Blaze today | Java reason |
|---|---|---|---|
| `len(e)`, `len(pig)`, `len("aé🐷z")` | 2, 4, 8 | 1, 1, 4 | `String.length()` of Latin-1 text |
| `e[0]`, `pig[-1]`, `pig[1:3]`, `"aé🐷"[::2]` | [C3], [B7], [9F 90], [61 A9 9F B7] | code points | char indexing |
| `len("é🐷".elems())`, `e.elems()` | 6, [[C3],[A9]] | 2, [[E9]] | one string per char |
| `"aé🐷b".find("b")`, `.index("🐷")`, `"éaé".rfind("é")` | 7, 3, 3 | 3, 2, 2 | char offsets |
| `"éé".count("\251")`, `"\251" in e`, `e.startswith("\303")` | 2, True, True | 0, False, False | substring search on bytes |
| `"\303\251" == e`, `"".join(["\303","\251"]) == e`, `{e: 1}.get("\303\251")` | True, True, 1 | False, False, None | octal escapes are bytes |
| `len("\377")`; `pig[:2] + pig[2:] == pig` | 1; True | 1; True | |
| `"a€b".split("\202")`, `e.partition("\251")`, `e.replace("\251","x")`, `e.removeprefix("\303")` | split inside a character | no match | |
| `"aé".upper()`, `"中".upper()`, `"AÉ".lower()` | ASCII only: [41 C3 A9], [E4 B8 AD], [61 C3 89] | Unicode case mapping | `Ascii.toUpperCase/toLowerCase` (`StringModule.java:175,183`) |
| `"中".capitalize()` | **[C4 B8 AD]**: first char through `Character.toUpperCase` turns lead byte E4 (ä) into C4 (Ä) | | `StringModule.java:531` |
| `"été".title()` | **[C3 A9 54 E3 A9]**: Ã after a letter is lowered to ã | | `Character.isLetter/isUpperCase/toLowerCase` per char (:548) |
| `isalpha/isalnum/isdigit("é")`, `islower("aé")`, `isupper("AÉ")`, `istitle("Éa")` | False, False, False, True, True, False | True, True, False, True, True, True | ASCII `[a-z]`, `[A-Z]` matchers (:873-876) |
| `"\205".isspace()`, `"\240".isspace()` | True, True | True, True | Guava `CharMatcher.whitespace()` |
| `" é ".strip()`, `"Å".strip()` (C3 85), `"\240a\240".strip()` | [C3 A9], [C3 85], [A0 61 A0] | strips U+0085/U+00A0 | ASCII whitespace under the flag (:206) |
| `"a\302\205b\nc".splitlines()` | splits on `\n` only | same | |
| `repr("é\001\177\200")` | `"` C3 A9 `\x01` 7F 80 `"` | escapes 7F, 80, non-printables | `Printer.escapeCharacter` escapes only < 0x20 (`Printer.java:239`) |
| `str(["é"])`, `"%s|%r" % (e, e)`, `"{}|{}".format(e, len(e))` | raw bytes, `|2` | | |
| `json.encode(e)`, `json.encode("\377")`, `json.encode("\001\177\200")` | raw bytes in quotes; `\u0001`; 7F, 80 raw | same except for code points | `Json.appendQuoted` (code points of Latin-1 chars are bytes) |
| `json.decode('"\\u00e9"')`, `'"\\ud83d\\udc37"'`, `'"\\ud83d"'` | [C3 A9], [F0 9F 90 B7], [EF BF BD] (U+FFFD) | one code point each | `Json.java:563`: escapes encoded to UTF-8, appended as Latin-1 |
| `hash(e)`, `hash(pig)` | 6214, 7307286 | 233, 1772442 | `String.hashCode` over the Latin-1 chars |
| `sorted(["z", e, "\377", "a", pig])`, `e < "z"` | byte order: a, z, é, 🐷, FF; False | code-point order (same for valid UTF-8) | UTF-16 units = bytes |
| `"{!r}".format(x)` | error "Missing argument '!r'" | | (not byte related; noted while building the cases) |
| `ord`, `chr` | not defined (bazel_lib provides them as dicts over octal escapes) | starlark-rust adds them; Blaze drops them | |

aspect_tools_telemetry `sha1.bzl` uses bazel_lib's `ord`, a dict whose keys are `"\0"`…`"\377"`.
In Bazel each key is one byte, `input.elems()` yields bytes, so `sha1` is real SHA-1 of the UTF-8
bytes (oracle: `sha1("é\n") = 6ee66ed9…`, `sha1("🐷") = b08429ef…`, both equal to `sha1sum`). In
Blaze today `"é".elems()` yields U+00E9, which happens to be a key, so the hash is SHA-1 of the
Latin-1 encoding (`6a715a82…`), and `"🐷"` is not a key: `ord` returns None and sha1 fails with
"Operation `<<` not supported for types `NoneType` and `int`". The workerd README starts with 👷,
so the telemetry repository rule fails or hashes differently depending on the text.

### 2.4 What leaves Bazel (oracle outputs)

Workspace `scripts/bytes/workspace`: a genrule `cmd = "printf '%s' 'é🐷' > $@"`, a rule writing
`ctx.actions.write(content = "é🐷\377\n")`, a `run_shell` with argument `"é\377"` and env
`ENV_E = "é"`, a target `fg_é`, and a repository rule that reads the README head, slices it and
writes it back.

| surface | what the oracle emits | mechanism |
|---|---|---|
| `print()` / DEBUG lines | raw bytes (`é🐷`, a raw FF) | events written through Latin-1 |
| output files (genrule, `ctx.actions.write`, `run_shell` argv) | raw bytes: `C3 A9 F0 9F 90 B7 FF 0A`; argv byte FF reaches the process | `getInternalStringBytes`; argv in the Latin-1 platform encoding |
| `repository_ctx.file(data[:3])` | `# \xF0` (invalid UTF-8, written as is) | `getInternalStringBytes` (`StarlarkBaseExternalContext.java:1516`) |
| `repository_ctx.template` with `{"workerd": "wörkerd"}` | correct UTF-8 | read and written as ISO_8859_1 |
| `query --output=label_kind`, `--output=build` | raw bytes (`fg_é`, `"é🐷\377"` with a raw FF) | default-charset writer = Latin-1 |
| `query --output=streamed_jsonproto` | real Unicode: `"fg_é"`; invalid byte FF becomes U+FFFD | `internalToUnicode` (ProtoOutputFormatter, 27 sites) |
| `cquery --output=jsonproto` | **`é` as the single byte E9, 🐷 as `?`** (not valid UTF-8) | `internalToUnicode`, then a default-charset (Latin-1) writer (`CqueryThreadsafeCallback.java:74`) |
| `aquery` text | `'é\U0001F437'`, FF as `�`; `ENV_E=é` | `internalToUnicode` + ShellEscaper escapes |
| `aquery --output=jsonproto` | arguments, env and `fileContents` as raw internal bytes | proto strings written through Latin-1 |
| `aquery … FileWriteContents` (text) | `w4PCqcOwwp/CkMK3w78K` = base64 of `C3 83 C2 A9 C3 B0 …` (**double encoded**) | `contents.getBytes(UTF_8)` on an internal string (`ActionGraphTextOutputFormatterCallback.java:353`) |
| action keys | genrule `fc8b3023…`, ShellE `db50b826…`, FileWrite `866f242e…` | `Fingerprint.addString` = protobuf `writeStringNoTag` = UTF-8 of the Latin-1 chars (double encoded) |

The last three rows are the important lesson. Java never marked those sites; the double encoding
falls out of applying a UTF-8 API to an internal string. Any design where Blaze's Rust strings are
real UTF-8 has to find and re-create each of them by hand.

### 2.5 Conversion sites in Java

`StringEncoding.{internalToUnicode, unicodeToInternal, internalToPlatform, platformToInternal}`:
249 calls in 50 files (175 / 33 / 19 / 22).

| area | calls | files |
|---|---|---|
| query output (proto, `AttributeFormatter`, aquery text) | 46 | ProtoOutputFormatter 27, AttributeFormatter 18, ActionGraphTextOutputFormatterCallback 1 |
| starlark_doc_extract (ModuleInfo protos) | 40 | ModuleInfoExtractor 20, StarlarkFunctionInfoExtractor 9, RuleInfoExtractor 5, AttributeInfoExtractor 3, ProviderNameGroupExtractor 3 |
| remote execution and caching | 35 | RemoteExecutionService 18, UploadManifest 10, RemoteExternalOverlayFileSystem 4, MerkleTreeComputer 3 |
| bzlmod lockfile and `mod` output | 28 | GsonTypeAdapterUtil 17, RepoOutputFormatter 6, AttributeValuesAdapter 4, VendorFileFunction 1 |
| execution log | 23 | CompactSpawnLogContext 21, SpawnLogContext 2 |
| runtime, info, options, logging, misc | 21 | RegexUtil 5, SimpleLogHandler 5, BazelRuleClassProvider 3, BuildLanguageInfoItem 2, BlazeRuntime, JavaHomeInfoItem, NetUtil, OutputFilter, Converters, ArtifactFactory 1 each |
| file system and paths | 16 | JavaIoFileSystem 4, UnixFileSystem 2, WindowsFileSystem 2, PathFragment 2, LocalDiffAwareness 2, Path, FileSystemUtils, FileSystemLock, DependencySet 1 each |
| downloader, archives | 14 | ProxyHelper 13, CompressedTarFunction 1 |
| process launch, workers, sandbox | 11 | WorkerMultiplexer 3, JavaSubprocessFactory 2, WindowsSubprocessFactory 2, SubprocessBuilder, WorkerSpawnRunner, LocalSpawnRunner, DarwinSandboxedSpawnRunner 1 each |
| platform and exec properties | 9 | PlatformUtils 6, ExecutionInfoModifier 3 |
| BEP | 3 | TargetCompleteEvent 3 |

(Some lines hold two calls, so the per-file sums give 246.)

Raw-byte sites outside `StringEncoding`: `ISO_8859_1` in 43 files (RunCommand 9, UiEventHandler 6,
FileSystemUtils 4, FileOutErr, DependencySet, GrpcServerImpl, IncludeParser,
StripPrefixedPath 3 each, the repository context, ParameterFileWriteAction, StarlarkAction,
LocalTemplateExpansionStrategy, SourceManifestAction, RepoMappingManifestAction, …), and
`StringUnsafe.getInternalStringBytes/newInstance` in 13 files (FileWriteAction 3,
RemoteRepoContentsCacheImpl 4, StarlarkBaseExternalContext 2, PathFragment 2, ParameterFile,
LazyWritePathsFileAction, LazyWriteNestedSetOfTupleAction, WorkerFilesHash, …). Plus the JVM's
implicit Latin-1 for every default-charset writer, file name, argv and environment string.

## 3. Blaze today

- PORTING.md "Strings": Rust `String` is real UTF-8 and `StringEncoding` is the identity
  (`// PORT: StringEncoding identity`). 96 markers in 22 crates (71 identity; 29 name a
  conversion).
- Source decoding is one function, `blaze_sky_loading::starlark_util::create_parser_input`
  (`String::from_utf8`; non-UTF-8 input under `off`/`warning` is `unported!`), and its copy in
  `blaze_bzlmod::module_file::compiled_module_file`.
- Blaze already re-creates Java's double encoding by hand where an oracle diff demanded it:
  `Fingerprint::add_internal_string`, `add_internal_strings`, `unicode_to_internal`, `latin1(...)`
  helpers. About 80 calls in 25 files (custom_command_line 12, digest_writer 11, execution_result 7,
  blaze_events io/printing 7, action_cache 4, …). `latin1_internal_string` in
  `blaze_repo_rules/src/starlark/execution_result.rs` decodes process output lossily because a
  Blaze `String` cannot hold Latin-1 bytes in internal form.
- Starlark ↔ Rust crossings: 458 functions in `#[starlark_module]` blocks, 105 string parameters,
  59 string returns, 179 `unpack_str`, 40 `StringValue`, 39 `alloc_str`.

## 4. Options

### A. Mirror Bazel: internal (Latin-1) strings everywhere

Blaze's `String`s hold the internal form, as Java's do: each char is <= U+00FF and stands for one
byte. Sources and file contents are decoded byte → char; the platform boundary encodes char → byte;
real Unicode exists only where Java calls `internalToUnicode`/`unicodeToInternal`.

What starlark-rust sees: Latin-1 text. `len`, indexing, slicing, `elems`, `find`/`index` offsets,
`split`, `replace`, comparison, sorting, `hash` (UTF-16 polynomial over chars), dict keys and
`%`/`format` are byte-correct with no change. Invalid UTF-8 fragments (`"é"[0]`, `"\377"`,
`data[:3]`) are ordinary chars, so storage stays valid Rust `str`.

Patches still needed (each mirrors a Java method; numbers continue PATCHES.md):

| patch | change | Java |
|---|---|---|
| 0019 lexer escapes | octal escapes give `char` 0..=0xFF, `> \377` is an error, `\x`, `\u`, `\U`, `\N` are errors with Bazel's text (today `"é"` would silently give byte E9 under A, not C3 A9) | `Lexer.java:373-404` |
| 0020 case mapping and predicates | `upper`/`lower` ASCII only; `isalpha`/`isalnum`/`islower`/`isupper` ASCII matchers; `isdigit` `javaDigit` (no non-ASCII digits in Latin-1); `capitalize`/`title`/`istitle` with Java `Character` mappings on Latin-1 chars (already right in the spike; pin with tests: `ÿ`→U+0178, `µ`→U+039C would leave Latin-1, as in Java) | `StringModule.java:175-876` |
| 0021 strip family | ASCII whitespace when the flag is on, Latin-1 whitespace when off | `StringModule.java:196-206, 332` |
| 0022 repr | escape `"`, `\`, `\r`, `\n`, `\t`; other chars < 0x20 as `\xNN`; everything else raw (Blaze's `printer::quote` already does this) | `Printer.java:239` |
| Blaze json | `decode`: `\uXXXX` runs encoded to UTF-8 (U+FFFD for lone surrogates), appended as Latin-1 chars, when `utf8_byte_strings()`; `encode` already right | `Json.java:494-575` |

Boundary sites in Blaze (grep at `0388d76` + working tree; each is the identity for ASCII):

| area | crates | sites | notes |
|---|---|---|---|
| source decoding | blaze_sky_loading `starlark_util`, blaze_bzlmod `compiled_module_file` | 2 | `fromLatin1`; also removes the `unported!` for non-UTF-8 files under `warning`/`off` |
| interpreter | third_party/starlark-rust, blaze_starlark json | 4 patches + 1 | above |
| Starlark-visible content in | blaze_repo_rules (`read`, `execute`, `template`, `module_ctx.read`, wasm), blaze_actions workspace status | ~10 | `execution_result::latin1_internal_string` becomes exact instead of lossy |
| Starlark-visible content out | blaze_actions/blaze_analysis/blaze_exec (FileWriteAction, ParameterFileWriteAction, template expansion, LazyWritePaths/NestedSetOfTuple, SourceManifest, RepoMappingManifest, unresolved symlink targets), blaze_repo_rules (`file`, `template`) | ~15 | `getInternalStringBytes` → Latin-1 encode |
| paths and file system | blaze_vfs `unix_file_system` (~22 syscalls, readdir, readlink), ~110 direct `std::fs` calls in ~45 files outside the VFS | ~25 in the VFS + ~110 mechanical | one `PathFragment`/`&str` → `OsStr` helper; most direct callers use ASCII output-base paths but must go through it |
| processes | blaze_shell, blaze_exec spawn runners, linux-sandbox argv, client argv/env/rc intake (blaze_client, blaze_runtime, blaze_options) | ~25 | argv/env/cwd bytes |
| console and text outputs | blaze_events (`io`, `printing`, OutErr), UiEventHandler port, query/cquery/aquery writers | ~25 | Latin-1 encode at the stream, plus the cquery writer's Unicode → Latin-1 `?` replacement |
| Unicode boundaries | 96 existing `StringEncoding` markers; query proto (27), `AttributeFormatter` (18), lockfile Gson (21), `mod` output (6), exec log (23), BEP (3), aquery text (1) as they are ported | ~100 | the Java list in 2.5 is the checklist |
| emulations to delete | `add_internal_string(s)`, `unicode_to_internal`, `latin1(...)` helpers | ~80 | become `add_string` / `as_bytes()`; a missed one triple-encodes, but only for non-ASCII and the oracle catches it |
| Starlark ↔ Rust natives | 458 functions, ~350 string crossings | **0** | internal in, internal out, as in Java |

Total: about 300 sites, ~80 of them deletions and ~110 mechanical `std::fs` path routings.

Costs and risks:
- Rust string APIs mean the wrong thing on internal text: `s.chars()` are bytes, `format!("{s}")`
  to a terminal must go through the Latin-1 encoder, `Path::new(s)` is wrong for non-ASCII. Any
  missed boundary is silent mojibake, but only for non-ASCII input, and the oracle surfaces show it.
- A Rust `String` holds a non-ASCII byte in 2 bytes. BUILD/.bzl sources are nearly all ASCII; an
  ASCII fast path (`is_ascii()` → reuse the buffer) keeps decoding free.
- Third-party Rust crates that take `&str` (serde_json for lockfile/registry JSON, URL parsing,
  regex) need `internal_to_unicode` first, which is exactly where Java calls it (Gson adapters,
  ProxyHelper, RegexUtil).
- A char > U+00FF must never appear in an internal string; `latin1_encode` panics on one (Java
  would silently produce a UTF-16 string). Values from Unicode sources (protos, JSON) go through
  `unicode_to_internal` first.
- Error columns: Bazel counts chars (= bytes); starlark-rust's columns over the Latin-1 text count
  chars too. Not verified beyond the ASCII traceback tests.

### B. Byte semantics in starlark-rust on real UTF-8 storage

Keep Blaze's Rust strings real UTF-8 and patch `StarlarkStr` so `len`, indexing, slicing, `elems`,
`find` and friends count bytes.

What breaks: byte operations create fragments that are not UTF-8 (`"é"[0]`, `pig[1:3]`,
`data[:3]`), and bazel_lib's `strings.bzl` creates 128 of them just by loading (`"\200"`…`"\377"`
as dict keys, which the sha1 case looks up). `StarlarkStr` is a `str` throughout starlark-rust
(`as_str`, `unpack_str`, `StringValue`, interning, `Display`, formatting, hashing, every method in
`methods.rs`). The ways to represent the fragments:
- raw `[u8]` storage: every `as_str()`/`unpack_str()` in starlark-rust and Blaze (~350 crossings)
  becomes fallible or lossy; most of the string module is rewritten; the patch is no longer small;
- an escape encoding for stray bytes inside valid UTF-8 (for example a private-use code point per
  byte): `len`/index map between units and bytes, and concatenation must re-merge `"\303" + "\251"`
  into `é` so that equality, hashing and dict lookup agree (oracle: `join_bytes_eq`,
  `dict_octal_key`); every operation becomes a normalisation hazard.

Even done well, B keeps the Rust side in real UTF-8, so every implicit Java re-encoding in 2.4
(action keys, FileWriteContents, aquery jsonproto, cquery output) must be re-created by hand at
sites Java never marked. Blaze already pays this (the ~80 emulations) and would keep paying it.
B rewrites the interpreter's core type in the vendored crate, against the patch-queue rule of
small patches.

### C. Alternatives considered

- **C1, Latin-1 only inside Starlark.** Decode sources as Latin-1 but convert at every Starlark ↔
  Rust crossing so the rest of Blaze stays UTF-8. Over 350 crossings, each fallible because a
  fragment has no UTF-8 form, plus all of B's implicit re-encodings. More sites than A and less
  faithful.
- **C2, a newtype for internal strings** (`InternalStr`, as the `StringEncoding` javadoc says the
  three kinds should be "as if they were different Java types"). The right end state for type
  safety, but it means changing the signature of every domain type (Label, PathFragment, attribute
  values, command lines). Do it as an optional later hardening of A: start with `PathFragment` ↔
  `OsStr` and the OutErr byte sink, the two places where mixing kinds costs the most.
- **C3, keep code points and special-case `elems`/`len`.** Rejected: fixes no boundary and breaks
  `"\303\251" == "é"`.

## 5. Recommendation

Option A. It is the port, not a redesign: Blaze's strings mean what Bazel's strings mean, so the
conversion sites are the ones Java marks (`StringEncoding`, `ISO_8859_1`, `StringUnsafe`) plus the
platform layer the JVM handles implicitly. The oracle quirks that Java never marked (double-encoded
action keys and FileWriteContents, raw aquery jsonproto) come out right by construction, and the
spike shows the interpreter side is already 57 of 69 correct with no patch.

This supersedes the 2026-11-17 decision ("code-point semantics for now") and the PORTING.md
"Strings" section. Both need the root's ruling; proposed decision text:

> 2026-11-18. Strings use Bazel's internal form: a Rust `String` whose chars are all <= U+00FF,
> one per byte (Java's Latin-1 internal strings). Sources and file contents are decoded byte →
> char, the platform layer (paths, argv, env, file contents, console) encodes char → byte, and
> `blaze_util::string_encoding` ports `StringEncoding` (internal ↔ Unicode) at Java's call sites.
> Replaces the 2026-11-17 code-point decision. Reason: workerd (aspect_tools_telemetry sha1.bzl),
> docs/research/07-byte-strings.md.

## 6. Migration plan

Every step is the identity on ASCII, so brotli and re2 must stay byte-identical after each one
(scripts/gate and the oracle scoreboard are the check). Steps 2 and 3 must both land before a
non-ASCII workspace works; in between, non-ASCII stays wrong in a different way, no worse than
today.

1. **Foundation** (blaze_util): `string_encoding` with `internal_to_unicode`,
   `unicode_to_internal`, `internal_to_platform` (→ bytes/`OsString`), `platform_to_internal`,
   `latin1_decode(&[u8]) -> Cow<str>`, `latin1_encode(&str) -> Cow<[u8]>` (ASCII fast paths; panic
   on a char > U+00FF). Port `StringEncodingTest` (`src/test/java/.../util/StringEncodingTest.java`,
   9 tests). Decision entry and PORTING.md rewrite.
2. **Platform and content boundaries**: VFS syscalls and directory entries; spawn argv/env/cwd and
   linux-sandbox args; client argv/env/rc intake; console OutErr and event printing; file-write
   actions and repository `file`/`template`/`read`/`execute`. Delete the ~80 double-encoding
   emulations in the same change as the code they serve (each becomes `add_string`/`as_bytes()`),
   so action keys stay equal on ASCII and become right on non-ASCII.
3. **Interpreter**: Latin-1 source decoding in both `create_parser_input`s (and the conformance
   runner); starlark-rust patches 0019–0022; Blaze `json.decode` byte-string branch keyed on
   `StarlarkSemantics::utf8_byte_strings()`. Update `tests/dialect.rs` deviations
   (`\x`/`\u` escapes become errors).
4. **Unicode outputs**: turn the 96 `StringEncoding` markers into real calls where Java calls
   `StringEncoding`; port the query proto (27), `AttributeFormatter` (18), lockfile Gson (21),
   exec log (23), BEP sites as their crates are touched; the cquery Latin-1 writer with `?`.
5. **Hardening (optional)**: typed `OsStr` conversion on `PathFragment`; a grep gate in
   scripts/gate that flags new `std::fs` calls with `&str` paths and `as_bytes()` on Starlark
   strings outside the boundary modules.

## 7. Test plan

- **Unit**: `StringEncodingTest` ported (step 1); each starlark-rust patch gets
  `patch_00NN_*` in `crates/starlark_conformance/tests/patches.rs` with oracle values from
  section 2.3.
- **ScriptTest in byte mode**: Bazel runs its script tests twice, `ScriptTest` and
  `ScriptTest_Latin1` (`-Dnet.starlark.java.eval.ScriptTest.utf8ByteStrings=true`: Latin-1 file
  decoding, the semantics flag on, `_utf8_byte_strings = True`; see `ScriptTest.java:244-259`).
  Add the same switch to `starlark_conformance run` (`--latin1`) and track both columns in
  PATCHES.md. `json.star` and `sorted.star` have byte-specific expectations.
- **Spike as regression test**: `crates/starlark_conformance/tests/byte_strings.rs` pins today's
  gaps (`LATIN1_GAPS`); each patch removes entries until the list is empty, and then the `Utf8`
  mode can go.
- **Oracle, end to end**: `scripts/bytes/run --binary <blaze>` runs the same workspace through any
  binary; compare `build.stderr` CASE/RCASE/LCASE lines, `outputs/` bytes, `query_*`, `cquery_*`
  and `aquery_*` files with the oracle's (`~/.cache/blaze/work/bytes-out`). Action keys of the three
  non-ASCII actions are in `aquery_*`. Then promote the workspace to a corpus-style project once
  Blaze builds it, so the scoreboard tracks it.
- **workerd**: the telemetry repository rule's hash (`id_day`, README with 👷) must match the
  oracle's lockfile/marker output once workerd fetching runs through Blaze.

## 8. Spike

Files (not production code):
- `scripts/bytes/run` and `scripts/bytes/workspace/`: builds the scratch workspace in
  `~/.cache/blaze/work/bytes` (copying `sha1.bzl` from the workerd fetch and bazel_lib's
  `lib/private/strings.bzl` from the repository cache), runs a binary with
  `--output_user_root=~/.cache/blaze/oracle-out/ur-bytes`, and stores everything in
  `~/.cache/blaze/work/bytes-out`. Oracle run: all 8 commands exit 0.
- `crates/starlark_conformance/tests/byte_strings.rs`, three tests, all passing:
  - `byte_string_cases_latin1_vs_utf8`: every case line of `cases.bzl` evaluated in its own module
    through `blaze_starlark` (real `json`, real loads of `sha1.bzl` and bazel_lib's `strings.bzl`),
    once with UTF-8 decoding (today) and once with Latin-1 decoding (option A, no patch).
    69 cases: today 12 match, option A 57. The 12 option-A gaps are exactly the patch list
    (`upper`/`lower` ×2, `islower`, `strip` ×2, `repr` ×3 including `str(list)` and `%r`,
    `json.decode` ×4).
  - `repository_ctx_read_hash_and_write`: the dedup.bzl flow with `rctx_read`/`rctx_file` natives
    implemented both ways. Option A: length 92, first bytes `[35, 32, 240, 159, 145, 183, 32, 96]`,
    sha1 `b22cdf2a…` (= oracle = `sha1sum`), and `data[:3]` written as `# \xF0` (= oracle file).
    Today: length 89, `NOT-BYTES`, sha1 fails with the `NoneType << int` error.
  - `file_write_bytes_and_aquery_contents`: the BUILD literal `"é🐷\377\n"` under option A encodes
    to the oracle's file bytes, and base64 of the Rust `String`'s own bytes is the oracle's
    `FileWriteContents` (`w4PCqcOwwp/CkMK3w78K`). This is the case for A: the double encoding Java
    produces implicitly is the Rust string's natural UTF-8.

The cache-dependent tests print `SKIP` and pass when `~/.cache/blaze/work/bytes` is missing (run
`scripts/bytes/run` first).

## 9. Open points

- The root must rule on A (decision entry above) before any production change.
- `capitalize`/`title` matched the oracle in the spike only because starlark-rust's char mappings
  agree with Java's on these inputs; patch 0020 should port Java's per-char logic and test the
  Latin-1 chars whose case mapping leaves Latin-1 (`ÿ`, `µ`, `ß`).
- Non-ASCII labels (`//:fg_é` works on the oracle; `Label.name` is bytes) and package paths need a
  Blaze run of the scratch workspace once steps 2–3 land.
- Windows/macOS reencoding branches of `StringEncoding` are out of scope (Linux only).
