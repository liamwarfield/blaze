//! `java.util.regex.Pattern` (JDK 25, the JDK embedded in Bazel 9.3.2) on top of the `regex`
//! crate.
//!
//! Bazel compiles user patterns (option values such as `--remote_download_regex`,
//! `--output_filter`, `--instrumentation_filter`, `--modify_execution_info`, query `filter()`,
//! scrubbing configurations) with `java.util.regex.Pattern`. The `regex` crate's dialect differs
//! from Java's, silently in places (`\w` is Unicode in the crate and ASCII in Java, `\<` is a
//! word boundary in the crate and a literal `<` in Java, `.` excludes more line terminators in
//! Java, ...). This module parses a Java pattern the way `Pattern.compile` does (the parser
//! below is a port of `Pattern.expr`, `sequence`, `atom`, `escape`, `clazz`, `range`, `family`,
//! `group0`, `closure`, `RemoveQEQuoting` and the escape helpers, with Java's cursor, so syntax
//! errors carry `PatternSyntaxException.getMessage()`), and writes the same pattern in the
//! crate's syntax. Every construct is either translated to an equivalent or fails loudly
//! (`unported!`, naming the construct):
//!
//! - Translated: literals (escaped as `\x{..}`), `\Q..\E` (Java's preprocessing),
//!   octal `\0n`, `\xhh`, `\x{h..}`, `\uhhhh` (and surrogate pairs), `\t \n \r \f \a \e \cX`,
//!   an escaped non-alphanumeric char as that char (`\<` is `<`); the ASCII classes
//!   `\w \W \d \D \s \S` and `\h \H \v \V \R`; `.` without `DOTALL` (Java excludes `\n`, `\r`,
//!   U+0085, U+2028, U+2029) and with it; `^ $ \A \z \Z` and `\b \B` (ASCII word chars);
//!   character classes with Java's union, intersection, negation and range rules; the POSIX
//!   classes `\p{Alpha}` etc. (ASCII in Java), the general categories `\p{L}`, `\p{IsLu}`,
//!   `\p{gc=Nd}` (both sides use Unicode 16.0), `\p{L1}`, `\p{all}`, `\p{IsAlphabetic}`,
//!   `\p{IsLowercase}`, `\p{IsUppercase}`, `\p{IsWhite_Space}`; `CASE_INSENSITIVE` (`(?i)`,
//!   ASCII letters only, as Java without `UNICODE_CASE`), `DOTALL`, `MULTILINE`, `LITERAL`;
//!   flag groups with Java's scoping (a `(?i)` lasts to the end of the enclosing group, across
//!   `|`); capturing, named and non-capturing groups; greedy and lazy quantifiers.
//! - Atomic groups `(?>..)` and possessive quantifiers (`*+ ++ ?+ {n,m}+`) change which matches
//!   are found only when something after them can still fail. They are translated (to `(?:..)`
//!   and the greedy quantifier) exactly when nothing can follow them: they end the pattern
//!   (through the ends of unquantified enclosing groups and alternatives) and the pattern is used
//!   with `Matcher.find()`. Then the first way they match is the match, as in Java. This is the
//!   shape of `RegexFilter.takeUnionOfRegexes` (`(?:(?>a)|(?>b))`). Elsewhere (followed by
//!   anything, inside a quantified group, or under `Matcher.matches()`, which requires the end of
//!   the input after them) they are `unported!`.
//! - `unported!`: backreferences (`\1`, `\k<name>`), lookahead and lookbehind, `\G`, `\X`,
//!   `\b{g}`, `\N{name}`, Unicode blocks and scripts (`\p{InGreek}`, `\p{IsLatin}`,
//!   `\p{sc=..}`), the `java*` properties and the Unicode binary properties not listed above,
//!   surrogate code units on their own, and the flags `UNIX_LINES` (`(?d)`), `COMMENTS`
//!   (`(?x)`), `UNICODE_CHARACTER_CLASS` (`(?U)`), `CANON_EQ` (`(?c)`), and `UNICODE_CASE`
//!   (`(?u)`) together with `CASE_INSENSITIVE`.
//! - Anchors whose Java meaning depends on line terminators near the input's end are exact
//!   unless the input has such a terminator; [`Pattern`] then fails loudly at match time:
//!   `$` and `\Z` (Java also before a final line terminator) on an input ending in `\n`, `\r`,
//!   U+0085, U+2028 or U+2029; `(?m)^` and `(?m)$` (Java knows five terminators and never
//!   matches `^` at the end of the input) on an input containing `\r`, U+0085, U+2028, U+2029,
//!   or (for `^`) an empty input or one ending in `\n`; `\b` and `\B` (Java counts a
//!   non-spacing mark after a letter or digit as a word char) on an input containing a
//!   non-spacing mark. The bare [`compile`] has no such check.
//!
//! Matching: both engines prefer the leftmost match and, among alternatives and quantifier
//! choices, the first one in Java's backtracking order, so `find()` gives the same span; the
//! crate compiles `\A(?:p)\z` for `matches()`. The input is matched by chars (Java: by code
//! points; a supplementary char is one char in both). Remaining PORT points: a syntax error
//! found by the crate after translation (its size limits, e.g. a repetition count over its
//! limit, which Java accepts) carries the crate's text; a lookbehind whose length Java cannot
//! bound (a syntax error in Java) is `unported!` like any lookbehind.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

// Java: java/util/regex/Pattern.java Pattern.UNIX_LINES and the other flag constants.
/// `Pattern.UNIX_LINES` (`(?d)`).
pub const UNIX_LINES: i32 = 0x01;
/// `Pattern.CASE_INSENSITIVE` (`(?i)`).
pub const CASE_INSENSITIVE: i32 = 0x02;
/// `Pattern.COMMENTS` (`(?x)`).
pub const COMMENTS: i32 = 0x04;
/// `Pattern.MULTILINE` (`(?m)`).
pub const MULTILINE: i32 = 0x08;
/// `Pattern.LITERAL`.
pub const LITERAL: i32 = 0x10;
/// `Pattern.DOTALL` (`(?s)`).
pub const DOTALL: i32 = 0x20;
/// `Pattern.UNICODE_CASE` (`(?u)`).
pub const UNICODE_CASE: i32 = 0x40;
/// `Pattern.CANON_EQ` (`(?c)`).
pub const CANON_EQ: i32 = 0x80;
/// `Pattern.UNICODE_CHARACTER_CLASS` (`(?U)`).
pub const UNICODE_CHARACTER_CLASS: i32 = 0x100;

/// `java.util.regex.PatternSyntaxException`.
// Java: java/util/regex/PatternSyntaxException.java
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternSyntaxException {
    desc: String,
    pattern: String,
    index: i32,
}

impl PatternSyntaxException {
    /// `getDescription()`.
    pub fn get_description(&self) -> &str {
        &self.desc
    }

    /// `getIndex()`.
    pub fn get_index(&self) -> i32 {
        self.index
    }

    /// `getPattern()`.
    pub fn get_pattern(&self) -> &str {
        &self.pattern
    }

    // Java: java/util/regex/PatternSyntaxException.java:102 PatternSyntaxException.getMessage
    /// `getMessage()`: the description, the index, the pattern and a caret under the index
    /// (`System.lineSeparator()` is `\n`). Lengths and positions count UTF-16 units, as Java.
    pub fn get_message(&self) -> String {
        let mut sb = String::new();
        sb.push_str(&self.desc);
        if self.index >= 0 {
            sb.push_str(" near index ");
            sb.push_str(&self.index.to_string());
        }
        sb.push('\n');
        sb.push_str(&self.pattern);
        let units: Vec<u16> = self.pattern.encode_utf16().collect();
        if self.index >= 0 && (self.index as usize) < units.len() {
            sb.push('\n');
            for &unit in &units[..self.index as usize] {
                sb.push(if unit == u16::from(b'\t') { '\t' } else { ' ' });
            }
            sb.push('^');
        }
        sb
    }
}

impl fmt::Display for PatternSyntaxException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl std::error::Error for PatternSyntaxException {}

