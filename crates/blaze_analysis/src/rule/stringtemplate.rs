//! Ports of `analysis/stringtemplate/{TemplateContext, TemplateExpander, Expansion,
//! ExpansionException}.java`.

use indexmap::IndexSet;

/// Exception thrown by `TemplateExpander::expand` if expansion fails.
// Java: com/google/devtools/build/lib/analysis/stringtemplate/ExpansionException.java:20 ExpansionException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ExpansionException(pub String);

/// Interface to be implemented by callers of `TemplateExpander` which defines the expansion of
/// each template variable and function.
// Java: com/google/devtools/build/lib/analysis/stringtemplate/TemplateContext.java:21 TemplateContext
pub trait TemplateContext {
    /// Returns the expansion of the specified template variable.
    fn lookup_variable(&self, name: &str) -> Result<String, ExpansionException>;

    /// Returns the expansion of the specified template function with the given parameter.
    fn lookup_function(&self, name: &str, param: &str) -> Result<String, ExpansionException>;
}

/// Result of a TemplateExpander expansion: the expanded string and the variables looked up.
// Java: com/google/devtools/build/lib/analysis/stringtemplate/Expansion.java:22 Expansion
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    pub expansion: String,
    pub looked_up_variables: IndexSet<String>,
}

/// Simple string template expansion. String templates consist of text interspersed with
/// `$(variable)` or `$(function value)` references, which are replaced by strings.
///
/// Note that while this pretends to be a parser, it skips over some potential parse errors such
/// as unbalanced parentheses.
// Java: com/google/devtools/build/lib/analysis/stringtemplate/TemplateExpander.java:28 TemplateExpander
pub struct TemplateExpander {
    /// PORT: Java scans a `char[]` (UTF-16 units); the special characters are all ASCII, so
    /// scanning Unicode scalar values finds the same positions.
    buffer: Vec<char>,
    length: usize,
    offset: usize,
}

impl TemplateExpander {
    fn new(expression: &str) -> TemplateExpander {
        let buffer: Vec<char> = expression.chars().collect();
        let length = buffer.len();
        TemplateExpander {
            buffer,
            length,
            offset: 0,
        }
    }

    /// If the template contains a single variable, expands it, otherwise returns `None`.
    // Java: com/google/devtools/build/lib/analysis/stringtemplate/TemplateExpander.java:55 TemplateExpander.expandSingleVariable
    pub fn expand_single_variable(
        expression: &str,
        context: &dyn TemplateContext,
    ) -> Result<Option<String>, ExpansionException> {
        let var = TemplateExpander::new(expression).get_single_variable()?;
        match var {
            Some(v) => Ok(Some(context.lookup_variable(&v)?)),
            None => Ok(None),
        }
    }

    /// Expands all references to template variables embedded within string `expr`, using the
    /// lookup function `context`. Fails if there are unterminated references, or invalid
    /// variables.
    // Java: com/google/devtools/build/lib/analysis/stringtemplate/TemplateExpander.java:73 TemplateExpander.expand
    pub fn expand(
        expression: &str,
        context: &dyn TemplateContext,
    ) -> Result<Expansion, ExpansionException> {
        if !expression.contains('$') {
            return Ok(Expansion {
                expansion: expression.to_owned(),
                looked_up_variables: IndexSet::new(),
            });
        }
        Self::expand_depth(expression, context, 0)
    }

    // Java: com/google/devtools/build/lib/analysis/stringtemplate/TemplateExpander.java:81 TemplateExpander.expand
    fn expand_depth(
        expression: &str,
        context: &dyn TemplateContext,
        depth: i32,
    ) -> Result<Expansion, ExpansionException> {
        if depth > 10 {
            // plenty!
            return Err(ExpansionException(format!(
                "potentially unbounded recursion during expansion of '{expression}'"
            )));
        }
        TemplateExpander::new(expression).expand_impl(context, depth)
    }

