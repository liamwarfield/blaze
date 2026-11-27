//! Port of `src/tools/starlark/java/com/google/devtools/starlark/common/DocstringUtils.java`:
//! parsing of Starlark function docstrings (summary, `Args:`, `Returns:`, `Deprecated:`).

use std::sync::LazyLock;

/// Parses a trimmed docstring.
///
/// The format of the docstring is as follows
///
/// ```text
/// """One-line summary: must be followed and may be preceded by a blank line.
///
/// Optional additional description like this.
///
/// If it's a function docstring and the function has more than one argument, the docstring has
/// to document these parameters as follows:
///
/// Args:
///   parameter1: description of the first parameter. Each parameter line
///     should be indented by one, preferably two, spaces (as here).
///   parameter2: description of the second
///     parameter that spans two lines. Each additional line should have a
///     hanging indentation of at least one, preferably two, additional spaces (as here).
///   another_parameter (unused, mutable): a parameter may be followed
///     by additional attributes in parentheses
///
/// Returns:
///   Description of the return value.
///   Should be indented by at least one, preferably two spaces (as here)
///   Can span multiple lines.
/// """
/// ```
///
/// We expect the docstring to already be trimmed and dedented to a minimal common indentation
/// level by `Starlark#trimDocString` or an equivalent PEP-257 style trim() implementation.
// Java: com/google/devtools/starlark/common/DocstringUtils.java:77 DocstringUtils.parseDocstring
pub fn parse_docstring(doc: &str, parse_errors: &mut Vec<DocstringParseError>) -> DocstringInfo {
    let mut parser = DocstringParser::new(doc);
    let result = parser.parse();
    parse_errors.append(&mut parser.errors);
    result
}

/// Encapsulates information about a Starlark function docstring.
// Java: com/google/devtools/starlark/common/DocstringUtils.java:85 DocstringUtils.DocstringInfo
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocstringInfo {
    /// The one-line summary at the start of the docstring.
    pub summary: String,
    /// Documentation of function parameters from the 'Args:' section.
    pub parameters: Vec<ParameterDoc>,
    /// Documentation of the return value from the 'Returns:' section, or empty if there is none.
    pub returns: String,
    /// Deprecation warning from the 'Deprecated:' section, or empty if there is none.
    pub deprecated: String,
    /// Rest of the docstring that is not part of any of the special sections above.
    pub long_description: String,
}

impl DocstringInfo {
    // Java: com/google/devtools/starlark/common/DocstringUtils.java:141 DocstringInfo.isSingleLineDocstring
    pub fn is_single_line_docstring(&self) -> bool {
        self.long_description.is_empty() && self.parameters.is_empty() && self.returns.is_empty()
    }
}

/// Contains information about the documentation for function parameters of a Starlark function.
// Java: com/google/devtools/starlark/common/DocstringUtils.java:157 DocstringUtils.ParameterDoc
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterDoc {
    pub parameter_name: String,
    /// e.g. a type annotation, "unused", "mutable"
    pub attributes: Vec<String>,
    pub description: String,
}

/// Contains error information to reflect a docstring parse error.
// Java: com/google/devtools/starlark/common/DocstringUtils.java:463 DocstringUtils.DocstringParseError
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocstringParseError {
    pub message: String,
    /// The line number (skipping leading blank lines, if any) in the original doc string which
    /// contains this error.
    pub line_number: usize,
    /// The contents of the original line that caused the parse error.
    pub line: String,
}

impl std::fmt::Display for DocstringParseError {
    // Java: com/google/devtools/starlark/common/DocstringUtils.java:474 DocstringParseError.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.line_number, self.message)
    }
}

// Java: com/google/devtools/starlark/common/DocstringUtils.java:345 DocstringParser.paramLineMatcher
static PARAM_LINE_MATCHER: LazyLock<regex::Regex> = LazyLock::new(|| {
    // Matcher.matches(): the whole line.
    blaze_util::java_regex::compile(
        r"\A(?:\s*(?<name>[*\w]+)( \(\s*(?<attributes>.*)\s*\))?: (?<description>.*))\z",
        0,
    )
    .expect("a valid pattern")
});

