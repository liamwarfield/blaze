//! `IncludeParser`: scans a source file for `#include` lines (and `#include_next`, `#import`,
//! `__has_include`) and the include hints that help it with computed includes.

use std::fmt;
use std::io::Read;

use blaze_actions::Artifact;
use blaze_exec::analysis::actions::get_input_path;
use blaze_exec::exec::action_execution_context::ActionExecutionContext;
use blaze_util::profiler::{Profiler, ProfilerTask};
use blaze_vfs::PathFragment;

use crate::cpp::include_scanner::IncludeScanningError;

/// File types supported by the grep-includes binary.
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:90 IncludeParser.GrepIncludesFileType
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrepIncludesFileType {
    Cpp,
    Swig,
}

impl GrepIncludesFileType {
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:100 GrepIncludesFileType.getFileType
    pub fn get_file_type(&self) -> &'static str {
        match self {
            GrepIncludesFileType::Cpp => "c++",
            GrepIncludesFileType::Swig => "swig",
        }
    }
}

/// The type of a hints [`Rule`].
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:112 Rule.Type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleType {
    Path,
    File,
    IncludeQuote,
    IncludeAngle,
}

/// Immutable object representation of the four columns making up a single Rule in a Hints set.
///
/// PORT: skipped, Bazel's `IncludeScanningModule.getIncludeHintsFilename()` returns null, so no INCLUDE_HINTS file is parsed: Hints.getRules, Rule.toString
///
/// PORT: no rule is ever created: Bazel's `IncludeScanningModule.getIncludeHintsFilename()`
/// returns null, so the hints are always [`HintsRules::empty`] (`Hints.getRules`, which parses an
/// INCLUDE_HINTS file, and `IncludeHintsFunction` are not ported). The fields are kept so that
/// the loops over the rules read as in Java.
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:111 IncludeParser.Rule
#[derive(Clone, Debug)]
pub struct Rule {
    rule_type: RuleType,
}

/// `SkyValue` encapsulating the source-state-dependent part of [`Hints`].
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:144 IncludeParser.HintsRules
#[derive(Clone, Debug, Default)]
pub struct HintsRules {
    rules: Vec<Rule>,
}

impl HintsRules {
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:145 HintsRules.EMPTY
    pub fn empty() -> HintsRules {
        HintsRules { rules: Vec::new() }
    }
}

/// This class is a representation of the INCLUDE_HINTS file. The hints file contains
/// regexp-based rules to help this simple include scanner cope with computed includes, which
/// would otherwise require a full preprocessor with symbol support.
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:182 IncludeParser.Hints
#[derive(Debug)]
pub struct Hints {
    rules: Vec<Rule>,
}

impl Hints {
    /// `ALLOWED_PREFIX`.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:184 Hints.ALLOWED_PREFIX
    pub const ALLOWED_PREFIX: &'static str = "third_party/";

    /// Constructs a hint set for a given INCLUDE_HINTS file to read.
    ///
    /// PORT: `syscallCache` and `artifactFactory` are read only when a rule matches.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:205 Hints.<init>
    pub fn new(hints_rules: HintsRules) -> Hints {
        Hints {
            rules: hints_rules.rules,
        }
    }

    /// Returns the "file" type hinted inclusions for a given path, caching results by path.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:248 Hints.getFileLevelHintedInclusionsLegacy
    pub fn get_file_level_hinted_inclusions_legacy(&self, path: &Artifact) -> Vec<Artifact> {
        if !path
            .get_exec_path_string()
            .starts_with(Self::ALLOWED_PREFIX)
        {
            return Vec::new();
        }
        // fileLevelHintsCache.get(path) -> getHintedInclusionsLegacy(path)
        self.get_hinted_inclusions_legacy(path)
    }

    /// Returns the "path" type hinted inclusions for the given paths.
    ///
    /// PORT: the Skyframe lookups (`ContainingPackageLookupValue`, `GlobValue`) happen only for a
    /// matching PATH rule; without rules no key is requested.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:262 Hints.getPathLevelHintedInclusions
    pub fn get_path_level_hinted_inclusions(&self, paths: &[PathFragment]) -> Vec<Artifact> {
        let path_strings: Vec<&str> = paths
            .iter()
            .map(|p| p.get_path_string())
            .filter(|p| p.starts_with(Self::ALLOWED_PREFIX))
            .collect();
        if path_strings.is_empty() {
            return Vec::new();
        }
        for rule in &self.rules {
            if rule.rule_type != RuleType::Path {
                continue;
            }
            blaze_util::unported!(
                "IncludeParser.Hints.getPathLevelHintedInclusions",
                "include hints rules (Bazel never reads an INCLUDE_HINTS file)"
            );
        }
        // env.getValuesAndExceptions(rulePaths) with no keys; hints == null.
        Vec::new()
    }