    // Java: com/google/devtools/build/lib/analysis/stringtemplate/TemplateExpander.java:90 TemplateExpander.expand
    fn expand_impl(
        &mut self,
        context: &dyn TemplateContext,
        depth: i32,
    ) -> Result<Expansion, ExpansionException> {
        let mut result = String::new();
        let mut looked_up_variables: IndexSet<String> = IndexSet::new();
        while self.offset < self.length {
            let c = self.buffer[self.offset];
            if c == '$' {
                // variable
                self.offset += 1;
                if self.offset >= self.length {
                    return Err(ExpansionException("unterminated $".to_owned()));
                }
                if self.buffer[self.offset] == '$' {
                    result.push('$');
                } else {
                    let var = self.scan_variable()?;
                    match var.find(' ') {
                        None => {
                            let mut value = context.lookup_variable(&var)?;
                            looked_up_variables.insert(var.clone());
                            if value != var {
                                let expansion = Self::expand_depth(&value, context, depth + 1)?;
                                value = expansion.expansion;
                                looked_up_variables.extend(expansion.looked_up_variables);
                            }
                            result.push_str(&value);
                        }
                        Some(space_index) => {
                            let name = &var[..space_index];
                            let param = var[space_index + 1..].trim_matches(java_trim_char);
                            let value = context.lookup_function(name, param)?;
                            looked_up_variables.insert(name.to_owned());
                            result.push_str(&value);
                        }
                    }
                }
            } else {
                result.push(c);
            }
            self.offset += 1;
        }
        Ok(Expansion {
            expansion: result,
            looked_up_variables,
        })
    }

    /// Starting at the current position, scans forward until the name of a template variable
    /// has been consumed. Returns the variable name and advances the position. If the variable
    /// is a single-char name, the position is left on that char. If it is a parenthesized name,
    /// the position is left on the closing paren.
    // Java: com/google/devtools/build/lib/analysis/stringtemplate/TemplateExpander.java:139 TemplateExpander.scanVariable
    fn scan_variable(&mut self) -> Result<String, ExpansionException> {
        let c = self.buffer[self.offset];
        match c {
            '(' => {
                // looks like $(SRCS)
                self.offset += 1;
                let start = self.offset;
                while self.offset < self.length && self.buffer[self.offset] != ')' {
                    self.offset += 1;
                }
                if self.offset >= self.length {
                    return Err(ExpansionException(
                        "unterminated variable reference".to_owned(),
                    ));
                }
                Ok(self.buffer[start..self.offset].iter().collect())
            }
            '{' => {
                // looks like ${SRCS}
                self.offset += 1;
                let start = self.offset;
                while self.offset < self.length && self.buffer[self.offset] != '}' {
                    self.offset += 1;
                }
                if self.offset >= self.length {
                    return Err(ExpansionException(
                        "unterminated variable reference".to_owned(),
                    ));
                }
                let expr: String = self.buffer[start..self.offset].iter().collect();
                Err(ExpansionException(format!(
                    "'${{{expr}}}' syntax is not supported; use '$({expr})' instead for \"Make\" variables, or escape the '$' as '$$' if you intended this for the shell"
                )))
            }
            '@' | '<' | '^' => Ok(c.to_string()),
            _ => {
                let start = self.offset;
                while self.offset + 1 < self.length
                    && is_java_identifier_part(self.buffer[self.offset + 1])
                {
                    self.offset += 1;
                }
                let expr: String = self.buffer[start..self.offset + 1].iter().collect();
                Err(ExpansionException(format!(
                    "'${expr}' syntax is not supported; use '$({expr})' instead for \"Make\" variables, or escape the '$' as '$$' if you intended this for the shell"
                )))
            }
        }
    }

    /// Returns the single variable name if the template is exactly one variable reference.
    // Java: com/google/devtools/build/lib/analysis/stringtemplate/TemplateExpander.java:193 TemplateExpander.getSingleVariable
    fn get_single_variable(&mut self) -> Result<Option<String>, ExpansionException> {
        if self.length > 0 && self.buffer[self.offset] == '$' {
            self.offset += 1;
            let result = self.scan_variable()?;
            if self.offset + 1 == self.length {
                return Ok(Some(result));
            }
        }
        Ok(None)
    }
}

