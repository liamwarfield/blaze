//! Port of the error-reporting part of Bazel's Starlark Resolver: every static error of a file, in
//! Java's order (load placement, top-level bindings, then a walk of the statements).
//!
//! Java: net/starlark/java/syntax/Resolver.java (resolveFile, createBindings, bind, use,
//! resolveFunction and the visit methods), NodeVisitor.java (the default traversal order).
//!
//! PORT: the binding indices, frames and cell/free-variable bookkeeping Java computes for the
//! evaluator are not needed for the errors and are not kept. `resolveTypeSyntax` (type
//! checking, off in Bazel unless `--experimental_starlark_type_checking`) is not ported: type
//! expressions are not resolved, as in Bazel's default.

use starlark_map::small_map::SmallMap;
use std::collections::HashSet;

use super::JavaOptions;
use super::ast::*;
use super::lexer::OffsetError;
use super::lexer::TokenKind;

/// Java `Resolver.Scope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Local,
    Global,
    Predeclared,
    Universal,
}

/// Java `Resolver.Module.Undefined`.
pub struct Undefined {
    pub message: String,
    /// `None`: no spelling suggestions (e.g. a guarded symbol).
    pub candidates: Option<Vec<String>>,
}

/// Java `Resolver.Module`: how the file's free names resolve.
pub trait ResolverModule {
    fn resolve(&self, name: &str) -> Result<Scope, Undefined>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    File,
    Function,
    Comprehension,
}

struct Binding {
    scope: Scope,
    is_syntactic: bool,
    first_start: usize,
}

struct Block<'t> {
    kind: BlockKind,
    /// Java's `HashMap`; kept in insertion order so that its Java iteration order can be
    /// computed (`java_hash_set_order`), see `get_all_symbols`.
    bindings: SmallMap<&'t str, usize>,
}

// Java: net/starlark/java/syntax/Resolver.java:33 Resolver
struct Resolver<'m, 't> {
    errors: Vec<OffsetError>,
    options: JavaOptions,
    module: &'m dyn ResolverModule,
    bindings: Vec<Binding>,
    /// Java's `HashMap`, in insertion order (see `Block::bindings`).
    toplevel: SmallMap<&'t str, usize>,
    /// Innermost last.
    blocks: Vec<Block<'t>>,
    loop_count: usize,
    /// Java `docCommentsMap`: the doc comments of each new global binding, in binding order.
    doc_comments_map: SmallMap<String, super::ast::DocComments>,
}

/// Resolves the file `stmts` against `module` and returns the errors in Java's order.
// Java: net/starlark/java/syntax/Resolver.java:1323 Resolver.resolveFile
pub fn resolve_file(
    stmts: &[Stmt],
    module: &dyn ResolverModule,
    options: JavaOptions,
) -> Vec<OffsetError> {
    resolve_file_with_doc_comments(stmts, module, options).0
}

/// [`resolve_file`], also returning the doc comments of the file's new global bindings (Java's
/// `StarlarkFile.docCommentsMap`, which `resolveFile` fills).
// Java: net/starlark/java/syntax/Resolver.java:1323 Resolver.resolveFile
pub fn resolve_file_with_doc_comments(
    stmts: &[Stmt],
    module: &dyn ResolverModule,
    options: JavaOptions,
) -> (Vec<OffsetError>, SmallMap<String, super::ast::DocComments>) {
    let mut r = Resolver {
        doc_comments_map: SmallMap::new(),
        errors: Vec::new(),
        options,
        module,
        bindings: Vec::new(),
        toplevel: SmallMap::new(),
        blocks: Vec::new(),
        loop_count: 0,
    };
    if options.require_load_statements_first {
        r.check_load_after_statement(stmts);
    }
    r.push_block(BlockKind::File);
    r.create_bindings_for_block(stmts);
    for s in stmts {
        r.visit_stmt(s);
    }
    r.blocks.pop();
    (r.errors, r.doc_comments_map)
}