    /// Performs the work of matching a given path against the hints and returns the expanded
    /// paths.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:396 Hints.getHintedInclusionsLegacy
    fn get_hinted_inclusions_legacy(&self, _artifact: &Artifact) -> Vec<Artifact> {
        for rule in &self.rules {
            if rule.rule_type != RuleType::File {
                continue;
            }
            blaze_util::unported!(
                "IncludeParser.Hints.getHintedInclusionsLegacy",
                "include hints rules (Bazel never reads an INCLUDE_HINTS file)"
            );
        }
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:454 Hints.getHintedInclusions
    fn get_hinted_inclusions(&self, _path: &Artifact) -> Vec<Inclusion> {
        for rule in &self.rules {
            if rule.rule_type != RuleType::IncludeAngle && rule.rule_type != RuleType::IncludeQuote
            {
                continue;
            }
            blaze_util::unported!(
                "IncludeParser.Hints.getHintedInclusions",
                "include hints rules (Bazel never reads an INCLUDE_HINTS file)"
            );
        }
        Vec::new()
    }
}

/// The format of the #include in the source file -- quoted, angle bracket, etc.
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:496 Inclusion.Kind
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Quote includes: `#include "name"`.
    Quote,
    /// Angle bracket includes: `#include <name>`.
    Angle,
    /// Quote next includes: `#include_next "name"`.
    NextQuote,
    /// Angle next includes: `#include_next <name>`.
    NextAngle,
}

impl Kind {
    /// Returns true if this is an `#include_next` inclusion.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:510 Kind.isNext
    pub fn is_next(&self) -> bool {
        matches!(self, Kind::NextAngle | Kind::NextQuote)
    }
}

impl fmt::Display for Kind {
    /// The Java enum constant's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Quote => "QUOTE",
            Kind::Angle => "ANGLE",
            Kind::NextQuote => "NEXT_QUOTE",
            Kind::NextAngle => "NEXT_ANGLE",
        })
    }
}

/// An immutable inclusion tuple. This models an `#include` or `#include_next` line in a file
/// without the context how this file got included.
///
/// PORT: Java interns the instances (`BlazeInterners.newWeakInterner`); equality is by value
/// either way.
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:492 IncludeParser.Inclusion
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:543 Inclusion.equals
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:554 Inclusion.hashCode
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Inclusion {
    /// The kind of inclusion.
    pub kind: Kind,
    /// The relative path of the inclusion.
    pub path_fragment: PathFragment,
}

impl Inclusion {
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:525 Inclusion.create
    pub fn create(include_target: &str, kind: Kind) -> Inclusion {
        Inclusion {
            kind,
            path_fragment: PathFragment::create(include_target),
        }
    }

    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:529 Inclusion.create
    pub fn create_from_fragment(path_fragment: PathFragment, kind: Kind) -> Inclusion {
        Inclusion {
            kind,
            path_fragment,
        }
    }

    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:533 Inclusion.getPathString
    pub fn get_path_string(&self) -> &str {
        self.path_fragment.get_path_string()
    }
}

impl fmt::Display for Inclusion {
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:538 Inclusion.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind, self.path_fragment.get_path_string())
    }
}

/// `Character.isWhitespace(c)` for a code point below 256: `\t`, `\n`, `\u000B`, `\f`, `\r`,
/// `\u001C`..`\u001F` and the space separator `' '` (U+00A0 is a non-breaking space and U+0085
/// a control character, so neither counts).
fn is_java_whitespace(c: u8) -> bool {
    matches!(c, 0x09..=0x0D | 0x1C..=0x1F | 0x20)
}

