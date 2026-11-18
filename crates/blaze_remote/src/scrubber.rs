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
/// PORT: the patterns are compiled by the `regex` crate, whose dialect differs from
/// `java.util.regex`:
/// - no backreferences, lookahead / lookbehind, possessive quantifiers, atomic groups or
///   `\Q...\E` quoting (such a pattern fails to compile and is a `PatternSyntaxException`);
/// - `.` excludes only `\n` (Java also excludes `\r`, `\u0085`, ` `, ` `) and `$`
///   matches only at the very end (Java also before a final line terminator);
/// - `\d`, `\w`, `\s`, `\b` are Unicode-aware (Java: ASCII unless `(?U)`);
/// - Java matches the UTF-16 code units of Bazel's "internal" strings (Latin-1 chars holding
///   UTF-8 bytes); here the pattern runs over the Unicode scalar values of the UTF-8 string.
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
        let find = Regex::new(regex).map_err(syntax_error)?;
        let matches = Regex::new(&format!(r"\A(?:{regex})\z")).map_err(syntax_error)?;
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