/// Translates `Pattern.compile(pattern, flags)` into the `regex` crate's syntax, for
/// `Matcher.find()` (the regex as is) or `matches()` (wrap it as `\A(?:..)\z`, see
/// [`Pattern`]). A Java syntax error is `Err(getMessage())`; a valid pattern that uses a
/// construct listed as `unported!` in the module docs panics.
pub fn translate(pattern: &str, flags: i32) -> Result<String, String> {
    match Translation::new(pattern, flags) {
        Ok(t) => t.emit(Mode::Find).map_err(|what| unported(pattern, &what)),
        Err(Fail::Syntax(e)) => Err(e.get_message()),
        Err(Fail::Unported(what)) => unported(pattern, &what),
    }
}

/// `Pattern.compile(pattern, flags)` as a `regex::Regex` for `Matcher.find()`.
///
/// It has none of [`Pattern`]'s checks at match time (the anchors and word boundaries listed in
/// the module docs): use [`Pattern`] unless the caller needs a `regex::Regex`.
pub fn compile(pattern: &str, flags: i32) -> Result<Regex, String> {
    let translated = translate(pattern, flags)?;
    Regex::new(&translated).map_err(|e| e.to_string())
}

fn unported(pattern: &str, what: &str) -> ! {
    crate::unported!(format!("java.util.regex.Pattern: {what} in '{pattern}'"))
}

/// A compiled `java.util.regex.Pattern`: `Matcher.find()` and `Matcher.matches()` on a fresh
/// matcher over the whole input.
///
/// Equality: `Pattern` does not override `equals`; callers compare [`Pattern::pattern`].
#[derive(Clone)]
pub struct Pattern {
    pattern: String,
    flags: i32,
    find: Regex,
    /// `Err` holds the construct that is `unported!` under `matches()` (an atomic group or a
    /// possessive quantifier that ends the pattern).
    matches: Result<Regex, String>,
    guards: Guards,
}

impl Pattern {
    // Java: java/util/regex/Pattern.java Pattern.compile(String, int)
    /// `Pattern.compile(pattern, flags)`; the error is `PatternSyntaxException.getMessage()`
    /// (or the `regex` crate's text, see the module docs).
    pub fn compile(pattern: &str, flags: i32) -> Result<Pattern, String> {
        let translation = match Translation::new(pattern, flags) {
            Ok(t) => t,
            Err(Fail::Syntax(e)) => return Err(e.get_message()),
            Err(Fail::Unported(what)) => unported(pattern, &what),
        };
        let find_source = translation
            .emit(Mode::Find)
            .unwrap_or_else(|what| unported(pattern, &what));
        let find = Regex::new(&find_source).map_err(|e| e.to_string())?;
        let matches = match translation.emit(Mode::Matches) {
            Ok(source) => Ok(Regex::new(&format!(r"\A(?:{source})\z")).map_err(|e| e.to_string())?),
            Err(what) => Err(what),
        };
        Ok(Pattern {
            pattern: pattern.to_string(),
            flags,
            find,
            matches,
            guards: translation.guards,
        })
    }

    /// `pattern()`.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Same as [`Pattern::pattern`] (for code written against `regex::Regex`).
    pub fn as_str(&self) -> &str {
        &self.pattern
    }

    /// `flags()` as given to [`Pattern::compile`].
    pub fn flags(&self) -> i32 {
        self.flags
    }

    /// The translated pattern for `find()`, without the checks of [`Pattern::find`].
    pub fn regex(&self) -> &Regex {
        &self.find
    }

    /// `matcher(input).find()`: the `start()` and `end()` of the first match, as byte offsets
    /// into `input`.
    pub fn find(&self, input: &str) -> Option<(usize, usize)> {
        self.guards.check(&self.pattern, input);
        self.find.find(input).map(|m| (m.start(), m.end()))
    }

    /// `matcher(input).find()`.
    pub fn is_match(&self, input: &str) -> bool {
        self.guards.check(&self.pattern, input);
        self.find.is_match(input)
    }

    /// `matcher(input).matches()`.
    pub fn matches(&self, input: &str) -> bool {
        self.guards.check(&self.pattern, input);
        match &self.matches {
            Ok(regex) => regex.is_match(input),
            Err(what) => unported(&self.pattern, &format!("{what} under Matcher.matches()")),
        }
    }
}

impl fmt::Debug for Pattern {
    /// `Pattern.toString()` (the pattern).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.pattern)
    }
}

impl fmt::Display for Pattern {
    /// `Pattern.toString()` (the pattern).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.pattern)
    }
}

/// The anchors whose translation is exact only on inputs without certain line terminators.
#[derive(Clone, Copy, Default, Debug)]
struct Guards {
    /// `$` or `\Z` without `MULTILINE`.
    dollar_end: bool,
    /// `$` with `MULTILINE`.
    multiline_dollar: bool,
    /// `^` with `MULTILINE`.
    multiline_caret: bool,
    /// `\b` or `\B`.
    bound: bool,
}

/// Java's line terminators other than `\n` (`Pattern.Dollar`, `Caret`).
fn is_other_line_terminator(c: char) -> bool {
    matches!(c, '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

static NON_SPACING_MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{Mn}").unwrap());

impl Guards {
    fn check(&self, pattern: &str, input: &str) {
        let last = input.chars().last();
        if self.dollar_end && last.is_some_and(|c| c == '\n' || is_other_line_terminator(c)) {
            unported(
                pattern,
                "'$' or '\\Z' (Java also matches before a final line terminator) on an input \
                 ending in a line terminator",
            );
        }
        if (self.multiline_dollar || self.multiline_caret)
            && input.chars().any(is_other_line_terminator)
        {
            unported(
                pattern,
                "MULTILINE '^' or '$' on an input with a line terminator other than '\\n'",
            );
        }
        if self.multiline_caret && (input.is_empty() || last == Some('\n')) {
            unported(
                pattern,
                "MULTILINE '^' (Java never matches it at the end of the input) on an input that \
                 is empty or ends in '\\n'",
            );
        }
        if self.bound && NON_SPACING_MARK.is_match(input) {
            unported(
                pattern,
                "'\\b' or '\\B' (Java counts a non-spacing mark after a letter or digit as a \
                 word char) on an input with a non-spacing mark",
            );
        }
    }
}

/// What the translated pattern is used for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// `Matcher.find()`: the end of the pattern is the end of the match.
    Find,
    /// `Matcher.matches()`: the end of the input must follow the pattern.
    Matches,
}

enum Fail {
    Syntax(PatternSyntaxException),
    /// A construct that changes how the rest of the pattern is read (so parsing stops).
    Unported(String),
}

/// `Pattern.Qtype`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Qtype {
    Greedy,
    Lazy,
    Possessive,
}

/// The parsed pattern. Leaves are already in the crate's syntax.
enum Node {
    /// Crate syntax for one item (a char, a slice of chars, a class, an anchor); `Raw("")` is
    /// the empty slice.
    Raw(String),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Group {
        kind: GroupKind,
        body: Box<Node>,
    },
    Repeat {
        body: Box<Node>,
        min: i32,
        /// `None`: `MAX_REPS` (unbounded).
        max: Option<i32>,
        qtype: Qtype,
    },
}

enum GroupKind {
    Capturing,
    Named(String),
    NonCapturing,
    Atomic,
}

/// A parsed pattern, ready to be written for either [`Mode`].
struct Translation {
    root: Node,
    /// The first construct the parser found that is `unported!` in every mode.
    unported: Option<String>,
    guards: Guards,
}

impl Translation {
    fn new(pattern: &str, flags: i32) -> Result<Translation, Fail> {
        let mut parser = Parser::new(pattern, flags);
        let root = parser.compile()?;
        Ok(Translation {
            root,
            unported: parser.unported,
            guards: parser.guards,
        })
    }

    /// The crate's syntax, or the `unported!` construct.
    fn emit(&self, mode: Mode) -> Result<String, String> {
        if let Some(what) = &self.unported {
            return Err(what.clone());
        }
        let mut out = String::new();
        emit(&self.root, mode == Mode::Find, &mut out)?;
        Ok(out)
    }
}

