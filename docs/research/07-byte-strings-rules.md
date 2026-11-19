# 07a — Byte strings: the rules of the migration

Companion of `07-byte-strings.md` (the study) for whoever converts a crate. Decision:
docs/decisions.md 2026-11-18 "Strings". State: **the switch is made**
(`blaze_util::string_encoding::STRINGS_ARE_INTERNAL == true` since 2026-11-19; both source
decoders call `parser_input_from_latin1`). Every Starlark string is now an internal string.
Write final-form code; there is no "before the switch" any more.

## 1. The three kinds of string

| kind | what it is in Rust | where it lives |
|---|---|---|
| **internal** | a `String`/`&str` whose chars are all `<= U+00FF`, one char per byte of the real byte sequence. `"é"` (C3 A9) is the two chars U+00C3 U+00A9; a lone byte FF is U+00FF. | everything Blaze keeps: Starlark strings, labels, target names, paths (`PathFragment`, `Path`), option values, environment, file names, action arguments, file contents handed to or produced by Starlark |
| **Unicode** | an ordinary Rust string | proto `string` fields received from outside, parsed JSON, URLs, Rust string literals (Java literals), text for third-party crates that interpret characters (regex, serde_json, url) |
| **platform** | bytes: `OsStr`, `&[u8]`, `CString` | system calls: file names, argv, environment, working directory |

`blaze_util::string_encoding` holds every conversion. Never write a private copy.

| Java | Rust (`blaze_util::string_encoding::`) |
|---|---|
| `new String(bytes, ISO_8859_1)`, `StringUnsafe.newInstance(bytes, LATIN1)`, `ParserInput.fromLatin1`, `readContent(path, ISO_8859_1)`, `new String(bytes)` / `toString()` with the default charset | `latin1_decode(bytes)` (never fails) |
| `StringUnsafe.getInternalStringBytes(s)` | `latin1_encode(s)` (panics with Java's message on a char above U+00FF) |
| `s.getBytes(ISO_8859_1)`, `s.getBytes()`, a `PrintStream`/`PrintWriter`/`Writer` with the default charset, `System.out.print` | `latin1_encode_lossy(s)` (`?` for a char above U+00FF) |
| `StringEncoding.internalToPlatform(s)` | `internal_to_platform(s)` → `Cow<OsStr>` |
| `StringEncoding.platformToInternal(s)`, a file name / argv / env value the JVM hands to Java code | `platform_to_internal(os_str)` |
| `StringEncoding.internalToUnicode(s)` | `internal_to_unicode(s)` (malformed UTF-8 → U+FFFD as the JDK does) |
| `StringEncoding.unicodeToInternal(s)` | `unicode_to_internal(s)` |
| `new String(bytes, UTF_8)` | `java_new_string_utf8(bytes)` (a **Unicode** string) |
| `s.getBytes(UTF_8)`, `Fingerprint.addString(s)`, `Hashing.hashString(s, UTF_8)`, a proto `string` field set from `s` and serialized, a `Writer` with UTF-8 | `s.as_bytes()` — whatever kind `s` is. On an internal string this is Java's "double encoding" (each byte >= 0x80 becomes two bytes); it is what the oracle emits, and it now falls out by itself. |
| `s.length()`, `s.charAt(i)`, `s.substring(a, b)`, `s.indexOf(...)` used as numbers | chars, not Rust byte offsets: `s.chars().count()`, `s.chars().nth(i)`, char-index slicing. Byte offsets from `find` used only to slice the same string are fine. |
| `s.compareTo(t)`, `String.CASE_INSENSITIVE_ORDER` | `str` order (`Ord`) is UTF-16 code unit order for internal strings; for Unicode strings above U+FFFF compare `encode_utf16()` |
| `s.hashCode()` | the polynomial over `encode_utf16()` units (for an internal string: over the chars) |
| `Character.isLetter/isDigit/isWhitespace/toUpperCase(c)`, `s.toLowerCase()` | Java's answer for the **char** (a Latin-1 char for an internal string: `Ã` U+00C3 is a letter and lowercases to `ã` even though it is the first byte of `é`). Do not assume Rust's `char` method agrees: `Character.isWhitespace` is true for U+001C–U+001F and false for U+0085 and U+00A0, the opposite of `char::is_whitespace`; `Character.isLetterOrDigit` on Latin-1 is written out in `blaze_shell::shell_utils::is_java_letter_or_digit`. Port the Java predicate you meet and test it on U+0080–U+00FF. |

The default charset of Bazel's server is **ISO-8859-1** (`blaze.cc:431`
`-Dfile.encoding=ISO-8859-1`, and the `en_US.ISO-8859-1` locale for `sun.jnu.encoding`). Old
comments in Blaze that say "the platform charset (UTF-8)" are wrong: fix them where you meet them.

