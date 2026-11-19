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

//! Code called by the parser to handle complex cases not handled by the grammar.

use crate::codemap::CodeMap;
use crate::codemap::Pos;
use crate::codemap::Span;
use crate::codemap::Spanned;
use crate::eval_exception::EvalException;
use crate::lexer::FStringConv as LexerFStringConv;
use crate::slice_vec_ext::VecExt;
use crate::syntax::DialectTypes;
use crate::syntax::ast::AssignIdentP;
use crate::syntax::ast::AssignOp;
use crate::syntax::ast::AssignP;
use crate::syntax::ast::AssignTarget;
use crate::syntax::ast::AssignTargetP;
use crate::syntax::ast::AstAssignTarget;
use crate::syntax::ast::AstExpr;
use crate::syntax::ast::AstFString;
use crate::syntax::ast::AstStmt;
use crate::syntax::ast::AstString;
use crate::syntax::ast::AstTypeExpr;
use crate::syntax::ast::Expr;
use crate::syntax::ast::FStringP;
use crate::syntax::ast::LoadP;
use crate::syntax::ast::Stmt;
use crate::syntax::ast::StmtP;
use crate::syntax::ast::ToAst;
use crate::syntax::ast::TypeExpr;
use crate::syntax::ast::TypeExprP;
use crate::syntax::state::ParserState;
use crate::syntax::type_expr::TypeExprUnpackP;

#[derive(Debug, thiserror::Error)]
enum GrammarUtilError {
    #[error("left-hand-side of assignment must take the form `a`, `a.b` or `a[b]`")]
    InvalidLhs,
    // BLAZE PATCH 0055: Java's text (Resolver.java:869), at the operator.
    #[error("cannot perform augmented assignment on a list or tuple expression")] // BLAZE PATCH 0055
    InvalidModifyLhs,
    #[error("type annotations not allowed on augmented assignments")]
    TypeAnnotationOnAssignOp,
    #[error("type annotations not allowed on multiple assignments")]
    TypeAnnotationOnTupleAssign,
    #[error("`load` statement requires at least two arguments")]
    LoadRequiresAtLeastTwoArguments,
    #[error("Trailing comma is allowed only in parenthesized tuples.")] // BLAZE PATCH 0055: Java Parser.java:635
    UnparenthesizedTupleTrailingComma,
}

/// Ensure we produce normalised Statements, rather than singleton Statements
pub fn statements(mut xs: Vec<AstStmt>, begin: usize, end: usize) -> AstStmt {
    if xs.len() == 1 {
        xs.pop().unwrap()
    } else {
        StmtP::Statements(xs).ast(begin, end)
    }
}

pub fn check_assign(codemap: &CodeMap, x: AstExpr) -> Result<AstAssignTarget, EvalException> {
    Ok(Spanned {
        span: x.span,
        node: match x.node {
            Expr::Tuple(xs) | Expr::List(xs) => {
                AssignTarget::Tuple(xs.into_try_map(|x| check_assign(codemap, x))?)
            }
            Expr::Dot(a, b) => AssignTarget::Dot(a, b),
            Expr::Index(a_b) => AssignTarget::Index(a_b),
            Expr::Identifier(x) => AssignTarget::Identifier(x.map(|s| AssignIdentP {
                ident: s.ident,
                payload: (),
            })),
            _ => {
                // BLAZE PATCH 0055: Java's text (Resolver.java:671 assign, "cannot assign to
                // '<expr>'"). PORT: Java prints the expression with NodePrinter; the source
                // text stands in (equal for expressions written in canonical form).
                let _ = GrammarUtilError::InvalidLhs; // BLAZE PATCH 0055
                return Err(EvalException::new_anyhow( // BLAZE PATCH 0055
                    anyhow::anyhow!("cannot assign to '{}'", codemap.source_span(x.span)), // BLAZE PATCH 0055
                    x.span,
                    codemap,
                ));
            }
        },
    })
}

