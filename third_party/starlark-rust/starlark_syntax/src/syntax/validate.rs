/*
 * Copyright 2018 The Starlark in Rust Authors.
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! AST for parsed starlark files.

use crate::syntax::DialectTypes;
use crate::syntax::ast::AstArgument;
use crate::syntax::ast::AstExpr;
use crate::syntax::ast::AstLiteral;
use crate::syntax::ast::AstParameter;
use crate::syntax::ast::AstStmt;
use crate::syntax::ast::CallArgsP;
use crate::syntax::ast::DefP;
use crate::syntax::ast::Expr;
use crate::syntax::ast::ForP;
use crate::syntax::ast::LambdaP;
use crate::syntax::ast::ParameterP;
use crate::syntax::ast::Stmt;
use crate::syntax::call::CallArgsUnpack;
use crate::syntax::def::DefParams;
use crate::syntax::state::ParserState;

impl Expr {
    /// We want to check a function call is well-formed.
    /// Our eventual plan is to follow the Python invariants, but for now, we are closer
    /// to the Starlark invariants.
    ///
    /// Python invariants are no positional arguments after named arguments,
    /// no *args after **kwargs, no repeated argument names.
    ///
    /// Starlark invariants are the above, plus at most one *args and the *args must appear
    /// after all positional and named arguments. The spec is silent on whether you are allowed
    /// multiple **kwargs.
    ///
    /// We allow at most one **kwargs.
    pub(crate) fn check_call(
        f: AstExpr,
        args: Vec<AstArgument>,
        parser_state: &mut ParserState<'_>,
    ) -> Expr {
        let args = CallArgsP { args };

        if let Err(e) = CallArgsUnpack::unpack(&args, parser_state.codemap) {
            parser_state.errors.push(e);
        }

        Expr::Call(Box::new(f), args)
    }
}

/// Validate all statements only occur where they are allowed to.
pub(crate) fn validate_module(stmt: &AstStmt, parser_state: &mut ParserState) {
    fn validate_params(params: &[AstParameter], parser_state: &mut ParserState) {
        if !parser_state.dialect.enable_keyword_only_arguments {
            for param in params {
                if let ParameterP::NoArgs = &param.node {
                    parser_state.error(
                        param.span,
                        "* keyword-only-arguments is not allowed in this dialect",
                    );
                }
            }
        }
        if !parser_state.dialect.enable_positional_only_arguments {
            for param in params {
                if let ParameterP::Slash = &param.node {
                    parser_state.error(
                        param.span,
                        "/ positional-only-arguments is not allowed in this dialect",
                    );
                }
            }
        }
        if let Err(e) = DefParams::unpack(params, parser_state.codemap) {
            parser_state.errors.push(e);
        }
    }

    // Inside a for, we allow continue/break, unless we go beneath a def.
    // Inside a def, we allow return.
    // All load's must occur at the top-level.
    // At the top-level we only allow for/if when the dialect permits it.
    fn f(
        stmt: &AstStmt,
        parser_state: &mut ParserState,
        top_level: bool,
        inside_for: bool,
        inside_def: bool,
    ) {
        let span = stmt.span;

        match &stmt.node {
            Stmt::Def(DefP { params, body, .. }) => {
                if !parser_state.dialect.enable_def {
                    parser_state.error(span, "`def` is not allowed in this dialect");
                }
                validate_params(params, parser_state);
                f(body, parser_state, false, false, true)
            }
            Stmt::For(ForP { body, .. }) => {
                if top_level && !parser_state.dialect.enable_top_level_stmt {
                    // BLAZE PATCH 0055: Java's text (Resolver.java:759).
                    parser_state.error(span, "for loops are not allowed at the top level. You may move it inside a function or use a comprehension, [f(x) for x in sequence]") // BLAZE PATCH 0055
                } else {
                    f(body, parser_state, false, true, inside_def)
                }
            }
            // BLAZE PATCH 0030: a var statement is allowed anywhere (it is not an `if`).
            Stmt::If(..) if stmt.blaze_var_statement().is_some() => {} // BLAZE PATCH 0030
            Stmt::If(..) | Stmt::IfElse(..) => {
                if top_level && !parser_state.dialect.enable_top_level_stmt {
                    // BLAZE PATCH 0055: Java's text (Resolver.java:854).
                    parser_state.error(span, "if statements are not allowed at the top level. You may move it inside a function or use an if expression (x if condition else y).") // BLAZE PATCH 0055
                } else {
                    stmt.node
                        .visit_stmt(|x| f(x, parser_state, false, inside_for, inside_def))
                }
            }
            Stmt::Break if !inside_for => {
                parser_state.error(span, "break statement must be inside a for loop") // BLAZE PATCH 0055: Java Resolver.java:783
            }
            Stmt::Continue if !inside_for => {
                parser_state.error(span, "continue statement must be inside a for loop") // BLAZE PATCH 0055: Java Resolver.java:783
            }
            Stmt::Return(_) if !inside_def => {
                parser_state.error(span, "return statements must be inside a function") // BLAZE PATCH 0055: Java Resolver.java:702
            }
            Stmt::Load(..) => {
                if !top_level {
                    parser_state.error(span, "load statement not at top level"); // BLAZE PATCH 0055: Java Resolver.java:775
                }
                if !parser_state.dialect.enable_load {
                    parser_state.error(span, "`load` is not allowed in this dialect");
                }
            }
            _ => stmt
                .node
                .visit_stmt(|x| f(x, parser_state, top_level, inside_for, inside_def)),
        }
    }

    fn expr(x: &AstExpr, parser_state: &mut ParserState) {
        match &x.node {
            Expr::Literal(AstLiteral::Ellipsis) => {
                if parser_state.dialect.enable_types == DialectTypes::Disable {
                    // BLAZE PATCH 0055: Java's text (Parser.java:777 parsePrimary).
                    parser_state.error(x.span, "syntax error at '...': ellipsis ('...') is not allowed outside type expressions"); // BLAZE PATCH 0055
                }
            }
            Expr::Lambda(LambdaP { params, .. }) => {
                if !parser_state.dialect.enable_lambda {
                    parser_state.error(x.span, "`lambda` is not allowed in this dialect");
                }
                validate_params(params, parser_state);
            }
            _ => {}
        }
        x.node.visit_expr(|x| expr(x, parser_state));
    }

    f(stmt, parser_state, true, false, false);

    stmt.visit_expr(|x| expr(x, parser_state));
}