/// Skips whitespace, \+NL pairs, and block-style /* */ comments. Assumes line comments are
/// handled outside. Does not handle digraphs, trigraphs or decahexagraphs.
///
/// Returns the resulting position after skipping whitespace and comments.
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:579 IncludeParser.skipWhitespace
pub(crate) fn skip_whitespace(chars: &[u8], mut pos: usize, end: usize) -> usize {
    while pos < end {
        if is_java_whitespace(chars[pos]) {
            pos += 1;
        } else if chars[pos] == b'\\' && pos + 1 < end && chars[pos + 1] == b'\n' {
            pos += 1;
        } else if chars[pos] == b'/' && pos + 1 < end && chars[pos + 1] == b'*' {
            pos += 2;
            while pos + 1 < end {
                let c = chars[pos];
                pos += 1;
                if c == b'*' && chars[pos] == b'/' {
                    pos += 1;
                    break; // proper comment end
                }
            }
        } else {
            // not whitespace
            return pos;
        }
    }
    pos // pos == len, meaning we fell off the end.
}

// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:602 IncludeParser.HAS_INCLUDE
const HAS_INCLUDE: &[u8] = b"__has_include";
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:604 IncludeParser.NECESSARY_HAS_INCLUDE_LENGTH
const NECESSARY_HAS_INCLUDE_LENGTH: usize = HAS_INCLUDE.len() + 5;

/// Returns the index of `chars` after the first occurrence of "__has_include" or -1 if no such
/// occurrence exists. Also requires that there be at least 5 characters after the
/// "__has_include", corresponding to a pair of parentheses and angle brackets/quotes and a
/// filename.
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:614 IncludeParser.skipThroughHasInclude
fn skip_through_has_include(chars: &[u8], mut pos: i64, end: usize) -> i64 {
    let last_pos = end as i64 - NECESSARY_HAS_INCLUDE_LENGTH as i64;
    while pos <= last_pos {
        let mut cur_pos = 0usize;
        while cur_pos < HAS_INCLUDE.len() && chars[pos as usize + cur_pos] == HAS_INCLUDE[cur_pos] {
            cur_pos += 1;
        }
        if cur_pos == HAS_INCLUDE.len() {
            return pos + cur_pos as i64;
        }
        // We're looking for "__has_include" as a preprocessing token, which means that it cannot
        // start in the middle of any characters we've already processed, nor at the mismatching
        // character.
        pos += cur_pos as i64 + 1;
    }
    -1
}

/// Checks for and skips a given token. Returns the resulting position if found, otherwise -1.
///
/// Like Java, a token that ends exactly at `end` is not found (the loop ends before the
/// position after the token is returned).
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:641 IncludeParser.expect
pub(crate) fn expect(chars: &[u8], mut pos: usize, end: usize, expected: &[u8]) -> i64 {
    let mut si = 0usize;
    while pos < end {
        if si == expected.len() {
            return pos as i64;
        }
        let c = chars[pos];
        pos += 1;
        let e = expected[si];
        si += 1;
        if c != e {
            return -1;
        }
    }
    -1
}

/// Finds the index of a given character token from a starting pos; -1 if not found.
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:663 IncludeParser.indexOf
fn index_of(chars: &[u8], mut pos: usize, end: usize, echar: u8) -> i64 {
    while pos < end {
        if chars[pos] == echar {
            return pos as i64;
        }
        pos += 1;
    }
    -1
}

/// Position of found include together with information about how to process the remaining
/// include line further.
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:933 IncludeParser.IncludesKeywordData
#[derive(Clone, Copy, Debug)]
struct IncludesKeywordData {
    pos: i64,
    can_have_next: bool,
    has_parens: bool,
}

impl IncludesKeywordData {
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:934 IncludesKeywordData.NONE
    const NONE: IncludesKeywordData = IncludesKeywordData {
        pos: -1,
        can_have_next: false,
        has_parens: false,
    };

    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:945 IncludesKeywordData.normal
    fn normal(pos: i64) -> IncludesKeywordData {
        IncludesKeywordData {
            pos,
            can_have_next: true,
            has_parens: false,
        }
    }

    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:949 IncludesKeywordData.importOrSwig
    fn import_or_swig(pos: i64) -> IncludesKeywordData {
        IncludesKeywordData {
            pos,
            can_have_next: false,
            has_parens: false,
        }
    }

    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:953 IncludesKeywordData.hasInclude
    fn has_include(pos: i64) -> IncludesKeywordData {
        IncludesKeywordData {
            pos,
            can_have_next: true,
            has_parens: true,
        }
    }
}