/// Java `String.trim()`: strips chars `<= ' '`.
fn java_trim_char(c: char) -> bool {
    c <= ' '
}

/// Java `Character.isJavaIdentifierPart(char)` for a `char` of an internal string (U+0000 to
/// U+00FF). The set is the one the JDK Bazel 9.3.2 embeds (25.0.2) reports for these 256
/// characters.
// Java: java/lang/Character.java Character.isJavaIdentifierPart
fn is_java_identifier_part(c: char) -> bool {
    let Ok(b) = u8::try_from(u32::from(c)) else {
        // An internal string holds no char above U+00FF.
        blaze_util::unported!(
            "Character.isJavaIdentifierPart (a char above U+00FF)",
            "{:?}",
            c
        );
    };
    matches!(
        b,
        0x00..=0x08
            | 0x0e..=0x1b
            | b'$'
            | b'0'..=b'9'
            | b'A'..=b'Z'
            | b'_'
            | b'a'..=b'z'
            | 0x7f..=0x9f
            | 0xa2..=0xa5
            | 0xaa
            | 0xad
            | 0xb5
            | 0xba
            | 0xc0..=0xd6
            | 0xd8..=0xf6
            | 0xf8..=0xff
    )
}

#[cfg(test)]
mod tests {
    // Ports of src/test/java/com/google/devtools/build/lib/analysis/stringtemplate/TemplateExpanderTest.java.
    use super::*;
    use std::collections::HashMap;

    struct MapContext {
        vars: HashMap<String, String>,
        functions: HashMap<String, String>,
    }

    impl TemplateContext for MapContext {
        fn lookup_variable(&self, name: &str) -> Result<String, ExpansionException> {
            self.vars
                .get(name)
                .cloned()
                .ok_or_else(|| ExpansionException(format!("$({name}) not defined")))
        }

        fn lookup_function(&self, name: &str, param: &str) -> Result<String, ExpansionException> {
            self.functions
                .get(name)
                .map(|f| format!("{f}({param})"))
                .ok_or_else(|| ExpansionException(format!("$({name}) not defined")))
        }
    }

    fn ctx(vars: &[(&str, &str)]) -> MapContext {
        MapContext {
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            functions: [("location".to_owned(), "loc".to_owned())]
                .into_iter()
                .collect(),
        }
    }

    fn expand(s: &str, c: &MapContext) -> Result<String, ExpansionException> {
        TemplateExpander::expand(s, c).map(|e| e.expansion)
    }

    #[test]
    fn expansion_with_no_variables() {
        assert_eq!(expand("hello world", &ctx(&[])).unwrap(), "hello world");
    }

    #[test]
    fn simple_variable_expansion() {
        let c = ctx(&[("SRCS", "src1 src2"), ("<", "src1")]);
        assert_eq!(expand("$(SRCS)", &c).unwrap(), "src1 src2");
        assert_eq!(expand("$<", &c).unwrap(), "src1");
        assert_eq!(expand("a $(SRCS) b", &c).unwrap(), "a src1 src2 b");
    }

    #[test]
    fn dollar_dollar() {
        assert_eq!(expand("$$PATH", &ctx(&[])).unwrap(), "$PATH");
    }

    #[test]
    fn recursive_expansion() {
        let c = ctx(&[("A", "$(B)"), ("B", "b")]);
        let e = TemplateExpander::expand("$(A)", &c).unwrap();
        assert_eq!(e.expansion, "b");
        assert_eq!(
            e.looked_up_variables.into_iter().collect::<Vec<_>>(),
            vec!["A".to_owned(), "B".to_owned()]
        );
    }

    #[test]
    fn infinite_recursion() {
        let c = ctx(&[("A", "$(A) ")]);
        assert_eq!(
            expand("$(A)", &c).unwrap_err().0,
            "potentially unbounded recursion during expansion of '$(A) '"
        );
    }