pub fn check_assignment(
    codemap: &CodeMap,
    lhs: AstExpr,
    ty: Option<Box<AstTypeExpr>>,
    op: Option<AssignOp>,
    rhs: AstExpr,
) -> Result<Stmt, EvalException> {
    if op.is_some() {
        // for augmented assignment, Starlark doesn't allow tuple/list
        match &lhs.node {
            Expr::Tuple(_) | Expr::List(_) => {
                // BLAZE PATCH 0055: located at the operator (Java getOperatorLocation).
                let src = codemap.source(); // BLAZE PATCH 0055
                let mut at = lhs.span.end().get() as usize; // BLAZE PATCH 0055
                while at < src.len() && matches!(src.as_bytes()[at], b' ' | b'\t' | b'\\' | b'\n' | b'\r' | b')') { // BLAZE PATCH 0055
                    at += 1; // BLAZE PATCH 0055
                } // BLAZE PATCH 0055
                return Err(EvalException::new_anyhow(
                    GrammarUtilError::InvalidModifyLhs.into(),
                    Span::new(Pos::new(at as u32), Pos::new(at as u32)), // BLAZE PATCH 0055
                    codemap,
                ));
            }
            _ => {}
        }
    }
    let lhs = check_assign(codemap, lhs)?;
    if let Some(ty) = &ty {
        let err = if op.is_some() {
            Some(GrammarUtilError::TypeAnnotationOnAssignOp)
        } else if matches!(lhs.node, AssignTargetP::Tuple(_)) {
            Some(GrammarUtilError::TypeAnnotationOnTupleAssign)
        } else {
            None
        };
        if let Some(err) = err {
            return Err(EvalException::new_anyhow(err.into(), ty.span, codemap));
        }
    }
    Ok(match op {
        None => Stmt::Assign(AssignP {
            lhs,
            ty: ty.map(|ty| *ty),
            rhs,
        }),
        Some(op) => Stmt::AssignModify(lhs, op, Box::new(rhs)),
    })
}

pub(crate) fn reject_unparenthesized_tuple_trailing_comma<T>(
    codemap: &CodeMap,
    begin: usize,
    end: usize,
) -> Result<T, EvalException> {
    // BLAZE PATCH 0055: Java reports the token after the trailing comma (Parser.java:635).
    let _ = begin; // BLAZE PATCH 0055
    let src = codemap.source(); // BLAZE PATCH 0055
    let mut at = end; // BLAZE PATCH 0055
    while at < src.len() && matches!(src.as_bytes()[at], b' ' | b'\t' | b'\\') { // BLAZE PATCH 0055
        at += 1; // BLAZE PATCH 0055
    } // BLAZE PATCH 0055
    Err(EvalException::new_anyhow(
        GrammarUtilError::UnparenthesizedTupleTrailingComma.into(),
        Span::new(Pos::new(at as _), Pos::new(at as _)), // BLAZE PATCH 0055
        codemap,
    ))
}

pub(crate) fn check_load_0(module: AstString, parser_state: &mut ParserState) -> Stmt {
    parser_state.errors.push(EvalException::new_anyhow(
        GrammarUtilError::LoadRequiresAtLeastTwoArguments.into(),
        module.span,
        parser_state.codemap,
    ));
    Stmt::Load(LoadP {
        module,
        args: Vec::new(),
        payload: (),
    })
}

/// An element of an f-string during parsing - either text or an expression.
pub enum FStringElement {
    /// Literal text in the f-string.
    Text(AstString),
    /// An expression with optional conversion specifier.
    Expr(AstExpr, LexerFStringConv),
}