/// Scans a source file for include directives.
///
/// PORT: `SwigIncludeParser` (the subclass for `.swig` files, used by `SwigIncludeScanner`) is
/// not ported: nothing in Bazel's rules creates a swig include scanning action.
// Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:84 IncludeParser
#[derive(Debug)]
pub struct IncludeParser {
    /// The externally-scoped immutable hints helper that is shared by all scanners.
    hints: Option<Hints>,
}

impl IncludeParser {
    /// Constructs a new FileParser.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:567 IncludeParser.<init>
    pub fn new(hints: Option<Hints>) -> IncludeParser {
        IncludeParser { hints }
    }

    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:484 IncludeParser.getHints
    pub fn get_hints(&self) -> Option<&Hints> {
        self.hints.as_ref()
    }

    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:742 IncludeParser.extractInclusion
    pub fn extract_inclusion_from_line(&self, line: &str) -> Option<Inclusion> {
        let bytes = blaze_util::string_encoding::latin1_encode(line);
        self.extract_inclusion(&bytes, 0, bytes.len())
    }

    /// Extracts a new, unresolved Inclusion from a line of source.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:755 IncludeParser.extractInclusion
    fn extract_inclusion(
        &self,
        chars: &[u8],
        line_begin: usize,
        line_end: usize,
    ) -> Option<Inclusion> {
        // expect WS#WS(include|include_next|__has_include\(_next\)?)WS\(?("name"|<name>|<name>)\)?
        let data = self.expect_include_keyword(chars, line_begin, line_end);
        let mut pos = data.pos;
        if pos == -1 || pos == line_end as i64 {
            return None;
        }
        let mut is_next = false;
        if data.can_have_next {
            let npos = expect(chars, pos as usize, line_end, b"_next");
            if npos >= 0 {
                is_next = true;
                pos = npos;
            }
        }
        let mut pos = skip_whitespace(chars, pos as usize, line_end);
        if pos == line_end {
            return None;
        }
        if data.has_parens {
            if chars[pos] != b'(' {
                return None;
            }
            pos += 1;
            pos = skip_whitespace(chars, pos, line_end);
            if pos == line_end {
                return None;
            }
        }
        if chars[pos] == b'"' || chars[pos] == b'<' {
            let qchar = chars[pos];
            pos += 1;
            let spos = pos;
            let close = index_of(
                chars,
                pos + 1,
                line_end,
                if qchar == b'<' { b'>' } else { b'"' },
            );
            if close < 0 {
                return None;
            }
            let pos = close as usize;
            if chars[spos] == b'/' {
                return None; // disallow absolute paths
            }
            // new String(chars, spos, pos - spos): the server's default charset is ISO-8859-1.
            let mut name =
                blaze_util::string_encoding::latin1_decode(&chars[spos..pos]).into_owned();
            if name.contains('\n') {
                // strip any \+NL pairs within name
                name = name.replace("\\\n", "");
            }
            if is_next {
                Some(Inclusion::create(
                    &name,
                    if qchar == b'"' {
                        Kind::NextQuote
                    } else {
                        Kind::NextAngle
                    },
                ))
            } else {
                Some(Inclusion::create(
                    &name,
                    if qchar == b'"' {
                        Kind::Quote
                    } else {
                        Kind::Angle
                    },
                ))
            }
        } else {
            self.create_other_inclusion(&chars[pos..line_end])
        }
    }

    /// Extracts all inclusions from characters of a file.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:813 IncludeParser.extractInclusions
    pub fn extract_inclusions(&self, chars: &[u8]) -> Vec<Inclusion> {
        let mut inclusions = Vec::new();
        let mut line_begin = 0usize; // the first char of each line
        let end = chars.len(); // the file end
        while line_begin < end {
            let mut line_end = line_begin; // the char after the last non-\n in each line
            // skip to the next \n or after end of buffer, ignoring continuations
            while line_end < end {
                if chars[line_end] == b'\n' {
                    break;
                } else if chars[line_end] == b'\\' {
                    line_end += 1;
                    // Java reads chars[lineEnd] unchecked: a file that ends in a backslash
                    // throws ArrayIndexOutOfBoundsException.
                    let Some(&c) = chars.get(line_end) else {
                        panic!(
                            "ArrayIndexOutOfBoundsException: Index {line_end} out of bounds for \
                             length {end}"
                        );
                    };
                    if c == b'\n' {
                        line_end += 1;
                    }
                } else {
                    line_end += 1;
                }
            }

            // TODO(bazel-team) handle multiline block comments /* */.

            // extract the inclusion, and save only the kind we care about.
            if let Some(inclusion) = self.extract_inclusion(chars, line_begin, line_end) {
                if self.is_valid_inclusion_kind(inclusion.kind) {
                    inclusions.push(inclusion);
                }
            }
            line_begin = line_end + 1; // next line starts after the previous line
        }
        inclusions
    }