/// Writes `node`. `tail`: nothing can follow `node` in a match (it ends the pattern through
/// the ends of unquantified groups and alternatives, and the pattern is used with `find()`).
fn emit(node: &Node, tail: bool, out: &mut String) -> Result<(), String> {
    match node {
        Node::Raw(s) => out.push_str(s),
        Node::Concat(items) => {
            for (i, item) in items.iter().enumerate() {
                emit(item, tail && i + 1 == items.len(), out)?;
            }
        }
        Node::Alt(alternatives) => {
            out.push_str("(?:");
            for (i, alternative) in alternatives.iter().enumerate() {
                if i > 0 {
                    out.push('|');
                }
                emit(alternative, tail, out)?;
            }
            out.push(')');
        }
        Node::Group { kind, body } => {
            match kind {
                GroupKind::Capturing => out.push('('),
                GroupKind::Named(name) => {
                    out.push_str("(?P<");
                    out.push_str(name);
                    out.push('>');
                }
                GroupKind::NonCapturing => out.push_str("(?:"),
                GroupKind::Atomic => {
                    if !tail {
                        return Err("atomic group (?>...) followed by more of the pattern".into());
                    }
                    // The first way the group matches ends the match, as in Java.
                    out.push_str("(?:");
                }
            }
            emit(body, tail, out)?;
            out.push(')');
        }
        Node::Repeat {
            body,
            min,
            max,
            qtype,
        } => {
            if *qtype == Qtype::Possessive && !tail {
                return Err("possessive quantifier followed by more of the pattern".into());
            }
            out.push_str("(?:");
            emit(body, false, out)?;
            out.push(')');
            match max {
                None => out.push_str(&format!("{{{min},}}")),
                Some(max) => out.push_str(&format!("{{{min},{max}}}")),
            }
            if *qtype == Qtype::Lazy {
                out.push('?');
            }
        }
    }
    Ok(())
}

/// Java's `\w` (`[a-zA-Z_0-9]`) as a crate class.
const ASCII_WORD: &str = "[0-9A-Za-z_]";
/// Java's `\W`.
const ASCII_NON_WORD: &str = "[^0-9A-Za-z_]";
/// Java's `\d`.
const ASCII_DIGIT: &str = "[0-9]";
const ASCII_NON_DIGIT: &str = "[^0-9]";
/// Java's `\s` (`[ \t\n\x0B\f\r]`).
const ASCII_SPACE: &str = r"[\x20\t\n\x0B\x0C\r]";
const ASCII_NON_SPACE: &str = r"[^\x20\t\n\x0B\x0C\r]";
/// `Pattern.HorizWS` (`\h`).
const HORIZ_WS: &str = r"[\x20\t\xA0\x{1680}\x{180E}\x{2000}-\x{200A}\x{202F}\x{205F}\x{3000}]";
const NON_HORIZ_WS: &str =
    r"[^\x20\t\xA0\x{1680}\x{180E}\x{2000}-\x{200A}\x{202F}\x{205F}\x{3000}]";
/// `Pattern.VertWS` (`\v`).
const VERT_WS: &str = r"[\n\x0B\x0C\r\x{85}\x{2028}\x{2029}]";
const NON_VERT_WS: &str = r"[^\n\x0B\x0C\r\x{85}\x{2028}\x{2029}]";
/// `Pattern.DOT()`: `.` without `DOTALL`.
const DOT: &str = r"[^\n\r\x{85}\x{2028}\x{2029}]";
/// `Pattern.ALL()`.
const ALL: &str = r"(?s:.)";
/// `Pattern.LineEnding` (`\R`): `\r\n` first, then one terminator (Java backtracks from
/// `\r\n` to `\r`).
const LINE_ENDING: &str = r"(?:\r\n|[\n\x0B\x0C\r\x{85}\x{2028}\x{2029}])";

/// A code point in the crate's syntax.
fn hex(c: u32) -> String {
    format!(r"\x{{{c:X}}}")
}

fn is_ascii_alpha(c: u32) -> bool {
    (u32::from(b'A')..=u32::from(b'Z')).contains(&c)
        || (u32::from(b'a')..=u32::from(b'z')).contains(&c)
}

fn is_ascii_digit(c: u32) -> bool {
    (u32::from(b'0')..=u32::from(b'9')).contains(&c)
}

fn is_ascii_hex_digit(c: u32) -> bool {
    char::from_u32(c).is_some_and(|c| c.is_ascii_hexdigit())
}

/// `ASCII.toLower`.
fn ascii_to_lower(c: u32) -> u32 {
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&c) {
        c + 0x20
    } else {
        c
    }
}

/// `ASCII.toUpper`.
fn ascii_to_upper(c: u32) -> u32 {
    if (u32::from(b'a')..=u32::from(b'z')).contains(&c) {
        c - 0x20
    } else {
        c
    }
}

fn is_surrogate(c: u32) -> bool {
    (0xD800..=0xDFFF).contains(&c)
}

/// `Pattern.BitClass`: the chars below 256 of a class.
#[derive(Default)]
struct BitClass {
    bits: Vec<u32>,
}

impl BitClass {
    fn render(&self) -> String {
        let mut s = String::from("[");
        for &c in &self.bits {
            s.push_str(&hex(c));
        }
        s.push(']');
        s
    }
}

/// `CharPredicate.union`.
fn union(prev: Option<String>, curr: String) -> String {
    match prev {
        None => curr,
        Some(prev) => format!("[{prev}{curr}]"),
    }
}

/// What `Pattern.escape` returns.
enum Esc {
    /// A char (`escape` returns it).
    Char(u32),
    /// A predicate (`escape` returns -1 with `predicate` set): a crate class.
    Pred(&'static str),
    /// A node (`escape` returns -1 with `root` set).
    Node(Node),
}

/// The parser of `Pattern` (the fields Java uses while compiling).
struct Parser {
    /// `normalizedPattern`.
    normalized_pattern: String,
    /// `temp`: the code points, then two zeros.
    temp: Vec<u32>,
    /// `patternLength`, in code points.
    pattern_length: usize,
    cursor: usize,
    flags0: i32,
    /// `capturingGroupCount` (group 0 counts).
    capturing_group_count: i32,
    /// `namedGroups`.
    named_groups: HashMap<String, i32>,
    unported: Option<String>,
    guards: Guards,
}

const ZERO: u32 = 0;

fn ch(c: char) -> u32 {
    c as u32
}

impl Parser {
    fn new(pattern: &str, flags: i32) -> Parser {
        Parser {
            normalized_pattern: pattern.to_string(),
            temp: Vec::new(),
            pattern_length: 0,
            cursor: 0,
            flags0: flags,
            capturing_group_count: 1,
            named_groups: HashMap::new(),
            unported: None,
            guards: Guards::default(),
        }
    }

    fn has(&self, f: i32) -> bool {
        self.flags0 & f != 0
    }

    fn mark_unported(&mut self, what: impl Into<String>) {
        if self.unported.is_none() {
            self.unported = Some(what.into());
        }
    }

    fn at(&self, i: usize) -> u32 {
        self.temp.get(i).copied().unwrap_or(ZERO)
    }

    // Java: java/util/regex/Pattern.java:2050 Pattern.accept
    fn accept(&mut self, c: u32, s: &str) -> Result<(), Fail> {
        let test_char = self.at(self.cursor);
        self.cursor += 1;
        if c != test_char {
            return Err(self.error(s));
        }
        Ok(())
    }

    // Java: java/util/regex/Pattern.java:2062 Pattern.mark
    fn mark(&mut self, c: u32) {
        let i = self.pattern_length;
        self.temp[i] = c;
    }

    // Java: java/util/regex/Pattern.java:2069 Pattern.peek
    fn peek(&self) -> u32 {
        self.at(self.cursor)
    }

    // Java: java/util/regex/Pattern.java:2079 Pattern.read
    fn read(&mut self) -> u32 {
        let c = self.at(self.cursor);
        self.cursor += 1;
        c
    }

    // Java: java/util/regex/Pattern.java:2098 Pattern.next
    fn next(&mut self) -> u32 {
        self.cursor += 1;
        self.at(self.cursor)
    }

    // Java: java/util/regex/Pattern.java:2186 Pattern.skip
    fn skip(&mut self) -> u32 {
        let i = self.cursor;
        let c = self.at(i + 1);
        self.cursor = i + 2;
        c
    }

    // Java: java/util/regex/Pattern.java:2196 Pattern.unread
    fn unread(&mut self) {
        self.cursor -= 1;
    }

    // Java: java/util/regex/Pattern.java:2204 Pattern.error
    fn error(&self, s: &str) -> Fail {
        Fail::Syntax(PatternSyntaxException {
            desc: s.to_string(),
            pattern: self.normalized_pattern.clone(),
            index: self.cursor as i32 - 1,
        })
    }

