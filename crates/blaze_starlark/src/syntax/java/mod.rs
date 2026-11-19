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
    }
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

/// A [`resolver::ResolverModule`] for a fresh module with these predeclared names (Java
/// `Module.withPredeclared` + `Starlark.UNIVERSE`; `Module.resolve`).
// Java: net/starlark/java/eval/Module.java:231 Module.resolve
pub struct PredeclaredModule<'a> {
    /// Predeclared and universal names, in the order Java's `HashSet` of candidates is filled.
    pub names: Vec<String>,
    /// The message for a predeclared name whose `GuardedValue` is not accessible.
    pub guarded: &'a dyn Fn(&str) -> Option<String>,
}

impl resolver::ResolverModule for PredeclaredModule<'_> {
    fn resolve(&self, name: &str) -> Result<resolver::Scope, resolver::Undefined> {
        if let Some(message) = (self.guarded)(name) {
            return Err(resolver::Undefined {
                message,
                candidates: None,
            });
        }
        if self.names.iter().any(|n| n == name) {
            return Ok(resolver::Scope::Predeclared);
        }
        Err(resolver::Undefined {
            message: format!("name '{name}' is not defined"),
            candidates: Some(self.names.clone()),
        })
    }
}
