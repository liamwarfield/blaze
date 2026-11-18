//! Java: com/google/devtools/build/lib/packages/DotBazelFileSyntaxChecker.java and
//! com/google/devtools/build/lib/packages/PackageFactory.java:578 PackageFactory.checkBuildSyntax.

use starlark::codemap::Span;
use starlark::syntax::AstModule;
use starlark::syntax::ast::ArgumentP;
use starlark::syntax::ast::AstExpr;
use starlark::syntax::ast::AstLiteral;
use starlark::syntax::ast::AstStmt;
use starlark::syntax::ast::ExprP;
use starlark::syntax::ast::StmtP;
use starlark_syntax::lexer::TokenInt;

use super::BuildFileFacts;
use super::location;
use crate::error::Location;
use crate::error::SyntaxError;

/// Java `DotBazelFileSyntaxChecker` constructor arguments.
#[derive(Debug, Clone, Copy)]
pub struct Checker<'a> {
    /// e.g. "BUILD files".
    pub where_: &'a str,
    pub can_load_bzl: bool,
    pub allow_literal_star_star_args: bool,
}

struct Run<'a, 'b> {
    c: &'b Checker<'a>,
    ast: &'b AstModule,
    errors: Vec<SyntaxError>,
    facts: Option<BuildFileFacts>,
}

impl Checker<'_> {
    /// `PackageFactory.checkBuildSyntax`'s configuration.
    pub const BUILD_FILES: Checker<'static> = Checker {
        where_: "BUILD files",
        can_load_bzl: true,
        allow_literal_star_star_args: false,
    };

    /// Java: DotBazelFileSyntaxChecker.check. With `build_facts`, also performs
    /// `checkBuildSyntax`'s glob/subpackages/generator-name extraction.
    pub fn check(&self, ast: &AstModule, build_facts: bool) -> (Vec<SyntaxError>, BuildFileFacts) {
        let mut run = Run {
            c: self,
            ast,
            errors: Vec::new(),
            facts: build_facts.then(BuildFileFacts::default),
        };
        run.stmt(ast.statement());
        (run.errors, run.facts.unwrap_or_default())
    }
}

impl Run<'_, '_> {
    fn error(&mut self, span: Span, message: String) {
        self.errors.push(SyntaxError {
            location: location(self.ast, span),
            message,
        });
    }

    fn fn_msg(&self) -> String {
        format!(
            "functions may not be defined in {}{}",
            self.c.where_,
            if self.c.can_load_bzl {
                ". You may move the function to a .bzl file and load it."
            } else {
                "."
            }
        )
    }

    fn stmt(&mut self, s: &AstStmt) {
        match &s.node {
            // Java: visit(LoadStatement)
            StmtP::Load(_) => {
                if !self.c.can_load_bzl {
                    self.error(
                        s.span,
                        format!("`load` statements may not be used in {}", self.c.where_),
                    );
                }
            }
            // Java: visit(DefStatement); traversal pruned.
            StmtP::Def(_) => {
                let m = self.fn_msg();
                self.error(s.span, m);
            }
            // Java: visit(ForStatement); traversal pruned.
            StmtP::For(_) => {
                let m = format!(
                    "`for` statements are not allowed in {}. You may inline the loop{} or as a last resort use a list comprehension.",
                    self.c.where_,
                    if self.c.can_load_bzl {
                        ", move it to a function definition (in a .bzl file),"
                    } else {
                        ""
                    }
                );
                self.error(s.span, m);
            }
            // Java: visit(IfStatement); traversal pruned.
            StmtP::If(..) | StmtP::IfElse(..) => {
                let m = format!(
                    "`if` statements are not allowed in {}. You may{} use an `if` expression for simple cases.",
                    self.c.where_,
                    if self.c.can_load_bzl {
                        " move conditional logic to a function definition (in a .bzl file), or"
                    } else {
                        ""
                    }
                );
                self.error(s.span, m);
            }
            StmtP::Statements(xs) => xs.iter().for_each(|x| self.stmt(x)),
            // Leaf statements: `visit_expr` yields their own expressions only.
            _ => s.visit_expr(|e| self.expr(e)),
        }
    }

