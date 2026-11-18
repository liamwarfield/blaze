//! The parts of Bazel's Resolver that starlark-rust does not enforce, driven by `FileOptions`.
//!
//! Java: net/starlark/java/syntax/Resolver.java: createBindings (548), bind (1159),
//! toplevelRebinding (1245), globalLocalConflict (1256), checkLoadAfterStatement (1275);
//! net/starlark/java/syntax/Lexer.java:422 (ASCII-only string literals).
//!
//! starlark-rust already rejects: top-level `if`/`for` in .bzl (dialect), loading `_private`
//! names, duplicate names in one load statement, undefined names.

use std::collections::HashMap;

use starlark::codemap::Span;
use starlark::syntax::AstModule;
use starlark::syntax::ast::AstExpr;
use starlark::syntax::ast::AstLiteral;
use starlark::syntax::ast::AstStmt;
use starlark::syntax::ast::ExprP;
use starlark::syntax::ast::StmtP;

use super::location;
use crate::error::Location;
use crate::error::SyntaxError;
use crate::file_kind::FileOptions;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Global,
    FileLocal,
}

struct Binding {
    scope: Scope,
    first: Span,
}

struct Run<'a> {
    ast: &'a AstModule,
    options: FileOptions,
    errors: Vec<SyntaxError>,
    /// Java `toplevel` (globals) and file-local `locals.bindings` of the file block.
    globals: HashMap<String, Binding>,
    locals: HashMap<String, Binding>,
}

/// Runs the checks. Errors are in Java's order for each rule (bindings in statement order, then
/// load placement, then string literals).
pub fn check(ast: &AstModule, options: FileOptions) -> Vec<SyntaxError> {
    let mut run = Run {
        ast,
        options,
        errors: Vec::new(),
        globals: HashMap::new(),
        locals: HashMap::new(),
    };
    let stmts: Vec<&AstStmt> = match &ast.statement().node {
        StmtP::Statements(xs) => xs.iter().collect(),
        _ => vec![ast.statement()],
    };
    for s in &stmts {
        run.create_bindings(s);
    }
    if options.require_load_statements_first {
        run.check_load_after_statement(&stmts);
    }
    if options.string_literals_are_ascii_only {
        for s in &stmts {
            run.ascii_stmt(s);
        }
    }
    run.errors
}

