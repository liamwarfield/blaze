//! Port of `lib/remote/Scrubber.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_exec::actions::{ActionOwner, Spawn};
use blaze_protos::remote_scrubbing::{Config, config};
use blaze_vfs::PathFragment;
use regex::Regex;

/// `java.util.regex.PatternSyntaxException`, as thrown by `Pattern.compile`.
///
/// PORT: Java throws it unchecked from the `Scrubber` constructor and `Scrubber.parse` turns it
/// into a `ConfigParseException`; here the constructor returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternSyntaxException {
    pattern: String,
    message: String,
}

impl PatternSyntaxException {
    /// `getPattern()`.
    pub fn get_pattern(&self) -> &str {
        &self.pattern
    }

    /// `getMessage()`.
    ///
    /// PORT: the text is the `regex` crate's description of the error, not `java.util.regex`'s.
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PatternSyntaxException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PatternSyntaxException {}

/// `java.util.regex.Pattern`, as far as the scrubber uses it (`matcher(s).matches()` and
/// `matcher(s).find()` with `start()` / `end()`).
///
/// The pattern is a Unicode string (a proto field); the input is an internal string (an exec
/// path or an argument: one Latin-1 char per byte). Java's `Pattern` runs over the UTF-16 code
/// units of the input, i.e. over its chars, as the `regex` crate does on a `&str`, so a pattern
/// char `é` (U+00E9) does not match the internal `é` (U+00C3 U+00A9), in Java as here.
///
/// The pattern is translated by [`translate_java_regex`] so that the constructs whose Java
/// default (no flags given to `Pattern.compile`) is ASCII-only, and which the `regex` crate makes
/// Unicode-aware, keep Java's meaning on the Latin-1 chars of an internal string:
/// `\w \W \d \D \s \S \b \B \h \H \v \V`, `.` (Java excludes `\r`, U+0085, U+2028,
/// U+2029 besides `\n`, unless `(?s)`), and `(?i)` (Java folds ASCII letters only, unless
/// `(?u)`).
///
/// PORT: what remains of the `regex` crate's dialect:
/// - no backreferences, lookahead / lookbehind, possessive quantifiers or atomic groups (such a
///   pattern fails to compile and is a `PatternSyntaxException`);
/// - `$` matches only at the very end (Java also before a final line terminator), and with
///   `(?m)` `^`/`$` see only `\n` as a line terminator;
/// - `\p{..}` classes are passed through: the POSIX names (`\p{Alpha}`, ...) are ASCII-only in
///   Java and Unicode in the crate, and `(?i)` does not fold them;
/// - the flags `(?x)`, `(?u)`, `(?U)`, `(?d)`, `(?c)` are not ported (`unported!`).
///
/// Both engines prefer the leftmost match and, among alternatives, the first one that matches
/// (Perl semantics), so `find()` returns the same span for patterns in the common subset.
/// `matches()` (the whole input must match) is the pattern anchored as `\A(?:p)\z`.
struct Pattern {
    /// `pattern()`.
    pattern: String,
    /// The pattern as written, for `Matcher.find()`.
    find: Regex,
    /// The pattern anchored at both ends, for `Matcher.matches()`.
    matches: Regex,
}

impl Pattern {
    /// `Pattern.compile(regex)`.
    fn compile(regex: &str) -> Result<Arc<Pattern>, PatternSyntaxException> {
        let syntax_error = |e: regex::Error| PatternSyntaxException {
            pattern: regex.to_string(),
            message: e.to_string(),
        };
        let translated = translate_java_regex(regex);
        let find = Regex::new(&translated).map_err(syntax_error)?;
        let matches = Regex::new(&format!(r"\A(?:{translated})\z")).map_err(syntax_error)?;
        Ok(Arc::new(Pattern {
            pattern: regex.to_string(),
            find,
            matches,
        }))
    }

    /// `matcher(input).matches()`.
    fn matches(&self, input: &str) -> bool {
        self.matches.is_match(input)
    }

    /// `matcher(input).find()`: the `start()` and `end()` of the first match.
    fn find(&self, input: &str) -> Option<(usize, usize)> {
        self.find.find(input).map(|m| (m.start(), m.end()))
    }
}

impl fmt::Debug for Pattern {
    /// `Pattern.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.pattern)
    }
}