    // Java: java/util/regex/Pattern.java:1905 Pattern.compile
    fn compile(&mut self) -> Result<Node, Fail> {
        if self.has(CANON_EQ) && !self.has(LITERAL) {
            return Err(Fail::Unported("CANON_EQ".into()));
        }
        if self.has(COMMENTS) {
            return Err(Fail::Unported("COMMENTS ((?x))".into()));
        }
        self.temp = self.normalized_pattern.chars().map(|c| c as u32).collect();
        self.pattern_length = self.temp.len();
        self.temp.push(ZERO);
        self.temp.push(ZERO);
        if !self.has(LITERAL) {
            self.remove_qe_quoting();
        }
        if self.has(LITERAL) {
            let chars: Vec<u32> = self.temp[..self.pattern_length].to_vec();
            return Ok(Node::Raw(self.new_slice(&chars)));
        }
        let root = self.expr()?;
        if self.pattern_length != self.cursor {
            if self.peek() == ch(')') {
                return Err(self.error("Unmatched closing ')'"));
            } else if self.cursor == self.pattern_length + 1
                && self.temp[self.pattern_length - 1] == ch('\\')
            {
                return Err(self.error("Unescaped trailing backslash"));
            } else {
                return Err(self.error("Unexpected internal error"));
            }
        }
        Ok(root)
    }

    // Java: java/util/regex/Pattern.java:1828 Pattern.RemoveQEQuoting
    fn remove_qe_quoting(&mut self) {
        let p_len = self.pattern_length;
        let temp = &self.temp;
        let mut i = 0;
        while i + 1 < p_len {
            if temp[i] != ch('\\') {
                i += 1;
            } else if temp[i + 1] != ch('Q') {
                i += 2;
            } else {
                break;
            }
        }
        if i + 1 >= p_len {
            // No \Q sequence found
            return;
        }
        let mut newtemp: Vec<u32> = temp[..i].to_vec();
        i += 2;
        let mut in_quote = true;
        let mut begin_quote = true;
        while i < p_len {
            let c = temp[i];
            i += 1;
            if c >= 0x80 || is_ascii_alpha(c) {
                newtemp.push(c);
            } else if is_ascii_digit(c) {
                if begin_quote {
                    // A unicode escape \[0xu] could be before this quote, and we don't want this
                    // numeric char to processed as part of the escape.
                    newtemp.extend([ch('\\'), ch('x'), ch('3')]);
                }
                newtemp.push(c);
            } else if c != ch('\\') {
                if in_quote {
                    newtemp.push(ch('\\'));
                }
                newtemp.push(c);
            } else if in_quote {
                if temp[i] == ch('E') {
                    i += 1;
                    in_quote = false;
                } else {
                    newtemp.push(ch('\\'));
                    newtemp.push(ch('\\'));
                }
            } else if temp[i] == ch('Q') {
                i += 1;
                in_quote = true;
                begin_quote = true;
                continue;
            } else {
                newtemp.push(c);
                if i != p_len {
                    newtemp.push(temp[i]);
                    i += 1;
                }
            }
            begin_quote = false;
        }
        self.pattern_length = newtemp.len();
        newtemp.push(ZERO);
        newtemp.push(ZERO);
        self.temp = newtemp;
    }

    // Java: java/util/regex/Pattern.java:2239 Pattern.expr
    fn expr(&mut self) -> Result<Node, Fail> {
        let mut alternatives = Vec::new();
        loop {
            alternatives.push(self.sequence()?);
            if self.peek() != ch('|') {
                break;
            }
            self.next();
        }
        Ok(if alternatives.len() == 1 {
            alternatives.pop().unwrap()
        } else {
            Node::Alt(alternatives)
        })
    }

    // Java: java/util/regex/Pattern.java:2290 Pattern.sequence
    fn sequence(&mut self) -> Result<Node, Fail> {
        let mut items = Vec::new();
        loop {
            let c = self.peek();
            let node = match char::from_u32(c).unwrap_or('\0') {
                '(' => {
                    // Because group handles its own closure, we need to treat it differently.
                    if let Some(node) = self.group0()? {
                        items.push(node);
                    }
                    continue;
                }
                '[' => Node::Raw(self.clazz(true)?),
                '\\' => {
                    let c = self.next();
                    if c == ch('p') || c == ch('P') {
                        let comp = c == ch('P');
                        let mut one_letter = true;
                        // Consume { if present
                        if self.next() != ch('{') {
                            self.unread();
                        } else {
                            one_letter = false;
                        }
                        Node::Raw(self.family(one_letter, comp)?)
                    } else {
                        self.unread();
                        self.atom()?
                    }
                }
                '^' => {
                    self.next();
                    if self.has(UNIX_LINES) {
                        self.mark_unported("UNIX_LINES ((?d)) '^'");
                    }
                    if self.has(MULTILINE) {
                        self.guards.multiline_caret = true;
                        Node::Raw("(?m:^)".into())
                    } else {
                        Node::Raw(r"\A".into())
                    }
                }
                '$' => {
                    self.next();
                    Node::Raw(self.dollar(self.has(MULTILINE)))
                }
                '.' => {
                    self.next();
                    if self.has(DOTALL) {
                        Node::Raw(ALL.into())
                    } else {
                        if self.has(UNIX_LINES) {
                            self.mark_unported("UNIX_LINES ((?d)) '.'");
                        }
                        Node::Raw(DOT.into())
                    }
                }
                '|' | ')' => break,
                // Now interpreting dangling ] and } as literals
                ']' | '}' => self.atom()?,
                '?' | '*' | '+' => {
                    self.next();
                    return Err(self.error(&format!(
                        "Dangling meta character '{}'",
                        char::from_u32(c).unwrap()
                    )));
                }
                '\0' if c == ZERO && self.cursor >= self.pattern_length => break,
                _ => self.atom()?,
            };
            let node = self.closure(node)?;
            items.push(node);
        }
        Ok(Node::Concat(items))
    }

    /// `new Dollar(multiline)` (or `UnixDollar`).
    fn dollar(&mut self, multiline: bool) -> String {
        if self.has(UNIX_LINES) {
            self.mark_unported("UNIX_LINES ((?d)) '$'");
        }
        if multiline {
            self.guards.multiline_dollar = true;
            "(?m:$)".into()
        } else {
            self.guards.dollar_end = true;
            r"\z".into()
        }
    }

    // Java: java/util/regex/Pattern.java:2417 Pattern.atom
    fn atom(&mut self) -> Result<Node, Fail> {
        let mut buffer: Vec<u32> = Vec::new();
        let mut prev: usize = 0;
        let mut c = self.peek();
        loop {
            match char::from_u32(c).unwrap_or('\u{FFFF}') {
                '*' | '+' | '?' | '{' => {
                    if buffer.len() > 1 {
                        // Unwind one character
                        self.cursor = prev;
                        buffer.pop();
                    }
                    break;
                }
                '$' | '.' | '^' | '(' | '[' | '|' | ')' => break,
                '\\' => {
                    c = self.next();
                    if c == ch('p') || c == ch('P') {
                        // Property
                        if !buffer.is_empty() {
                            // Slice is waiting; handle it first
                            self.unread();
                            break;
                        }
                        // No slice; just return the family node
                        let comp = c == ch('P');
                        let mut one_letter = true;
                        if self.next() != ch('{') {
                            self.unread();
                        } else {
                            one_letter = false;
                        }
                        return Ok(Node::Raw(self.family(one_letter, comp)?));
                    }
                    self.unread();
                    prev = self.cursor;
                    match self.escape(false, buffer.is_empty(), false)? {
                        Esc::Char(e) => {
                            buffer.push(e);
                            c = self.peek();
                            continue;
                        }
                        Esc::Pred(p) if buffer.is_empty() => return Ok(Node::Raw(p.into())),
                        Esc::Node(node) if buffer.is_empty() => return Ok(node),
                        _ => {
                            // Unwind meta escape sequence
                            self.cursor = prev;
                            break;
                        }
                    }
                }
                '\0' if c == ZERO && self.cursor >= self.pattern_length => break,
                _ => {
                    prev = self.cursor;
                    buffer.push(c);
                    c = self.next();
                    continue;
                }
            }
        }
        if buffer.len() == 1 {
            Ok(Node::Raw(self.single_node(buffer[0])))
        } else {
            Ok(Node::Raw(self.new_slice(&buffer)))
        }
    }