// Java: com/google/devtools/starlark/common/DocstringUtils.java:349 DocstringParser.attributesSeparator
static ATTRIBUTES_SEPARATOR: LazyLock<regex::Regex> =
    LazyLock::new(|| blaze_util::java_regex::compile(r"\s*,\s*", 0).expect("a valid pattern"));

// Java: com/google/devtools/starlark/common/DocstringUtils.java:176 DocstringUtils.DocstringParser
struct DocstringParser {
    /// Current line number within the docstring, 1-based; 0 indicates that parsing has not
    /// started; `line_number > lines.len()` indicates EOF.
    line_number: usize,
    /// Whether we've seen a special section, e.g. 'Args:', already.
    special_sections_started: bool,
    /// List of all parsed lines in the docstring.
    lines: Vec<String>,
    /// The current line in the docstring.
    line: String,
    /// Errors that occurred so far.
    errors: Vec<DocstringParseError>,
}

impl DocstringParser {
    // Java: com/google/devtools/starlark/common/DocstringUtils.java:200 DocstringParser.<init>
    fn new(docstring: &str) -> Self {
        let mut parser = DocstringParser {
            line_number: 0,
            special_sections_started: false,
            // Splitter.on("\n").split(docstring)
            lines: docstring.split('\n').map(str::to_owned).collect(),
            line: String::new(),
            errors: Vec::new(),
        };
        // Load the summary line
        parser.next_line();
        parser
    }

    /// Move on to the next line and update the parser's internal state accordingly. Returns
    /// whether there are lines remaining to be parsed.
    // Java: com/google/devtools/starlark/common/DocstringUtils.java:212 DocstringParser.nextLine
    fn next_line(&mut self) -> bool {
        if self.line_number < self.lines.len() {
            self.line = self.lines[self.line_number].clone();
            self.line_number += 1;
            true
        } else {
            self.line = String::new();
            self.line_number = self.lines.len() + 1;
            false
        }
    }

    // Java: com/google/devtools/starlark/common/DocstringUtils.java:224 DocstringParser.eof
    fn eof(&self) -> bool {
        self.line_number > self.lines.len()
    }

    // Java: com/google/devtools/starlark/common/DocstringUtils.java:228 DocstringParser.getIndentation
    fn get_indentation(line: &str) -> usize {
        line.bytes().take_while(|b| *b == b' ').count()
    }

    // Java: com/google/devtools/starlark/common/DocstringUtils.java:236 DocstringParser.error
    fn error(&mut self, message: impl Into<String>) {
        self.error_at(self.line_number, message);
    }

    // Java: com/google/devtools/starlark/common/DocstringUtils.java:240 DocstringParser.error
    fn error_at(&mut self, line_number: usize, message: impl Into<String>) {
        self.errors.push(DocstringParseError {
            message: message.into(),
            line_number,
            line: self.lines[line_number - 1].clone(),
        });
    }

    // Java: com/google/devtools/starlark/common/DocstringUtils.java:244 DocstringParser.parseArgumentSection
    fn parse_argument_section(
        &mut self,
        params: &mut Vec<ParameterDoc>,
        returns: &str,
        deprecated: &str,
    ) {
        self.check_section_start(!params.is_empty());
        if !returns.is_empty() {
            self.error("'Args:' section should go before the 'Returns:' section");
        }
        if !deprecated.is_empty() {
            self.error("'Args:' section should go before the 'Deprecated:' section");
        }
        let mut parsed = self.parse_parameters();
        params.append(&mut parsed);
    }

