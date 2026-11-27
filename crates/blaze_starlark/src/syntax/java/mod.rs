//! Bazel's Starlark front end (scanner, parser with error recovery, resolver), ported to report a
//! file's static errors exactly as Bazel does: every error, in Bazel's order, at Bazel's
//! positions. starlark-rust's parser stops at the first error, so Blaze uses this front end for
//! the error list of a file that does not compile; the tree it builds is only used for errors.
//!
//! Java: net/starlark/java/syntax/{Lexer,Parser,Resolver,FileLocations}.java.

pub mod ast;
pub mod dot_bazel;
pub mod lexer;
pub mod parser;
pub mod resolver;

use crate::error::Location;
use crate::error::SyntaxError;

/// Java `FileOptions` (the fields the scanner, parser and resolver read).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JavaOptions {
    pub allow_load_private_symbols: bool,
    pub allow_toplevel_rebinding: bool,
    pub load_binds_globally: bool,
    pub require_load_statements_first: bool,
    pub string_literals_are_ascii_only: bool,
    pub allow_type_syntax: bool,
    pub tolerate_invalid_type_expressions: bool,
}

impl JavaOptions {
    /// Java `FileOptions.DEFAULT`.
    pub const DEFAULT: JavaOptions = JavaOptions {
        allow_load_private_symbols: false,
        allow_toplevel_rebinding: false,
        load_binds_globally: false,
        require_load_statements_first: true,
        string_literals_are_ascii_only: false,
        allow_type_syntax: false,
        tolerate_invalid_type_expressions: false,
    };

    /// The options Bazel parses a file of this kind with: Blaze's [`crate::file_kind::FileOptions`]
    /// and, for type syntax, BzlCompileFunction's `tolerateInvalidTypeExpressions(!doTypeChecking)`
    /// (type checking is off by default).
    pub fn from_file_options(o: crate::file_kind::FileOptions) -> JavaOptions {
        JavaOptions {
            allow_load_private_symbols: o.allow_load_private_symbols,
            allow_toplevel_rebinding: o.allow_toplevel_rebinding,
            load_binds_globally: o.load_binds_globally,
            require_load_statements_first: o.require_load_statements_first,
            string_literals_are_ascii_only: o.string_literals_are_ascii_only,
            allow_type_syntax: o.allow_type_syntax,
            tolerate_invalid_type_expressions: true,
        }
    }
}

/// Java `FileLocations`: char offsets to lines and columns.
// Java: net/starlark/java/syntax/FileLocations.java
pub struct FileLocations {
    file: String,
    line_starts: Vec<usize>,
}

impl FileLocations {
    pub fn new(file: &str, chars: &[char]) -> FileLocations {
        let mut line_starts = vec![0];
        for (i, c) in chars.iter().enumerate() {
            if *c == '\n' {
                line_starts.push(i + 1);
            }
        }
        FileLocations {
            file: file.to_owned(),
            line_starts,
        }
    }

    // Java: net/starlark/java/syntax/FileLocations.java:69 FileLocations.getLocation
    pub fn location(&self, offset: usize) -> Location {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        Location {
            file: self.file.clone(),
            line: line + 1,
            column: offset - self.line_starts[line] + 1,
        }
    }
}

/// A parsed file: its statements and the scanner's and parser's errors, in Java's order.
pub struct ParsedFile {
    pub statements: Vec<ast::Stmt>,
    pub errors: Vec<SyntaxError>,
    pub locs: FileLocations,
    /// The `#:` comment lines of the file (Java `StarlarkFile.getComments()` with a doc comment
    /// prefix).
    pub doc_comment_lines: Vec<ast::DocCommentLine>,
}

/// Java `StarlarkFile.parse`: scans and parses `source` (an internal string: one char per byte),
/// reporting every scanner and parser error.
// Java: net/starlark/java/syntax/Parser.java:195 Parser.parseFile
pub fn parse_file(file: &str, source: &str, options: JavaOptions) -> ParsedFile {
    let chars: Vec<char> = source.chars().collect();
    let locs = FileLocations::new(file, &chars);
    let mut line_starts = vec![0];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            line_starts.push(i + 1);
        }
    }
    let lexer = lexer::Lexer::new(&chars, &line_starts, options);
    let mut p = parser::Parser::new(lexer, options);
    let statements = p.parse_file_input();
    let doc_comment_lines = p.doc_comment_lines().to_vec();
    let errors = p
        .errors
        .into_iter()
        .map(|e| SyntaxError {
            location: locs.location(e.offset),
            message: e.message,
        })
        .collect();
    ParsedFile {
        statements,
        errors,
        locs,
        doc_comment_lines,
    }
}