    /// `newCharProperty(single(ch))` outside a class.
    fn single_node(&mut self, c: u32) -> String {
        self.literal(c)
    }

    /// `newSlice(buf, count, hasSupplementary)`: `SliceI` folds ASCII letters only.
    fn new_slice(&mut self, chars: &[u32]) -> String {
        let mut s = String::new();
        for &c in chars {
            s.push_str(&self.literal(c));
        }
        s
    }

    /// One literal char with the current flags (`Single`, `SingleI`, a char of a `Slice`).
    fn literal(&mut self, c: u32) -> String {
        if is_surrogate(c) || char::from_u32(c).is_none() {
            self.mark_unported(format!("surrogate code unit U+{c:04X} on its own"));
            return String::new();
        }
        if self.has(CASE_INSENSITIVE) {
            if self.has(UNICODE_CASE) {
                self.mark_unported("UNICODE_CASE ((?u)) with CASE_INSENSITIVE");
                return String::new();
            }
            let lower = ascii_to_lower(c);
            let upper = ascii_to_upper(c);
            if c < 0x80 && lower != upper {
                return format!("[{}{}]", hex(lower), hex(upper));
            }
        }
        hex(c)
    }

    // Java: java/util/regex/Pattern.java:2519 Pattern.ref
    fn backref(&mut self, mut ref_num: i32) -> Node {
        loop {
            let c = self.peek();
            if !is_ascii_digit(c) {
                break;
            }
            let new_ref_num = ref_num * 10 + (c - ch('0')) as i32;
            // Add another number if it doesn't make a group that doesn't exist
            if self.capturing_group_count - 1 < new_ref_num {
                break;
            }
            ref_num = new_ref_num;
            self.read();
        }
        self.mark_unported(format!("backreference \\{ref_num}"));
        Node::Raw(String::new())
    }

    // Java: java/util/regex/Pattern.java:2553 Pattern.escape
    fn escape(&mut self, inclass: bool, create: bool, isrange: bool) -> Result<Esc, Fail> {
        let c = self.skip();
        let illegal = |p: &Parser| Err(p.error("Illegal/unsupported escape sequence"));
        let e = match char::from_u32(c) {
            Some(e) if e.is_ascii_alphanumeric() => e,
            _ => return Ok(Esc::Char(c)),
        };
        match e {
            '0' => return Ok(Esc::Char(self.o()?)),
            '1'..='9' => {
                if inclass {
                    return illegal(self);
                }
                if create {
                    return Ok(Esc::Node(self.backref((c - ch('0')) as i32)));
                }
                return Ok(Esc::Node(Node::Raw(String::new())));
            }
            'A' => {
                if inclass {
                    return illegal(self);
                }
                return Ok(Esc::Node(Node::Raw(r"\A".into())));
            }
            'B' => {
                if inclass {
                    return illegal(self);
                }
                if create {
                    self.guards.bound = true;
                }
                return Ok(Esc::Node(Node::Raw(r"(?-u:\B)".into())));
            }
            'D' => return Ok(Esc::Pred(ASCII_NON_DIGIT)),
            'G' => {
                if inclass {
                    return illegal(self);
                }
                if create {
                    self.mark_unported("\\G");
                }
                return Ok(Esc::Node(Node::Raw(String::new())));
            }
            'H' => return Ok(Esc::Pred(NON_HORIZ_WS)),
            'N' => {
                self.n()?;
                return Ok(Esc::Char(0));
            }
            'R' => {
                if inclass {
                    return illegal(self);
                }
                return Ok(Esc::Node(Node::Raw(LINE_ENDING.into())));
            }
            'S' => return Ok(Esc::Pred(ASCII_NON_SPACE)),
            'V' => return Ok(Esc::Pred(NON_VERT_WS)),
            'W' => return Ok(Esc::Pred(ASCII_NON_WORD)),
            'X' => {
                if inclass {
                    return illegal(self);
                }
                if create {
                    self.mark_unported("\\X (grapheme cluster)");
                }
                return Ok(Esc::Node(Node::Raw(String::new())));
            }
            'Z' => {
                if inclass {
                    return illegal(self);
                }
                if create {
                    return Ok(Esc::Node(Node::Raw(self.dollar(false))));
                }
                return Ok(Esc::Node(Node::Raw(String::new())));
            }
            'a' => return Ok(Esc::Char(0x07)),
            'b' => {
                if inclass {
                    return illegal(self);
                }
                if create {
                    if self.peek() == ch('{') {
                        if self.skip() == ch('g') {
                            if self.read() == ch('}') {
                                self.mark_unported("\\b{g} (grapheme boundary)");
                                return Ok(Esc::Node(Node::Raw(String::new())));
                            }
                            // error missing trailing }
                            return illegal(self);
                        }
                        self.unread();
                        self.unread();
                    }
                    self.guards.bound = true;
                }
                return Ok(Esc::Node(Node::Raw(r"(?-u:\b)".into())));
            }
            'c' => return Ok(Esc::Char(self.c()?)),
            'd' => return Ok(Esc::Pred(ASCII_DIGIT)),
            'e' => return Ok(Esc::Char(0x1B)),
            'f' => return Ok(Esc::Char(0x0C)),
            'h' => return Ok(Esc::Pred(HORIZ_WS)),
            'k' => {
                if inclass {
                    return illegal(self);
                }
                if self.read() != ch('<') {
                    return Err(self.error("\\k is not followed by '<' for named capturing group"));
                }
                let first = self.read();
                let name = self.groupname(first)?;
                if !self.named_groups.contains_key(&name) {
                    return Err(
                        self.error(&format!("named capturing group <{name}> does not exist"))
                    );
                }
                if create {
                    self.mark_unported(format!("backreference \\k<{name}>"));
                }
                return Ok(Esc::Node(Node::Raw(String::new())));
            }
            'n' => return Ok(Esc::Char(ch('\n'))),
            'r' => return Ok(Esc::Char(ch('\r'))),
            's' => return Ok(Esc::Pred(ASCII_SPACE)),
            't' => return Ok(Esc::Char(ch('\t'))),
            'u' => return Ok(Esc::Char(self.u()?)),
            'v' => {
                // '\v' was implemented as VT/0x0B in releases < 1.8; 0x0B is returned if isrange.
                if isrange {
                    return Ok(Esc::Char(0x0B));
                }
                return Ok(Esc::Pred(VERT_WS));
            }
            'w' => return Ok(Esc::Pred(ASCII_WORD)),
            'x' => return Ok(Esc::Char(self.x()?)),
            'z' => {
                if inclass {
                    return illegal(self);
                }
                return Ok(Esc::Node(Node::Raw(r"\z".into())));
            }
            // C E F I J K L M O P Q T U Y g i j l m o p q y
            _ => {}
        }
        illegal(self)
    }

    // Java: java/util/regex/Pattern.java:2796 Pattern.clazz
    /// A character class as a crate class (`[...]`).
    fn clazz(&mut self, consume: bool) -> Result<String, Fail> {
        let mut prev: Option<String> = None;
        let mut curr: Option<String> = None;
        let mut bits = BitClass::default();
        // Java shares `bits` by reference: chars added after it became part of `prev` change
        // `prev` too. That shape is not ported.
        let mut bits_captured = false;
        let mut is_neg = false;
        let mut has_bits = false;
        let mut c = self.next();
        // Negates if first char in a class, otherwise literal
        if c == ch('^') && self.temp[self.cursor - 1] == ch('[') {
            c = self.next();
            is_neg = true;
        }
        loop {
            if c == ch('[') {
                let nested = self.clazz(true)?;
                curr = Some(nested.clone());
                prev = Some(union(prev, nested));
                c = self.peek();
                continue;
            } else if c == ch('&') {
                c = self.next();
                if c == ch('&') {
                    c = self.next();
                    let mut right: Option<String> = None;
                    while c != ch(']') && c != ch('&') {
                        let r = if c == ch('[') {
                            self.clazz(true)?
                        } else {
                            // abc&&def
                            self.unread();
                            self.clazz(false)?
                        };
                        right = Some(union(right, r));
                        c = self.peek();
                    }
                    if has_bits {
                        // bits used, union has high precedence
                        let rendered = bits.render();
                        if prev.is_none() {
                            prev = Some(rendered.clone());
                            curr = Some(rendered);
                        } else {
                            prev = Some(union(prev, rendered));
                        }
                        has_bits = false;
                        bits_captured = true;
                    }
                    if right.is_some() {
                        curr = right.clone();
                    }
                    match prev.take() {
                        None => match right {
                            None => return Err(self.error("Bad class syntax")),
                            Some(right) => prev = Some(right),
                        },
                        Some(p) => match &curr {
                            None => return Err(self.error("Bad intersection syntax")),
                            Some(cu) => prev = Some(format!("[{p}&&{cu}]")),
                        },
                    }
                    continue;
                } else {
                    // treat as a literal &
                    self.unread();
                }
            } else if c == ZERO {
                if self.cursor >= self.pattern_length {
                    return Err(self.error("Unclosed character class"));
                }
            } else if c == ch(']') && (prev.is_some() || has_bits) {
                if consume {
                    self.next();
                }
                let p = match prev {
                    None => bits.render(),
                    Some(p) if has_bits => union(Some(p), bits.render()),
                    Some(p) => p,
                };
                return Ok(if is_neg { format!("[^{p}]") } else { p });
            }
            match self.range(&mut bits)? {
                None => {
                    // the bits used
                    has_bits = true;
                    if bits_captured {
                        self.mark_unported(
                            "character class with members after an intersection's left operand",
                        );
                    }
                }
                Some(r) => {
                    curr = Some(r.clone());
                    prev = Some(union(prev, r));
                }
            }
            c = self.peek();
        }
    }