/// Java's `\w` (`[a-zA-Z_0-9]`), as the inside of a `regex` class.
const JAVA_WORD: &str = r"0-9A-Za-z_";
/// Java's `\d` (`[0-9]`).
const JAVA_DIGIT: &str = r"0-9";
/// Java's `\s` (`[ \t\n\x0B\f\r]`).
const JAVA_SPACE: &str = r"\x20\t\n\x0B\x0C\r";
/// Java's `\h` (`[ \t\xA0\u1680\u180e\u2000-\u200a\u202f\u205f\u3000]`).
const JAVA_HORIZONTAL: &str =
    r"\x20\t\xA0\x{1680}\x{180E}\x{2000}-\x{200A}\x{202F}\x{205F}\x{3000}";
/// Java's `\v` (`[\n\x0B\f\r\x85\u2028\u2029]`).
const JAVA_VERTICAL: &str = r"\n\x0B\x0C\r\x{85}\x{2028}\x{2029}";
/// What Java's `.` does not match without `DOTALL`: the line terminators of
/// `Pattern.LineEnding` (`\n`, `\r`, U+0085, U+2028, U+2029).
const JAVA_DOT_EXCLUDES: &str = r"\n\r\x{85}\x{2028}\x{2029}";

/// The flags of `java.util.regex.Pattern` that the translation tracks itself.
#[derive(Clone, Copy, Default)]
struct JavaRegexFlags {
    /// `CASE_INSENSITIVE` (`(?i)`): ASCII letters only (no `UNICODE_CASE`).
    case_insensitive: bool,
    /// `DOTALL` (`(?s)`).
    dotall: bool,
}