    /// Extracts all inclusions from a given source file.
    ///
    /// PORT: the remote path (`remoteIncludeScanner != null && grepIncludes != null`, the
    /// `SpawnIncludeScanner` that runs grep-includes as a spawn) is `unported!`. It needs a
    /// toolchain whose `CcToolchainInfo` has a `_grep_includes` file; rules_cc's `cc_toolchain`
    /// has none, so with Bazel's own C++ rules `grepIncludes` is always null.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:861 IncludeParser.extractInclusions
    pub fn extract_inclusions_from_file(
        &self,
        file: &Artifact,
        action_execution_context: &dyn ActionExecutionContext,
        grep_includes: Option<&Artifact>,
        is_output_file: bool,
    ) -> Result<Vec<Inclusion>, IncludeScanningError> {
        if grep_includes.is_some() {
            blaze_util::unported!(
                "SpawnIncludeScanner.shouldParseRemotely",
                "a C++ toolchain with a grep-includes tool (rules_cc's cc_toolchain has none)"
            );
        }
        if is_output_file && !file_system_supports_input_discovery(action_execution_context) {
            // Ensure that the file's metadata is available, which possibly requires a Skyframe
            // restart.
            let _unused = action_execution_context
                .get_input_metadata_provider()
                .get_input_metadata(&blaze_exec::actions::action_adapters::artifact_input(file))?;
        }
        let mut inclusions = {
            let _c =
                Profiler::instance().profile(ProfilerTask::SCANNER, file.get_exec_path_string());
            let path = get_input_path(action_execution_context, file);
            let mut content = Vec::new();
            path.get_input_stream()?.read_to_end(&mut content)?;
            self.extract_inclusions(&content)
        };
        if let Some(hints) = &self.hints {
            inclusions.extend(hints.get_hinted_inclusions(file));
        }
        Ok(inclusions)
    }

    /// Returns type of the scanned file.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:925 IncludeParser.getFileType
    pub fn get_file_type(&self) -> GrepIncludesFileType {
        GrepIncludesFileType::Cpp
    }

    /// Parses include keyword in the provided char array and returns position immediately after
    /// include keyword or -1 if keyword was not found, along with information to aid future
    /// parsing.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:963 IncludeParser.expectIncludeKeyword
    fn expect_include_keyword(
        &self,
        chars: &[u8],
        position: usize,
        end: usize,
    ) -> IncludesKeywordData {
        let pos = expect(chars, skip_whitespace(chars, position, end), end, b"#");
        if pos > 0 {
            let npos = skip_whitespace(chars, pos as usize, end);
            let pos = expect(chars, npos, end, b"include");
            if pos > 0 {
                return IncludesKeywordData::normal(pos);
            }
            let pos = expect(chars, npos, end, b"import");
            if pos > 0 {
                return IncludesKeywordData::import_or_swig(pos);
            }
            let pos = skip_through_has_include(chars, npos as i64, end);
            if pos > 0 {
                return IncludesKeywordData::has_include(pos);
            }
        }
        IncludesKeywordData::NONE
    }

    /// Returns true if we interested in the given inclusion kind.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:981 IncludeParser.isValidInclusionKind
    fn is_valid_inclusion_kind(&self, _kind: Kind) -> bool {
        true
    }

    /// Returns inclusion object for non-standard inclusion cases or null if inclusion should be
    /// ignored.
    // Java: com/google/devtools/build/lib/includescanning/IncludeParser.java:990 IncludeParser.createOtherInclusion
    fn create_other_inclusion(&self, _inclusion_content: &[u8]) -> Option<Inclusion> {
        None
    }
}