    // Java: java/util/regex/Pattern.java:2901 Pattern.bitsOrSingle
    fn bits_or_single(&mut self, bits: &mut BitClass, c: u32) -> Option<String> {
        if c < 256
            && !(self.has(CASE_INSENSITIVE)
                && self.has(UNICODE_CASE)
                && [0xff, 0xb5, 0x49, 0x69, 0x53, 0x73, 0x4b, 0x6b, 0xc5, 0xe5].contains(&c))
        {
            // BitClass.add(c, flags0)
            if self.has(CASE_INSENSITIVE) {
                if c < 0x80 {
                    bits.bits.push(ascii_to_upper(c));
                    bits.bits.push(ascii_to_lower(c));
                } else if self.has(UNICODE_CASE) {
                    self.mark_unported("UNICODE_CASE ((?u)) with CASE_INSENSITIVE");
                }
            }
            bits.bits.push(c);
            return None;
        }
        Some(format!("[{}]", self.single_in_class(c)))
    }

    // Java: java/util/regex/Pattern.java:2935 Pattern.single
    /// The inside of a crate class for `single(ch)`.
    fn single_in_class(&mut self, c: u32) -> String {
        if is_surrogate(c) || char::from_u32(c).is_none() {
            self.mark_unported(format!("surrogate code unit U+{c:04X} on its own"));
            return hex(0);
        }
        if self.has(CASE_INSENSITIVE) {
            if self.has(UNICODE_CASE) {
                self.mark_unported("UNICODE_CASE ((?u)) with CASE_INSENSITIVE");
            } else if c < 0x80 {
                let lower = ascii_to_lower(c);
                let upper = ascii_to_upper(c);
                if lower != upper {
                    return format!("{}{}", hex(lower), hex(upper));
                }
            }
        }
        hex(c)
    }

    // Java: java/util/regex/Pattern.java:2961 Pattern.range
    fn range(&mut self, bits: &mut BitClass) -> Result<Option<String>, Fail> {
        let mut c = self.peek();
        if c == ch('\\') {
            c = self.next();
            if c == ch('p') || c == ch('P') {
                // A property
                let comp = c == ch('P');
                let mut one_letter = true;
                // Consume { if present
                if self.next() != ch('{') {
                    self.unread();
                } else {
                    one_letter = false;
                }
                return Ok(Some(self.family(one_letter, comp)?));
            }
            // ordinary escape
            let isrange = self.at(self.cursor + 1) == ch('-');
            self.unread();
            match self.escape(true, true, isrange)? {
                Esc::Char(e) => c = e,
                Esc::Pred(p) => return Ok(Some(p.into())),
                Esc::Node(_) => unreachable!("escape in a class returns a char or a predicate"),
            }
        } else {
            self.next();
        }
        if self.peek() == ch('-') {
            let end_range = self.at(self.cursor + 1);
            if end_range == ch('[') {
                return Ok(self.bits_or_single(bits, c));
            }
            if end_range != ch(']') {
                self.next();
                let m = self.peek();
                let m: i64 = if m == ch('\\') {
                    match self.escape(true, false, true)? {
                        Esc::Char(e) => i64::from(e),
                        _ => -1,
                    }
                } else {
                    self.next();
                    i64::from(m)
                };
                if m < i64::from(c) {
                    return Err(self.error("Illegal character range"));
                }
                let m = m as u32;
                if is_surrogate(c) || is_surrogate(m) {
                    self.mark_unported("surrogate code unit in a class range");
                }
                let mut s = format!("[{}-{}", hex(c), hex(m));
                if self.has(CASE_INSENSITIVE) {
                    if self.has(UNICODE_CASE) {
                        self.mark_unported("UNICODE_CASE ((?u)) with CASE_INSENSITIVE");
                    }
                    // CIRange: an ASCII input char also matches when its other case is in range.
                    for letter in (ch('A')..=ch('Z')).chain(ch('a')..=ch('z')) {
                        let other = if letter < ch('a') {
                            ascii_to_lower(letter)
                        } else {
                            ascii_to_upper(letter)
                        };
                        if (c..=m).contains(&other) && !(c..=m).contains(&letter) {
                            s.push_str(&hex(letter));
                        }
                    }
                }
                s.push(']');
                return Ok(Some(s));
            }
        }
        Ok(self.bits_or_single(bits, c))
    }

    // Java: java/util/regex/Pattern.java:3019 Pattern.family
    fn family(&mut self, single_letter: bool, is_complement: bool) -> Result<String, Fail> {
        self.next();
        let name: String;
        if single_letter {
            let c = self.temp[self.cursor];
            name = char::from_u32(c).map(String::from).unwrap_or_default();
            self.read();
        } else {
            let i = self.cursor;
            self.mark(ch('}'));
            while self.read() != ch('}') {}
            self.mark(ZERO);
            let j = self.cursor;
            if j > self.pattern_length {
                return Err(self.error("Unclosed character family"));
            }
            if i + 1 >= j {
                return Err(self.error("Empty character family"));
            }
            name = self.temp[i..j - 1]
                .iter()
                .filter_map(|&c| char::from_u32(c))
                .collect();
        }
        let ci = self.has(CASE_INSENSITIVE);
        let p: Option<String> = if let Some(i) = name.find('=') {
            // property construct \p{name=value}
            let value = &name[i + 1..];
            let key = name[..i].to_ascii_lowercase();
            match key.as_str() {
                "sc" | "script" => {
                    self.mark_unported(format!("Unicode script \\p{{{name}}}"));
                    Some(hex(0))
                }
                "blk" | "block" => {
                    self.mark_unported(format!("Unicode block \\p{{{name}}}"));
                    Some(hex(0))
                }
                "gc" | "general_category" => self.for_property(value, ci),
                _ => None,
            }
            .map(Ok)
            .unwrap_or_else(|| {
                Err(self.error(&format!(
                    "Unknown Unicode property {{name=<{key}>, value=<{value}>}}"
                )))
            })
            .map(Some)?
        } else if let Some(block) = name.strip_prefix("In") {
            self.mark_unported(format!("Unicode block \\p{{In{block}}}"));
            Some(hex(0))
        } else if let Some(short_name) = name.strip_prefix("Is") {
            // \p{IsGeneralCategory} and \p{IsScriptName}
            let upper = short_name.to_uppercase();
            match self.for_unicode_property(&upper, ci) {
                Some(p) => Some(p),
                None => match self.for_property(short_name, ci) {
                    Some(p) => Some(p),
                    None => {
                        self.mark_unported(format!(
                            "Unicode script (or unknown property) \\p{{{name}}}"
                        ));
                        Some(hex(0))
                    }
                },
            }
        } else {
            match self.for_property(&name, ci) {
                Some(p) => Some(p),
                None => {
                    return Err(self.error(&format!("Unknown character property name {{{name}}}")));
                }
            }
        };
        let p = p.unwrap();
        Ok(if is_complement { format!("[^{p}]") } else { p })
    }