/// Translates a `java.util.regex.Pattern` (compiled without flags) into the `regex` crate's
/// syntax, so that the constructs listed on [`Pattern`] keep Java's ASCII meaning on internal
/// strings.
///
/// The `regex` crate cannot run in its ASCII mode (`(?-u)`) on a `&str` (`.` and negated classes
/// could then match part of a UTF-8 sequence), and `regex::bytes` would see the UTF-8 bytes of
/// the Latin-1 chars instead of the chars Java sees. So the translation stays in Unicode mode and
/// spells out Java's classes, never emits `(?i)` or `(?s)` (it folds ASCII letters and expands
/// `.` itself), and uses the crate's ASCII word boundary `(?-u:\b)`.
// Java: java/util/regex/Pattern.java (JDK 25, the JDK embedded in Bazel 9.3.2)
fn translate_java_regex(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() * 2);
    // One entry per open group; the last is the scope of the flags.
    let mut scopes: Vec<JavaRegexFlags> = vec![JavaRegexFlags::default()];
    let mut i = 0;
    while i < chars.len() {
        let flags = *scopes.last().expect("scope");
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() => {
                let e = chars[i + 1];
                i += 2;
                if let Some((set, negated)) = java_class_escape(e) {
                    out.push_str(if negated { "[^" } else { "[" });
                    out.push_str(set);
                    out.push(']');
                    continue;
                }
                match e {
                    'b' => out.push_str(r"(?-u:\b)"),
                    'B' => out.push_str(r"(?-u:\B)"),
                    'Q' => {
                        // `\Q...\E`: every char up to `\E` (or the end) is a literal.
                        while i < chars.len() {
                            if chars[i] == '\\' && chars.get(i + 1) == Some(&'E') {
                                i += 2;
                                break;
                            }
                            push_literal(&mut out, chars[i], flags.case_insensitive);
                            i += 1;
                        }
                    }
                    'p' | 'P' | 'N' | 'k' => {
                        // A name in braces or angle brackets, copied as is (never folded).
                        out.push('\\');
                        out.push(e);
                        i = copy_bracketed(&chars, i, &mut out);
                    }
                    'x' | 'u' => match parse_numeric_escape(&chars, i, e) {
                        Some((ch, next)) => {
                            push_literal(&mut out, ch, flags.case_insensitive);
                            i = next;
                        }
                        None => {
                            out.push('\\');
                            out.push(e);
                        }
                    },
                    _ => {
                        out.push('\\');
                        out.push(e);
                    }
                }
            }
            '.' => {
                if flags.dotall {
                    out.push_str("(?s:.)");
                } else {
                    out.push_str("[^");
                    out.push_str(JAVA_DOT_EXCLUDES);
                    out.push(']');
                }
                i += 1;
            }
            '[' => {
                i = translate_class(&chars, i + 1, flags.case_insensitive, &mut out);
            }
            '(' => {
                i += 1;
                if chars.get(i) == Some(&'?') {
                    // `(?flags)` or `(?flags:`.
                    let mut j = i + 1;
                    while j < chars.len() && (chars[j].is_ascii_alphabetic() || chars[j] == '-') {
                        j += 1;
                    }
                    if matches!(chars.get(j), Some(')') | Some(':')) {
                        let mut on = true;
                        let mut new_flags = flags;
                        let mut passed = String::new();
                        for &flag in &chars[i + 1..j] {
                            match flag {
                                '-' => {
                                    on = false;
                                    passed.push('-');
                                }
                                'i' => new_flags.case_insensitive = on,
                                's' => new_flags.dotall = on,
                                'm' => passed.push('m'),
                                other => blaze_util::unported!(format!(
                                    "java.util.regex.Pattern flag ({other}) in scrubbing pattern \
                                     '{pattern}'"
                                )),
                            }
                        }
                        let passed = passed.trim_end_matches('-');
                        if chars[j] == ')' {
                            // Sets the flags until the end of the enclosing group.
                            *scopes.last_mut().expect("scope") = new_flags;
                            if !passed.is_empty() {
                                out.push_str("(?");
                                out.push_str(passed);
                                out.push(')');
                            }
                            i = j + 1;
                            continue;
                        }
                        out.push_str("(?");
                        out.push_str(passed);
                        out.push(':');
                        scopes.push(new_flags);
                        i = j + 1;
                        continue;
                    }
                    // Another `(?` construct: a named group's name is copied as is.
                    out.push_str("(?");
                    i += 1;
                    if chars.get(i) == Some(&'P') {
                        out.push('P');
                        i += 1;
                    }
                    if chars.get(i) == Some(&'<')
                        && chars.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic())
                    {
                        while i < chars.len() {
                            out.push(chars[i]);
                            i += 1;
                            if chars[i - 1] == '>' {
                                break;
                            }
                        }
                    }
                } else {
                    out.push('(');
                }
                scopes.push(flags);
            }
            ')' => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
                out.push(')');
                i += 1;
            }
            '{' => {
                // A quantifier, copied as is.
                i = copy_bracketed(&chars, i, &mut out);
            }
            _ => {
                if flags.case_insensitive && c.is_ascii_alphabetic() {
                    push_literal(&mut out, c, true);
                } else {
                    out.push(c);
                }
                i += 1;
            }
        }
    }
    out
}

/// The predefined classes of `Pattern` (`\w` and the like): the inside of the `regex` class with
/// Java's members, and whether the escape is the negation.
fn java_class_escape(e: char) -> Option<(&'static str, bool)> {
    Some(match e {
        'w' => (JAVA_WORD, false),
        'W' => (JAVA_WORD, true),
        'd' => (JAVA_DIGIT, false),
        'D' => (JAVA_DIGIT, true),
        's' => (JAVA_SPACE, false),
        'S' => (JAVA_SPACE, true),
        'h' => (JAVA_HORIZONTAL, false),
        'H' => (JAVA_HORIZONTAL, true),
        'v' => (JAVA_VERTICAL, false),
        'V' => (JAVA_VERTICAL, true),
        _ => return None,
    })
}

/// Copies `{...}` or `<...>` starting at `chars[i]` (if it opens one) and returns the index after
/// it.
fn copy_bracketed(chars: &[char], mut i: usize, out: &mut String) -> usize {
    let close = match chars.get(i) {
        Some('{') => '}',
        Some('<') => '>',
        _ => return i,
    };
    while i < chars.len() {
        out.push(chars[i]);
        i += 1;
        if chars[i - 1] == close {
            break;
        }
    }
    i
}