    // Java: com/google/devtools/starlark/common/DocstringUtils.java:256 DocstringParser.parse
    fn parse(&mut self) -> DocstringInfo {
        let summary = self.line.clone();
        let mut non_standard_deprecation = self.check_for_non_standard_deprecation(&summary);
        if !self.next_line() {
            return DocstringInfo {
                summary,
                parameters: Vec::new(),
                returns: String::new(),
                deprecated: non_standard_deprecation,
                long_description: String::new(),
            };
        }
        if !self.line.is_empty() {
            self.error("the one-line summary should be followed by a blank line");
        } else {
            self.next_line();
        }
        let mut long_description_lines: Vec<String> = Vec::new();
        let mut params: Vec<ParameterDoc> = Vec::new();
        let mut returns = String::new();
        let mut deprecated = String::new();
        let mut description_body_after_special_sections_reported = false;
        while !self.eof() {
            match self.line.as_str() {
                "Args:" | "Arguments:" => {
                    self.parse_argument_section(&mut params, &returns, &deprecated);
                }
                "Returns:" => {
                    self.check_section_start(!returns.is_empty());
                    if !deprecated.is_empty() {
                        self.error("'Returns:' section should go before the 'Deprecated:' section");
                    }
                    returns = self.parse_section_after_heading();
                }
                "Deprecated:" => {
                    self.check_section_start(!deprecated.is_empty());
                    deprecated = self.parse_section_after_heading();
                }
                _ => {
                    if self.special_sections_started
                        && !description_body_after_special_sections_reported
                    {
                        self.error("description body should go before the special sections");
                        description_body_after_special_sections_reported = true;
                    }
                    if deprecated.is_empty() && non_standard_deprecation.is_empty() {
                        let line = self.line.clone();
                        non_standard_deprecation = self.check_for_non_standard_deprecation(&line);
                    }
                    if self.line.starts_with("Returns: ") {
                        self.error(
                            "the return value should be documented in a section, like this:\n\n\
                             Returns:\n  <documentation here>\n\n\
                             For more details, please have a look at the documentation.",
                        );
                    }
                    long_description_lines.push(self.line.clone());
                    self.next_line();
                }
            }
        }
        if deprecated.is_empty() {
            deprecated = non_standard_deprecation;
        }
        DocstringInfo {
            summary,
            parameters: params,
            returns,
            deprecated,
            long_description: long_description_lines.join("\n"),
        }
    }

    // Java: com/google/devtools/starlark/common/DocstringUtils.java:315 DocstringParser.checkSectionStart
    fn check_section_start(&mut self, duplicate_section: bool) {
        self.special_sections_started = true;
        if duplicate_section {
            let message = format!("duplicate '{}' section", self.line);
            self.error(message);
        }
    }

    // Java: com/google/devtools/starlark/common/DocstringUtils.java:322 DocstringParser.checkForNonStandardDeprecation
    fn check_for_non_standard_deprecation(&mut self, line: &str) -> String {
        // line.toLowerCase(Locale.ROOT).startsWith("deprecated:")
        if java_to_lower_case_root(line).starts_with("deprecated:") || line.contains("DEPRECATED") {
            self.error(
                "use a 'Deprecated:' section for deprecations, similar to a 'Returns:' section:\n\n\
                 Deprecated:\n  <reason and alternative>\n\n\
                 For more details, please have a look at the documentation.",
            );
            return line.to_owned();
        }
        String::new()
    }

    // Java: com/google/devtools/starlark/common/DocstringUtils.java:351 DocstringParser.parseParameters
    fn parse_parameters(&mut self) -> Vec<ParameterDoc> {
        let section_line_number = self.line_number;
        self.next_line();
        let mut params: Vec<ParameterDoc> = Vec::new();
        let mut expected_param_line_indentation: Option<usize> = None;
        while !self.eof() {
            if self.line.is_empty() {
                self.next_line();
                continue;
            }
            let actual_indentation = Self::get_indentation(&self.line);
            if actual_indentation == 0 {
                break;
            }
            let expected = *expected_param_line_indentation.get_or_insert(actual_indentation);
            if expected != actual_indentation {
                self.error(format!(
                    "inconsistent indentation of parameter lines (before: {}; here: {} spaces)",
                    expected, actual_indentation
                ));
            }
            let param_line_number = self.line_number;
            let trimmed_line = self.line[actual_indentation..].to_owned();
            let Some(captures) = PARAM_LINE_MATCHER.captures(&trimmed_line) else {
                self.error(
                    "invalid parameter documentation (expected format: \"parameter_name: \
                     documentation\"). For more details, please have a look at the documentation.",
                );
                self.next_line();
                continue;
            };
            let parameter_name = captures
                .name("name")
                .expect("checkNotNull(matcher.group(\"name\"))")
                .as_str()
                .to_owned();
            let attributes: Vec<String> = match captures.name("attributes") {
                None => Vec::new(),
                // Arrays.asList(attributesSeparator.split(attributesString)): Java's split drops
                // trailing empty strings.
                Some(a) => java_split(&ATTRIBUTES_SEPARATOR, a.as_str()),
            };
            let mut description = captures
                .name("description")
                .map(|d| d.as_str().to_owned())
                .unwrap_or_default();
            self.parse_continued_param_description(actual_indentation, &mut description);
            let parameter_description = java_trim(&description).to_owned();
            if parameter_description.is_empty() {
                self.error_at(
                    param_line_number,
                    format!("empty parameter description for '{parameter_name}'"),
                );
            }
            params.push(ParameterDoc {
                parameter_name,
                attributes,
                description: parameter_description,
            });
        }
        if params.is_empty() {
            self.error_at(section_line_number, "section is empty or badly formatted");
        }
        params
    }