    // Java: java/util/regex/CharPredicates.java:225 CharPredicates.forUnicodeProperty
    fn for_unicode_property(&mut self, name: &str, ci: bool) -> Option<String> {
        // getUnicodePredicate
        let unicode = match name {
            "ALPHABETIC" => Some(r"\p{Alphabetic}"),
            "LOWERCASE" if !ci => Some(r"\p{Lowercase}"),
            "UPPERCASE" if !ci => Some(r"\p{Uppercase}"),
            "WHITESPACE" | "WHITE_SPACE" => Some(r"\p{White_Space}"),
            "ASSIGNED"
            | "CONTROL"
            | "EMOJI"
            | "EMOJI_PRESENTATION"
            | "EMOJI_MODIFIER"
            | "EMOJI_MODIFIER_BASE"
            | "EMOJI_COMPONENT"
            | "EXTENDED_PICTOGRAPHIC"
            | "HEXDIGIT"
            | "HEX_DIGIT"
            | "IDEOGRAPHIC"
            | "JOINCONTROL"
            | "JOIN_CONTROL"
            | "LETTER"
            | "LOWERCASE"
            | "NONCHARACTERCODEPOINT"
            | "NONCHARACTER_CODE_POINT"
            | "TITLECASE"
            | "PUNCTUATION"
            | "UPPERCASE"
            | "WORD" => {
                self.mark_unported(format!("Unicode property \\p{{Is{name}}}"));
                Some("")
            }
            _ => None,
        };
        if let Some(p) = unicode {
            return Some(if p.is_empty() {
                hex(0)
            } else {
                format!("[{p}]")
            });
        }
        // getPosixPredicate: the Unicode variants of the POSIX classes.
        match name {
            "ALPHA" | "LOWER" | "UPPER" | "SPACE" | "PUNCT" | "XDIGIT" | "ALNUM" | "CNTRL"
            | "DIGIT" | "BLANK" | "GRAPH" | "PRINT" => {
                self.mark_unported(format!("Unicode POSIX property \\p{{Is{name}}}"));
                Some(hex(0))
            }
            _ => None,
        }
    }

    // Java: java/util/regex/CharPredicates.java:268 CharPredicates.forProperty
    fn for_property(&mut self, name: &str, ci: bool) -> Option<String> {
        const CASED: &str = r"[\p{Lu}\p{Ll}\p{Lt}]";
        let p: &str = match name {
            "Lu" | "Ll" | "Lt" if ci => CASED,
            "Cn" | "Lu" | "Ll" | "Lt" | "Lm" | "Lo" | "Mn" | "Me" | "Mc" | "Nd" | "Nl" | "No"
            | "Zs" | "Zl" | "Zp" | "Cc" | "Cf" | "Co" | "Cs" | "Pd" | "Ps" | "Pe" | "Pc" | "Po"
            | "Sm" | "Sc" | "Sk" | "So" | "Pi" | "Pf" | "L" | "M" | "N" | "Z" | "C" | "P" | "S" => {
                return Some(format!(r"[\p{{{name}}}]"));
            }
            "LC" => CASED,
            "LD" => r"[\p{L}\p{Nd}]",
            "L1" => r"[\x00-\xFF]",
            "all" => r"[\x{0}-\x{10FFFF}]",
            "ASCII" => r"[\x00-\x7F]",
            "Alnum" => "[0-9A-Za-z]",
            "Alpha" => "[A-Za-z]",
            "Blank" => r"[\x20\t]",
            "Cntrl" => r"[\x00-\x1F\x7F]",
            "Digit" => "[0-9]",
            "Graph" => r"[\x21-\x7E]",
            "Lower" => {
                if ci {
                    "[A-Za-z]"
                } else {
                    "[a-z]"
                }
            }
            "Print" => r"[\x20-\x7E]",
            "Punct" => r"[\x21-\x2F\x3A-\x40\x5B-\x60\x7B-\x7E]",
            "Space" => r"[\x20\t\n\x0B\x0C\r]",
            "Upper" => {
                if ci {
                    "[A-Za-z]"
                } else {
                    "[A-Z]"
                }
            }
            "XDigit" => "[0-9A-Fa-f]",
            "javaLowerCase"
            | "javaUpperCase"
            | "javaAlphabetic"
            | "javaIdeographic"
            | "javaTitleCase"
            | "javaDigit"
            | "javaDefined"
            | "javaLetter"
            | "javaLetterOrDigit"
            | "javaJavaIdentifierStart"
            | "javaJavaIdentifierPart"
            | "javaUnicodeIdentifierStart"
            | "javaUnicodeIdentifierPart"
            | "javaIdentifierIgnorable"
            | "javaSpaceChar"
            | "javaWhitespace"
            | "javaISOControl"
            | "javaMirrored" => {
                self.mark_unported(format!("Java character property \\p{{{name}}}"));
                return Some(hex(0));
            }
            _ => return None,
        };
        Some(p.to_string())
    }

    // Java: java/util/regex/Pattern.java (Pattern.groupname)
    fn groupname(&mut self, mut c: u32) -> Result<String, Fail> {
        let mut sb = String::new();
        if !is_ascii_alpha(c) {
            return Err(self.error("capturing group name does not start with a Latin letter"));
        }
        loop {
            sb.push(char::from_u32(c).unwrap());
            c = self.read();
            if !(is_ascii_alpha(c) || is_ascii_digit(c)) {
                break;
            }
        }
        if c != ch('>') {
            return Err(self.error("named capturing group is missing trailing '>'"));
        }
        Ok(sb)
    }

    // Java: java/util/regex/Pattern.java:3134 Pattern.group0
    /// A group with its quantifier, or `None` for a flag group `(?i)`.
    fn group0(&mut self) -> Result<Option<Node>, Fail> {
        let save = self.flags0;
        let kind;
        let body;
        let mut c = self.next();
        if c == ch('?') {
            c = self.skip();
            match char::from_u32(c).unwrap_or('\0') {
                ':' => {
                    // (?:xxx) pure group
                    body = self.expr()?;
                    kind = GroupKind::NonCapturing;
                }
                '=' | '!' => {
                    // (?=xxx) and (?!xxx) lookahead
                    body = self.expr()?;
                    self.mark_unported("lookahead");
                    kind = GroupKind::NonCapturing;
                }
                '>' => {
                    // (?>xxx) independent group
                    body = self.expr()?;
                    kind = GroupKind::Atomic;
                }
                '<' => {
                    // (?<xxx) look behind
                    c = self.read();
                    if c != ch('=') && c != ch('!') {
                        // named captured group
                        let name = self.groupname(c)?;
                        if self.named_groups.contains_key(&name) {
                            return Err(self.error(&format!(
                                "Named capturing group <{name}> is already defined"
                            )));
                        }
                        self.capturing_group_count += 1;
                        self.named_groups
                            .insert(name.clone(), self.capturing_group_count - 1);
                        body = self.expr()?;
                        kind = GroupKind::Named(name);
                    } else {
                        body = self.expr()?;
                        self.mark_unported("lookbehind");
                        kind = GroupKind::NonCapturing;
                    }
                }
                '$' | '@' => return Err(self.error("Unknown group type")),
                _ => {
                    // (?xxx:) inlined match flags
                    self.unread();
                    self.add_flag();
                    if self.has(COMMENTS) {
                        return Err(Fail::Unported("COMMENTS ((?x))".into()));
                    }
                    if self.has(CANON_EQ) {
                        self.mark_unported("CANON_EQ ((?c))");
                    }
                    if self.has(UNICODE_CHARACTER_CLASS) {
                        self.mark_unported("UNICODE_CHARACTER_CLASS ((?U))");
                    }
                    c = self.read();
                    if c == ch(')') {
                        // Inline modifier only
                        return Ok(None);
                    }
                    if c != ch(':') {
                        return Err(self.error("Unknown inline modifier"));
                    }
                    body = self.expr()?;
                    kind = GroupKind::NonCapturing;
                }
            }
        } else {
            // (xxx) a regular group
            self.capturing_group_count += 1;
            body = self.expr()?;
            kind = GroupKind::Capturing;
        }
        self.accept(ch(')'), "Unclosed group")?;
        self.flags0 = save;
        // Check for quantifiers
        let node = Node::Group {
            kind,
            body: Box::new(body),
        };
        Ok(Some(self.closure(node)?))
    }