    fn expr(&mut self, e: &AstExpr) {
        match &e.node {
            // Java: visit(LambdaExpression); traversal pruned.
            ExprP::Lambda(_) => {
                let m = self.fn_msg();
                self.error(e.span, m);
            }
            ExprP::Call(f, args) => {
                // Java: visit(CallExpression) -> rejectStarArgs, then (BUILD) glob extraction and
                // generator names, then continue into nested calls.
                for a in &args.args {
                    match &a.node {
                        ArgumentP::KwArgs(v) => {
                            if !self.c.allow_literal_star_star_args {
                                let m = format!(
                                    "**kwargs arguments are not allowed in {}. Pass the arguments in explicitly.",
                                    self.c.where_
                                );
                                self.error(a.span, m);
                            }
                            if !matches!(v.node, ExprP::Dict(_)) {
                                let m = format!(
                                    "**kwargs arguments must be a literal dict in {}.",
                                    self.c.where_
                                );
                                self.error(a.span, m);
                            }
                        }
                        ArgumentP::Args(_) => {
                            let m = format!(
                                "*args arguments are not allowed in {}. Pass the arguments in explicitly.",
                                self.c.where_
                            );
                            self.error(a.span, m);
                        }
                        _ => {}
                    }
                }
                if self.facts.is_some() {
                    self.extract_glob_patterns(f, args);
                    self.record_generator_name(e);
                }
                e.visit_expr(|c| self.expr(c));
            }
            _ => e.visit_expr(|c| self.expr(c)),
        }
    }

    // Java: PackageFactory.java:592 extractGlobPatterns
    fn extract_glob_patterns(
        &mut self,
        f: &AstExpr,
        args: &starlark::syntax::ast::CallArgsP<starlark::syntax::ast::AstNoPayload>,
    ) {
        let ExprP::Identifier(id) = &f.node else {
            return;
        };
        let function_name = id.node.ident.as_str();
        if function_name != "glob" && function_name != "subpackages" {
            return;
        }
        let mut exclude_directories: Option<&AstExpr> = None;
        let mut include: Option<&AstExpr> = None;
        for (i, arg) in args.args.iter().enumerate() {
            match &arg.node {
                ArgumentP::Positional(v) => {
                    if i == 0 {
                        include = Some(v);
                    }
                }
                ArgumentP::Named(name, v) => {
                    if name.node == "include" {
                        include = Some(v);
                    } else if name.node == "exclude_directories" {
                        exclude_directories = Some(v);
                    }
                }
                _ => {}
            }
        }
        let Some(include) = include else { return };
        let ExprP::List(elems) = &include.node else {
            return;
        };
        let facts = self.facts.as_mut().unwrap();
        for elem in elems {
            if let ExprP::Literal(AstLiteral::String(pattern)) = &elem.node {
                // exclude_directories is (oddly) an int with default 1.
                let mut exclude = true;
                if let Some(ExprP::Literal(AstLiteral::Int(v))) =
                    exclude_directories.map(|e| &e.node)
                    && matches!(v.node, TokenInt::I32(0))
                {
                    exclude = false;
                }
                let p = pattern.node.clone();
                if function_name == "glob" {
                    if exclude {
                        facts.globs.push(p);
                    } else {
                        facts.globs_with_dirs.push(p);
                    }
                } else {
                    facts.subpackages.push(p);
                }
            }
        }
    }

    // Java: PackageFactory.java:641 recordGeneratorName
    fn record_generator_name(&mut self, call: &AstExpr) {
        let ExprP::Call(_, args) = &call.node else {
            return;
        };
        for arg in &args.args {
            if let ArgumentP::Named(name, v) = &arg.node
                && name.node == "name"
                && let ExprP::Literal(AstLiteral::String(s)) = &v.node
            {
                let loc: Location = Location::bazel_position(&self.ast.file_span(call.span));
                self.facts
                    .as_mut()
                    .unwrap()
                    .generator_name_by_location
                    .push((loc, s.node.clone()));
            }
        }
    }
}