## 2. What to look for in a crate

Work from both sides.

**From the Java side.** In the classes your crate ports, find every `StringEncoding.`,
`ISO_8859_1`, `UTF_8`, `StringUnsafe.`, `getBytes(`, `new String(`, `PrintStream`, `PrintWriter`,
`OutputStreamWriter`, `InputStreamReader`, `readContent`, `readLines`, `writeContent`,
`FileSystemUtils.read*`/`write*`, `Files.readAllLines`. Each one is a boundary; make the Rust say
the same thing (table above). `grep -rn` under
`~/.cache/blaze/src/bazel-9.3.2/src/main/java/<package>`; 07-byte-strings.md 2.5 lists the 249
`StringEncoding` calls by file.

**From the Rust side.** In your crate's `src/`, classify every hit of:

- `as_bytes()`, `.bytes()`, `into_bytes()`, `write_all(`, `write!`/`writeln!` to a byte sink:
  bytes leaving. Which Java encoder is at that site? ISO-8859-1 or default charset →
  `latin1_encode` / `latin1_encode_lossy`; UTF-8 → `as_bytes()` stays. A loop over `bytes()`
  that stands for Java iterating chars of an internal string ("one char per UTF-8 byte") becomes a
  loop over `chars()`.
- `String::from_utf8`, `from_utf8_lossy`, `from_utf8_unchecked`, `str::from_utf8`,
  `read_to_string`, `to_string_lossy`, `to_str()`, `into_string()`, `display()`: bytes entering.
  ISO-8859-1 / default charset / platform string in Java → `latin1_decode` /
  `platform_to_internal`. UTF-8 in Java → `java_new_string_utf8` (the result is Unicode; follow
  Java on whether it is then converted with `unicodeToInternal`).
- `std::fs::*`, `File::open/create`, `OpenOptions`, `std::path::Path::new`, `PathBuf::from`,
  `Command::new`, `.arg(`, `.env(`, `current_dir`, `std::env::var`, `env::vars`, `env::args`,
  `CString::new`: the platform boundary. A path/argument/variable that is an internal string
  goes through `internal_to_platform`; what comes back goes through `platform_to_internal`
  (`std::env::var_os`, `args_os`, `vars_os`, never the `String` variants, which panic or drop
  values that are not UTF-8). Prefer `blaze_vfs::Path` / `FileSystem` (already converted) over
  `std::fs` where the Java uses `Path`.
- existing markers: `PORT: StringEncoding identity`, `PORT: StringEncoding`, `byte-strings`.
  Each becomes the real call (or the comment goes, when the code is simply right now).