impl Run<'_> {
    fn error(&mut self, span: Span, message: String) {
        self.errors.push(SyntaxError {
            location: location(self.ast, span),
            message,
        });
    }

    // Java: Resolver.createBindings for a top-level statement.
    fn create_bindings(&mut self, s: &AstStmt) {
        match &s.node {
            StmtP::Assign(a) => {
                let mut ids = Vec::new();
                a.lhs
                    .visit_lvalue(|id| ids.push((id.node.ident.clone(), id.span)));
                for (name, span) in ids {
                    self.bind(&name, span, false);
                }
            }
            StmtP::AssignModify(lhs, _, _) => {
                let mut ids = Vec::new();
                lhs.visit_lvalue(|id| ids.push((id.node.ident.clone(), id.span)));
                for (name, span) in ids {
                    self.bind(&name, span, false);
                }
            }
            StmtP::Def(d) => {
                let (name, span) = (d.name.node.ident.clone(), d.name.span);
                self.bind(&name, span, false);
            }
            StmtP::Load(l) => {
                for arg in &l.args {
                    let (name, span) = (arg.local.node.ident.clone(), arg.local.span);
                    self.bind(&name, span, true);
                }
            }
            // `if`/`for` at top level are rejected elsewhere; their bindings are global in Java
            // (createBindingsForBlock), which matters only for BUILD files, where rebinding is
            // allowed anyway.
            StmtP::Statements(xs) => xs.iter().for_each(|x| self.create_bindings(x)),
            _ => {}
        }
    }

    // Java: Resolver.bind (top-level cases)
    fn bind(&mut self, name: &str, span: Span, is_load: bool) {
        if !(is_load && !self.options.load_binds_globally) {
            // Global binding.
            match self.globals.get(name).map(|b| b.first) {
                None => {
                    self.globals.insert(
                        name.to_owned(),
                        Binding {
                            scope: Scope::Global,
                            first: span,
                        },
                    );
                    if let Some(prev) = self.locals.get(name).map(|b| (b.scope, b.first)) {
                        self.global_local_conflict(name, span, Scope::Global, prev);
                    }
                }
                Some(first) => self.toplevel_rebinding(name, span, first),
            }
        } else {
            match self.locals.get(name).map(|b| b.first) {
                None => {
                    self.locals.insert(
                        name.to_owned(),
                        Binding {
                            scope: Scope::FileLocal,
                            first: span,
                        },
                    );
                }
                Some(first) => self.toplevel_rebinding(name, span, first),
            }
            if let Some(prev) = self.globals.get(name).map(|b| (b.scope, b.first)) {
                self.global_local_conflict(name, span, Scope::FileLocal, prev);
            }
        }
    }

    // Java: Resolver.java:1245 toplevelRebinding
    fn toplevel_rebinding(&mut self, name: &str, span: Span, first: Span) {
        if !self.options.allow_toplevel_rebinding {
            self.error(span, format!("'{name}' redeclared at top level"));
            self.error(first, format!("'{name}' previously declared here"));
        }
    }

    // Java: Resolver.java:1256 globalLocalConflict
    fn global_local_conflict(&mut self, name: &str, span: Span, scope: Scope, prev: (Scope, Span)) {
        let qual = |s: Scope| {
            if s == Scope::Global {
                "global"
            } else {
                "file-local"
            }
        };
        self.error(
            span,
            format!("conflicting {} declaration of '{name}'", qual(scope)),
        );
        self.error(
            prev.1,
            format!("'{name}' previously declared as {} here", qual(prev.0)),
        );
    }

    // Java: Resolver.java:1275 checkLoadAfterStatement
    fn check_load_after_statement(&mut self, stmts: &[&AstStmt]) {
        let mut first: Option<Span> = None;
        for s in stmts {
            // Ignore string literals (e.g. docstrings).
            if let StmtP::Expression(e) = &s.node
                && matches!(e.node, ExprP::Literal(AstLiteral::String(_)))
            {
                continue;
            }
            if let StmtP::Load(_) = &s.node {
                let Some(f) = first else { continue };
                self.error(
                    s.span,
                    "load statements must appear before any other statement".to_owned(),
                );
                self.error(f, "\tfirst non-load statement appears here".to_owned());
            }
            if first.is_none() {
                first = Some(s.span);
            }
        }
    }

    // Java: Lexer.java:422 (stringLiteralsAreAsciiOnly)
    fn ascii_stmt(&mut self, s: &AstStmt) {
        if let StmtP::Load(l) = &s.node {
            self.ascii_literal(l.module.span, &l.module.node);
            for a in &l.args {
                self.ascii_literal(a.their.span, &a.their.node);
            }
            return;
        }
        s.visit_expr(|e| self.ascii_expr(e));
        s.visit_stmt(|c| self.ascii_stmt(c));
    }

    fn ascii_expr(&mut self, e: &AstExpr) {
        if let ExprP::Literal(AstLiteral::String(s)) = &e.node {
            self.ascii_literal(e.span, &s.node);
        }
        e.visit_expr(|c| self.ascii_expr(c));
    }

    fn ascii_literal(&mut self, span: Span, value: &str) {
        if value.is_ascii() {
            return;
        }
        let fs = self.ast.file_span(span);
        let raw = fs.source_span();
        let mut loc = Location::of_span_start(&fs);
        let message = match raw.char_indices().find(|(_, c)| !c.is_ascii()) {
            Some((i, _)) => {
                loc.column += raw[..i].chars().count();
                "string literal contains non-ASCII character"
            }
            // Only escapes produce the non-ASCII character.
            None => "octal escape sequence denotes non-ASCII character",
        };
        self.errors.push(SyntaxError {
            location: loc,
            message: message.to_owned(),
        });
    }
}