/// `actionExecutionContext.fileSystemSupportsInputDiscovery()`:
/// `outputService.actionFileSystemType().supportsInputDiscovery()`, false only for
/// `REMOTE_FILE_SYSTEM`.
///
/// PORT: the execution context does not carry the flag. The `RemoteActionFileSystem` of build
/// without the bytes is the only action file system Blaze has, and it is present exactly when
/// the output service's type is `REMOTE_FILE_SYSTEM`.
// Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:380 ActionExecutionContext.fileSystemSupportsInputDiscovery
// Java: com/google/devtools/build/lib/vfs/OutputService.java:104 ActionFileSystemType.supportsInputDiscovery
fn file_system_supports_input_discovery(ctx: &dyn ActionExecutionContext) -> bool {
    ctx.get_remote_action_file_system().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser() -> IncludeParser {
        IncludeParser::new(Some(Hints::new(HintsRules::empty())))
    }

    fn inc(name: &str, kind: Kind) -> Option<Inclusion> {
        Some(Inclusion::create(name, kind))
    }

    #[test]
    fn extract_inclusion_kinds() {
        let p = parser();
        assert_eq!(
            p.extract_inclusion_from_line("#include \"a/b.h\""),
            inc("a/b.h", Kind::Quote)
        );
        assert_eq!(
            p.extract_inclusion_from_line("  #  include <vector>"),
            inc("vector", Kind::Angle)
        );
        assert_eq!(
            p.extract_inclusion_from_line("#include_next <c.h>"),
            inc("c.h", Kind::NextAngle)
        );
        assert_eq!(
            p.extract_inclusion_from_line("#include_next \"c.h\""),
            inc("c.h", Kind::NextQuote)
        );
        assert_eq!(
            p.extract_inclusion_from_line("#import \"m.h\""),
            inc("m.h", Kind::Quote)
        );
        // #import cannot have _next: "_next" is not skipped, so no quote follows.
        assert_eq!(p.extract_inclusion_from_line("#import_next \"m.h\""), None);
        assert_eq!(
            p.extract_inclusion_from_line("#if __has_include(<x.h>)"),
            inc("x.h", Kind::Angle)
        );
        assert_eq!(
            p.extract_inclusion_from_line("#elif __has_include_next(\"y.h\")"),
            inc("y.h", Kind::NextQuote)
        );
        assert_eq!(
            p.extract_inclusion_from_line("/* c */ # /* d */ include \"z.h\""),
            inc("z.h", Kind::Quote)
        );
    }

    #[test]
    fn extract_inclusion_rejects() {
        let p = parser();
        // Computed include.
        assert_eq!(p.extract_inclusion_from_line("#include NASTY"), None);
        // Absolute path.
        assert_eq!(p.extract_inclusion_from_line("#include </usr/a.h>"), None);
        // Unterminated.
        assert_eq!(p.extract_inclusion_from_line("#include \"a.h"), None);
        // Keyword at the end of the line: expect() does not find a token that ends at `end`.
        assert_eq!(p.extract_inclusion_from_line("#include"), None);
        assert_eq!(p.extract_inclusion_from_line("int x;"), None);
        // __has_include needs five more characters.
        assert_eq!(p.extract_inclusion_from_line("#if __has_include(<a>"), None);
        // The search for the closing quote starts one past the opening one.
        assert_eq!(p.extract_inclusion_from_line("#include \"\""), None);
    }

    #[test]
    fn extract_inclusions_handles_continuations() {
        let p = parser();
        let src = b"#include \"a.h\"\n#include \\\n  <b.h>\nint main() {}\n#include \"c\\\n.h\"\n";
        assert_eq!(
            p.extract_inclusions(src),
            vec![
                Inclusion::create("a.h", Kind::Quote),
                Inclusion::create("b.h", Kind::Angle),
                Inclusion::create("c.h", Kind::Quote),
            ]
        );
    }

    #[test]
    #[should_panic(expected = "ArrayIndexOutOfBoundsException")]
    fn extract_inclusions_trailing_backslash_throws() {
        parser().extract_inclusions(b"#include \"a.h\"\n\\");
    }

    #[test]
    fn inclusion_to_string() {
        assert_eq!(
            Inclusion::create("a/b.h", Kind::NextAngle).to_string(),
            "NEXT_ANGLE:a/b.h"
        );
    }
}
