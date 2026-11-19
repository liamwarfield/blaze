//! `DotBazelFileSyntaxChecker`'s errors on Java's syntax tree, so that a BUILD file's errors come
//! in Bazel's order (parse errors, then these, then the resolver's), also for files that
//! starlark-rust's parser rejects. The glob/generator-name extraction stays in
//! [`crate::syntax::dot_bazel`].
//!
//! Java: com/google/devtools/build/lib/packages/DotBazelFileSyntaxChecker.java, NodeVisitor.java
//! (traversal order).

use super::ast::*;
use super::lexer::OffsetError;
use crate::syntax::dot_bazel::Checker;

/// Java `DotBazelFileSyntaxChecker.check`: the errors, in Java's order.
// Java: com/google/devtools/build/lib/packages/DotBazelFileSyntaxChecker.java
pub fn check(c: &Checker<'_>, stmts: &[Stmt]) -> Vec<OffsetError> {
    let mut r = Run {
        c,
        errors: Vec::new(),
    };
    for s in stmts {
        r.stmt(s);
    }
    r.errors
}

struct Run<'a, 'b> {
    c: &'b Checker<'a>,
    errors: Vec<OffsetError>,
}

impl Run<'_, '_> {
    fn error(&mut self, offset: usize, message: String) {
        self.errors.push(OffsetError { offset, message });
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

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Load { start, .. } => {
                if !self.c.can_load_bzl {
                    let m = format!("`load` statements may not be used in {}", self.c.where_);
                    self.error(*start, m);
                }
            }
            Stmt::Def { start, .. } => {
                let m = self.fn_msg();
                self.error(*start, m);
            }
            Stmt::For { start, .. } => {
                let m = format!(
                    "`for` statements are not allowed in {}. You may inline the loop{} or as a last resort use a list comprehension.",
                    self.c.where_,
                    if self.c.can_load_bzl {
                        ", move it to a function definition (in a .bzl file),"
                    } else {
                        ""
                    }
                );
                self.error(*start, m);
            }
            Stmt::If { start, .. } => {
                let m = format!(
                    "`if` statements are not allowed in {}. You may{} use an `if` expression for simple cases.",
                    self.c.where_,
                    if self.c.can_load_bzl {
                        " move conditional logic to a function definition (in a .bzl file), or"
                    } else {
                        ""
                    }
                );
                self.error(*start, m);
            }
            Stmt::Assign { lhs, ty, rhs, .. } => {
                self.expr(lhs);
                if let Some(t) = ty {
                    self.expr(t);
                }
                self.expr(rhs);
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::Return { result, .. } => {
                if let Some(r) = result {
                    self.expr(r);
                }
            }
            Stmt::Flow { .. } => {}
            Stmt::TypeAlias { def, .. } => self.expr(def),
            Stmt::Var { ty, .. } => self.expr(ty),
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Lambda { start, .. } => {
                let m = self.fn_msg();
                self.error(*start, m);
            }
            Expr::Call { func, args } => {
                for a in args {
                    match a {
                        Argument::StarStar(o, v) => {
                            if !self.c.allow_literal_star_star_args {
                                let m = format!(
                                    "**kwargs arguments are not allowed in {}. Pass the arguments in explicitly.",
                                    self.c.where_
                                );
                                self.error(*o, m);
                            }
                            if !matches!(v, Expr::Dict { .. }) {
                                let m = format!(
                                    "**kwargs arguments must be a literal dict in {}.",
                                    self.c.where_
                                );
                                self.error(*o, m);
                            }
                        }
                        Argument::Star(o, _) => {
                            let m = format!(
                                "*args arguments are not allowed in {}. Pass the arguments in explicitly.",
                                self.c.where_
                            );
                            self.error(*o, m);
                        }
                        _ => {}
                    }
                }
                self.expr(func);
                for a in args {
                    self.expr(a.value());
                }
            }
            Expr::Ident(_)
            | Expr::Int { .. }
            | Expr::Float { .. }
            | Expr::Str { .. }
            | Expr::Ellipsis { .. } => {}
            Expr::List { elems, .. } => elems.iter().for_each(|x| self.expr(x)),
            Expr::Dict { entries, .. } => {
                for (k, v) in entries {
                    self.expr(k);
                    self.expr(v);
                }
            }
            Expr::Comprehension { body, clauses, .. } => {
                match &**body {
                    CompBody::Expr(x) => self.expr(x),
                    CompBody::Entry(k, v) => {
                        self.expr(k);
                        self.expr(v);
                    }
                }
                for c in clauses {
                    match c {
                        Clause::For { vars, iterable } => {
                            self.expr(vars);
                            self.expr(iterable);
                        }
                        Clause::If { cond } => self.expr(cond),
                    }
                }
            }
            Expr::Dot { obj, .. } => self.expr(obj),
            Expr::Index { obj, key } => {
                self.expr(obj);
                self.expr(key);
            }
            Expr::Slice {
                obj,
                start,
                stop,
                step,
            } => {
                self.expr(obj);
                for x in [start, stop, step].into_iter().flatten() {
                    self.expr(x);
                }
            }
            Expr::Binary { x, y, .. } => {
                self.expr(x);
                self.expr(y);
            }
            Expr::Unary { x, .. } => self.expr(x),
            Expr::Conditional { then, cond, els } => {
                self.expr(then);
                self.expr(cond);
                self.expr(els);
            }
            Expr::Cast { ty, value, .. } => {
                self.expr(ty);
                self.expr(value);
            }
            Expr::IsInstance { value, ty, .. } => {
                self.expr(value);
                self.expr(ty);
            }
            Expr::TypeApplication { args, .. } => args.iter().for_each(|x| self.expr(x)),
        }
    }
}