- the seam functions and hand-made double encoding, all of which exist to produce from a
  Unicode string what Java computes from the internal one. They are now wrong or redundant:
  - `string_encoding::to_internal(s)` → `s`
  - `string_encoding::internal_string_bytes(s)` → `latin1_encode(s)`
  - `string_encoding::internal_string_from_bytes(b)` → `latin1_decode(b)` (no `Result`; delete
    the error handling and any `unported!` for "not UTF-8")
  - `string_encoding::internal_length(s)` → `s.chars().count()`
  - `Fingerprint::add_internal_string(s)` → `add_string(s)`; `add_internal_strings` likewise
  - `blaze_protos::java::unicode_to_internal(s)` / local `latin1(...)`, `latin1_internal_string`,
    `unicode_to_internal` helpers applied to an *internal* string to imitate Java's double
    encoding → delete the call (the string already is internal). Keep it only where Java really
    calls `StringEncoding.unicodeToInternal` on a Unicode string.
  - `if strings_are_internal() { .. } else { .. }` → the `true` branch.
  - byte-wise predicates "because internal strings are one char per UTF-8 byte" → per `char`.
- Rust literals with non-ASCII chars in non-test code: check the Java literal. A Java literal is
  a Unicode string; where Java prints it through the Latin-1 console it shows `?`.
- `.len()` / byte-offset arithmetic that stands for Java `length()`/`charAt`/column numbers
  (padding, truncation to N chars, line/column positions, `substring` with constants).
- `to_uppercase`/`to_lowercase`/`is_alphabetic`/`is_whitespace`/`trim()` on internal strings:
  compare with the Java method (table above). `trim()` in Java (`String.trim`) strips chars
  `<= U+0020`; Rust `trim()` strips Unicode whitespace (U+0085, U+00A0 are Latin-1 chars!).
  `strip()` in Java uses `Character.isWhitespace`.
- third-party crates that take `&str` and interpret characters (regex, serde_json, toml, url,
  semver, glob matchers): what does Java pass there? Java's `Pattern` on an internal string
  matches Latin-1 chars; Bazel converts with `internalToUnicode` first at `RegexUtil`,
  `OutputFilter`, `Converters`. Do exactly what Java does at that site.

ASCII must be untouched by every change: all conversions are the identity on ASCII and return
`Cow::Borrowed`.

## 3. Tests

- Port or extend the Java test of each site you change; expected values come from the Java test
  or from the oracle (`scripts/bytes/run`, saved in `~/.cache/blaze/work/bytes-out`; probe the
  oracle yourself for anything else, under `systemd-run ... -p MemoryMax=12G`), never from Blaze.
- A test that fed a non-ASCII **Unicode** literal where the real input is a file or argv now
  feeds the internal form: `latin1_decode("é".as_bytes())`, i.e. `"\u{c3}\u{a9}"`. Tests that
  wrapped code in `with_strings_are_internal_for_testing(false, ..)` describe a state that is
  gone: delete that half, keep the `true` half (without the wrapper).
- Add at least one non-ASCII test per boundary you convert: `é` (C3 A9), `🐷` (F0 9F 90 B7) and
  a byte that is not UTF-8 (FF).
- Un-ignore tests whose ignore reason was the missing Latin-1 decoding.

## 4. Check before you report

- `scripts/cargo-capped clippy -p <your crates> --all-targets -- -D warnings`, `rustfmt
  --edition 2024` on changed files, `scripts/cargo-capped test -p <your crates>` (report passed /
  failed / ignored and name every test you changed or added).
- `scripts/cargo-capped build -p blaze`, then
  `systemd-run --user --collect --wait --quiet --pipe --same-dir -p MemoryMax=12G -p MemorySwapMax=0
  scripts/bytes/run --binary $PWD/target/debug/blaze --out <your own dir> --output-user-root
  <your own dir>` and `scripts/bytes/compare --candidate <your own dir> --verbose`. Report
  `matching N / 97` and which remaining differences are in your crates. (Other workers run the
  same build; cargo serializes them. Use your own `--out` and `--output-user-root`, under
  `~/.cache/blaze/work/bytes-<name>` and `~/.cache/blaze/oracle-out/ur-bytes-<name>`.)
- List, per file, each site converted (Java reference), each emulation deleted, and each site
  you left with the reason. That list goes into the migration record.