impl<'t> Resolver<'_, 't> {
    fn errorf(&mut self, offset: usize, message: String) {
        self.errors.push(OffsetError { offset, message });
    }

    fn push_block(&mut self, kind: BlockKind) {
        self.blocks.push(Block {
            kind,
            bindings: SmallMap::new(),
        });
    }

    fn locals(&self) -> &Block<'t> {
        self.blocks.last().unwrap()
    }

    fn at_file(&self) -> bool {
        self.locals().kind == BlockKind::File
    }

    fn create_bindings_for_block(&mut self, stmts: &'t [Stmt]) {
        for s in stmts {
            self.create_bindings(s);
        }
    }

    // Java: net/starlark/java/syntax/Resolver.java:549 Resolver.createBindings
    fn create_bindings(&mut self, stmt: &'t Stmt) {
        match stmt {
            Stmt::Assign {
                lhs,
                ty,
                doc_comments,
                ..
            } => {
                if ty.is_some() {
                    if let Expr::Ident(id) = lhs {
                        self.bind_with_doc_comments(id, false, true, doc_comments.as_ref());
                    }
                } else {
                    self.create_bindings_for_lhs_with_doc_comments(lhs, doc_comments.as_ref());
                }
            }
            Stmt::Var {
                name, doc_comments, ..
            } => {
                self.bind_with_doc_comments(name, false, true, doc_comments.as_ref());
            }
            Stmt::If { then, els, .. } => {
                self.create_bindings_for_block(then);
                if let Some(e) = els {
                    self.create_bindings_for_block(e);
                }
            }
            Stmt::For { vars, body, .. } => {
                self.create_bindings_for_lhs(vars);
                self.create_bindings_for_block(body);
            }
            Stmt::Def {
                name,
                type_params,
                params,
                ret,
                ..
            } => {
                let has_type = params.iter().any(|p| p.ty().is_some())
                    || ret.is_some()
                    || !type_params.is_empty();
                self.bind(name, false, has_type);
            }
            Stmt::Load { bindings, .. } => {
                let mut names = HashSet::new();
                for (local, orig) in bindings {
                    if orig.name.starts_with('_') && !self.options.allow_load_private_symbols {
                        self.errorf(
                            orig.start,
                            format!("symbol '{}' is private and cannot be imported", orig.name),
                        );
                    }
                    if names.insert(local.name.as_str()) {
                        self.bind(local, true, false);
                    } else {
                        self.errorf(
                            local.start,
                            format!("load statement defines '{}' more than once", local.name),
                        );
                    }
                }
            }
            // TYPE_ALIAS binds only when resolveTypeSyntax (not ported, off by default).
            Stmt::TypeAlias { .. } | Stmt::Expr(_) | Stmt::Flow { .. } | Stmt::Return { .. } => {}
        }
    }

    // Java: net/starlark/java/syntax/Identifier.java:116 Identifier.boundIdentifiers
    fn create_bindings_for_lhs(&mut self, lhs: &'t Expr) {
        self.create_bindings_for_lhs_with_doc_comments(lhs, None);
    }

    // Java: net/starlark/java/syntax/Resolver.java:595 Resolver.createBindingsForLHS
    fn create_bindings_for_lhs_with_doc_comments(
        &mut self,
        lhs: &'t Expr,
        doc_comments: Option<&super::ast::DocComments>,
    ) {
        let mut ids = Vec::new();
        collect_bound_identifiers(lhs, &mut ids);
        for id in ids {
            self.bind_with_doc_comments(&id, false, false, doc_comments);
        }
    }

    // Java: net/starlark/java/syntax/Resolver.java:1157 Resolver.bind
    fn bind(&mut self, id: &'t Ident, is_load: bool, has_type: bool) -> bool {
        self.bind_with_doc_comments(id, is_load, has_type, None)
    }

    // Java: net/starlark/java/syntax/Resolver.java:1157 Resolver.bind
    fn bind_with_doc_comments(
        &mut self,
        id: &'t Ident,
        is_load: bool,
        has_type: bool,
        doc_comments: Option<&super::ast::DocComments>,
    ) -> bool {
        let name: &'t str = &id.name;
        let mut is_new = false;
        let bind;
        if self.at_file() && !(is_load && !self.options.load_binds_globally) {
            match self.toplevel.get(&name) {
                None => {
                    is_new = true;
                    bind = self.bindings.len();
                    self.bindings.push(Binding {
                        scope: Scope::Global,
                        is_syntactic: true,
                        first_start: id.start,
                    });
                    if let Some(doc_comments) = doc_comments {
                        self.doc_comments_map
                            .insert(name.to_owned(), doc_comments.clone());
                    }
                    self.toplevel.insert(name, bind);
                    if let Some(&prev_local) = self.locals().bindings.get(&name) {
                        self.global_local_conflict(id, Scope::Global, prev_local);
                    }
                }
                Some(&b) => {
                    bind = b;
                    self.toplevel_rebinding(id, b);
                }
            }
        } else {
            match self.locals().bindings.get(&name) {
                None => {
                    is_new = true;
                    bind = self.bindings.len();
                    self.bindings.push(Binding {
                        scope: Scope::Local,
                        is_syntactic: true,
                        first_start: id.start,
                    });
                    self.blocks.last_mut().unwrap().bindings.insert(name, bind);
                }
                Some(&b) => bind = b,
            }
            if is_load {
                if !is_new {
                    self.toplevel_rebinding(id, bind);
                }
                if let Some(&prev) = self.toplevel.get(&name) {
                    if self.bindings[prev].scope == Scope::Global {
                        let scope = self.bindings[bind].scope;
                        self.global_local_conflict(id, scope, prev);
                    }
                }
            }
        }
        if has_type && !is_new {
            if self.bindings[bind].is_syntactic {
                self.errorf(
                    id.start,
                    format!(
                        "type annotation on '{}' may only appear at its first declaration",
                        id.name
                    ),
                );
                let first = self.bindings[bind].first_start;
                self.errorf(first, format!("'{}' first declared here", id.name));
            } else {
                self.errorf(
                    id.start,
                    format!("symbol '{}' cannot be annotated with a type", id.name),
                );
            }
        }
        is_new
    }

    fn toplevel_rebinding(&mut self, id: &'t Ident, prev: usize) {
        if !self.options.allow_toplevel_rebinding {
            self.errorf(id.start, format!("'{}' redeclared at top level", id.name));
            if self.bindings[prev].is_syntactic {
                let first = self.bindings[prev].first_start;
                self.errorf(first, format!("'{}' previously declared here", id.name));
            }
        }
    }

    fn global_local_conflict(&mut self, id: &'t Ident, scope: Scope, prev: usize) {
        let qual = |s: Scope| {
            if s == Scope::Global {
                "global"
            } else {
                "file-local"
            }
        };
        let newqual = qual(scope);
        let oldqual = qual(self.bindings[prev].scope);
        self.errorf(
            id.start,
            format!("conflicting {newqual} declaration of '{}'", id.name),
        );
        if self.bindings[prev].is_syntactic {
            let first = self.bindings[prev].first_start;
            self.errorf(
                first,
                format!("'{}' previously declared as {oldqual} here", id.name),
            );
        }
    }

    // Java: net/starlark/java/syntax/Resolver.java:1277 Resolver.checkLoadAfterStatement
    fn check_load_after_statement(&mut self, stmts: &'t [Stmt]) {
        let mut first: Option<usize> = None;
        for s in stmts {
            if let Stmt::Expr(Expr::Str { .. }) = s {
                continue;
            }
            if let Stmt::Load { start, .. } = s {
                let Some(f) = first else { continue };
                self.errorf(
                    *start,
                    "load statements must appear before any other statement".to_owned(),
                );
                self.errorf(f, "\tfirst non-load statement appears here".to_owned());
            }
            if first.is_none() {
                first = Some(s.start());
            }
        }
    }

    // Java: net/starlark/java/syntax/Resolver.java:378 Resolver.assign
    fn assign(&mut self, lhs: &'t Expr) {
        match lhs {
            Expr::Ident(_) => {}
            Expr::Index { .. } => self.visit_expr(lhs),
            Expr::List { elems, .. } => {
                for e in elems {
                    self.assign(e);
                }
            }
            Expr::Dot { obj, .. } => self.visit_expr(obj),
            _ => self.errorf(
                lhs.start(),
                format!("cannot assign to '{}'", expr_to_string(lhs)),
            ),
        }
    }

    // Java: net/starlark/java/syntax/Resolver.java:922 Resolver.use
    fn use_ident(&mut self, id: &'t Ident) {
        let name = &id.name;
        for b in self.blocks.iter().rev() {
            if b.bindings.contains_key(name.as_str()) {
                return;
            }
        }
        if self.toplevel.contains_key(name.as_str()) {
            return;
        }
        match self.module.resolve(name) {
            Ok(scope) => {
                let bind = self.bindings.len();
                self.bindings.push(Binding {
                    scope,
                    is_syntactic: false,
                    first_start: id.start,
                });
                self.toplevel.insert(name, bind);
            }
            Err(ex) => {
                if !is_valid_identifier(name) {
                    self.errorf(id.start, "contains syntax errors".to_owned());
                } else if let Some(candidates) = ex.candidates {
                    let all = self.get_all_symbols(candidates);
                    let suggestion = starlark::errors::spell_checker::did_you_mean(
                        name,
                        all.iter().map(String::as_str),
                    );
                    self.errorf(id.start, format!("{}{}", ex.message, suggestion));
                } else {
                    self.errorf(id.start, ex.message);
                }
            }
        }
    }

    // Java: net/starlark/java/syntax/Resolver.java:1266 Resolver.getAllSymbols
    // Each source is a Java HashMap/HashSet whose iteration order (filled in insertion order)
    // decides the insertion order into `all`, a HashSet, whose own order is the result.
    fn get_all_symbols(&self, predeclared: Vec<String>) -> Vec<String> {
        use starlark::errors::spell_checker::java_hash_set_order;
        let mut words = Vec::new();
        for b in self.blocks.iter().rev() {
            words.extend(java_hash_set_order(
                b.bindings.keys().map(|k| (*k).to_owned()),
            ));
        }
        // Module.resolve's `candidates`, a HashSet filled in this order (Module.java:252).
        words.extend(java_hash_set_order(predeclared));
        words.extend(java_hash_set_order(
            self.toplevel.keys().map(|k| (*k).to_owned()),
        ));
        java_hash_set_order(words)
    }

    fn visit_block(&mut self, stmts: &'t [Stmt]) {
        for s in stmts {
            self.visit_stmt(s);
        }
    }

    fn visit_stmt(&mut self, s: &'t Stmt) {
        match s {
            // Java: Resolver.visit(AssignmentStatement)
            Stmt::Assign {
                lhs,
                op,
                op_offset,
                rhs,
                ..
            } => {
                self.visit_expr(rhs);
                if op.is_some() && matches!(lhs, Expr::List { .. }) {
                    self.errorf(
                        *op_offset,
                        "cannot perform augmented assignment on a list or tuple expression"
                            .to_owned(),
                    );
                }
                self.assign(lhs);
            }
            Stmt::Expr(e) => self.visit_expr(e),
            // Java: Resolver.visit(IfStatement)
            Stmt::If {
                start,
                cond,
                then,
                els,
            } => {
                if self.at_file() {
                    self.errorf(
                        *start,
                        "if statements are not allowed at the top level. You may move it inside a function or use an if expression (x if condition else y).".to_owned(),
                    );
                }
                self.visit_expr(cond);
                self.visit_block(then);
                if let Some(e) = els {
                    self.visit_block(e);
                }
            }
            // Java: Resolver.visit(ForStatement)
            Stmt::For {
                start,
                vars,
                collection,
                body,
            } => {
                if self.at_file() {
                    self.errorf(
                        *start,
                        "for loops are not allowed at the top level. You may move it inside a function or use a comprehension, [f(x) for x in sequence]".to_owned(),
                    );
                }
                self.loop_count += 1;
                self.visit_expr(collection);
                self.assign(vars);
                self.visit_block(body);
                self.loop_count -= 1;
            }
            // Java: Resolver.visit(DefStatement)
            Stmt::Def {
                params, ret, body, ..
            } => {
                let _ = ret;
                self.resolve_function(params, body, None);
            }
            // Java: Resolver.visit(ReturnStatement)
            Stmt::Return { start, result } => {
                if self.at_file() {
                    self.errorf(
                        *start,
                        "return statements must be inside a function".to_owned(),
                    );
                }
                if let Some(r) = result {
                    self.visit_expr(r);
                }
            }
            // Java: Resolver.visit(LoadStatement)
            Stmt::Load { start, .. } => {
                if !self.at_file() {
                    self.errorf(*start, "load statement not at top level".to_owned());
                }
            }
            // Java: Resolver.visit(FlowStatement)
            Stmt::Flow { kind, start } => {
                if *kind != TokenKind::Pass && self.loop_count == 0 {
                    self.errorf(
                        *start,
                        format!("{kind} statement must be inside a for loop"),
                    );
                }
            }
            // Java: Resolver.visit(TypeAliasStatement)
            Stmt::TypeAlias { start, .. } => {
                if !self.at_file() {
                    self.errorf(*start, "type alias statement not at top level".to_owned());
                }
            }
            Stmt::Var { .. } => {}
        }
    }

    // Java: net/starlark/java/syntax/Resolver.java:1016 Resolver.resolveFunction
    fn resolve_function(
        &mut self,
        params: &'t [Parameter],
        body: &'t [Stmt],
        lambda_body: Option<&'t Expr>,
    ) {
        for p in params {
            if let Parameter::Optional(_, _, d) = p {
                self.visit_expr(d);
            }
        }
        self.push_block(BlockKind::Function);
        let mut star: Option<&Parameter> = None;
        let mut star_star: Option<&Ident> = None;
        let mut seen_optional = false;
        let mut num_keyword_only = 0;
        for p in params {
            match p {
                Parameter::Mandatory(id, _) => {
                    if let Some(ss) = star_star {
                        self.errorf(
                            p.start(),
                            format!(
                                "required parameter {} may not follow **{}",
                                id.name, ss.name
                            ),
                        );
                    } else if star.is_some() {
                        num_keyword_only += 1;
                    } else if seen_optional {
                        self.errorf(
                            p.start(),
                            format!(
                                "required positional parameter {} may not follow an optional parameter",
                                id.name
                            ),
                        );
                    }
                    self.bind_param(p);
                }
                Parameter::Optional(..) => {
                    seen_optional = true;
                    if let Some(ss) = star_star {
                        self.errorf(
                            p.start(),
                            format!("optional parameter may not follow **{}", ss.name),
                        );
                    } else if star.is_some() {
                        num_keyword_only += 1;
                    }
                    self.bind_param(p);
                }
                Parameter::Star(..) => {
                    if let Some(ss) = star_star {
                        self.errorf(
                            p.start(),
                            format!("* parameter may not follow **{}", ss.name),
                        );
                    } else if star.is_some() {
                        self.errorf(p.start(), "multiple * parameters not allowed".to_owned());
                    } else {
                        star = Some(p);
                    }
                }
                Parameter::StarStar(_, id, _) => {
                    if star_star.is_some() {
                        self.errorf(p.start(), "multiple ** parameters not allowed".to_owned());
                    }
                    star_star = Some(id);
                }
            }
        }
        if let Some(s) = star {
            if s.ident().is_some() {
                self.bind_param(s);
            } else if num_keyword_only == 0 {
                self.errorf(
                    s.start(),
                    "bare * must be followed by keyword-only parameters".to_owned(),
                );
            }
        }
        if star_star.is_some() {
            // The last `**` parameter (Java keeps the last one seen).
            if let Some(p) = params
                .iter()
                .rev()
                .find(|p| matches!(p, Parameter::StarStar(..)))
            {
                self.bind_param(p);
            }
        }
        match lambda_body {
            Some(e) => {
                // Java: a lambda's body is `return <expr>` (ReturnStatement.make).
                self.visit_expr(e);
            }
            None => {
                self.create_bindings_for_block(body);
                self.visit_block(body);
            }
        }
        self.blocks.pop();
    }

    // Java: net/starlark/java/syntax/Resolver.java:1137 Resolver.bindParam
    fn bind_param(&mut self, p: &'t Parameter) {
        let Some(id) = p.ident() else { return };
        if !self.bind(&id, false, false) {
            self.errorf(p.start(), format!("duplicate parameter: {}", id.name));
        }
    }

    fn visit_expr(&mut self, e: &'t Expr) {
        match e {
            Expr::Ident(id) => self.use_ident(id),
            Expr::Int { .. } | Expr::Float { .. } | Expr::Str { .. } | Expr::Ellipsis { .. } => {}
            Expr::List { elems, .. } => {
                for x in elems {
                    self.visit_expr(x);
                }
            }
            Expr::Dict { entries, .. } => {
                for (k, v) in entries {
                    self.visit_expr(k);
                    self.visit_expr(v);
                }
            }
            // Java: Resolver.visit(Comprehension)
            Expr::Comprehension { body, clauses, .. } => {
                let Some(Clause::For { iterable, .. }) = clauses.first() else {
                    return;
                };
                self.visit_expr(iterable);
                self.push_block(BlockKind::Comprehension);
                for c in clauses {
                    if let Clause::For { vars, .. } = c {
                        self.create_bindings_for_lhs(vars);
                    }
                }
                for (i, c) in clauses.iter().enumerate() {
                    match c {
                        Clause::For { vars, iterable } => {
                            if i > 0 {
                                self.visit_expr(iterable);
                            }
                            self.assign(vars);
                        }
                        Clause::If { cond } => self.visit_expr(cond),
                    }
                }
                match &**body {
                    CompBody::Expr(x) => self.visit_expr(x),
                    CompBody::Entry(k, v) => {
                        self.visit_expr(k);
                        self.visit_expr(v);
                    }
                }
                self.blocks.pop();
            }
            // Java: Resolver.visit(CallExpression)
            Expr::Call { func, args } => {
                let mut seen_varargs = false;
                let mut seen_kwargs = false;
                let mut keywords: Option<HashSet<&str>> = None;
                for a in args {
                    match a {
                        Argument::Positional(_) => {
                            if seen_varargs {
                                self.errorf(
                                    a.start(),
                                    "positional argument may not follow *args".to_owned(),
                                );
                            } else if seen_kwargs {
                                self.errorf(
                                    a.start(),
                                    "positional argument may not follow **kwargs".to_owned(),
                                );
                            } else if keywords.is_some() {
                                self.errorf(
                                    a.start(),
                                    "positional argument may not follow keyword argument"
                                        .to_owned(),
                                );
                            }
                        }
                        Argument::Keyword(id, _) => {
                            if seen_varargs {
                                self.errorf(
                                    a.start(),
                                    format!("keyword argument {} may not follow *args", id.name),
                                );
                            } else if seen_kwargs {
                                self.errorf(
                                    a.start(),
                                    format!("keyword argument {} may not follow **kwargs", id.name),
                                );
                            }
                            if !keywords
                                .get_or_insert_with(HashSet::new)
                                .insert(id.name.as_str())
                            {
                                self.errorf(
                                    a.start(),
                                    format!("duplicate keyword argument: {}", id.name),
                                );
                            }
                        }
                        Argument::Star(..) => {
                            if seen_kwargs {
                                self.errorf(a.start(), "*args may not follow **kwargs".to_owned());
                            } else if seen_varargs {
                                self.errorf(a.start(), "multiple *args not allowed".to_owned());
                            }
                            seen_varargs = true;
                        }
                        Argument::StarStar(..) => {
                            if seen_kwargs {
                                self.errorf(a.start(), "multiple **kwargs not allowed".to_owned());
                            }
                            seen_kwargs = true;
                        }
                    }
                }
                self.visit_expr(func);
                for a in args {
                    self.visit_expr(a.value());
                }
            }
            Expr::Dot { obj, .. } => self.visit_expr(obj),
            Expr::Index { obj, key } => {
                self.visit_expr(obj);
                self.visit_expr(key);
            }
            Expr::Slice {
                obj,
                start,
                stop,
                step,
            } => {
                self.visit_expr(obj);
                for x in [start, stop, step].into_iter().flatten() {
                    self.visit_expr(x);
                }
            }
            Expr::Binary { x, y, .. } => {
                self.visit_expr(x);
                self.visit_expr(y);
            }
            Expr::Unary { x, .. } => self.visit_expr(x),
            Expr::Conditional { then, cond, els } => {
                self.visit_expr(then);
                self.visit_expr(cond);
                self.visit_expr(els);
            }
            // Java: Resolver.visit(LambdaExpression)
            Expr::Lambda { params, body, .. } => {
                self.resolve_function(params, &[], Some(body));
            }
            // Java: Resolver.visit(CastExpression): the type only when resolveTypeSyntax.
            Expr::Cast { value, .. } => self.visit_expr(value),
            // Java: Resolver.visit(IsInstanceExpression)
            Expr::IsInstance { start, .. } => {
                self.errorf(*start, "isinstance() is not yet supported".to_owned());
            }
            Expr::TypeApplication { constructor, args } => {
                self.use_ident(constructor);
                for a in args {
                    self.visit_expr(a);
                }
            }
        }
    }
}

fn collect_bound_identifiers<'t>(lhs: &'t Expr, out: &mut Vec<&'t Ident>) {
    match lhs {
        Expr::Ident(id) => out.push(id),
        Expr::List { elems, .. } => {
            for e in elems {
                collect_bound_identifiers(e, out);
            }
        }
        _ => {}
    }
}
