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

//! Recursive descent + Pratt expression parser for Starlark.
//!
//! The grammar implemented here is the official [Starlark language
//! specification][starlark-spec]. Production names in this file (`parse_def_stmt`,
//! `parse_simple_stmt`, `parse_test`, `parse_or_test`, `parse_and_test`,
//! `parse_atom`, `parse_for_clause`, etc.) mirror the non-terminals in the
//! spec's grammar so the two can be cross-referenced.
//!
//! The Pratt parsing algorithm used here for operator precedence follows
//! [matklad's "Simple but Powerful Pratt Parsing"][pratt].
//!
//! [starlark-spec]: https://github.com/bazelbuild/starlark/blob/master/spec.md#grammar-reference
//! [pratt]: https://matklad.github.io/2020/04/13/simple-but-powerful-pratt-parsing.html

use crate::codemap::Pos;
use crate::codemap::Span;
use crate::codemap::Spanned;
use crate::eval_exception::EvalException;
use crate::lexer;
use crate::lexer::Token;
use crate::syntax::ast::*;
use crate::syntax::grammar_util;
use crate::lexer::TokenInt; // BLAZE PATCH 0030
use crate::syntax::state::ParserState;

pub(crate) type Lexeme = Result<(usize, Token, usize), EvalException>;

/// Pop the current token after the caller has already verified (via peek/match)
/// that it is the named variant. Returns `(start, payload, end)`. In debug
/// builds, the inner `unreachable!` arm acts as an assertion of the invariant.
/// Use at payload-carrying sites (Identifier, Int, Float, FStringStart) that would
/// otherwise hand-roll `take_current().expect(...)` plus a destructuring
/// `match` with an `unreachable!` arm.
macro_rules! take_payload {
    ($self:expr, Token::$variant:ident) => {{
        let (l, tok, r) = $self.take_current().expect("token present per peek above");
        match tok {
            Token::$variant(v) => (l, v, r),
            _ => unreachable!("take_payload guarded by peek check above"),
        }
    }};
}

/// BLAZE PATCH 0020: Java `TokenKind.CAST` / `TokenKind.ISINSTANCE`.
enum TypeKeyword {
    Cast,
    IsInstance,
}

pub(crate) struct ParserRd<'a, I: Iterator<Item = Lexeme>> {
    tokens: I,
    /// Current token with span, or None if EOF.
    current: Option<(usize, Token, usize)>,
    /// Deferred lexer error encountered while advancing the token stream.
    pending_error: Option<EvalException>,
    /// Byte offset (into the source string) of the end of the last consumed
    /// token, used to construct EOF spans when `current` is None.
    last_end: usize,
    state: ParserState<'a>,
    /// BLAZE PATCH 0020: Bazel's type syntax (`DialectTypes::ParseOnly`).
    type_syntax: bool,
    /// BLAZE PATCH 0020: nesting depth of `def` bodies (type aliases are top-level only).
    def_depth: u32,
    /// BLAZE PATCH 0047: Java Parser.insideTypeExpr (Parser.java:120), set while parsing a type
    /// annotation.
    in_type_expr: bool, // BLAZE PATCH 0047
    /// BLAZE PATCH 0054: the last consumed token was a string literal.
    prev_was_string: bool, // BLAZE PATCH 0054
}

impl<'a, I: Iterator<Item = Lexeme>> ParserRd<'a, I> {
    pub(crate) fn new(mut tokens: I, state: ParserState<'a>) -> Self {
        let (current, pending_error) = match Self::next_from(&mut tokens) {
            Ok(current) => (current, None),
            Err(err) => (None, Some(err)),
        };
        // BLAZE PATCH 0020
        let type_syntax = state.dialect.enable_types == crate::syntax::DialectTypes::ParseOnly;
        ParserRd {
            tokens,
            current,
            pending_error,
            last_end: 0,
            state,
            type_syntax,
            def_depth: 0,
            in_type_expr: false, // BLAZE PATCH 0047
            prev_was_string: false, // BLAZE PATCH 0054
        }
    }