/// `\xhh`, `\x{h...h}` or `\uhhhh` after the escape letter at `chars[i - 1]`: the char and the
/// index after the escape.
fn parse_numeric_escape(chars: &[char], i: usize, e: char) -> Option<(char, usize)> {
    let hex = |from: usize, to: usize| -> Option<u32> {
        let digits: String = chars.get(from..to)?.iter().collect();
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(&digits, 16).ok()
    };
    let (value, next) = match e {
        'x' if chars.get(i) == Some(&'{') => {
            let close = i + chars[i..].iter().position(|&c| c == '}')?;
            (hex(i + 1, close)?, close + 1)
        }
        'x' => (hex(i, i + 2)?, i + 2),
        'u' => (hex(i, i + 4)?, i + 4),
        _ => return None,
    };
    Some((char::from_u32(value)?, next))
}

/// A literal char, escaped for the `regex` crate; with `CASE_INSENSITIVE` an ASCII letter also
/// matches its other case (`ASCII.toLower`/`toUpper`), and no other char is folded.
fn push_literal(out: &mut String, c: char, case_insensitive: bool) {
    if case_insensitive && c.is_ascii_alphabetic() {
        out.push('[');
        out.push(c.to_ascii_lowercase());
        out.push(c.to_ascii_uppercase());
        out.push(']');
    } else {
        out.push_str(&format!(r"\x{{{:X}}}", c as u32));
    }
}

/// Translates a character class whose `[` is at `chars[i - 1]`; returns the index after its `]`.
///
/// With `CASE_INSENSITIVE` each single char and range also matches the other case of the ASCII
/// letters in it, as Java folds each member (`Pattern.range`/`single` with `ASCII.toLower` /
/// `toUpper`); negation and intersection apply to the folded members, as in Java.
fn translate_class(
    chars: &[char],
    mut i: usize,
    case_insensitive: bool,
    out: &mut String,
) -> usize {
    out.push('[');
    if chars.get(i) == Some(&'^') {
        out.push('^');
        i += 1;
    }
    let mut first = true;
    while i < chars.len() {
        let c = chars[i];
        if c == ']' && !first {
            out.push(']');
            return i + 1;
        }
        first = false;
        if c == '[' {
            i = translate_class(chars, i + 1, case_insensitive, out);
            continue;
        }
        if c == '&' && chars.get(i + 1) == Some(&'&') {
            out.push_str("&&");
            i += 2;
            continue;
        }
        // A single char, possibly the start of a range.
        let (lo, next) = match class_atom(chars, i, out) {
            Some(atom) => atom,
            None => {
                // A predefined class or a property, already written.
                i = class_atom_end(chars, i);
                continue;
            }
        };
        i = next;
        let mut hi = lo;
        if chars.get(i) == Some(&'-') && chars.get(i + 1).is_some_and(|&c| c != ']') {
            let mut scratch = String::new();
            if let Some((end, after)) = class_atom(chars, i + 1, &mut scratch) {
                hi = end;
                i = after;
            }
        }
        out.push_str(&format!(r"\x{{{:X}}}", lo as u32));
        if hi != lo {
            out.push_str(&format!(r"-\x{{{:X}}}", hi as u32));
        }
        if case_insensitive {
            for letter in ('A'..='Z').chain('a'..='z') {
                let other = if letter.is_ascii_uppercase() {
                    letter.to_ascii_lowercase()
                } else {
                    letter.to_ascii_uppercase()
                };
                if (lo..=hi).contains(&other) && !(lo..=hi).contains(&letter) {
                    out.push(letter);
                }
            }
        }
    }
    // Unclosed: the `regex` crate reports the syntax error.
    i
}