    /// Parses additional lines that can come after "param: foo" in an 'Args' section.
    // Java: com/google/devtools/starlark/common/DocstringUtils.java:407 DocstringParser.parseContinuedParamDescription
    fn parse_continued_param_description(
        &mut self,
        baseline_indentation: usize,
        description: &mut String,
    ) {
        // Two iterations: first buffer lines and find the minimal indent, then trim to the min
        let mut buffer: Vec<String> = Vec::new();
        let mut continuation_indentation = usize::MAX;
        while self.next_line() {
            if !self.line.is_empty() {
                if Self::get_indentation(&self.line) <= baseline_indentation {
                    break;
                }
                continuation_indentation =
                    continuation_indentation.min(Self::get_indentation(&self.line));
            }
            buffer.push(self.line.clone());
        }

        for buf_line in &buffer {
            description.push('\n');
            if !buf_line.is_empty() {
                description.push_str(&buf_line[continuation_indentation..]);
            }
        }
    }

    // Java: com/google/devtools/starlark/common/DocstringUtils.java:430 DocstringParser.parseSectionAfterHeading
    fn parse_section_after_heading(&mut self) -> String {
        let section_line_number = self.line_number;
        self.next_line();
        let mut contents = String::new();
        let mut first_line = true;
        while !self.eof() {
            let trimmed_line: String;
            if self.line.is_empty() {
                trimmed_line = self.line.clone();
            } else if Self::get_indentation(&self.line) == 0 {
                break;
            } else if Self::get_indentation(&self.line) < 2 {
                self.error(
                    "text in a section has to be indented by two spaces (relative to the left \
                     margin of the docstring)",
                );
                trimmed_line = self.line[Self::get_indentation(&self.line)..].to_owned();
            } else {
                trimmed_line = self.line[2..].to_owned();
            }
            if !first_line {
                contents.push('\n');
            }
            contents.push_str(&trimmed_line);
            self.next_line();
            first_line = false;
        }
        let result = java_trim(&contents).to_owned();
        if result.is_empty() {
            self.error_at(section_line_number, "section is empty");
        }
        result
    }
}

/// `String.trim()`: strips the characters `<= ' '` from both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `String.toLowerCase(Locale.ROOT)` for the `startsWith("deprecated:")` test.
fn java_to_lower_case_root(s: &str) -> String {
    s.to_lowercase()
}

/// `Pattern.split(input)`: trailing empty strings are removed; an input without a match is the
/// one-element array of the input.
fn java_split(pattern: &regex::Regex, input: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    let mut index = 0;
    for m in pattern.find_iter(input) {
        // A zero-length match at the beginning never produces an empty leading substring.
        if index == 0 && m.start() == 0 && m.end() == 0 {
            continue;
        }
        result.push(input[index..m.start()].to_owned());
        index = m.end();
    }
    if index == 0 && result.is_empty() {
        return vec![input.to_owned()];
    }
    result.push(input[index..].to_owned());
    while result.last().is_some_and(String::is_empty) {
        result.pop();
    }
    result
}