/// Java `Expression.parse(input, options)`: scans and parses `source` as one expression; the
/// scanner's and parser's errors (Java throws `SyntaxError.Exception` when there is any).
// Java: net/starlark/java/syntax/Parser.java:248 Parser.parseExpression
pub fn parse_expression(file: &str, source: &str, options: JavaOptions) -> Vec<SyntaxError> {
    let chars: Vec<char> = source.chars().collect();
    let locs = FileLocations::new(file, &chars);
    let mut line_starts = vec![0];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            line_starts.push(i + 1);
        }
    }
    let lexer = lexer::Lexer::new(&chars, &line_starts, options);
    let mut p = parser::Parser::new(lexer, options);
    let _ = p.parse_expression_input();
    p.errors
        .into_iter()
        .map(|e| SyntaxError {
            location: locs.location(e.offset),
            message: e.message,
        })
        .collect()
}

/// Java `Program.compileFile`'s static errors: `parsed`'s errors followed by the resolver's.
// Java: net/starlark/java/syntax/Program.java:101 Program.compileFile
pub fn compile_errors(
    parsed: &ParsedFile,
    module: &dyn resolver::ResolverModule,
    options: JavaOptions,
) -> Vec<SyntaxError> {
    let mut errors = parsed.errors.clone();
    errors.extend(
        resolver::resolve_file(&parsed.statements, module, options)
            .into_iter()
            .map(|e| SyntaxError {
                location: parsed.locs.location(e.offset),
                message: e.message,
            }),
    );
    errors
}

/// The doc comments of a compiled file: Java `Program.getDocCommentsMap()` (the doc comments
/// attached to the file's new global bindings, by name, in binding order) and
/// `Program.getUnusedDocCommentLines()` (the other `#:` comment lines, in file order).
#[derive(Debug, Clone, Default)]
pub struct ProgramDocComments {
    pub doc_comments_map: starlark_map::small_map::SmallMap<String, ast::DocComments>,
    pub unused_doc_comment_lines: Vec<ast::DocCommentLine>,
}

/// Java `Program.compileFile(file, env)`'s doc comments of `parsed` resolved against `module`.
// Java: net/starlark/java/syntax/Program.java:101 Program.compileFile
pub fn program_doc_comments(
    parsed: &ParsedFile,
    module: &dyn resolver::ResolverModule,
    options: JavaOptions,
) -> ProgramDocComments {
    let (_, doc_comments_map) =
        resolver::resolve_file_with_doc_comments(&parsed.statements, module, options);
    // Find unused doc comments.
    let used_doc_comment_lines: std::collections::HashSet<&ast::DocCommentLine> = doc_comments_map
        .values()
        .flat_map(|doc_comments| doc_comments.lines.iter())
        .collect();
    let unused_doc_comment_lines = parsed
        .doc_comment_lines
        .iter()
        .filter(|c| !used_doc_comment_lines.contains(c))
        .cloned()
        .collect();
    ProgramDocComments {
        doc_comments_map,
        unused_doc_comment_lines,
    }
}

/// A [`resolver::ResolverModule`] for a fresh module with these predeclared names (Java
/// `Module.withPredeclared` + `Starlark.UNIVERSE`; `Module.resolve`).
// Java: net/starlark/java/eval/Module.java:231 Module.resolve
pub struct PredeclaredModule<'a> {
    /// Predeclared and universal names, in the order Java's `HashSet` of candidates is filled.
    names: Vec<String>,
    set: std::collections::HashSet<String>,
    /// The message for a predeclared name whose `GuardedValue` is not accessible.
    guarded: &'a dyn Fn(&str) -> Option<String>,
}

impl<'a> PredeclaredModule<'a> {
    pub fn new(names: Vec<String>, guarded: &'a dyn Fn(&str) -> Option<String>) -> Self {
        let set = names.iter().cloned().collect();
        PredeclaredModule {
            names,
            set,
            guarded,
        }
    }
}