    // Java: java/util/regex/Pattern.java:3330 Pattern.addFlag
    fn add_flag(&mut self) {
        let mut c = self.peek();
        loop {
            match char::from_u32(c).unwrap_or('\0') {
                'i' => self.flags0 |= CASE_INSENSITIVE,
                'm' => self.flags0 |= MULTILINE,
                's' => self.flags0 |= DOTALL,
                'd' => self.flags0 |= UNIX_LINES,
                'u' => self.flags0 |= UNICODE_CASE,
                'c' => self.flags0 |= CANON_EQ,
                'x' => self.flags0 |= COMMENTS,
                'U' => self.flags0 |= UNICODE_CHARACTER_CLASS | UNICODE_CASE,
                '-' => {
                    // subFlag then fall through
                    self.next();
                    self.sub_flag();
                    return;
                }
                _ => return,
            }
            c = self.next();
        }
    }

    // Java: java/util/regex/Pattern.java:3372 Pattern.subFlag
    fn sub_flag(&mut self) {
        let mut c = self.peek();
        loop {
            match char::from_u32(c).unwrap_or('\0') {
                'i' => self.flags0 &= !CASE_INSENSITIVE,
                'm' => self.flags0 &= !MULTILINE,
                's' => self.flags0 &= !DOTALL,
                'd' => self.flags0 &= !UNIX_LINES,
                'u' => self.flags0 &= !UNICODE_CASE,
                'c' => self.flags0 &= !CANON_EQ,
                'x' => self.flags0 &= !COMMENTS,
                'U' => self.flags0 &= !(UNICODE_CHARACTER_CLASS | UNICODE_CASE),
                _ => return,
            }
            c = self.next();
        }
    }

    // Java: java/util/regex/Pattern.java Pattern.qtype
    fn qtype(&mut self) -> Qtype {
        let c = self.next();
        if c == ch('?') {
            self.next();
            Qtype::Lazy
        } else if c == ch('+') {
            self.next();
            Qtype::Possessive
        } else {
            Qtype::Greedy
        }
    }

    fn repeat(prev: Node, min: i32, max: Option<i32>, qtype: Qtype) -> Node {
        Node::Repeat {
            body: Box::new(prev),
            min,
            max,
            qtype,
        }
    }

    // Java: java/util/regex/Pattern.java:3442 Pattern.closure
    fn closure(&mut self, prev: Node) -> Result<Node, Fail> {
        let c = self.peek();
        match char::from_u32(c).unwrap_or('\0') {
            '?' => {
                let q = self.qtype();
                Ok(Self::repeat(prev, 0, Some(1), q))
            }
            '*' => {
                let q = self.qtype();
                Ok(Self::repeat(prev, 0, None, q))
            }
            '+' => {
                let q = self.qtype();
                Ok(Self::repeat(prev, 1, None, q))
            }
            '{' => {
                let mut c = self.skip();
                if !is_ascii_digit(c) {
                    return Err(self.error("Illegal repetition"));
                }
                let mut cmin: i32 = 0;
                let cmax: i32;
                let overflow = |p: &Parser| Err(p.error("Illegal repetition range"));
                loop {
                    cmin = match cmin
                        .checked_mul(10)
                        .and_then(|v| v.checked_add((c - ch('0')) as i32))
                    {
                        Some(v) => v,
                        None => return overflow(self),
                    };
                    c = self.read();
                    if !is_ascii_digit(c) {
                        break;
                    }
                }
                if c == ch(',') {
                    c = self.read();
                    if c == ch('}') {
                        self.unread();
                        let q = self.qtype();
                        return Ok(Self::repeat(prev, cmin, None, q));
                    }
                    let mut m: i32 = 0;
                    while is_ascii_digit(c) {
                        m = match m
                            .checked_mul(10)
                            .and_then(|v| v.checked_add((c - ch('0')) as i32))
                        {
                            Some(v) => v,
                            None => return overflow(self),
                        };
                        c = self.read();
                    }
                    cmax = m;
                } else {
                    cmax = cmin;
                }
                if c != ch('}') {
                    return Err(self.error("Unclosed counted closure"));
                }
                if cmax < cmin {
                    return Err(self.error("Illegal repetition range"));
                }
                self.unread();
                let q = self.qtype();
                Ok(Self::repeat(prev, cmin, Some(cmax), q))
            }
            _ => Ok(prev),
        }
    }

    // Java: java/util/regex/Pattern.java:3498 Pattern.c
    fn c(&mut self) -> Result<u32, Fail> {
        if self.cursor < self.pattern_length {
            return Ok(self.read() ^ 64);
        }
        Err(self.error("Illegal control escape sequence"))
    }

    // Java: java/util/regex/Pattern.java:3508 Pattern.o
    fn o(&mut self) -> Result<u32, Fail> {
        let is_octal = |c: u32| (ch('0')..=ch('7')).contains(&c);
        let n = self.read();
        if is_octal(n) {
            let m = self.read();
            if is_octal(m) {
                let o = self.read();
                if is_octal(o) && (ch('0')..=ch('3')).contains(&n) {
                    return Ok((n - ch('0')) * 64 + (m - ch('0')) * 8 + (o - ch('0')));
                }
                self.unread();
                return Ok((n - ch('0')) * 8 + (m - ch('0')));
            }
            self.unread();
            return Ok(n - ch('0'));
        }
        Err(self.error("Illegal octal escape sequence"))
    }

    // Java: java/util/regex/Pattern.java:3529 Pattern.x
    fn x(&mut self) -> Result<u32, Fail> {
        let mut n = self.read();
        if is_ascii_hex_digit(n) {
            let m = self.read();
            if is_ascii_hex_digit(m) {
                return Ok(char::from_u32(n).unwrap().to_digit(16).unwrap() * 16
                    + char::from_u32(m).unwrap().to_digit(16).unwrap());
            }
        } else if n == ch('{') && is_ascii_hex_digit(self.peek()) {
            let mut c: u32 = 0;
            loop {
                n = self.read();
                if !is_ascii_hex_digit(n) {
                    break;
                }
                c = (c << 4) + char::from_u32(n).unwrap().to_digit(16).unwrap();
                if c > 0x10FFFF {
                    return Err(self.error("Hexadecimal codepoint is too big"));
                }
            }
            if n != ch('}') {
                return Err(self.error("Unclosed hexadecimal escape sequence"));
            }
            return Ok(c);
        }
        Err(self.error("Illegal hexadecimal escape sequence"))
    }

    // Java: java/util/regex/Pattern.java:3561 Pattern.uxxxx
    fn uxxxx(&mut self) -> Result<u32, Fail> {
        let mut n: u32 = 0;
        for _ in 0..4 {
            let c = self.read();
            if !is_ascii_hex_digit(c) {
                return Err(self.error("Illegal Unicode escape sequence"));
            }
            n = n * 16 + char::from_u32(c).unwrap().to_digit(16).unwrap();
        }
        Ok(n)
    }

    // Java: java/util/regex/Pattern.java:3573 Pattern.u
    fn u(&mut self) -> Result<u32, Fail> {
        let n = self.uxxxx()?;
        if (0xD800..=0xDBFF).contains(&n) {
            let cur = self.cursor;
            if self.read() == ch('\\') && self.read() == ch('u') {
                let n2 = self.uxxxx()?;
                if (0xDC00..=0xDFFF).contains(&n2) {
                    return Ok(0x10000 + ((n - 0xD800) << 10) + (n2 - 0xDC00));
                }
            }
            self.cursor = cur;
        }
        Ok(n)
    }

    // Java: java/util/regex/Pattern.java:3587 Pattern.N
    fn n(&mut self) -> Result<(), Fail> {
        if self.read() == ch('{') {
            let i = self.cursor;
            while self.read() != ch('}') {
                if self.cursor >= self.pattern_length {
                    return Err(self.error("Unclosed character name escape sequence"));
                }
            }
            let name: String = self.temp[i..self.cursor - 1]
                .iter()
                .filter_map(|&c| char::from_u32(c))
                .collect();
            // Character.codePointOf(name): the Unicode name table is not ported.
            self.mark_unported(format!("\\N{{{name}}}"));
            return Ok(());
        }
        Err(self.error("Illegal character name escape sequence"))
    }
}