    #[test]
    fn unterminated_errors() {
        let c = ctx(&[]);
        assert_eq!(expand("foo $", &c).unwrap_err().0, "unterminated $");
        assert_eq!(
            expand("$(foo", &c).unwrap_err().0,
            "unterminated variable reference"
        );
    }

    #[test]
    fn brace_syntax_rejected() {
        assert_eq!(
            expand("${foo}", &ctx(&[])).unwrap_err().0,
            "'${foo}' syntax is not supported; use '$(foo)' instead for \"Make\" variables, or escape the '$' as '$$' if you intended this for the shell"
        );
        assert_eq!(
            expand("$foo bar", &ctx(&[])).unwrap_err().0,
            "'$foo' syntax is not supported; use '$(foo)' instead for \"Make\" variables, or escape the '$' as '$$' if you intended this for the shell"
        );
    }

    #[test]
    fn function_expansion() {
        let c = ctx(&[]);
        assert_eq!(expand("$(location  //a:b )", &c).unwrap(), "loc(//a:b)");
    }

    /// `$name`: Java names the longest run of `Character.isJavaIdentifierPart` characters of the
    /// internal string in its message (TemplateExpander.java:139; the character set is the
    /// JDK's, 25.0.2).
    #[test]
    fn dollar_name_syntax_error_names_the_identifier() {
        let c = ctx(&[]);
        let message = |expr: &str| {
            format!(
                "'${expr}' syntax is not supported; use '$({expr})' instead for \"Make\" variables, or escape the '$' as '$$' if you intended this for the shell"
            )
        };
        assert_eq!(expand("echo $foo bar", &c).unwrap_err().0, message("foo"));
        assert_eq!(expand("$a.b", &c).unwrap_err().0, message("a"));
        assert_eq!(expand("$a_b$c-d", &c).unwrap_err().0, message("a_b$c"));
        // U+0001 is an ignorable identifier character.
        assert_eq!(expand("$a\u{1}b c", &c).unwrap_err().0, message("a\u{1}b"));
        // The internal string of "µ" (U+00B5) is the chars C2 B5, both identifier parts; of
        // "→" (U+2192) E2 86 92, likewise. "é🐷" is C3 A9 F0 9F 90 B7: U+00A9 is not a part, so
        // only the first char is named; "êà" is C3 AA C3 A0 (U+00A0 is not a part). A raw byte
        // FF is a part; U+00D7 (×) and U+00A0 are not. Expected values from the JDK (25.0.5).
        let i = |s: &str| blaze_util::string_encoding::latin1_decode(s.as_bytes()).into_owned();
        assert_eq!(
            expand(&i("$x\u{b5}y z"), &c).unwrap_err().0,
            message(&i("x\u{b5}y"))
        );
        assert_eq!(
            expand(&i("$x\u{2192} z"), &c).unwrap_err().0,
            message(&i("x\u{2192}"))
        );
        assert_eq!(
            expand(&i("$\u{e9}\u{1F437} z"), &c).unwrap_err().0,
            message("\u{c3}")
        );
        assert_eq!(
            expand(&i("$\u{ea}\u{e0} z"), &c).unwrap_err().0,
            message("\u{c3}\u{aa}\u{c3}")
        );
        assert_eq!(
            expand("$x\u{ff}y z", &c).unwrap_err().0,
            message("x\u{ff}y")
        );
        assert_eq!(expand("$x\u{d7}y", &c).unwrap_err().0, message("x"));
        assert_eq!(expand("$x\u{a0}y", &c).unwrap_err().0, message("x"));
        // A non-identifier first character is still named.
        assert_eq!(expand("$-x", &c).unwrap_err().0, message("-x"));
    }

    #[test]
    fn single_variable() {
        let c = ctx(&[("A", "a")]);
        assert_eq!(
            TemplateExpander::expand_single_variable("$(A)", &c).unwrap(),
            Some("a".to_owned())
        );
        assert_eq!(
            TemplateExpander::expand_single_variable("x$(A)", &c).unwrap(),
            None
        );
    }
}