/// A literal char of a class at `chars[i]` and the index after it, or `None` when the item is a
/// predefined class (`\w`, ...) or a property (`\p{..}`), which this writes to `out`.
fn class_atom(chars: &[char], i: usize, out: &mut String) -> Option<(char, usize)> {
    let c = chars[i];
    if c != '\\' || i + 1 >= chars.len() {
        return Some((c, i + 1));
    }
    let e = chars[i + 1];
    if let Some((set, negated)) = java_class_escape(e) {
        if negated {
            out.push_str("[^");
            out.push_str(set);
            out.push(']');
        } else {
            out.push_str(set);
        }
        return None;
    }
    match e {
        'p' | 'P' => {
            out.push('\\');
            out.push(e);
            copy_bracketed(chars, i + 2, out);
            None
        }
        'x' | 'u' => match parse_numeric_escape(chars, i + 2, e) {
            Some(atom) => Some(atom),
            None => Some((e, i + 2)),
        },
        't' => Some(('\t', i + 2)),
        'n' => Some(('\n', i + 2)),
        'r' => Some(('\r', i + 2)),
        'f' => Some(('\x0C', i + 2)),
        'a' => Some(('\x07', i + 2)),
        'e' => Some(('\x1B', i + 2)),
        _ if e.is_ascii_alphanumeric() => {
            // Another escape (`\0`, `\c`, ...): copied as is for the `regex` crate.
            out.push('\\');
            out.push(e);
            None
        }
        _ => Some((e, i + 2)),
    }
}

/// The index after a class item for which [`class_atom`] returned `None`.
fn class_atom_end(chars: &[char], i: usize) -> usize {
    match chars.get(i + 1) {
        Some('p') | Some('P') => {
            let mut scratch = String::new();
            copy_bracketed(chars, i + 2, &mut scratch)
        }
        _ => i + 2,
    }
}

/// `Objects.equals(pattern1, pattern2)`: `Pattern` does not override `equals`, so two patterns
/// are equal only when they are the same object.
fn pattern_equals(a: &Arc<Pattern>, b: &Arc<Pattern>) -> bool {
    Arc::ptr_eq(a, b)
}

/// `Pattern.hashCode()` (the identity hash).
fn pattern_hash<H: Hasher>(pattern: &Arc<Pattern>, state: &mut H) {
    (Arc::as_ptr(pattern) as usize).hash(state);
}

/// An error that occurred while parsing the scrubbing configuration.
// Java: com/google/devtools/build/lib/remote/Scrubber.java:49 Scrubber.ConfigParseException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigParseException {
    message: String,
}

impl ConfigParseException {
    // Java: com/google/devtools/build/lib/remote/Scrubber.java:51 ConfigParseException.<init>
    #[allow(dead_code)] // Only `Scrubber.parse` creates it, and that is not ported.
    fn new(message: String) -> ConfigParseException {
        ConfigParseException { message }
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ConfigParseException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConfigParseException {}

/// The `Scrubber` implements scrubbing of remote cache keys.
///
/// See the documentation for the `--experimental_remote_scrub_config` flag for more information.
// Java: com/google/devtools/build/lib/remote/Scrubber.java:46 Scrubber
#[derive(Debug, Clone)]
pub struct Scrubber {
    spawn_scrubbers: Vec<SpawnScrubber>,
}

impl Scrubber {
    /// `new Scrubber(configProto)` (`@VisibleForTesting` in Java).
    // Java: com/google/devtools/build/lib/remote/Scrubber.java:59 Scrubber.<init>
    pub fn new(config_proto: &Config) -> Result<Scrubber, PatternSyntaxException> {
        let mut spawn_scrubbers = Vec::new();
        for rule_proto in &config_proto.rules {
            spawn_scrubbers.push(SpawnScrubber::new(rule_proto)?);
        }
        // Reverse the order so that later rules supersede earlier ones.
        spawn_scrubbers.reverse();
        Ok(Scrubber { spawn_scrubbers })
    }

    /// Constructs a `Scrubber` from the given configuration file, which must contain a `Config`
    /// protocol buffer in text format.
    // Java: com/google/devtools/build/lib/remote/Scrubber.java:73 Scrubber.parse
    pub fn parse(_config_path: &str) -> Result<Scrubber, ConfigParseException> {
        // PORT: Blaze has no protobuf text-format parser. Java reads the file, merges it into a
        // `Config` with `TextFormat.getParser()`, calls the constructor and reports an
        // `IOException` as `ConfigParseException(e.getMessage())` and a `PatternSyntaxException`
        // as `ConfigParseException("in regex '<pattern>': <message>")`.
        blaze_util::unported!("Scrubber.parse (TextFormat)")
    }

    /// Returns a `SpawnScrubber` suitable for a `Spawn`, or `None` if the spawn does not need to
    /// be scrubbed.
    // Java: com/google/devtools/build/lib/remote/Scrubber.java:91 Scrubber.forSpawn
    // Kept as Java's loop.
    #[allow(clippy::manual_find)]
    pub fn for_spawn(&self, spawn: &dyn Spawn) -> Option<&SpawnScrubber> {
        for spawn_scrubber in &self.spawn_scrubbers {
            if spawn_scrubber.matches(spawn) {
                return Some(spawn_scrubber);
            }
        }
        None
    }
}

// Java: com/google/devtools/build/lib/remote/Scrubber.java:101 Scrubber.equals
impl PartialEq for Scrubber {
    fn eq(&self, that: &Scrubber) -> bool {
        if std::ptr::eq(self, that) {
            return true;
        }
        self.spawn_scrubbers == that.spawn_scrubbers
    }
}

impl Eq for Scrubber {}

// Java: com/google/devtools/build/lib/remote/Scrubber.java:109 Scrubber.hashCode
impl Hash for Scrubber {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.spawn_scrubbers.hash(state);
    }
}

/// Encapsulates a set of transformations required to scrub the remote cache key for a set of
/// spawns.
// Java: com/google/devtools/build/lib/remote/Scrubber.java:117 Scrubber.SpawnScrubber
#[derive(Debug, Clone)]
pub struct SpawnScrubber {
    mnemonic_pattern: Arc<Pattern>,
    label_pattern: Arc<Pattern>,
    kind_pattern: Arc<Pattern>,
    match_tools: bool,

