# Java regular expressions (`blaze_util::java_regex`)

Before this change, every site below compiled Bazel's `java.util.regex.Pattern` patterns with
the `regex` crate untranslated (`\w` Unicode instead of ASCII, `\<` a word boundary instead of
`<`, `.` matching `\r`, `(?i)` folding non-ASCII letters, syntax errors in the crate's words),
or through small per-site subset translators (`scrubber.rs`, `py_builtins.rs`, the `(?U)` and
`(?>` rewrites in `stubs.rs` and `regex_filter.rs`). They now all go through one module.
Constructs that used to compile through the old subset translators or through the crate
(atomic groups outside the end of a `find()` pattern, possessive quantifiers there, lookaround,
backreferences, `\G`, ...) are now loud (`unported!`) where they are not translated exactly.

## What it is

`crates/blaze_util/src/java_regex.rs` ports the parser of `java.util.regex.Pattern` of JDK 25
(the JDK embedded in Bazel 9.3.2; sources from that JDK's `src.zip`): `RemoveQEQuoting`, `expr`,
`sequence`, `atom`, `escape`, `clazz`, `range`, `bitsOrSingle`, `single`, `newSlice`, `family`,
`group0`, `addFlag`, `subFlag`, `closure`, the escape helpers `c`, `o`, `x`, `u`, `N`, and
`CharPredicates.forProperty` / `getUnicodePredicate` / `getPosixPredicate`, with Java's cursor.
It writes the same pattern in the `regex` crate's syntax. A Java syntax error is
`PatternSyntaxException.getMessage()`, character for character (index and caret line included).
`java_regex/case_table.rs` holds JDK 25's `Character.toUpperCase` / `toLowerCase` for every code
point either changes (2,933 entries), for `UNICODE_CASE`.

API: `translate(pattern, flags) -> Result<String, String>`, `compile(pattern, flags) ->
Result<regex::Regex, String>` (find mode, no match-time checks), and `Pattern::compile(pattern,
flags)` with `find` (byte offsets), `is_match` (`Matcher.find()`), `matches`
(`Matcher.matches()`), `pattern`, `regex`. Flags are `Pattern`'s bits (`DOTALL` = 0x20, ...).

Matching: both engines take the leftmost match and, among alternatives and quantifier choices,
the first in Java's backtracking order, so `find()` spans agree, with these exceptions, which are
`unported!`:

- A greedy repetition (`*`, `+`, `{n,m}` with `m >= 2`), or a lazy one with a finite maximum of
  2 or more (`{1,2}?`, `{2}?`, `{0,2}?`), whose body can match the empty string before trying a
  non-empty match and can consume input (`(?:a*|b)*`, `(?:\s*|#.*)*`, `(?:x??)*`,
  `(?:|ab|a){1,2}?`). Java ends such a loop at the first empty iteration and continues after it
  (`Pattern.Loop.match` / `LazyLoop.match`, the `i > locals[beginIndex]` test); the crate drops
  the empty iteration (or, for a counted repetition, which it writes as nested optionals, follows
  it by more iterations) and tries the body's later choices first: JDK 25 finds (0,1) for
  `(?:a*|b)*` on `abab`, the crate (0,4); (0,5) for `(?:|ab|a){1,2}?(?:b|c)` on `ababc`, the
  crate (0,2). For these `Pattern::find` (the span) and `translate`/`compile` (a bare `Regex`
  whose spans callers use) are `unported!`; `is_match` and `matches` agree and stay exact. An
  unbounded lazy repetition tries the exit first in both and agrees.
- A repetition of 2 or more whose nullable, consuming body contains a zero-width assertion
  (`(?:a|\b){2}b`): Java never follows an empty iteration with a non-empty one, the crate does,
  and where the assertion holds at one position and not at the next even the boolean differs
  (JDK 25: `(?:a|\b){2}b` does not match `abc`, the crate does). `unported!` in every mode.

`matches()` is the translation anchored as `\A(?:..)\z`. Inputs are matched by chars (Java: code
points).

## Constructs

| Construct | State |
| --- | --- |
| Literals, `\Q..\E` (Java's preprocessing, also in classes) | translated exactly |
| `\0n` `\0nn` `\0mnn`, `\xhh`, `\x{h..}`, `\uhhhh` (surrogate pairs joined) | translated exactly |
| `\t \n \r \f \a \e \cX`; `\` + non-alphanumeric (`\<` is `<`) | translated exactly |
| Unknown letter escapes (`\y`, `\E` alone, ...), `[\b]`, bad octal / hex | Java's error |
| `\w \W \d \D \s \S` (ASCII), with `(?U)` the Unicode classes | translated exactly |
| `\h \H \v \V` (`\v` is `\x0B` as a range end), `\R` | translated exactly |
| `.` without `DOTALL` (excludes `\n \r` U+0085 U+2028 U+2029), with it | translated exactly |
| `^` `\A` `\z`, `$` `\Z` (see match-time table), `(?m)^` `(?m)$` | translated, checked |
| `\b \B` (ASCII word chars; Unicode with `(?U)`) | translated, checked |
| Classes: ranges, `]` first, nested union, `&&`, negation, `[a-\d]` error | translated exactly |
| A class member after an `&&` operand that Java's shared `BitClass` mutates | LOUD |
| POSIX `\p{Alpha}` etc. (ASCII), `\p{Lower}`/`\p{Upper}` under `(?i)` | translated exactly |
| `\p{L}` `\p{IsLu}` `\p{gc=Nd}` `\pL` `\P{..}`, `LC` `LD` `L1` `all` | exact, Unicode 16.0 |
| `\p{IsX}` binary properties (Emoji, ...), `\p{IsAlpha}` | exact, Unicode 16.0 |
| `\p{InBlock}`, `\p{blk=..}`, scripts `\p{IsLatin}` `\p{sc=..}`, `\p{java..}` | LOUD |
| Unknown property names (not `Is`/`In`), `\p{a=b}` with another key | Java's error |
| `(?i)` ASCII folding; `(?iu)` as `SingleU` `SliceU` `CIRangeU` `BitClass` | translated exactly |
| `(?s)` `(?m)` `(?U)` `(?u)`, `-`, `(?flags:..)`, scoping across `|` | translated exactly |
| `Pattern.LITERAL` | translated exactly |
| `(?d)` UNIX_LINES (at `.`, `^`, `$`), `(?c)` CANON_EQ | LOUD |
| `(?x)` COMMENTS | LOUD (parsing stops) |
| Groups `( )`, `(?: )`, `(?<name> )` (Java's name rules and errors) | translated exactly |
| Greedy and lazy `* + ? {n} {n,} {n,m}`; `{n}` with nothing before it | translated exactly |
| Possessive `*+ ++ ?+ {n,m}+`, atomic `(?> )` | see next paragraph |
| Lookahead `(?= )` `(?! )`, lookbehind `(?<= )` `(?<! )` | LOUD |
| Backreferences `\1`..`\9`, `\k<name>` (missing name: Java's error) | LOUD |
| `\G`, `\X`, `\b{g}`, `\N{name}`, a lone surrogate | LOUD |
| Dangling `* + ?`, `{` not a quantifier, unclosed group or class, `(?P<` | Java's error |

Possessive quantifiers and atomic groups are translated (to the greedy quantifier and
`(?:..)`) only where nothing can follow them: at the end of the pattern through the ends of
unquantified enclosing groups and alternatives, and only for `find()`. There the first way they
match is the match, as in Java; this is the shape of `RegexFilter.takeUnionOfRegexes`,
`(?:(?>a)|(?>b))`. Anywhere else, and always under `matches()` (the end of the input must follow
them), they are LOUD: `not ported yet: java.util.regex.Pattern: atomic group (?>...) followed by
more of the pattern [under Matcher.matches()] in '<pattern>'` (likewise "possessive
quantifier ..."). Compile-time LOUD texts are `not ported yet: java.util.regex.Pattern: <construct>
in '<pattern>'` with `<construct>` one of `lookahead`, `lookbehind`, `backreference \N`,
`backreference \k<name>`, `\G`, `\X (grapheme cluster)`, `\b{g} (grapheme boundary)`,
`\N{name}`, `Unicode block \p{..}`, `Unicode script \p{..}`, `Unicode script (or unknown
property) \p{..}`, `Java character property \p{..}`, `surrogate code unit U+XXXX on its own`,
`surrogate code unit in a class range`, `UNIX_LINES ((?d)) '.'|'^'|'$'`, `CANON_EQ ((?c))`,
`CANON_EQ`, `COMMENTS ((?x))`, `character class with members after an intersection's left
operand`.

## Match-time checks (`Pattern::find`, `is_match`, `matches`)

| Pattern construct | Input that triggers `unported!` | Example |
| --- | --- | --- |
| `$` or `\Z` without `MULTILINE` | last char `\n`, `\r`, U+0085, U+2028, U+2029 | `a$`, `"a\n"` |
| `(?m)^` or `(?m)$` | contains `\r`, U+0085, U+2028 or U+2029 | `(?m)a$` on `"a\r\n"` |
| `(?m)^` | empty, or ends in `\n` | `(?m)^` on `""` |
| `\b` or `\B` | contains a non-spacing mark (`\p{Mn}`) | `a\b` on a, U+0301, b |
| greedy repetition, or lazy with a finite maximum of 2 or more, whose body can match empty before a non-empty string (`find` only) | any | `(?:a*\|b)*`, `(?:\|ab\|a){1,2}?` |

Java's `$` also matches before a final line terminator, its `MULTILINE` anchors know five
terminators and `^` never matches at the end of the input, and its `\b` counts a non-spacing mark
after a letter or digit as a word char. The bare `compile()` returns a `regex::Regex` without
these checks.

## Remaining deviation

A pattern the crate rejects after translation (its size limits, for example a repetition count
over its limit, which Java accepts) is an error with the crate's text, not Java's.

## Callers

`RegexPatternOption` serves every option with `RegexPatternConverter` (`--output_filter`,
`--remote_download_regex`, ...); `JavaPattern` every `RegexFilter` option
(`--instrumentation_filter`, `--per_file_copt`, `--modify_execution_info`, ...);
`regex_filter_expression.rs` also serves `ActionFilterFunction` and the aquery processor.
`as given`: the caller's string, unconverted.

| File | Java | Flags | Input |
| --- | --- | --- | --- |
| `blaze_options` `converters.rs` | `RegexPatternConverter` | `DOTALL` | Unicode |
| `blaze_optdefs` `regex_filter.rs` | `RegexFilter` | 0 | as given |
| `blaze_query_engine` `regex_filter_expression.rs` | `RegexFilterExpression` | 0 | internal |
| `blaze_cquery` `aquery_action_filter.rs` | `AqueryActionFilter` | 0 | internal |
| `blaze_events` `output_filter.rs` | `RegexOutputFilter` | option's | Unicode |
| `blaze_rules` `python/py_builtins.rs` | `String.matches` | 0 | internal |
| `blaze_packages` `builtins/stubs.rs` `regex_match` | `PyBuiltins.regexMatch` | 0 | internal |
| `blaze_cc_features` `cc_common.rs` | `CcCommon.CoptsFilter` | 0 | as given |
| `blaze_remote` `scrubber.rs` | `Scrubber` | 0 | internal |

Not routed:

- `blaze_repo_fetch/src/downloader/url_rewriter.rs` (`UrlRewriterConfig`): needs
  `PatternSyntaxException.getDescription()` / `getIndex()` and capture groups, which the API does
  not expose yet (`Pattern::compile_with_exception`, `Pattern::captures`).
- `blaze_packages/src/build/unix_glob.rs`: the regex is built by Bazel from a glob wildcard
  (`UnixGlob.makePatternFromWildcard`), translated by hand there.
- Constant patterns of Bazel's own code written directly in crate syntax: `sem_ver.rs`,
  `rc_file.rs`, `lockfile.rs`, `version.rs`, `proxy_helper.rs`, `patch_util.rs`,
  `cc_toolchain_features_lib.rs`, `spawn_log_reconstructor.rs`, `java_starlark_common.rs`,
  `byte_stream_build_event_artifact_uploader.rs`, `caffeine_spec.rs`, and two in
  `converters.rs`. Two use Unicode crate semantics where Java's are ASCII: `\d` in
  `sem_ver.rs` and `(?i:..S..)` in `caffeine_spec.rs` (also matches U+017F).

## Tests and how to regenerate them

`crates/blaze_util/tests/java_regex_test.rs` holds 381 rows (pattern, flags, input, Java's find
span and `matches()` or Java's error message, and whether Blaze must be loud), all produced by
`java.util.regex.Pattern` of JDK 25:

```
java crates/blaze_util/tests/data/JavaRegexRows.java > rows.txt
```

and paste the output between the `ROWS BEGIN` / `ROWS END` markers of the test. Rows that are
translated for `find()` but loud under `matches()` are listed in its `LOUD_UNDER_MATCHES`.

`tests/data/java_regex_nullable_rows.rs` (42,640 rows: 41 bodies that can match the empty
string, under 16 quantifiers (greedy and lazy, counted and unbounded), 5 suffixes, 13 inputs;
find span and `matches()` of JDK 25; patterns Java rejects are left out) is
generated by `java crates/blaze_util/tests/data/JavaRegexNullableRows.java`; the test
`nullable_repetitions_follow_java_or_are_loud` requires every row to agree with Java or fail
loudly, and prints how many loud rows would have agreed.
The case table is regenerated with

```
java crates/blaze_util/tests/data/JavaCaseTable.java
```

whose output (after its first line, a count) is the body of `JAVA_CASE` in
`java_regex/case_table.rs`. Through the binary: `scripts/remote/regex-probe <oracle|BINARY> <tag>`
(`--remote_download_regex` with patterns whose Java and crate meanings differ).