/// Build an f-string from parsed parts (new parsing approach with proper expression support).
pub(crate) fn fstring_from_parts(
    parts: Vec<FStringElement>,
    begin: usize,
    end: usize,
    parser_state: &mut ParserState,
) -> AstFString {
    if !parser_state.dialect.enable_f_strings {
        parser_state.error(
            Span::new(Pos::new(begin as _), Pos::new(end as _)),
            "Your Starlark dialect must enable f-strings to use them",
        );
    }

    let mut format = String::new();
    let mut expressions = Vec::new();

    for part in parts {
        match part {
            FStringElement::Text(text) => {
                // Escape any literal braces in the text for the format string
                for c in text.node.chars() {
                    match c {
                        '{' => format.push_str("{{"),
                        '}' => format.push_str("}}"),
                        _ => format.push(c),
                    }
                }
            }
            FStringElement::Expr(expr, conv) => {
                expressions.push(expr);
                match conv {
                    LexerFStringConv::None | LexerFStringConv::Str => format.push_str("{}"),
                    LexerFStringConv::Repr => format.push_str("{!r}"),
                }
            }
        }
    }

    format.shrink_to_fit();

    FStringP {
        format: format.ast(begin, end),
        expressions,
    }
    .ast(begin, end)
}

#[derive(thiserror::Error, Debug)]
enum DialectError {
    #[error("type annotations are not allowed in this dialect")]
    Types,
}

#[allow(dead_code)] // BLAZE PATCH 0055
fn err<T>(codemap: &CodeMap, span: Span, err: DialectError) -> Result<T, EvalException> {
    Err(EvalException::new_anyhow(err.into(), span, codemap))
}

/// BLAZE PATCH 0020: prefix of the error for Bazel type syntax that is not ported yet. Callers
/// turn it into a loud failure instead of a user-facing syntax error.
pub const BLAZE_TYPE_SYNTAX_NOT_PORTED: &str = "BLAZE not ported yet: Bazel type syntax: ";

pub(crate) fn dialect_check_type(
    state: &ParserState,
    x: Spanned<Expr>,
) -> Result<Spanned<TypeExpr>, EvalException> {
    let span = x.span;
    if state.dialect.enable_types == DialectTypes::Disable {
        // BLAZE PATCH 0055: Java's text and position (Parser.java:1066 checkAllowTypeSyntax):
        // the `:` or `->` before the annotation.
        let src = state.codemap.source(); // BLAZE PATCH 0055
        let mut i = x.span.begin().get() as usize; // BLAZE PATCH 0055
        while i > 0 && matches!(src.as_bytes()[i - 1], b' ' | b'\t' | b'\n' | b'\r' | b'\\') { // BLAZE PATCH 0055
            i -= 1; // BLAZE PATCH 0055
        } // BLAZE PATCH 0055
        let (at, tok) = if src[..i].ends_with("->") { (i - 2, "->") } else { (i.saturating_sub(1), ":") }; // BLAZE PATCH 0055
        let _ = DialectError::Types; // BLAZE PATCH 0055
        return Err(EvalException::new_anyhow( // BLAZE PATCH 0055
            anyhow::anyhow!("syntax error at '{tok}': type annotations are disallowed"), // BLAZE PATCH 0055
            Span::new(Pos::new(at as u32), Pos::new(at as u32 + tok.len() as u32)), // BLAZE PATCH 0055
            state.codemap, // BLAZE PATCH 0055
        )); // BLAZE PATCH 0055
    }

    // BLAZE PATCH 0020: Bazel 9.3.2 parses annotations as arbitrary expressions unless type
    // checking is on (FileOptions.tolerateInvalidTypeExpressions; Parser.java:1084
    // Parser.parseTypeExprWithFallback).
    if state.dialect.enable_types != DialectTypes::ParseOnly {
        TypeExprUnpackP::unpack(&x, state.codemap).map_err(EvalException::from)?;
    }

    Ok(x.map(|node| TypeExprP {
        expr: Spanned { node, span },
        payload: (),
    }))
}

/// Create an error for an invalid f-string conversion specifier.
pub(crate) fn fstring_conv_error(l: usize, r: usize, state: &ParserState) -> EvalException {
    let span = Span::new(Pos::new(l as u32), Pos::new(r as u32));
    EvalException::parser_error(
        "invalid f-string conversion specifier, expected 's' or 'r'",
        span,
        state.codemap,
    )
}