    omitted_input_patterns: Vec<Arc<Pattern>>,
    /// `ImmutableMap<Pattern, String>`, in insertion order (patterns are compared by identity,
    /// so no two keys are ever equal).
    arg_replacements: Vec<(Arc<Pattern>, String)>,
    salt: String,
}

impl SpawnScrubber {
    // Java: com/google/devtools/build/lib/remote/Scrubber.java:128 SpawnScrubber.<init>
    fn new(rule_proto: &config::Rule) -> Result<SpawnScrubber, PatternSyntaxException> {
        // `getMatcher()` / `getTransform()` of an unset field are the default instances.
        let default_matcher = config::Matcher::default();
        let matcher_proto = rule_proto.matcher.as_ref().unwrap_or(&default_matcher);
        let mnemonic_pattern =
            Pattern::compile(SpawnScrubber::empty_to_all(&matcher_proto.mnemonic))?;
        let label_pattern = Pattern::compile(SpawnScrubber::empty_to_all(&matcher_proto.label))?;
        let kind_pattern = Pattern::compile(SpawnScrubber::empty_to_all(&matcher_proto.kind))?;
        let match_tools = matcher_proto.match_tools;

        let default_transform = config::Transform::default();
        let transform_proto = rule_proto.transform.as_ref().unwrap_or(&default_transform);
        let mut omitted_input_patterns = Vec::new();
        for omitted_input in &transform_proto.omitted_inputs {
            omitted_input_patterns.push(Pattern::compile(omitted_input)?);
        }
        let mut arg_replacements = Vec::new();
        for r in &transform_proto.arg_replacements {
            arg_replacements.push((Pattern::compile(&r.source)?, r.target.clone()));
        }
        let salt = transform_proto.salt.clone();
        Ok(SpawnScrubber {
            mnemonic_pattern,
            label_pattern,
            kind_pattern,
            match_tools,
            omitted_input_patterns,
            arg_replacements,
            salt,
        })
    }

    // Java: com/google/devtools/build/lib/remote/Scrubber.java:146 SpawnScrubber.emptyToAll
    fn empty_to_all(s: &str) -> &str {
        if s.is_empty() { ".*" } else { s }
    }

    /// Whether this scrubber applies to the given `Spawn`.
    // Java: com/google/devtools/build/lib/remote/Scrubber.java:151 SpawnScrubber.matches
    fn matches(&self, spawn: &dyn Spawn) -> bool {
        let mnemonic = spawn.get_mnemonic();
        let action_owner = spawn.get_resource_owner().get_owner();
        let label = match action_owner.get_label() {
            Some(label) => label.get_canonical_form(),
            None => panic!("NullPointerException: ActionOwner.getLabel()"),
        };
        let kind = action_owner_get_target_kind(action_owner);
        let is_for_tool = action_owner_is_build_configuration_for_tool(action_owner);

        (!is_for_tool || self.match_tools)
            && self.mnemonic_pattern.matches(&mnemonic)
            && self.label_pattern.matches(&label)
            && self.kind_pattern.matches(&kind)
    }