    #[inline]
    fn next_from(tokens: &mut I) -> Result<Option<(usize, Token, usize)>, EvalException> {
        match tokens.next() {
            Some(Ok(t)) => Ok(Some(t)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }

    /// Advance past the current token. Hot-path version: caller does not need
    /// the consumed token. Avoids the ~56-byte copy that `take_current` would
    /// produce for the return value.
    #[inline]
    fn advance(&mut self) {
        self.prev_was_string = matches!(self.current, Some((_, Token::String(_), _))); // BLAZE PATCH 0054
        if let Some((_, _, end)) = &self.current {
            self.last_end = *end;
        }
        match Self::next_from(&mut self.tokens) {
            Ok(current) => self.current = current,
            Err(err) => {
                self.current = None;
                self.pending_error = Some(err);
            }
        }
    }

    /// Advance and return the consumed token. Use only at sites that need to
    /// extract data from the token (Identifier, Int, Float, String, FStringStart).
    #[inline]
    fn take_current(&mut self) -> Option<(usize, Token, usize)> {
        let old = self.current.take();
        self.prev_was_string = matches!(old, Some((_, Token::String(_), _))); // BLAZE PATCH 0054
        if let Some((_, _, end)) = &old {
            self.last_end = *end;
        }
        match Self::next_from(&mut self.tokens) {
            Ok(current) => self.current = current,
            Err(err) => {
                self.current = None;
                self.pending_error = Some(err);
            }
        }
        old
    }

    #[inline]
    fn peek(&self) -> Option<&Token> {
        self.current.as_ref().map(|(_, t, _)| t)
    }

    #[inline]
    fn pos(&self) -> usize {
        self.current
            .as_ref()
            .map_or(self.last_end, |(start, _, _)| *start)
    }

    #[inline]
    fn expect(&mut self, expected: &Token) -> Result<(usize, usize), EvalException> {
        match &self.current {
            Some((start, tok, end)) if tok == expected => {
                let span = (*start, *end);
                self.advance();
                Ok(span)
            }
            _ => Err(self.error_expected(&format!("{}", expected))),
        }
    }

    #[inline]
    fn eat(&mut self, expected: &Token) -> bool {
        match &self.current {
            Some((_, tok, _)) if tok == expected => {
                self.advance();
                true
            }
            _ => false,
        }
    }

    fn skip_newlines(&mut self) {
        while self.eat(&Token::Newline) {}
    }

    /// Advance past the current token after the caller has already verified
    /// (via peek/match) that it is `expected`. In debug builds, asserts the
    /// invariant; in release builds, equivalent to `advance()`.
    #[inline]
    fn consume(&mut self, expected: &Token) {
        debug_assert_eq!(self.peek(), Some(expected), "consume token mismatch");
        self.advance();
    }

    fn error_expected(&mut self, what: &str) -> EvalException {
        if let Some(err) = self.pending_error.take() {
            return err;
        }

        // BLAZE PATCH 0054: Java's text (Parser.java:334 syntaxError): "syntax error at
        // 'TOKEN': expected KIND", or "indentation error" at an unexpected indent.
        // BLAZE PATCH 0054: Java Parser.java:1688 parseSuite reports this text alone.
        if what == "new indentation block" { // BLAZE PATCH 0054
            let pos = self.pos(); // BLAZE PATCH 0054
            return self.error_at(pos, pos, "expected an indented block".to_owned()); // BLAZE PATCH 0054
        } // BLAZE PATCH 0054
        // BLAZE PATCH 0054: Java Parser.java:597, :751: a `for` after a call argument or a
        // parenthesized expression.
        if what == "symbol ')'" && self.peek() == Some(&Token::For) { // BLAZE PATCH 0054
            return self.error_java("Starlark does not support Python-style generator expressions"); // BLAZE PATCH 0054
        } // BLAZE PATCH 0054
        // BLAZE PATCH 0054: Java Parser.java:673 parseStringLiteral: a string literal right
        // after another.
        if matches!(self.peek(), Some(Token::String(_))) && self.prev_was_string { // BLAZE PATCH 0054
            let pos = self.pos(); // BLAZE PATCH 0054
            return self.error_at(pos, pos, "Implicit string concatenation is forbidden, use the + operator".to_owned()); // BLAZE PATCH 0054
        } // BLAZE PATCH 0054
        let what = java_expected(what); // BLAZE PATCH 0054
        let pos = self.pos();
        let (span, msg) = match &self.current {
            // Java Lexer.java:256: the last char of the indentation.
            Some((_, Token::Indent, end)) => ( // BLAZE PATCH 0054
                Span::new(Pos::new(end.saturating_sub(1) as u32), Pos::new(*end as u32)), // BLAZE PATCH 0054
                "indentation error".to_owned(), // BLAZE PATCH 0054
            ), // BLAZE PATCH 0054
            Some((start, tok, end)) => (
                Span::new(Pos::new(*start as u32), Pos::new(*end as u32)),
                format!("syntax error at '{}': {}", java_token_string(tok), what), // BLAZE PATCH 0054
            ),
            None => (
                Span::new(Pos::new(pos as u32), Pos::new(pos as u32)),
                format!("syntax error at 'EOF': {}", what), // BLAZE PATCH 0054
            ),
        };
        EvalException::parser_error(msg, span, self.state.codemap)
    }

    /// BLAZE PATCH 0054: `expect` with Java's message for this context
    /// (`syntax error at 'TOKEN': <message>`).
    fn expect_java(&mut self, expected: &Token, message: &str) -> Result<(usize, usize), EvalException> {
        if self.peek() == Some(expected) {
            return self.expect(expected);
        }
        Err(self.error_java(message))
    }

    /// BLAZE PATCH 0054: Java's `syntaxError(message)` at the current token.
    fn error_java(&mut self, message: &str) -> EvalException {
        if let Some(err) = self.pending_error.take() {
            return err;
        }
        let pos = self.pos();
        let (start, end, tok) = match &self.current {
            Some((s, t, e)) => (*s, *e, java_token_string(t)),
            None => (pos, pos, "EOF".to_owned()),
        };
        self.error_at(start, end, format!("syntax error at '{tok}': {message}"))
    }

    fn error_at(&self, start: usize, end: usize, msg: String) -> EvalException {
        EvalException::parser_error(
            msg,
            Span::new(Pos::new(start as u32), Pos::new(end as u32)),
            self.state.codemap,
        )
    }

    // ==================== Top-level ====================

    pub(crate) fn parse_module(mut self) -> Result<AstStmt, EvalException> {
        // Skip leading newlines
        self.skip_newlines();
        let l = self.pos();
        let mut stmts = Vec::new();
        while self.peek().is_some() {
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        let r = self.last_end;
        if let Some(err) = self.pending_error {
            Err(err)
        } else {
            Ok(grammar_util::statements(stmts, l, r))
        }
    }

    // ==================== Statements ====================

    fn parse_stmt(&mut self) -> Result<AstStmt, EvalException> {
        match self.peek() {
            Some(Token::Def) => self.parse_def_stmt(),
            Some(Token::If) => self.parse_if_stmt(),
            Some(Token::For) => self.parse_for_stmt(),
            _ => self.parse_simple_stmt(),
        }
    }

    fn parse_def_stmt(&mut self) -> Result<AstStmt, EvalException> {
        let l = self.pos();
        self.consume(&Token::Def);
        let name = self.parse_assign_ident()?;
        // BLAZE PATCH 0020: `def f[T, U](...)`.
        self.parse_optional_type_parameters()?;
        self.expect(&Token::OpeningRound)?;
        let params = self.parse_comma_separated_def_params()?;
        self.expect(&Token::ClosingRound)?;
        let return_type = if self.eat(&Token::MinusGreater) {
            Some(Box::new(self.parse_type_expr()?))
        } else {
            None
        };
        self.expect(&Token::Colon)?;
        self.def_depth += 1; // BLAZE PATCH 0020
        let body = self.parse_suite();
        self.def_depth -= 1; // BLAZE PATCH 0020
        let body = body?;
        let r = self.last_end;
        Ok(StmtP::Def(DefP {
            name,
            params,
            return_type,
            body: Box::new(body),
            payload: (),
        })
        .ast(l, r))
    }

    fn parse_if_stmt(&mut self) -> Result<AstStmt, EvalException> {
        let l = self.pos();
        self.consume(&Token::If);
        self.parse_if_body(l)
    }

    fn parse_if_body(&mut self, l: usize) -> Result<AstStmt, EvalException> {
        let cond = self.parse_test()?;
        self.expect(&Token::Colon)?;
        let suite = self.parse_suite()?;
        let r = self.last_end;
        match self.peek() {
            Some(Token::Elif) => {
                let el_l = self.pos();
                self.consume(&Token::Elif);
                let else_clause = self.parse_if_body(el_l)?;
                let r = else_clause.span.end().get() as usize;
                Ok(Stmt::IfElse(cond, Box::new((suite, else_clause))).ast(l, r))
            }
            Some(Token::Else) => {
                self.consume(&Token::Else);
                self.expect(&Token::Colon)?;
                let else_suite = self.parse_suite()?;
                let r2 = self.last_end;
                Ok(Stmt::IfElse(cond, Box::new((suite, else_suite))).ast(l, r2))
            }
            _ => Ok(Stmt::If(cond, Box::new(suite)).ast(l, r)),
        }
    }

    fn parse_for_stmt(&mut self) -> Result<AstStmt, EvalException> {
        let l = self.pos();
        self.consume(&Token::For);
        let var_expr = self.parse_expr_list()?;
        self.expect(&Token::In)?;
        let over = self.parse_test()?;
        self.expect(&Token::Colon)?;
        let body = self.parse_suite()?;
        let r = self.last_end;
        let var = grammar_util::check_assign(self.state.codemap, var_expr)?;
        Ok(Stmt::For(ForP {
            var,
            over,
            body: Box::new(body),
        })
        .ast(l, r))
    }

    fn parse_suite(&mut self) -> Result<AstStmt, EvalException> {
        if self.peek() == Some(&Token::Newline) {
            // Indented block
            self.skip_newlines();
            self.expect(&Token::Indent)?;
            self.skip_newlines();
            let l = self.pos();
            let mut stmts = Vec::new();
            loop {
                stmts.push(self.parse_stmt()?);
                self.skip_newlines();
                if self.peek() == Some(&Token::Dedent) || self.peek().is_none() {
                    break;
                }
            }
            let r = self.last_end;
            self.eat(&Token::Dedent);
            Ok(grammar_util::statements(stmts, l, r))
        } else {
            // Simple suite on same line
            self.parse_simple_stmt()
        }
    }

    fn parse_simple_stmt(&mut self) -> Result<AstStmt, EvalException> {
        let l = self.pos();
        let first = self.parse_small_stmt()?;
        let mut stmts = vec![first];
        while self.eat(&Token::Semicolon) {
            if self.peek() == Some(&Token::Newline) || self.peek().is_none() {
                break;
            }
            stmts.push(self.parse_small_stmt()?);
        }
        // Consume optional trailing semicolon already handled above
        self.expect(&Token::Newline)?;
        let r = self.last_end;
        if stmts.len() == 1 {
            Ok(stmts.pop().expect("non-empty after parsing"))
        } else {
            Ok(Stmt::Statements(stmts).ast(l, r))
        }
    }

    fn parse_small_stmt(&mut self) -> Result<AstStmt, EvalException> {
        match self.peek() {
            Some(Token::Return) => {
                let l = self.pos();
                self.advance();
                let expr = match self.peek() {
                    Some(Token::Newline) | Some(Token::Semicolon) | None => None,
                    _ => Some(self.parse_test_list(false)?),
                };
                let r = self.last_end;
                Ok(Stmt::Return(expr).ast(l, r))
            }
            Some(Token::Break) => {
                let l = self.pos();
                self.advance();
                Ok(Stmt::Break.ast(l, self.last_end))
            }
            Some(Token::Continue) => {
                let l = self.pos();
                self.advance();
                Ok(Stmt::Continue.ast(l, self.last_end))
            }
            Some(Token::Pass) => {
                let l = self.pos();
                self.advance();
                Ok(Stmt::Pass.ast(l, self.last_end))
            }
            Some(Token::Load) => self.parse_load_stmt(),
            _ => self.parse_assign_or_expr_stmt(),
        }
    }

    fn parse_assign_or_expr_stmt(&mut self) -> Result<AstStmt, EvalException> {
        let l = self.pos();
        let first = self.parse_test()?;
        // BLAZE PATCH 0020: `type X = T`. This is the only context in which an identifier can be
        // immediately followed by another identifier; the first one is the soft keyword `type`.
        // Java: net/starlark/java/syntax/Parser.java:1545 Parser.parseSmallStatement
        if self.type_syntax
            && matches!(&first.node, Expr::Identifier(id) if id.node.ident == "type")
            && matches!(self.peek(), Some(Token::Identifier(_)))
        {
            return self.parse_type_alias_statement_tail(l);
        }
        let lhs_is_expr_list = self.peek() == Some(&Token::Comma);
        let lhs = if lhs_is_expr_list {
            self.parse_test_list_tail(l, first, false)?
        } else {
            first
        };

        // Check for type annotation
        let colon = self.pos(); // BLAZE PATCH 0030
        let ty = if self.eat(&Token::Colon) {
            Some(Box::new(self.parse_type_expr()?))
        } else {
            None
        };

        // Check for assignment operator
        // Returns (true, None) for `=`, (true, Some(op)) for `+=` etc, (false, None) for no op
        let (is_assign, assign_op) = self.parse_assign_op();
        if is_assign {
            let rhs = self.parse_test_list(false)?;
            let r = self.last_end;
            let stmt = grammar_util::check_assignment(self.state.codemap, lhs, ty, assign_op, rhs)?;
            Ok(stmt.ast(l, r))
        } else if self.type_syntax && ty.is_some() { // BLAZE PATCH 0030
            // BLAZE PATCH 0030: a var statement, `x: T` (Parser.java:1582). Bazel binds `x` in the
            // enclosing block (Resolver.java:562 createBindings) and executing it does nothing
            // (Eval.java:324), so `x` reads as "referenced before assignment" until assigned.
            // The AST has no node for a binding without a value: it becomes `if 0: x = 0`, whose
            // assignment binds `x` in the same block and never runs (the annotation is dropped,
            // as `ParseOnly` ignores annotations).
            let r = self.last_end;
            let Expr::Identifier(id) = lhs.node else { // BLAZE PATCH 0030
                // Java: Parser.java:1586.
                return Err(self.error_at( // BLAZE PATCH 0030
                    colon, // BLAZE PATCH 0030
                    colon + 1, // BLAZE PATCH 0030
                    "type annotations must have a single identifier on the left-hand side" // BLAZE PATCH 0030
                        .to_owned(), // BLAZE PATCH 0030
                )); // BLAZE PATCH 0030
            }; // BLAZE PATCH 0030
            let id_span = id.span; // BLAZE PATCH 0030
            let zero = |span: Span| { // BLAZE PATCH 0030
                Spanned { // BLAZE PATCH 0030
                    span, // BLAZE PATCH 0030
                    node: Expr::Literal(AstLiteral::Int(Spanned { // BLAZE PATCH 0030
                        span, // BLAZE PATCH 0030
                        node: TokenInt::I32(0), // BLAZE PATCH 0030
                    })), // BLAZE PATCH 0030
                } // BLAZE PATCH 0030
            }; // BLAZE PATCH 0030
            let target = Spanned { // BLAZE PATCH 0030
                span: id_span, // BLAZE PATCH 0030
                node: AssignTarget::Identifier(id.map(|s| AssignIdentP { // BLAZE PATCH 0030
                    ident: s.ident, // BLAZE PATCH 0030
                    payload: (), // BLAZE PATCH 0030
                })), // BLAZE PATCH 0030
            }; // BLAZE PATCH 0030
            let assign = Stmt::Assign(AssignP { // BLAZE PATCH 0030
                lhs: target, // BLAZE PATCH 0030
                ty: None, // BLAZE PATCH 0030
                rhs: zero(id_span), // BLAZE PATCH 0030
            }); // BLAZE PATCH 0030
            let body = Spanned { span: id_span, node: assign }; // BLAZE PATCH 0030
            Ok(Stmt::If(zero(id_span), Box::new(body)).ast(l, r)) // BLAZE PATCH 0030
        } else if lhs_is_expr_list || ty.is_some() {
            Err(self.error_expected("newline")) // BLAZE PATCH 0054: Java parses an expression statement
        } else {
            // Expression statement
            let r = self.last_end;
            Ok(Stmt::Expression(lhs).ast(l, r))
        }
    }

    fn parse_assign_op(&mut self) -> (bool, Option<AssignOp>) {
        let op = match self.peek() {
            Some(Token::Equal) => None,
            Some(Token::PlusEqual) => Some(AssignOp::Add),
            Some(Token::MinusEqual) => Some(AssignOp::Subtract),
            Some(Token::StarEqual) => Some(AssignOp::Multiply),
            Some(Token::SlashEqual) => Some(AssignOp::Divide),
            Some(Token::SlashSlashEqual) => Some(AssignOp::FloorDivide),
            Some(Token::PercentEqual) => Some(AssignOp::Percent),
            Some(Token::AmpersandEqual) => Some(AssignOp::BitAnd),
            Some(Token::PipeEqual) => Some(AssignOp::BitOr),
            Some(Token::CaretEqual) => Some(AssignOp::BitXor),
            Some(Token::LessLessEqual) => Some(AssignOp::LeftShift),
            Some(Token::GreaterGreaterEqual) => Some(AssignOp::RightShift),
            _ => return (false, None),
        };
        self.advance();
        (true, op)
    }

    fn parse_load_stmt(&mut self) -> Result<AstStmt, EvalException> {
        let l = self.pos();
        self.advance(); // consume 'load'
        self.expect(&Token::OpeningRound)?;
        let module = self.parse_string_literal()?;

        // Check for empty load: load("module")
        // BLAZE PATCH 0054: Java Parser.java:1407 parseLoadStatement.
        if self.peek() == Some(&Token::ClosingRound) { // BLAZE PATCH 0054
            return Err(self.error_java("expected at least one symbol to load")); // BLAZE PATCH 0054
        } // BLAZE PATCH 0054
        if self.eat(&Token::ClosingRound) {
            let r = self.last_end;
            let stmt = grammar_util::check_load_0(module, &mut self.state);
            return Ok(stmt.ast(l, r));
        }

        self.expect(&Token::Comma)?;

        // Parse load symbols
        let mut args: Vec<(AstAssignIdent, AstString)> = Vec::new();
        let mut commas: Vec<Spanned<Comma>> = Vec::new();
        loop {
            if self.peek() == Some(&Token::ClosingRound) {
                break;
            }
            let sym = self.parse_load_symbol()?;
            if self.peek() == Some(&Token::Comma) {
                let cl = self.pos();
                self.advance();
                let cr = self.last_end;
                commas.push(Comma.ast(cl, cr));
                args.push(sym);
            } else {
                // Last symbol without trailing comma
                self.expect(&Token::ClosingRound)?;
                let r = self.last_end;
                // Build the load args
                let load_args: Vec<_> = args
                    .into_iter()
                    .zip(commas)
                    .map(|((local, their), comma)| LoadArgP {
                        local,
                        their,
                        comma: Some(comma),
                    })
                    .chain(std::iter::once(LoadArgP {
                        local: sym.0,
                        their: sym.1,
                        comma: None,
                    }))
                    .collect();
                return Ok(Stmt::Load(LoadP {
                    module,
                    args: load_args,
                    payload: (),
                })
                .ast(l, r));
            }
        }

        self.expect(&Token::ClosingRound)?;
        let r = self.last_end;

        if args.is_empty() {
            let stmt = grammar_util::check_load_0(module, &mut self.state);
            return Ok(stmt.ast(l, r));
        }

        // All args had trailing commas
        let load_args: Vec<_> = args
            .into_iter()
            .zip(commas)
            .map(|((local, their), comma)| LoadArgP {
                local,
                their,
                comma: Some(comma),
            })
            .collect();
        Ok(Stmt::Load(LoadP {
            module,
            args: load_args,
            payload: (),
        })
        .ast(l, r))
    }

    fn parse_load_symbol(&mut self) -> Result<(AstAssignIdent, AstString), EvalException> {
        // Either "name" or ident = "name"
        match self.peek() {
            // BLAZE PATCH 0048: `cast` / `isinstance` are keywords under type syntax, not a
            // local name (Java Parser.parseLoadStatement expects an IDENTIFIER).
            Some(Token::Identifier(_)) if self.at_type_keyword().is_none() => { // BLAZE PATCH 0048
                // Check if this is `ident = "string"` form
                let (id_l, ident, id_r) = take_payload!(self, Token::Identifier);
                if self.eat(&Token::Equal) {
                    // Named form: ident = "string"
                    // BLAZE PATCH 0054: Java Parser.java:1457 parseLoadSymbol.
                    if !matches!(self.peek(), Some(Token::String(_))) { // BLAZE PATCH 0054
                        return Err(self.error_java("expected string")); // BLAZE PATCH 0054
                    } // BLAZE PATCH 0054
                    let their = self.parse_string_literal()?;
                    let local = Spanned {
                        span: Span::new(Pos::new(id_l as u32), Pos::new(id_r as u32)),
                        node: AssignIdentP { ident, payload: () },
                    };
                    Ok((local, their))
                } else {
                    // Oops, not an ident = form. This should be a string literal.
                    // But we already consumed an identifier. This is an error.
                    Err(self.error_at(
                        id_l,
                        id_r,
                        "Parse error: expected string literal in load statement".to_owned(),
                    ))
                }
            }
            Some(Token::String(_)) => {
                let s = self.parse_string_literal()?;
                let local = Spanned {
                    span: s.span,
                    node: AssignIdentP {
                        ident: s.node.clone(),
                        payload: (),
                    },
                };
                Ok((local, s))
            }
            _ => Err(self.error_expected("either a literal string or an identifier")), // BLAZE PATCH 0054: Java Parser.java:1437
        }
    }

    // ==================== Expressions ====================

    /// Parse a comma-separated list of `Test` exprs, producing a Tuple if >1
    /// or trailing comma. When `allow_trailing_comma` is false, an
    /// unparenthesized tuple ending in a trailing comma is rejected — this
    /// matches the LALRPOP `UnparenTuple<Test>` production used at
    /// assignment-RHS and return-value positions.
    fn parse_test_list(&mut self, allow_trailing_comma: bool) -> Result<AstExpr, EvalException> {
        let l = self.pos();
        let first = self.parse_test()?;
        if self.peek() != Some(&Token::Comma) {
            return Ok(first);
        }
        self.parse_test_list_tail(l, first, allow_trailing_comma)
    }

    fn parse_test_list_tail(
        &mut self,
        l: usize,
        first: AstExpr,
        allow_trailing_comma: bool,
    ) -> Result<AstExpr, EvalException> {
        let mut items = vec![first];
        let mut trailing_comma = false;
        while self.eat(&Token::Comma) {
            trailing_comma = true;
            if self.is_test_start() {
                items.push(self.parse_test()?);
                trailing_comma = false;
            } else {
                break;
            }
        }
        let r = self.last_end;
        if items.len() == 1 && !trailing_comma {
            Ok(items.pop().expect("non-empty after parsing"))
        } else if trailing_comma && !allow_trailing_comma {
            grammar_util::reject_unparenthesized_tuple_trailing_comma(self.state.codemap, l, self.pos()) // BLAZE PATCH 0055
        } else {
            Ok(Expr::Tuple(items).ast(l, r))
        }
    }

    /// Parse a comma-separated list of `Expr` (BitOrExpr level), for assignment LHS
    /// and `for` variable. Excludes comparisons, `and`, `or`, `not`, `in`.
    /// In the LALRPOP grammar: `ExprList = UnparenTuple<Expr>` where
    /// `Expr = {BitOrExpr}`. Trailing commas are always rejected at these
    /// positions, so this function takes no `allow_trailing_comma` parameter.
    fn parse_expr_list(&mut self) -> Result<AstExpr, EvalException> {
        // BitOr has left_bp=7, so min_bp=7 excludes comparisons (5) and and/or (1-4).
        let l = self.pos();
        let first = self.parse_bitor_expr()?;
        if self.peek() != Some(&Token::Comma) {
            return Ok(first);
        }
        let mut items = vec![first];
        let mut trailing_comma = false;
        while self.eat(&Token::Comma) {
            trailing_comma = true;
            if self.is_expr_start() {
                items.push(self.parse_bitor_expr()?);
                trailing_comma = false;
            } else {
                break;
            }
        }
        let r = self.last_end;
        if items.len() == 1 && !trailing_comma {
            Ok(items.pop().expect("non-empty after parsing"))
        } else if trailing_comma {
            grammar_util::reject_unparenthesized_tuple_trailing_comma(self.state.codemap, l, self.pos()) // BLAZE PATCH 0055
        } else {
            Ok(Expr::Tuple(items).ast(l, r))
        }
    }

    /// Parse at the BitOrExpr precedence level (excludes comparisons, and, or).
    fn parse_bitor_expr(&mut self) -> Result<AstExpr, EvalException> {
        let lhs = self.parse_unary()?;
        self.continue_infix(lhs, 7)
    }

    fn is_test_start(&self) -> bool {
        self.is_expr_start() || matches!(self.peek(), Some(Token::Lambda))
    }

    fn is_expr_start(&self) -> bool {
        matches!(
            self.peek(),
            Some(
                Token::Identifier(_)
                    | Token::Int(_)
                    | Token::Float(_)
                    | Token::String(_)
                    | Token::FStringStart(_)
                    | Token::OpeningRound
                    | Token::OpeningSquare
                    | Token::OpeningCurly
                    | Token::Plus
                    | Token::Minus
                    | Token::Tilde
                    | Token::Not
                    | Token::Ellipsis
            )
        )
    }

    /// Parse a `Test` expression (includes ternary if/else and lambda).
    fn parse_test(&mut self) -> Result<AstExpr, EvalException> {
        if self.peek() == Some(&Token::Lambda) {
            return self.parse_lambda();
        }
        let expr = self.parse_expr(0)?;
        self.continue_ternary(expr)
    }

    /// Parse an OrTest (for use in ternary condition and comprehension `if` clauses).
    fn parse_or_test(&mut self) -> Result<AstExpr, EvalException> {
        self.parse_expr(0)
    }

    fn parse_lambda(&mut self) -> Result<AstExpr, EvalException> {
        let l = self.pos();
        self.consume(&Token::Lambda);
        let params = self.parse_comma_separated_lambda_params()?;
        self.expect(&Token::Colon)?;
        let body = self.parse_test()?;
        let r = self.last_end;
        Ok(Expr::Lambda(LambdaP {
            params,
            body: Box::new(body),
            payload: (),
        })
        .ast(l, r))
    }

    // ==================== Pratt Expression Parser ====================

    fn is_comparison(op: BinOp) -> bool {
        matches!(
            op,
            BinOp::Equal
                | BinOp::NotEqual
                | BinOp::Less
                | BinOp::Greater
                | BinOp::LessOrEqual
                | BinOp::GreaterOrEqual
                | BinOp::In
                | BinOp::NotIn
        )
    }

    /// Reject a comparison directly following another comparison: Starlark forbids
    /// chains like `a < b < c` because comparisons are not associative.
    fn reject_chained_comparison(&mut self, prev_op: Option<BinOp>) -> Result<(), EvalException> { // BLAZE PATCH 0054
        if let Some(next_tok) = self.peek() {
            if let Some((next_op, _, _)) = Self::infix_binding_power(next_tok) {
                if Self::is_comparison(next_op) {
                    // BLAZE PATCH 0054: Java's text (Parser.java:1028 parseBinOpExpression).
                    let (start, end) = self.current.as_ref().map(|(s, _, e)| (*s, *e)).unwrap(); // BLAZE PATCH 0054
                    let this = java_token_string(next_tok); // BLAZE PATCH 0054
                    let prev = prev_op.map(java_binop).unwrap_or_default(); // BLAZE PATCH 0054
                    return Err(self.error_at( // BLAZE PATCH 0054
                        start, // BLAZE PATCH 0054
                        end, // BLAZE PATCH 0054
                        format!("Operator '{prev}' is not associative with operator '{this}'. Use parens."), // BLAZE PATCH 0054
                    ));
                }
            }
        }
        Ok(())
    }

    /// Binding powers for binary operators, encoding the [Starlark operator
    /// precedence table][starlark-prec] (lowest precedence = lowest binding
    /// power).
    ///
    /// Returns `(op, left_bp, right_bp)`. For left-associative operators
    /// `left_bp < right_bp`; for non-associative comparisons `left_bp == right_bp`
    /// (chained comparisons like `a < b < c` are rejected by
    /// [`reject_chained_comparison`](Self::reject_chained_comparison)).
    ///
    /// [starlark-prec]: https://github.com/bazelbuild/starlark/blob/master/spec.md#operator-precedence
    fn infix_binding_power(tok: &Token) -> Option<(BinOp, u8, u8)> {
        let r = match tok {
            Token::Or => (BinOp::Or, 1, 2),
            Token::And => (BinOp::And, 3, 4),
            // Comparisons — not associative, so left_bp == right_bp
            Token::EqualEqual => (BinOp::Equal, 5, 6),
            Token::BangEqual => (BinOp::NotEqual, 5, 6),
            Token::LessThan => (BinOp::Less, 5, 6),
            Token::GreaterThan => (BinOp::Greater, 5, 6),
            Token::LessEqual => (BinOp::LessOrEqual, 5, 6),
            Token::GreaterEqual => (BinOp::GreaterOrEqual, 5, 6),
            Token::In => (BinOp::In, 5, 6),
            Token::Not => (BinOp::NotIn, 5, 6), // `not in` — handled specially
            Token::Pipe => (BinOp::BitOr, 7, 8),
            Token::Caret => (BinOp::BitXor, 9, 10),
            Token::Ampersand => (BinOp::BitAnd, 11, 12),
            Token::LessLess => (BinOp::LeftShift, 13, 14),
            Token::GreaterGreater => (BinOp::RightShift, 13, 14),
            Token::Plus => (BinOp::Add, 15, 16),
            Token::Minus => (BinOp::Subtract, 15, 16),
            Token::Star => (BinOp::Multiply, 17, 18),
            Token::Percent => (BinOp::Percent, 17, 18),
            Token::Slash => (BinOp::Divide, 17, 18),
            Token::SlashSlash => (BinOp::FloorDivide, 17, 18),
            _ => return None,
        };
        Some(r)
    }

    fn parse_expr(&mut self, min_bp: u8) -> Result<AstExpr, EvalException> {
        // Handle `not` as a prefix at binding power 5 (same level as comparisons).
        // In the LALRPOP grammar: NotTest = "not" NotTest | CompTest
        // This means `not x in y` parses as `not (x in y)`.
        let mut lhs = if self.peek() == Some(&Token::Not) && min_bp <= 5 {
            let l = self.pos();
            self.advance();
            let e = self.parse_expr(5)?;
            let r = self.last_end;
            Expr::Not(Box::new(e)).ast(l, r)
        } else {
            self.parse_unary()?
        };

        while let Some(tok) = self.peek() {
            // Handle `not in` as a two-token operator
            if matches!(tok, Token::Not) {
                let left_bp = 5u8;
                let right_bp = 6u8;
                if left_bp < min_bp {
                    break;
                }
                let l = lhs.span.begin();
                self.consume(&Token::Not);
                if self.peek() != Some(&Token::In) {
                    return Err(self.error_expected("'in'")); // BLAZE PATCH 0054: Java Parser.java:1015
                }
                self.consume(&Token::In);
                let rhs = self.parse_expr(right_bp)?;
                let r = rhs.span.end();
                lhs = Expr::Op(Box::new(lhs), BinOp::NotIn, Box::new(rhs))
                    .ast(l.get() as usize, r.get() as usize);
                self.reject_chained_comparison(Some(BinOp::NotIn))?; // BLAZE PATCH 0054
                continue;
            }

            let Some((op, left_bp, right_bp)) = Self::infix_binding_power(tok) else {
                break;
            };
            if left_bp < min_bp {
                break;
            }
            let l = lhs.span.begin();
            self.advance(); // consume operator
            let rhs = self.parse_expr(right_bp)?;
            let r = rhs.span.end();
            lhs =
                Expr::Op(Box::new(lhs), op, Box::new(rhs)).ast(l.get() as usize, r.get() as usize);
            if Self::is_comparison(op) {
                self.reject_chained_comparison(Some(op))?; // BLAZE PATCH 0054
            }
        }

        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<AstExpr, EvalException> {
        match self.peek() {
            Some(Token::Plus) => {
                let l = self.pos();
                self.advance();
                let e = self.parse_unary()?;
                let r = self.last_end;
                Ok(Expr::Plus(Box::new(e)).ast(l, r))
            }
            Some(Token::Minus) => {
                let l = self.pos();
                self.advance();
                let e = self.parse_unary()?;
                let r = self.last_end;
                Ok(Expr::Minus(Box::new(e)).ast(l, r))
            }
            Some(Token::Tilde) => {
                let l = self.pos();
                self.advance();
                let e = self.parse_unary()?;
                let r = self.last_end;
                Ok(Expr::BitNot(Box::new(e)).ast(l, r))
            }
            _ => self.parse_primary(),
        }
    }

    // ==================== Primary Expressions ====================

    fn parse_primary(&mut self) -> Result<AstExpr, EvalException> {
        let mut lhs = self.parse_atom()?;

        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    let l = lhs.span.begin().get() as usize;
                    self.consume(&Token::Dot);
                    let ident = self.parse_identifier_string()?;
                    let r = self.last_end;
                    lhs = Expr::Dot(Box::new(lhs), ident).ast(l, r);
                }
                Some(Token::OpeningRound) => {
                    let l = lhs.span.begin().get() as usize;
                    self.consume(&Token::OpeningRound);
                    let args = self.parse_comma_separated_args()?;
                    // BLAZE PATCH 0054: Java Parser.parseArguments expects `,` between
                    // arguments (Parser.java:583).
                    if !args.is_empty() // BLAZE PATCH 0054
                        && !matches!(self.peek(), Some(Token::ClosingRound | Token::For)) // BLAZE PATCH 0054
                    { // BLAZE PATCH 0054
                        return Err(self.error_java("expected ,")); // BLAZE PATCH 0054
                    } // BLAZE PATCH 0054
                    self.expect(&Token::ClosingRound)?;
                    let r = self.last_end;
                    lhs = Expr::check_call(lhs, args, &mut self.state).ast(l, r);
                }
                Some(Token::OpeningSquare) => {
                    let l = lhs.span.begin().get() as usize;
                    self.consume(&Token::OpeningSquare);
                    lhs = self.parse_index_or_slice(lhs, l)?;
                }
                _ => break,
            }
        }

        Ok(lhs)
    }

    fn parse_index_or_slice(&mut self, expr: AstExpr, l: usize) -> Result<AstExpr, EvalException> {
        // Could be: [test], [test, test], [test:test:test], or various slice forms
        if self.peek() == Some(&Token::Colon) {
            // [:...] — slice with no start
            self.advance();
            let stop = if self.peek() != Some(&Token::ClosingSquare)
                && self.peek() != Some(&Token::Colon)
            {
                Some(Box::new(self.parse_test()?))
            } else {
                None
            };
            let step = if self.eat(&Token::Colon) {
                if self.peek() != Some(&Token::ClosingSquare) {
                    Some(Box::new(self.parse_test()?))
                } else {
                    None
                }
            } else {
                None
            };
            self.expect(&Token::ClosingSquare)?;
            let r = self.last_end;
            Ok(Expr::Slice(Box::new(expr), None, stop, step).ast(l, r))
        } else {
            let first = self.parse_test()?;
            match self.peek() {
                Some(Token::Colon) => {
                    // Slice: [start:...]
                    self.advance();
                    let stop = if self.peek() != Some(&Token::ClosingSquare)
                        && self.peek() != Some(&Token::Colon)
                    {
                        Some(Box::new(self.parse_test()?))
                    } else {
                        None
                    };
                    let step = if self.eat(&Token::Colon) {
                        if self.peek() != Some(&Token::ClosingSquare) {
                            Some(Box::new(self.parse_test()?))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    self.expect(&Token::ClosingSquare)?;
                    let r = self.last_end;
                    Ok(Expr::Slice(Box::new(expr), Some(Box::new(first)), stop, step).ast(l, r))
                }
                Some(Token::Comma) => {
                    // Index2: [a, b]
                    self.advance();
                    let second = self.parse_test()?;
                    self.expect(&Token::ClosingSquare)?;
                    let r = self.last_end;
                    Ok(Expr::Index2(Box::new((expr, first, second))).ast(l, r))
                }
                _ => {
                    // Simple index: [a]
                    self.expect(&Token::ClosingSquare)?;
                    let r = self.last_end;
                    Ok(Expr::Index(Box::new((expr, first))).ast(l, r))
                }
            }
        }
    }

    fn parse_atom(&mut self) -> Result<AstExpr, EvalException> {
        // BLAZE PATCH 0020
        match self.at_type_keyword() {
            Some(TypeKeyword::Cast) => return self.parse_cast_expression(),
            Some(TypeKeyword::IsInstance) => return self.parse_is_instance_expression(),
            None => {}
        }
        match self.peek() {
            Some(Token::Identifier(_)) => {
                let (l, ident, r) = take_payload!(self, Token::Identifier);
                let ident_node = IdentP { ident, payload: () };
                Ok(Expr::Identifier(ident_node.ast(l, r)).ast(l, r))
            }
            Some(Token::Int(_)) => {
                let (l, i, r) = take_payload!(self, Token::Int);
                Ok(Expr::Literal(AstLiteral::Int(i.ast(l, r))).ast(l, r))
            }
            Some(Token::Float(_)) => {
                let (l, f, r) = take_payload!(self, Token::Float);
                Ok(Expr::Literal(AstLiteral::Float(f.ast(l, r))).ast(l, r))
            }
            Some(Token::String(_)) => {
                let l = self.pos();
                let s = self.parse_string_literal()?;
                let r = self.last_end;
                Ok(Expr::Literal(AstLiteral::String(s)).ast(l, r))
            }
            Some(Token::Bytes(_)) => {
                let (l, b, r) = take_payload!(self, Token::Bytes);
                Ok(Expr::Literal(AstLiteral::Bytes(b.ast(l, r))).ast(l, r))
            }
            Some(Token::FStringStart(_)) => {
                let (l, _, _) = take_payload!(self, Token::FStringStart);
                let mut parts = Vec::new();
                while !matches!(self.peek(), Some(Token::FStringEnd) | None) {
                    match self.peek() {
                        Some(Token::FStringText(_)) => {
                            let (tl, s, tr) = take_payload!(self, Token::FStringText);
                            parts.push(grammar_util::FStringElement::Text(s.ast(tl, tr)));
                        }
                        Some(Token::FStringExprStart) => {
                            self.advance();
                            let expr = self.parse_test()?;
                            let conv = match self.peek() {
                                Some(Token::FStringBang) => {
                                    let bl = self.pos();
                                    self.advance();
                                    if !matches!(self.peek(), Some(Token::Identifier(_))) {
                                        return Err(grammar_util::fstring_conv_error(
                                            bl,
                                            self.pos(),
                                            &self.state,
                                        ));
                                    }
                                    let (_, id, r) = take_payload!(self, Token::Identifier);
                                    match id.as_str() {
                                        "s" => lexer::FStringConv::Str,
                                        "r" => lexer::FStringConv::Repr,
                                        _ => {
                                            return Err(grammar_util::fstring_conv_error(
                                                bl,
                                                r,
                                                &self.state,
                                            ));
                                        }
                                    }
                                }
                                _ => lexer::FStringConv::None,
                            };
                            self.expect(&Token::FStringExprEnd)?;
                            parts.push(grammar_util::FStringElement::Expr(expr, conv));
                        }
                        _ => break,
                    }
                }
                self.expect(&Token::FStringEnd)?;
                let r = self.last_end;
                let fstring = grammar_util::fstring_from_parts(parts, l, r, &mut self.state);
                Ok(Expr::FString(fstring).ast(l, r))
            }
            Some(Token::Ellipsis) => {
                let l = self.pos();
                self.advance();
                let r = self.last_end;
                // BLAZE PATCH 0047: Java Parser.parsePrimary (Parser.java:775-778) under type
                // syntax: `...` only inside a type expression. (Without type syntax `validate`
                // rejects it; with type checking, upstream rules.)
                if self.type_syntax && !self.in_type_expr { // BLAZE PATCH 0047
                    let err = self.error_at( // BLAZE PATCH 0047
                        l, // BLAZE PATCH 0047
                        r, // BLAZE PATCH 0047
                        "syntax error at '...': ellipsis ('...') is not allowed outside type expressions".to_owned(), // BLAZE PATCH 0047 0054: Java's syntaxError prefix
                    ); // BLAZE PATCH 0047
                    self.state.errors.push(err); // BLAZE PATCH 0047
                } // BLAZE PATCH 0047
                Ok(Expr::Literal(AstLiteral::Ellipsis).ast(l, r))
            }
            Some(Token::OpeningRound) => {
                let l = self.pos();
                self.consume(&Token::OpeningRound);
                if self.eat(&Token::ClosingRound) {
                    let r = self.last_end;
                    return Ok(Expr::Tuple(vec![]).ast(l, r));
                }
                let expr = self.parse_test_list(true)?;
                self.expect(&Token::ClosingRound)?;
                Ok(expr)
            }
            Some(Token::OpeningSquare) => self.parse_list_or_comprehension(),
            Some(Token::OpeningCurly) => self.parse_dict_or_comprehension(),
            _ => Err(self.error_expected("expression")),
        }
    }

    fn parse_list_or_comprehension(&mut self) -> Result<AstExpr, EvalException> {
        let l = self.pos();
        self.consume(&Token::OpeningSquare);

        if self.eat(&Token::ClosingSquare) {
            let r = self.last_end;
            return Ok(Expr::List(vec![]).ast(l, r));
        }

        let first = self.parse_test()?;

        // Check for comprehension
        if self.peek() == Some(&Token::For) {
            let (for_clause, clauses) = self.parse_comp_clauses()?;
            self.expect_java(&Token::ClosingSquare, "expected ']', 'for' or 'if'")?; // BLAZE PATCH 0054: Java Parser.java:889
            let r = self.last_end;
            return Ok(
                Expr::ListComprehension(Box::new(first), Box::new(for_clause), clauses).ast(l, r),
            );
        }

        // Regular list
        // BLAZE PATCH 0054: Java Parser.java:949 parseListMaker.
        if !matches!(self.peek(), Some(Token::Comma | Token::ClosingSquare)) { // BLAZE PATCH 0054
            return Err(self.error_java("expected ',', 'for' or ']'")); // BLAZE PATCH 0054
        } // BLAZE PATCH 0054
        let mut items = vec![first];
        while self.eat(&Token::Comma) {
            if self.peek() == Some(&Token::ClosingSquare) {
                break;
            }
            items.push(self.parse_test()?);
        }
        self.expect(&Token::ClosingSquare)?;
        let r = self.last_end;
        Ok(Expr::List(items).ast(l, r))
    }

    fn parse_dict_or_comprehension(&mut self) -> Result<AstExpr, EvalException> {
        let l = self.pos();
        self.consume(&Token::OpeningCurly);

        if self.eat(&Token::ClosingCurly) {
            let r = self.last_end;
            return Ok(Expr::Dict(vec![]).ast(l, r));
        }

        let key = self.parse_test()?;
        self.expect(&Token::Colon)?;
        let value = self.parse_test()?;

        // Check for comprehension
        if self.peek() == Some(&Token::For) {
            let (for_clause, clauses) = self.parse_comp_clauses()?;
            self.expect_java(&Token::ClosingCurly, "expected '}', 'for' or 'if'")?; // BLAZE PATCH 0054: Java Parser.java:889
            let r = self.last_end;
            return Ok(Expr::DictComprehension(
                Box::new((key, value)),
                Box::new(for_clause),
                clauses,
            )
            .ast(l, r));
        }

        // Regular dict
        let mut entries = vec![(key, value)];
        while self.eat(&Token::Comma) {
            if self.peek() == Some(&Token::ClosingCurly) {
                break;
            }
            let k = self.parse_test()?;
            self.expect(&Token::Colon)?;
            let v = self.parse_test()?;
            entries.push((k, v));
        }
        self.expect(&Token::ClosingCurly)?;
        let r = self.last_end;
        Ok(Expr::Dict(entries).ast(l, r))
    }

    /// Parse the `{CompClause}` tail of a comprehension. `Comp` matches the
    /// spec's `CompClause` non-terminal: `for LoopVariables in Test | if Test`.
    fn parse_comp_clauses(&mut self) -> Result<(ForClause, Vec<Clause>), EvalException> {
        let for_clause = self.parse_for_clause()?;
        let mut clauses = Vec::new();
        loop {
            match self.peek() {
                Some(Token::For) => clauses.push(Clause::For(self.parse_for_clause()?)),
                Some(Token::If) => {
                    self.advance();
                    clauses.push(Clause::If(self.parse_or_test()?));
                }
                _ => break,
            }
        }
        Ok((for_clause, clauses))
    }

    fn parse_for_clause(&mut self) -> Result<ForClause, EvalException> {
        self.expect(&Token::For)?;
        let var_expr = self.parse_expr_list()?;
        self.expect(&Token::In)?;
        let over = self.parse_or_test()?;
        let var = grammar_util::check_assign(self.state.codemap, var_expr)?;
        Ok(ForClause { var, over })
    }

    // ==================== Helpers ====================

    fn parse_identifier_string(&mut self) -> Result<AstString, EvalException> {
        // BLAZE PATCH 0020: `cast` and `isinstance` are keywords when type syntax is allowed.
        if self.at_type_keyword().is_some() {
            return Err(self.error_expected("identifier"));
        }
        match &self.current {
            Some((_, Token::Identifier(_), _)) => match self.take_current() {
                Some((l, Token::Identifier(s), r)) => Ok(s.ast(l, r)),
                _ => unreachable!(),
            },
            _ => Err(self.error_expected("identifier")),
        }
    }

    fn parse_assign_ident(&mut self) -> Result<AstAssignIdent, EvalException> {
        let id = self.parse_identifier_string()?;
        Ok(Spanned {
            span: id.span,
            node: AssignIdentP {
                ident: id.node,
                payload: (),
            },
        })
    }

    fn parse_string_literal(&mut self) -> Result<AstString, EvalException> {
        match &self.current {
            Some((_, Token::String(_), _)) => match self.take_current() {
                Some((l, Token::String(s), r)) => Ok(s.ast(l, r)),
                _ => unreachable!(),
            },
            _ => Err(self.error_expected("string literal")),
        }
    }

    // ==================== BLAZE PATCH 0020: Bazel type syntax ====================

    /// The extra keyword at the current token, if type syntax is allowed.
    // Java: net/starlark/java/syntax/Lexer.java:615 Lexer.identifierOrKeyword
    fn at_type_keyword(&self) -> Option<TypeKeyword> {
        if !self.type_syntax {
            return None;
        }
        match self.peek() {
            Some(Token::Identifier(s)) if s == "cast" => Some(TypeKeyword::Cast),
            Some(Token::Identifier(s)) if s == "isinstance" => Some(TypeKeyword::IsInstance),
            _ => None,
        }
    }

    // cast_expression = 'cast' '(' TypeExpr ',' expr [','] ')'
    // Java: net/starlark/java/syntax/Parser.java:558 Parser.parseCastExpression
    // The value of a cast is the value of its operand (Eval.java:568).
    fn parse_cast_expression(&mut self) -> Result<AstExpr, EvalException> {
        self.advance();
        self.expect(&Token::OpeningRound)?;
        self.parse_type_expr()?;
        self.expect(&Token::Comma)?;
        let value = self.parse_test()?;
        self.eat(&Token::Comma);
        self.expect(&Token::ClosingRound)?;
        Ok(value)
    }

    // isinstance_expression = 'isinstance' '(' expr ',' TypeExpr [','] ')'
    // Java: net/starlark/java/syntax/Parser.java:573 Parser.parseIsInstanceExpression
    fn parse_is_instance_expression(&mut self) -> Result<AstExpr, EvalException> {
        let l = self.pos();
        self.advance();
        self.expect(&Token::OpeningRound)?;
        let value = self.parse_test()?;
        self.expect(&Token::Comma)?;
        self.parse_type_expr()?;
        self.eat(&Token::Comma);
        self.expect(&Token::ClosingRound)?;
        let r = self.last_end;
        // Java: net/starlark/java/syntax/Resolver.java:897 Resolver.visit(IsInstanceExpression)
        let err = self.error_at(l, r, "isinstance() is not yet supported".to_owned());
        self.state.errors.push(err);
        Ok(value)
    }

    // type_alias_stmt_tail = identifier optional_type_params '=' TypeExpr
    // Java: net/starlark/java/syntax/Parser.java:1235 Parser.parseTypeAliasStatementTail
    // Without type resolution the statement binds nothing (Resolver.java:623) and does nothing
    // (Eval.java:322).
    fn parse_type_alias_statement_tail(&mut self, l: usize) -> Result<AstStmt, EvalException> {
        self.parse_assign_ident()?;
        self.parse_optional_type_parameters()?;
        self.expect(&Token::Equal)?;
        self.parse_type_expr()?;
        let r = self.last_end;
        if self.def_depth > 0 {
            // Java: net/starlark/java/syntax/Resolver.java:906 Resolver.visit(TypeAliasStatement)
            let err = self.error_at(l, r, "type alias statement not at top level".to_owned());
            self.state.errors.push(err);
        }
        Ok(StmtP::Pass.ast(l, r))
    }

    // optional_type_params = ['[' identifier {',' identifier} [','] ']']
    // Java: net/starlark/java/syntax/Parser.java:1255 Parser.parseOptionalTypeParameters
    fn parse_optional_type_parameters(&mut self) -> Result<(), EvalException> {
        if !self.type_syntax || self.peek() != Some(&Token::OpeningSquare) {
            return Ok(());
        }
        self.advance();
        let mut unique: Vec<String> = Vec::new();
        loop {
            let id = self.parse_identifier_string()?;
            if unique.contains(&id.node) {
                let err = self.error_at(
                    id.span.begin().get() as usize,
                    id.span.end().get() as usize,
                    format!("duplicate type parameter '{}'", id.node),
                );
                self.state.errors.push(err);
            }
            unique.push(id.node);
            if self.peek() == Some(&Token::ClosingSquare) || self.peek().is_none() {
                break;
            }
            self.expect(&Token::Comma)?;
            if self.peek() == Some(&Token::ClosingSquare) {
                break;
            }
        }
        self.expect(&Token::ClosingSquare)?;
        Ok(())
    }

    fn parse_type_expr(&mut self) -> Result<AstTypeExpr, EvalException> {
        // BLAZE PATCH 0047: Java Parser.parseTypeExprWithFallback (Parser.java:1084-1095) sets
        // insideTypeExpr, and clears it (not restores it) afterwards.
        self.in_type_expr = true; // BLAZE PATCH 0047
        let expr = self.parse_test(); // BLAZE PATCH 0047
        self.in_type_expr = false; // BLAZE PATCH 0047
        grammar_util::dialect_check_type(&self.state, expr?) // BLAZE PATCH 0047
    }

    // ==================== Parameters ====================

    fn parse_comma_separated_def_params(&mut self) -> Result<Vec<AstParameter>, EvalException> {
        let mut params = Vec::new();
        if self.peek() == Some(&Token::ClosingRound) {
            return Ok(params);
        }
        loop {
            params.push(self.parse_def_param()?);
            if !self.eat(&Token::Comma) {
                break;
            }
            if self.peek() == Some(&Token::ClosingRound) {
                break;
            }
        }
        Ok(params)
    }

    fn parse_def_param(&mut self) -> Result<AstParameter, EvalException> {
        let l = self.pos();
        match self.peek() {
            Some(Token::Slash) => {
                self.advance();
                let r = self.last_end;
                Ok(Parameter::Slash.ast(l, r))
            }
            Some(Token::StarStar) => {
                self.advance();
                let name = self.parse_assign_ident()?;
                let ty = self.parse_optional_type()?;
                let r = self.last_end;
                Ok(Parameter::KwArgs(name, ty).ast(l, r))
            }
            Some(Token::Star) => {
                self.advance();
                if let Some(Token::Identifier(_)) = self.peek() {
                    let name = self.parse_assign_ident()?;
                    let ty = self.parse_optional_type()?;
                    let r = self.last_end;
                    Ok(Parameter::Args(name, ty).ast(l, r))
                } else {
                    let r = self.last_end;
                    Ok(Parameter::NoArgs.ast(l, r))
                }
            }
            _ => {
                let name = self.parse_assign_ident()?;
                let ty = self.parse_optional_type()?;
                if self.eat(&Token::Equal) {
                    let default = self.parse_test()?;
                    let r = self.last_end;
                    Ok(Parameter::Normal(name, ty, Some(Box::new(default))).ast(l, r))
                } else {
                    let r = self.last_end;
                    Ok(Parameter::Normal(name, ty, None).ast(l, r))
                }
            }
        }
    }

    fn parse_comma_separated_lambda_params(&mut self) -> Result<Vec<AstParameter>, EvalException> {
        let mut params = Vec::new();
        if self.peek() == Some(&Token::Colon) {
            return Ok(params);
        }
        loop {
            params.push(self.parse_lambda_param()?);
            if !self.eat(&Token::Comma) {
                break;
            }
            if self.peek() == Some(&Token::Colon) {
                break;
            }
        }
        Ok(params)
    }

    fn parse_lambda_param(&mut self) -> Result<AstParameter, EvalException> {
        let l = self.pos();
        match self.peek() {
            Some(Token::Slash) => {
                self.advance();
                let r = self.last_end;
                Ok(Parameter::Slash.ast(l, r))
            }
            Some(Token::StarStar) => {
                self.advance();
                let name = self.parse_assign_ident()?;
                let r = self.last_end;
                Ok(Parameter::KwArgs(name, None).ast(l, r))
            }
            Some(Token::Star) => {
                self.advance();
                if let Some(Token::Identifier(_)) = self.peek() {
                    let name = self.parse_assign_ident()?;
                    let r = self.last_end;
                    Ok(Parameter::Args(name, None).ast(l, r))
                } else {
                    let r = self.last_end;
                    Ok(Parameter::NoArgs.ast(l, r))
                }
            }
            _ => {
                let name = self.parse_assign_ident()?;
                if self.eat(&Token::Equal) {
                    let default = self.parse_test()?;
                    let r = self.last_end;
                    Ok(Parameter::Normal(name, None, Some(Box::new(default))).ast(l, r))
                } else {
                    let r = self.last_end;
                    Ok(Parameter::Normal(name, None, None).ast(l, r))
                }
            }
        }
    }

    fn parse_optional_type(&mut self) -> Result<Option<Box<AstTypeExpr>>, EvalException> {
        if self.eat(&Token::Colon) {
            Ok(Some(Box::new(self.parse_type_expr()?)))
        } else {
            Ok(None)
        }
    }

    // ==================== Arguments ====================

    fn parse_comma_separated_args(&mut self) -> Result<Vec<AstArgument>, EvalException> {
        let mut args = Vec::new();
        if self.peek() == Some(&Token::ClosingRound) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_argument()?);
            if !self.eat(&Token::Comma) {
                break;
            }
            if self.peek() == Some(&Token::ClosingRound) {
                break;
            }
        }
        Ok(args)
    }

    fn parse_argument(&mut self) -> Result<AstArgument, EvalException> {
        let l = self.pos();
        match self.peek() {
            Some(Token::StarStar) => {
                self.advance();
                let e = self.parse_test()?;
                let r = self.last_end;
                Ok(Argument::KwArgs(e).ast(l, r))
            }
            Some(Token::Star) => {
                self.advance();
                let e = self.parse_test()?;
                let r = self.last_end;
                Ok(Argument::Args(e).ast(l, r))
            }
            // BLAZE PATCH 0048: under type syntax `cast` and `isinstance` are keywords
            // (Lexer.java:615), so `f(cast(T, x))` is a cast expression argument
            // (Parser.parseArgument -> parseTest), not a call of a name `cast`.
            Some(Token::Identifier(_)) if self.at_type_keyword().is_none() => { // BLAZE PATCH 0048
                // Could be `ident = expr` (named) or just a positional expr
                // We need to tentatively parse as identifier, then check for `=`
                let (id_l, ident, id_r) = take_payload!(self, Token::Identifier);

                if self.eat(&Token::Equal) {
                    // Named argument
                    let name: AstString = ident.ast(id_l, id_r);
                    let value = self.parse_test()?;
                    let r = self.last_end;
                    Ok(Argument::Named(name, value).ast(l, r))
                } else {
                    // It's a positional argument starting with an identifier
                    // We already consumed the identifier, so rebuild as an expr and continue
                    let ident_expr =
                        Expr::Identifier(IdentP { ident, payload: () }.ast(id_l, id_r))
                            .ast(id_l, id_r);
                    // Continue parsing as primary suffix + infix
                    let expr = self.continue_primary(ident_expr)?;
                    let expr = self.continue_infix(expr, 0)?;
                    // Handle ternary if
                    let expr = self.continue_ternary(expr)?;
                    let r = self.last_end;
                    Ok(Argument::Positional(expr).ast(l, r))
                }
            }
            _ => {
                let e = self.parse_test()?;
                let r = self.last_end;
                Ok(Argument::Positional(e).ast(l, r))
            }
        }
    }

    /// Continue parsing primary suffixes (.attr, [index], (call)) on an already-parsed LHS.
    fn continue_primary(&mut self, mut lhs: AstExpr) -> Result<AstExpr, EvalException> {
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    let l = lhs.span.begin().get() as usize;
                    self.advance();
                    let ident = self.parse_identifier_string()?;
                    let r = self.last_end;
                    lhs = Expr::Dot(Box::new(lhs), ident).ast(l, r);
                }
                Some(Token::OpeningRound) => {
                    let l = lhs.span.begin().get() as usize;
                    self.advance();
                    let args = self.parse_comma_separated_args()?;
                    // BLAZE PATCH 0054: Java Parser.parseArguments expects `,` between
                    // arguments (Parser.java:583).
                    if !args.is_empty() // BLAZE PATCH 0054
                        && !matches!(self.peek(), Some(Token::ClosingRound | Token::For)) // BLAZE PATCH 0054
                    { // BLAZE PATCH 0054
                        return Err(self.error_java("expected ,")); // BLAZE PATCH 0054
                    } // BLAZE PATCH 0054
                    self.expect(&Token::ClosingRound)?;
                    let r = self.last_end;
                    lhs = Expr::check_call(lhs, args, &mut self.state).ast(l, r);
                }
                Some(Token::OpeningSquare) => {
                    let l = lhs.span.begin().get() as usize;
                    self.advance();
                    lhs = self.parse_index_or_slice(lhs, l)?;
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    /// Continue parsing infix operators on an already-parsed LHS.
    fn continue_infix(&mut self, mut lhs: AstExpr, min_bp: u8) -> Result<AstExpr, EvalException> {
        while let Some(tok) = self.peek() {
            if matches!(tok, Token::Not) {
                let (_, left_bp, right_bp) = (BinOp::NotIn, 5u8, 6u8);
                if left_bp < min_bp {
                    break;
                }
                let l = lhs.span.begin();
                self.advance();
                if self.peek() != Some(&Token::In) {
                    return Err(self.error_expected("'in'")); // BLAZE PATCH 0054: Java Parser.java:1015
                }
                self.advance();
                let rhs = self.parse_expr(right_bp)?;
                let r = rhs.span.end();
                lhs = Expr::Op(Box::new(lhs), BinOp::NotIn, Box::new(rhs))
                    .ast(l.get() as usize, r.get() as usize);
                self.reject_chained_comparison(Some(BinOp::NotIn))?; // BLAZE PATCH 0054
                continue;
            }

            let Some((op, left_bp, right_bp)) = Self::infix_binding_power(tok) else {
                break;
            };
            if left_bp < min_bp {
                break;
            }
            let l = lhs.span.begin();
            self.advance();
            let rhs = self.parse_expr(right_bp)?;
            let r = rhs.span.end();
            lhs =
                Expr::Op(Box::new(lhs), op, Box::new(rhs)).ast(l.get() as usize, r.get() as usize);
            if Self::is_comparison(op) {
                self.reject_chained_comparison(Some(op))?; // BLAZE PATCH 0054
            }
        }
        Ok(lhs)
    }

    /// Handle ternary `expr if cond else expr` after the first expression.
    fn continue_ternary(&mut self, expr: AstExpr) -> Result<AstExpr, EvalException> {
        if self.peek() == Some(&Token::If) {
            let l = expr.span.begin().get() as usize;
            self.advance();
            let cond = self.parse_or_test()?;
            // BLAZE PATCH 0054: Java Parser.java:1306 parseTest, at the start of the expression.
            if self.peek() != Some(&Token::Else) { // BLAZE PATCH 0054
                if let Some(err) = self.pending_error.take() { // BLAZE PATCH 0054
                    return Err(err); // BLAZE PATCH 0054
                } // BLAZE PATCH 0054
                return Err(self.error_at(l, l, "missing else clause in conditional expression or semicolon before if".to_owned())); // BLAZE PATCH 0054
            } // BLAZE PATCH 0054
            self.expect(&Token::Else)?;
            let else_expr = self.parse_test()?;
            let r = self.last_end;
            Ok(Expr::If(Box::new((cond, expr, else_expr))).ast(l, r))
        } else {
            Ok(expr)
        }
    }
}

/// Recursive descent + Pratt parse entry point.
///
/// The internal `ParserRd<'a, I>` struct in this module owns its `ParserState`,
/// so we construct a temporary owned `ParserState` by reborrowing the caller's
/// `&mut Vec<EvalException>`. The temporary lives only for this call; on
/// return, the borrow is released and the caller regains exclusive access.
pub(crate) fn parse_module<I: Iterator<Item = Lexeme>>(
    state: &mut ParserState<'_>,
    tokens: I,
) -> Result<AstStmt, EvalException> {
    let temp_state = ParserState {
        dialect: state.dialect,
        codemap: state.codemap,
        errors: &mut *state.errors,
    };
    ParserRd::new(tokens, temp_state).parse_module()
}

/// BLAZE PATCH 0054: Java `TokenKind.toString` of the token kind (net/starlark/java/syntax/
/// TokenKind.java), as `syntax error at 'TOKEN'` prints a token without a value.
fn java_kind(tok: &Token) -> Option<&'static str> {
    Some(match tok {
        Token::Indent => "indent",
        Token::Dedent => "outdent",
        Token::Newline => "newline",
        Token::And => "and",
        Token::Else => "else",
        Token::Load => "load",
        Token::Break => "break",
        Token::For => "for",
        Token::Not => "not",
        Token::Continue => "continue",
        Token::If => "if",
        Token::Or => "or",
        Token::Def => "def",
        Token::In => "in",
        Token::Pass => "pass",
        Token::Elif => "elif",
        Token::Return => "return",
        Token::Lambda => "lambda",
        Token::Comma => ",",
        Token::Semicolon => ";",
        Token::Colon => ":",
        Token::PlusEqual => "+=",
        Token::MinusEqual => "-=",
        Token::StarEqual => "*=",
        Token::SlashEqual => "/=",
        Token::SlashSlashEqual => "//=",
        Token::PercentEqual => "%=",
        Token::EqualEqual => "==",
        Token::BangEqual => "!=",
        Token::LessEqual => "<=",
        Token::GreaterEqual => ">=",
        Token::StarStar => "**",
        Token::MinusGreater => "->",
        Token::Equal => "=",
        Token::LessThan => "<",
        Token::GreaterThan => ">",
        Token::Minus => "-",
        Token::Plus => "+",
        Token::Star => "*",
        Token::Percent => "%",
        Token::Slash => "/",
        Token::SlashSlash => "//",
        Token::Dot => ".",
        Token::Ampersand => "&",
        Token::Pipe => "|",
        Token::Caret => "^",
        Token::LessLess => "<<",
        Token::GreaterGreater => ">>",
        Token::Tilde => "~",
        Token::AmpersandEqual => "&=",
        Token::PipeEqual => "|=",
        Token::CaretEqual => "^=",
        Token::LessLessEqual => "<<=",
        Token::GreaterGreaterEqual => ">>=",
        Token::Ellipsis => "...",
        Token::OpeningSquare => "[",
        Token::OpeningCurly => "{",
        Token::OpeningRound => "(",
        Token::ClosingSquare => "]",
        Token::ClosingCurly => "}",
        Token::ClosingRound => ")",
        _ => return None,
    })
}

/// BLAZE PATCH 0054: Java `Parser.tokenString` (Parser.java:188): a string literal's value in
/// double quotes, an identifier's name, a number's value, else the kind.
fn java_token_string(tok: &Token) -> String {
    match tok {
        Token::String(s) => format!("\"{s}\""),
        Token::Identifier(s) => s.clone(),
        Token::Int(i) => match i {
            crate::lexer::TokenInt::I32(i) => i.to_string(),
            crate::lexer::TokenInt::BigInt(i) => i.to_string(),
        },
        Token::Float(f) => java_double_to_string(*f),
        _ => match java_kind(tok) {
            Some(k) => k.to_owned(),
            None => tok.to_string(),
        },
    }
}

/// BLAZE PATCH 0054: Java `Double.toString` (shortest digits; plain notation for 1e-3 <= |d| <
/// 1e7 with at least one fraction digit, else `d.dddE[-]n`).
fn java_double_to_string(d: f64) -> String {
    if d.is_nan() {
        return "NaN".to_owned();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if d == 0.0 {
        return if d.is_sign_negative() { "-0.0" } else { "0.0" }.to_owned();
    }
    let e = format!("{:e}", d.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if d < 0.0 { "-" } else { "" };
    let a = d.abs();
    if (1e-3..1e7).contains(&a) {
        let s = if exp >= 0 {
            let int_len = exp as usize + 1;
            let mut ds = digits.clone();
            while ds.len() < int_len {
                ds.push('0');
            }
            let (i, f) = ds.split_at(int_len);
            format!("{i}.{}", if f.is_empty() { "0" } else { f })
        } else {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        };
        format!("{sign}{s}")
    } else {
        let (i, f) = digits.split_at(1);
        format!("{sign}{i}.{}E{exp}", if f.is_empty() { "0" } else { f })
    }
}

/// BLAZE PATCH 0054: Java's spelling of what the parser expected (`expected <TokenKind>` and
/// the parser's own phrases).
fn java_expected(what: &str) -> String {
    let w = what
        .strip_prefix("symbol '")
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| what.strip_prefix("keyword '").and_then(|s| s.strip_suffix('\'')));
    match (w, what) {
        (Some(k), _) => format!("expected {k}"),
        (None, "new line") => "expected newline".to_owned(),
        (None, "new indentation block") => "expected an indented block".to_owned(),
        (None, w) => format!("expected {w}"),
    }
}

/// BLAZE PATCH 0054: Java `TokenKind.toString` of a binary operator.
fn java_binop(op: BinOp) -> &'static str {
    match op {
        BinOp::Equal => "==",
        BinOp::NotEqual => "!=",
        BinOp::Less => "<",
        BinOp::Greater => ">",
        BinOp::LessOrEqual => "<=",
        BinOp::GreaterOrEqual => ">=",
        BinOp::In => "in",
        BinOp::NotIn => "not in",
        _ => "",
    }
}