impl resolver::ResolverModule for PredeclaredModule<'_> {
    fn resolve(&self, name: &str) -> Result<resolver::Scope, resolver::Undefined> {
        if let Some(message) = (self.guarded)(name) {
            return Err(resolver::Undefined {
                message,
                candidates: None,
            });
        }
        if self.set.contains(name) {
            return Ok(resolver::Scope::Predeclared);
        }
        Err(resolver::Undefined {
            message: format!("name '{name}' is not defined"),
            candidates: Some(self.names.clone()),
        })
    }
}

#[cfg(test)]
mod doc_comments_tests {
    use super::*;

    fn compile_doc_comments(source: &str, options: JavaOptions) -> ProgramDocComments {
        let parsed = parse_file("foo.star", source, options);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let guarded = |_: &str| None;
        let module = PredeclaredModule::new(vec!["pre".to_owned()], &guarded);
        program_doc_comments(&parsed, &module, options)
    }

    fn texts(p: &ProgramDocComments) -> Vec<(String, String)> {
        p.doc_comments_map
            .iter()
            .map(|(k, v)| (k.clone(), v.get_text()))
            .collect()
    }

    // Java: src/test/java/net/starlark/java/syntax/ProgramTest.java:33 ProgramTest.docComments
    #[test]
    fn doc_comments() {
        let program = compile_doc_comments(
            "#: Doc comment for A\n#: multiline\nFOO = 1\nBAR, BAZ = (2, 3)  #: Applies to LHS list\n",
            JavaOptions::DEFAULT,
        );
        assert_eq!(
            texts(&program),
            vec![
                ("FOO".to_owned(), "Doc comment for A\nmultiline".to_owned()),
                ("BAR".to_owned(), "Applies to LHS list".to_owned()),
                ("BAZ".to_owned(), "Applies to LHS list".to_owned()),
            ]
        );
    }

    // Java: src/test/java/net/starlark/java/syntax/ProgramTest.java:50 ProgramTest.docComments_unused
    #[test]
    fn doc_comments_unused() {
        let program = compile_doc_comments(
            "#: Unused - separated by a non-doc comment line\n\
             # Non-doc comment line - not in module.unassignedDocComments()\n\
             A = 1\n\
             \n\
             #: Unused - overridden by trailing doc comment\n\
             B = 2  #: Trailing doc comment for B overrides preceding doc comment block\n\
             \n\
             def func():\n\
             \x20   #: Unused - not a global assignment\n\
             \x20   C = 3\n\
             # Another non-doc comment line - not in module.unassignedDocComments()\n",
            JavaOptions::DEFAULT,
        );
        assert_eq!(
            texts(&program),
            vec![(
                "B".to_owned(),
                "Trailing doc comment for B overrides preceding doc comment block".to_owned()
            )]
        );
        let unused: Vec<&str> = program
            .unused_doc_comment_lines
            .iter()
            .map(|c| c.get_doc_comment_text())
            .collect();
        assert_eq!(
            unused,
            vec![
                "Unused - separated by a non-doc comment line",
                "Unused - overridden by trailing doc comment",
                "Unused - not a global assignment",
            ]
        );
    }

    // Java: src/test/java/net/starlark/java/syntax/ResolverTest.java:648 ResolverTest.testDocComments
    #[test]
    fn test_doc_comments() {
        let options = JavaOptions {
            allow_type_syntax: true,
            ..JavaOptions::DEFAULT
        };
        let program = compile_doc_comments(
            "#: Doc for FOO\n#: multiline\nFOO = 1\n\nBAR, BAZ = (2, 3)  #: Applies to LHS list\n\n\
             #: Applies to var annotation without initialier\nQUX : pre\n\
             QUUX : pre #: And the trailing version...\n",
            options,
        );
        assert_eq!(
            texts(&program),
            vec![
                ("FOO".to_owned(), "Doc for FOO\nmultiline".to_owned()),
                ("BAR".to_owned(), "Applies to LHS list".to_owned()),
                ("BAZ".to_owned(), "Applies to LHS list".to_owned()),
                (
                    "QUX".to_owned(),
                    "Applies to var annotation without initialier".to_owned()
                ),
                ("QUUX".to_owned(), "And the trailing version...".to_owned()),
            ]
        );
    }
}