    /// Whether an input with the given exec-relative path should be omitted from the cache key.
    // Java: com/google/devtools/build/lib/remote/Scrubber.java:165 SpawnScrubber.shouldOmitInput
    pub fn should_omit_input(&self, exec_path: &PathFragment) -> bool {
        for pattern in &self.omitted_input_patterns {
            if pattern.matches(exec_path.get_path_string()) {
                return true;
            }
        }
        false
    }

    /// Transforms a command line argument.
    // Java: com/google/devtools/build/lib/remote/Scrubber.java:175 SpawnScrubber.transformArgument
    pub fn transform_argument(&self, arg: &str) -> String {
        let mut arg = arg.to_string();
        for (pattern, replacement) in &self.arg_replacements {
            // Don't use Pattern#replaceFirst because it allows references to capture groups.
            if let Some((start, end)) = pattern.find(&arg) {
                arg = format!("{}{}{}", &arg[..start], replacement, &arg[end..]);
            }
        }
        arg
    }

    /// Returns the scrubbing salt.
    // Java: com/google/devtools/build/lib/remote/Scrubber.java:189 SpawnScrubber.getSalt
    pub fn get_salt(&self) -> &str {
        &self.salt
    }
}

// Java: com/google/devtools/build/lib/remote/Scrubber.java:194 SpawnScrubber.equals
impl PartialEq for SpawnScrubber {
    fn eq(&self, that: &SpawnScrubber) -> bool {
        if std::ptr::eq(self, that) {
            return true;
        }
        self.match_tools == that.match_tools
            && pattern_equals(&self.mnemonic_pattern, &that.mnemonic_pattern)
            && pattern_equals(&self.label_pattern, &that.label_pattern)
            && pattern_equals(&self.kind_pattern, &that.kind_pattern)
            // List.equals: the same patterns in the same order.
            && self.omitted_input_patterns.len() == that.omitted_input_patterns.len()
            && self
                .omitted_input_patterns
                .iter()
                .zip(&that.omitted_input_patterns)
                .all(|(a, b)| pattern_equals(a, b))
            // Map.equals: the same entries, in any order.
            && self.arg_replacements.len() == that.arg_replacements.len()
            && self.arg_replacements.iter().all(|(pattern, replacement)| {
                that.arg_replacements
                    .iter()
                    .any(|(p, r)| pattern_equals(pattern, p) && replacement == r)
            })
            && self.salt == that.salt
    }
}

impl Eq for SpawnScrubber {}

// Java: com/google/devtools/build/lib/remote/Scrubber.java:209 SpawnScrubber.hashCode
impl Hash for SpawnScrubber {
    fn hash<H: Hasher>(&self, state: &mut H) {
        pattern_hash(&self.mnemonic_pattern, state);
        pattern_hash(&self.label_pattern, state);
        pattern_hash(&self.kind_pattern, state);
        self.match_tools.hash(state);
        for pattern in &self.omitted_input_patterns {
            pattern_hash(pattern, state);
        }
        // PORT: `Map.hashCode()` does not depend on the entry order; the replacements are left
        // out of the hash (equal scrubbers still hash equally).
        self.salt.hash(state);
    }
}

/// `actionOwner.getTargetKind()`.
// Java: com/google/devtools/build/lib/actions/ActionOwner.java:111 ActionOwner.getTargetKind
fn action_owner_get_target_kind(action_owner: &ActionOwner) -> String {
    action_owner.get_target_kind().to_owned()
}

/// `actionOwner.isBuildConfigurationForTool()`.
// Java: com/google/devtools/build/lib/actions/ActionOwner.java:169 ActionOwner.isBuildConfigurationForTool
fn action_owner_is_build_configuration_for_tool(action_owner: &ActionOwner) -> bool {
    action_owner.is_build_configuration_for_tool()
}
