//! Bazel's BUILD-file syntax restrictions, as a pass over the starlark-rust AST.
//!
//! Java: com/google/devtools/build/lib/packages/DotBazelFileSyntaxChecker.java (visitor) and
//! PackageFactory.checkBuildSyntax (glob prefetch / generator_name extraction, not ported here).
//! starlark-rust has no dialect switch for these rules, and doing them as an AST pass lets Blaze
//! emit Bazel's exact text.

use starlark::codemap::Span;
use starlark::syntax::AstModule;
use starlark::syntax::ast::ArgumentP;
use starlark::syntax::ast::AstExpr;
use starlark::syntax::ast::AstStmt;
use starlark::syntax::ast::ExprP;
use starlark::syntax::ast::StmtP;

/// One syntax error: 1-based line and column, Bazel message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// Checker configuration (Java constructor arguments).
pub struct Checker<'a> {
    pub where_: &'a str,
    pub can_load_bzl: bool,
    pub allow_literal_star_star_args: bool,
}

impl Checker<'_> {
    /// `checkBuildSyntax`'s configuration.
    pub fn build_files() -> Checker<'static> {
        Checker {
            where_: "BUILD files",
            can_load_bzl: true,
            allow_literal_star_star_args: false,
        }
    }

    /// Java: DotBazelFileSyntaxChecker.check
    pub fn check(&self, ast: &AstModule) -> Vec<SyntaxError> {
        let mut errors = Vec::new();
        self.stmt(ast, ast.statement(), &mut errors);
        errors
    }

    fn error(&self, ast: &AstModule, span: Span, message: String, out: &mut Vec<SyntaxError>) {
        let pos = ast.file_span(span).resolve_span().begin;
        out.push(SyntaxError {
            line: pos.line + 1,
            column: pos.column + 1,
            message,
        });
    }

    fn fn_msg(&self) -> String {
        format!(
            "functions may not be defined in {}{}",
            self.where_,
            if self.can_load_bzl {
                ". You may move the function to a .bzl file and load it."
            } else {
                "."
            }
        )
    }

    fn stmt(&self, ast: &AstModule, s: &AstStmt, out: &mut Vec<SyntaxError>) {
        match &s.node {
            StmtP::Load(_) => {
                if !self.can_load_bzl {
                    self.error(
                        ast,
                        s.span,
                        format!("`load` statements may not be used in {}", self.where_),
                        out,
                    );
                }
            }
            // Pruned: the root error is enough.
            StmtP::Def(_) => self.error(ast, s.span, self.fn_msg(), out),
            StmtP::For(_) => self.error(
                ast,
                s.span,
                format!(
                    "`for` statements are not allowed in {}. You may inline the loop{} or as a last resort use a list comprehension.",
                    self.where_,
                    if self.can_load_bzl {
                        ", move it to a function definition (in a .bzl file),"
                    } else {
                        ""
                    }
                ),
                out,
            ),
            StmtP::If(..) | StmtP::IfElse(..) => {
                self.error(
                    ast,
                    s.span,
                    format!(
                        "`if` statements are not allowed in {}. You may{} use an `if` expression for simple cases.",
                        self.where_,
                        if self.can_load_bzl {
                            " move conditional logic to a function definition (in a .bzl file), or"
                        } else {
                            ""
                        }
                    ),
                    out,
                );
                // Java's NodeVisitor continues into the if statement's children.
                self.children(ast, s, out);
            }
            _ => self.children(ast, s, out),
        }
    }

    /// Visits direct children in source order (`StmtP::visit_expr` would also descend into
    /// nested statements, so compound statements are matched here).
    fn children(&self, ast: &AstModule, s: &AstStmt, out: &mut Vec<SyntaxError>) {
        match &s.node {
            StmtP::Statements(xs) => xs.iter().for_each(|x| self.stmt(ast, x, out)),
            StmtP::If(c, t) => {
                self.expr(ast, c, out);
                self.stmt(ast, t, out);
            }
            StmtP::IfElse(c, b) => {
                self.expr(ast, c, out);
                self.stmt(ast, &b.0, out);
                self.stmt(ast, &b.1, out);
            }
            StmtP::For(f) => {
                self.expr(ast, &f.over, out);
                self.stmt(ast, &f.body, out);
            }
            // Leaf statements: visit_expr yields only their own expressions.
            _ => s.visit_expr(|e| self.expr(ast, e, out)),
        }
    }

    fn expr(&self, ast: &AstModule, e: &AstExpr, out: &mut Vec<SyntaxError>) {
        match &e.node {
            ExprP::Lambda(_) => self.error(ast, e.span, self.fn_msg(), out),
            ExprP::Call(_, args) => {
                for a in &args.args {
                    match &a.node {
                        ArgumentP::KwArgs(v) => {
                            if !self.allow_literal_star_star_args {
                                self.error(
                                    ast,
                                    a.span,
                                    format!(
                                        "**kwargs arguments are not allowed in {}. Pass the arguments in explicitly.",
                                        self.where_
                                    ),
                                    out,
                                );
                            }
                            if !matches!(v.node, ExprP::Dict(_)) {
                                self.error(
                                    ast,
                                    a.span,
                                    format!(
                                        "**kwargs arguments must be a literal dict in {}.",
                                        self.where_
                                    ),
                                    out,
                                );
                            }
                        }
                        ArgumentP::Args(_) => self.error(
                            ast,
                            a.span,
                            format!(
                                "*args arguments are not allowed in {}. Pass the arguments in explicitly.",
                                self.where_
                            ),
                            out,
                        ),
                        _ => {}
                    }
                }
                e.visit_expr(|c| self.expr(ast, c, out));
            }
            _ => e.visit_expr(|c| self.expr(ast, c, out)),
        }
    }
}

/// Kind of Starlark file by name: Some(true) for BUILD files, Some(false) for .bzl.
pub fn file_kind(p: &std::path::Path) -> Option<bool> {
    let name = p.file_name()?.to_str()?;
    if name == "BUILD" || name == "BUILD.bazel" || name.ends_with(".BUILD") {
        Some(true)
    } else if name.ends_with(".bzl") {
        Some(false)
    } else {
        None
    }
}

/// Parses a .bzl or BUILD file with the Bazel dialect; BUILD files also get `Checker`.
pub fn parse_file(p: &std::path::Path) -> Result<(), String> {
    let is_build = file_kind(p).ok_or("not a Starlark file")?;
    let src = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
    let dialect = if is_build {
        crate::dialect::build_dialect()
    } else {
        crate::dialect::bzl_dialect()
    };
    let ast = AstModule::parse(&p.to_string_lossy(), src, &dialect)
        .map_err(|e| crate::harness::bare_message(&e))?;
    if is_build {
        let errs = Checker::build_files().check(&ast);
        if let Some(e) = errs.first() {
            return Err(format!("{}:{}: {}", e.line, e.column, e.message));
        }
    }
    Ok(())
}
