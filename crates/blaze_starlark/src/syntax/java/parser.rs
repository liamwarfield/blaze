//! Port of Bazel's recursive-descent Starlark parser with its error recovery.
//!
//! Java: net/starlark/java/syntax/Parser.java.

use super::JavaOptions;
use super::ast::*;
use super::lexer::Lexer;
use super::lexer::OffsetError;
use super::lexer::TokenKind;
use super::lexer::TokenKind as T;
use super::lexer::TokenValue;

const STATEMENT_TERMINATOR_SET: &[TokenKind] =
    &[T::Eof, T::Newline, T::DocCommentTrailing, T::Semi];
const LIST_TERMINATOR_SET: &[TokenKind] = &[T::Eof, T::RBracket, T::Semi];
const DICT_TERMINATOR_SET: &[TokenKind] = &[T::Eof, T::RBrace, T::Semi];
const EXPR_LIST_TERMINATOR_SET: &[TokenKind] = &[
    T::Eof,
    T::Newline,
    T::DocCommentTrailing,
    T::Equals,
    T::RBrace,
    T::RBracket,
    T::RParen,
    T::Semi,
];
const EXPR_TERMINATOR_SET: &[TokenKind] = &[
    T::Colon,
    T::Comma,
    T::Eof,
    T::For,
    T::Minus,
    T::Percent,
    T::Plus,
    T::RBracket,
    T::RParen,
    T::Slash,
];

const OPERATOR_PRECEDENCE: &[&[TokenKind]] = &[
    &[T::Or],
    &[T::And],
    &[T::Not],
    &[
        T::EqualsEquals,
        T::NotEquals,
        T::Less,
        T::LessEquals,
        T::Greater,
        T::GreaterEquals,
        T::In,
        T::NotIn,
    ],
    &[T::Pipe],
    &[T::Caret],
    &[T::Ampersand],
    &[T::GreaterGreater, T::LessLess],
    &[T::Minus, T::Plus],
    &[T::Slash, T::SlashSlash, T::Star, T::Percent],
];

fn augmented(kind: TokenKind) -> Option<TokenKind> {
    Some(match kind {
        T::PlusEquals => T::Plus,
        T::MinusEquals => T::Minus,
        T::StarEquals => T::Star,
        T::SlashEquals => T::Slash,
        T::SlashSlashEquals => T::SlashSlash,
        T::PercentEquals => T::Percent,
        T::AmpersandEquals => T::Ampersand,
        T::CaretEquals => T::Caret,
        T::PipeEquals => T::Pipe,
        T::GreaterGreaterEquals => T::GreaterGreater,
        T::LessLessEquals => T::LessLess,
        _ => return None,
    })
}

const FORBIDDEN_KEYWORDS: &[TokenKind] = &[
    T::As,
    T::Assert,
    T::Class,
    T::Del,
    T::Except,
    T::Finally,
    T::From,
    T::Global,
    T::Import,
    T::Is,
    T::Nonlocal,
    T::Raise,
    T::Try,
    T::With,
    T::While,
    T::Yield,
];

/// Java `Parser.tokenString`.
// Java: net/starlark/java/syntax/Parser.java:188 Parser.tokenString
fn token_string(kind: TokenKind, value: &TokenValue) -> String {
    match (kind, value) {
        (T::String, TokenValue::Str(s)) => format!("\"{s}\""),
        (_, TokenValue::Str(s)) => s.clone(),
        (_, TokenValue::Int(s)) => s.clone(),
        (_, TokenValue::Float(f)) => java_double_to_string(*f),
        // A doc comment's value is a DocComments object; its toString is not used by Bazel's
        // error paths that reach a user.
        _ => kind.to_string(),
    }
}

// Java: net/starlark/java/syntax/Parser.java:33 Parser
pub struct Parser<'a> {
    lexer: Lexer<'a>,
    options: JavaOptions,
    pub errors: Vec<OffsetError>,
    errors_count: usize,
    recovery_mode: bool,
    inside_type_expr: bool,
}

impl<'a> Parser<'a> {
    pub fn new(lexer: Lexer<'a>, options: JavaOptions) -> Self {
        let mut p = Parser {
            lexer,
            options,
            errors: Vec::new(),
            errors_count: 0,
            recovery_mode: false,
            inside_type_expr: false,
        };
        p.next_token();
        p
    }

    fn kind(&self) -> TokenKind {
        self.lexer.kind
    }

    // Java: net/starlark/java/syntax/Parser.java:321 Parser.reportError
    fn report_error(&mut self, offset: usize, message: String) {
        self.errors_count += 1;
        if self.errors_count <= 5 {
            self.errors.push(OffsetError { offset, message });
        }
    }

    fn syntax_error(&mut self, message: &str) {
        let (s, k, v) = (self.lexer.start, self.lexer.kind, self.lexer.value.clone());
        self.syntax_error_at(s, k, &v, message);
    }

    // Java: net/starlark/java/syntax/Parser.java:334 Parser.syntaxError
    fn syntax_error_at(
        &mut self,
        offset: usize,
        kind: TokenKind,
        value: &TokenValue,
        message: &str,
    ) {
        if !self.recovery_mode {
            if kind == T::Indent {
                self.report_error(offset, "indentation error".to_owned());
            } else {
                let t = token_string(kind, value);
                self.report_error(offset, format!("syntax error at '{t}': {message}"));
            }
            self.recovery_mode = true;
        }
    }

    fn expect(&mut self, kind: TokenKind) -> usize {
        if self.kind() != kind {
            self.syntax_error(&format!("expected {kind}"));
        }
        self.next_token()
    }

    fn expect_and_recover(&mut self, kind: TokenKind) -> usize {
        if self.kind() != kind {
            self.syntax_error(&format!("expected {kind}"));
        } else {
            self.recovery_mode = false;
        }
        self.next_token()
    }

    // Java: net/starlark/java/syntax/Parser.java:364 Parser.syncPast
    fn sync_past(&mut self, set: &[TokenKind]) -> usize {
        while !set.contains(&self.kind()) {
            self.next_token();
        }
        let end = self.lexer.end;
        self.next_token();
        end
    }

    // Java: net/starlark/java/syntax/Parser.java:382 Parser.syncTo
    fn sync_to(&mut self, set: &[TokenKind]) -> usize {
        let mut previous = self.lexer.end;
        self.next_token();
        let mut current = previous;
        while !set.contains(&self.kind()) {
            self.next_token();
            previous = current;
            current = self.lexer.end;
        }
        previous
    }

    // Java: net/starlark/java/syntax/Parser.java:420 Parser.checkForbiddenKeywords
    fn check_forbidden_keywords(&mut self) {
        let k = self.kind();
        if !FORBIDDEN_KEYWORDS.contains(&k) {
            return;
        }
        let message = match k {
            T::Assert => "'assert' not supported, use 'fail' instead".to_owned(),
            T::Del => {
                "'del' not supported, use '.pop()' to delete an item from a dictionary or a list"
                    .to_owned()
            }
            T::Import => "'import' not supported, use 'load' instead".to_owned(),
            T::Is => "'is' not supported, use '==' instead".to_owned(),
            T::Raise => "'raise' not supported, use 'fail' instead".to_owned(),
            T::Try => "'try' not supported, all exceptions are fatal".to_owned(),
            T::While => "'while' not supported, use 'for' instead".to_owned(),
            k => format!("keyword '{k}' not supported"),
        };
        let s = self.lexer.start;
        self.report_error(s, message);
    }

    // Java: net/starlark/java/syntax/Parser.java:440 Parser.nextToken
    fn next_token(&mut self) -> usize {
        let prev = self.lexer.start;
        if self.kind() != T::Eof {
            self.lexer.next_token(&mut self.errors);
        }
        self.check_forbidden_keywords();
        prev
    }

    fn make_error_expression(&self, start: usize, end: usize) -> Expr {
        Expr::Ident(Ident {
            name: self.lexer.buffer_slice(start, end),
            start,
        })
    }

    // Java: net/starlark/java/syntax/Parser.java:462 Parser.parseArgument
    fn parse_argument(&mut self) -> Argument {
        if self.kind() == T::StarStar {
            let o = self.next_token();
            let e = self.parse_test();
            return Argument::StarStar(o, e);
        }
        if self.kind() == T::Star {
            let o = self.next_token();
            let e = self.parse_test();
            return Argument::Star(o, e);
        }
        let expr = self.parse_test();
        if let Expr::Ident(id) = &expr {
            if self.kind() == T::Equals {
                let id = id.clone();
                self.next_token();
                let arg = self.parse_test();
                return Argument::Keyword(id, arg);
            }
        }
        Argument::Positional(expr)
    }

    // Java: net/starlark/java/syntax/Parser.java:497 Parser.parseParameter
    fn parse_parameter(&mut self, def_statement: bool) -> Parameter {
        if self.kind() == T::StarStar {
            let o = self.next_token();
            let id = self.parse_ident();
            let ty = if def_statement {
                self.maybe_parse_type_annotation_after(T::Colon)
            } else {
                None
            };
            return Parameter::StarStar(o, id, ty);
        }
        if self.kind() == T::Star {
            let o = self.next_token();
            if self.kind() == T::Identifier {
                let id = self.parse_ident();
                let ty = if def_statement {
                    self.maybe_parse_type_annotation_after(T::Colon)
                } else {
                    None
                };
                return Parameter::Star(o, Some(id), ty);
            }
            return Parameter::Star(o, None, None);
        }
        let id = self.parse_ident();
        let ty = if def_statement {
            self.maybe_parse_type_annotation_after(T::Colon)
        } else {
            None
        };
        if self.kind() == T::Equals {
            self.next_token();
            let e = self.parse_test();
            return Parameter::Optional(id, ty, e);
        }
        Parameter::Mandatory(id, ty)
    }

    fn parse_call_suffix(&mut self, func: Expr) -> Expr {
        let mut args = Vec::new();
        self.expect(T::LParen);
        if self.kind() != T::RParen {
            args = self.parse_arguments();
        }
        self.expect(T::RParen);
        Expr::Call {
            func: Box::new(func),
            args,
        }
    }

    fn check_allow_type_syntax(
        &mut self,
        offset: usize,
        kind: TokenKind,
        value: &TokenValue,
    ) -> bool {
        if self.options.allow_type_syntax {
            true
        } else {
            self.syntax_error_at(offset, kind, value, "type annotations are disallowed");
            false
        }
    }

    fn parse_cast_expression(&mut self) -> Expr {
        let (s, k, v) = (self.lexer.start, self.kind(), self.lexer.value.clone());
        self.check_allow_type_syntax(s, k, &v);
        let start = self.expect(T::Cast);
        self.expect(T::LParen);
        let ty = self.parse_type_expr_with_fallback();
        self.expect(T::Comma);
        let value = self.parse_test();
        if self.kind() == T::Comma {
            self.expect(T::Comma);
        }
        self.expect(T::RParen);
        Expr::Cast {
            start,
            ty: Box::new(ty),
            value: Box::new(value),
        }
    }

    fn parse_is_instance_expression(&mut self) -> Expr {
        let (s, k, v) = (self.lexer.start, self.kind(), self.lexer.value.clone());
        self.check_allow_type_syntax(s, k, &v);
        let start = self.expect(T::IsInstance);
        self.expect(T::LParen);
        let value = self.parse_test();
        self.expect(T::Comma);
        let ty = self.parse_type_expr_with_fallback();
        if self.kind() == T::Comma {
            self.expect(T::Comma);
        }
        self.expect(T::RParen);
        Expr::IsInstance {
            start,
            value: Box::new(value),
            ty: Box::new(ty),
        }
    }

    // Java: net/starlark/java/syntax/Parser.java:588 Parser.parseArguments
    fn parse_arguments(&mut self) -> Vec<Argument> {
        let mut seen = false;
        let mut list = Vec::new();
        while self.kind() != T::RParen && self.kind() != T::Eof {
            if seen {
                if self.kind() == T::For {
                    self.syntax_error(
                        "Starlark does not support Python-style generator expressions",
                    );
                }
                self.expect(T::Comma);
                if self.kind() == T::RParen {
                    break;
                }
            }
            list.push(self.parse_argument());
            seen = true;
        }
        list
    }

    fn parse_selector_suffix(&mut self, e: Expr) -> Expr {
        self.expect(T::Dot);
        if self.kind() == T::Identifier {
            let id = self.parse_ident();
            return Expr::Dot {
                obj: Box::new(e),
                field: id,
            };
        }
        self.syntax_error("expected identifier after dot");
        self.sync_to(EXPR_TERMINATOR_SET);
        e
    }

    fn parse_expr_list(&mut self, list: &mut Vec<Expr>, trailing_comma_allowed: bool) {
        while self.kind() == T::Comma {
            self.expect(T::Comma);
            if EXPR_LIST_TERMINATOR_SET.contains(&self.kind()) {
                if !trailing_comma_allowed {
                    let s = self.lexer.start;
                    self.report_error(
                        s,
                        "Trailing comma is allowed only in parenthesized tuples.".to_owned(),
                    );
                }
                break;
            }
            list.push(self.parse_test());
        }
    }

    fn parse_dict_entry_list(&mut self) -> Vec<(Expr, Expr)> {
        let mut list = Vec::new();
        while self.kind() != T::RBrace {
            list.push(self.parse_dict_entry());
            if self.kind() == T::Comma {
                self.next_token();
            } else {
                break;
            }
        }
        list
    }

    fn parse_dict_entry(&mut self) -> (Expr, Expr) {
        let key = self.parse_test();
        self.expect(T::Colon);
        let value = self.parse_test();
        (key, value)
    }

    fn parse_string_literal(&mut self) -> Expr {
        // The token's value is moved out: nothing reads it after `next_token`.
        let value = match std::mem::replace(&mut self.lexer.value, TokenValue::None) {
            TokenValue::Str(s) => s,
            _ => String::new(),
        };
        let start = self.lexer.start;
        self.next_token();
        if self.kind() == T::String {
            let s = self.lexer.start;
            self.report_error(
                s,
                "Implicit string concatenation is forbidden, use the + operator".to_owned(),
            );
        }
        Expr::Str { start, value }
    }

    // Java: net/starlark/java/syntax/Parser.java:690 Parser.parsePrimary
    fn parse_primary(&mut self) -> Expr {
        match self.kind() {
            T::Int => {
                let value = match std::mem::replace(&mut self.lexer.value, TokenValue::None) {
                    TokenValue::Int(s) => s,
                    _ => "0".to_owned(),
                };
                let start = self.lexer.start;
                self.next_token();
                Expr::Int { start, value }
            }
            T::Float => {
                let value = match &self.lexer.value {
                    TokenValue::Float(f) => *f,
                    _ => 0.0,
                };
                let start = self.lexer.start;
                self.next_token();
                Expr::Float { start, value }
            }
            T::String => self.parse_string_literal(),
            T::Identifier => Expr::Ident(self.parse_ident()),
            T::LBracket => self.parse_list_maker(),
            T::LBrace => self.parse_dict_expression(),
            T::LParen => {
                let lparen = self.next_token();
                if self.kind() == T::RParen {
                    self.next_token();
                    return Expr::List {
                        is_tuple: true,
                        start: Some(lparen),
                        elems: Vec::new(),
                    };
                }
                let e = self.parse_test();
                if self.kind() == T::RParen {
                    self.next_token();
                    return e;
                }
                if self.kind() == T::Comma {
                    let mut elems = vec![e];
                    self.parse_expr_list(&mut elems, true);
                    self.expect(T::RParen);
                    return Expr::List {
                        is_tuple: true,
                        start: Some(lparen),
                        elems,
                    };
                }
                if self.kind() == T::For {
                    self.syntax_error(
                        "Starlark does not support Python-style generator expressions",
                    );
                }
                self.expect(T::RParen);
                let end = self.sync_to(EXPR_TERMINATOR_SET);
                self.make_error_expression(lparen, end)
            }
            T::Minus | T::Plus | T::Tilde => {
                let op = self.kind();
                let offset = self.next_token();
                let x = self.parse_primary_with_suffix();
                Expr::Unary {
                    op,
                    offset,
                    x: Box::new(x),
                }
            }
            T::Cast => self.parse_cast_expression(),
            T::IsInstance => self.parse_is_instance_expression(),
            T::Ellipsis => {
                if !self.inside_type_expr {
                    self.syntax_error("ellipsis ('...') is not allowed outside type expressions");
                }
                let start = self.next_token();
                Expr::Ellipsis { start }
            }
            _ => {
                let start = self.lexer.start;
                self.syntax_error("expected expression");
                let end = self.sync_to(EXPR_TERMINATOR_SET);
                self.make_error_expression(start, end)
            }
        }
    }

    fn parse_primary_with_suffix(&mut self) -> Expr {
        let mut e = self.parse_primary();
        loop {
            match self.kind() {
                T::Dot => e = self.parse_selector_suffix(e),
                T::LBracket => e = self.parse_slice_suffix(e),
                T::LParen => e = self.parse_call_suffix(e),
                _ => return e,
            }
        }
    }

    fn parse_slice_suffix(&mut self, e: Expr) -> Expr {
        self.expect(T::LBracket);
        let mut start = None;
        let mut stop = None;
        let mut step = None;
        if self.kind() != T::Colon {
            let s = self.parse_expr();
            if self.kind() == T::RBracket {
                self.expect(T::RBracket);
                return Expr::Index {
                    obj: Box::new(e),
                    key: Box::new(s),
                };
            }
            start = Some(Box::new(s));
        }
        self.expect(T::Colon);
        if self.kind() != T::Colon && self.kind() != T::RBracket {
            stop = Some(Box::new(self.parse_test()));
        }
        if self.kind() == T::Colon {
            self.expect(T::Colon);
            if self.kind() != T::RBracket {
                step = Some(Box::new(self.parse_test()));
            }
        }
        self.expect(T::RBracket);
        Expr::Slice {
            obj: Box::new(e),
            start,
            stop,
            step,
        }
    }

    fn parse_for_loop_variables(&mut self) -> Expr {
        let e1 = self.parse_primary_with_suffix();
        if self.kind() != T::Comma {
            return e1;
        }
        let mut elems = vec![e1];
        while self.kind() == T::Comma {
            self.expect(T::Comma);
            if EXPR_LIST_TERMINATOR_SET.contains(&self.kind()) {
                break;
            }
            elems.push(self.parse_primary_with_suffix());
        }
        Expr::List {
            is_tuple: true,
            start: None,
            elems,
        }
    }

    // Java: net/starlark/java/syntax/Parser.java:866 Parser.parseComprehensionSuffix
    fn parse_comprehension_suffix(
        &mut self,
        loffset: usize,
        body: CompBody,
        closing: TokenKind,
    ) -> Expr {
        let mut clauses = Vec::new();
        loop {
            if self.kind() == T::For {
                self.next_token();
                let vars = self.parse_for_loop_variables();
                self.expect(T::In);
                let iterable = self.parse_test_prec(0);
                clauses.push(Clause::For { vars, iterable });
            } else if self.kind() == T::If {
                self.next_token();
                let cond = self.parse_test_no_cond();
                clauses.push(Clause::If { cond });
            } else if self.kind() == closing {
                break;
            } else {
                self.syntax_error(&format!("expected '{closing}', 'for' or 'if'"));
                let end = self.sync_past(LIST_TERMINATOR_SET);
                return self.make_error_expression(loffset, end);
            }
        }
        self.expect(closing);
        Expr::Comprehension {
            is_dict: closing == T::RBrace,
            start: loffset,
            body: Box::new(body),
            clauses,
        }
    }

    fn parse_list_maker(&mut self) -> Expr {
        let lbracket = self.expect(T::LBracket);
        if self.kind() == T::RBracket {
            self.next_token();
            return Expr::List {
                is_tuple: false,
                start: Some(lbracket),
                elems: Vec::new(),
            };
        }
        let e = self.parse_test();
        match self.kind() {
            T::RBracket => {
                self.next_token();
                Expr::List {
                    is_tuple: false,
                    start: Some(lbracket),
                    elems: vec![e],
                }
            }
            T::For => self.parse_comprehension_suffix(lbracket, CompBody::Expr(e), T::RBracket),
            T::Comma => {
                let mut elems = vec![e];
                self.parse_expr_list(&mut elems, true);
                if self.kind() == T::RBracket {
                    self.next_token();
                    return Expr::List {
                        is_tuple: false,
                        start: Some(lbracket),
                        elems,
                    };
                }
                self.expect(T::RBracket);
                let end = self.sync_past(LIST_TERMINATOR_SET);
                self.make_error_expression(lbracket, end)
            }
            _ => {
                self.syntax_error("expected ',', 'for' or ']'");
                let end = self.sync_past(LIST_TERMINATOR_SET);
                self.make_error_expression(lbracket, end)
            }
        }
    }

    fn parse_dict_expression(&mut self) -> Expr {
        let lbrace = self.expect(T::LBrace);
        if self.kind() == T::RBrace {
            self.next_token();
            return Expr::Dict {
                start: lbrace,
                entries: Vec::new(),
            };
        }
        let (k, v) = self.parse_dict_entry();
        if self.kind() == T::For {
            return self.parse_comprehension_suffix(lbrace, CompBody::Entry(k, v), T::RBrace);
        }
        let mut entries = vec![(k, v)];
        if self.kind() == T::Comma {
            self.expect(T::Comma);
            entries.extend(self.parse_dict_entry_list());
        }
        if self.kind() == T::RBrace {
            self.next_token();
            return Expr::Dict {
                start: lbrace,
                entries,
            };
        }
        self.expect(T::RBrace);
        let end = self.sync_past(DICT_TERMINATOR_SET);
        self.make_error_expression(lbrace, end)
    }

    fn parse_ident(&mut self) -> Ident {
        if self.kind() != T::Identifier {
            let start = self.lexer.start;
            let end = self.expect(T::Identifier);
            let Expr::Ident(id) = self.make_error_expression(start, end) else {
                unreachable!()
            };
            return id;
        }
        let name = match std::mem::replace(&mut self.lexer.value, TokenValue::None) {
            TokenValue::Str(s) => s,
            _ => String::new(),
        };
        let start = self.next_token();
        Ident { name, start }
    }

    // Java: net/starlark/java/syntax/Parser.java:1004 Parser.parseBinOpExpression
    fn parse_bin_op_expression(&mut self, prec: usize) -> Expr {
        let mut x = self.parse_test_prec(prec + 1);
        let mut last_op: Option<TokenKind> = None;
        loop {
            if self.kind() == T::Not {
                self.expect(T::Not);
                if self.kind() != T::In {
                    self.syntax_error("expected 'in'");
                }
                self.lexer.kind = T::NotIn;
            }
            let op = self.kind();
            if !OPERATOR_PRECEDENCE[prec].contains(&op) {
                return x;
            }
            if let Some(last) = last_op {
                if OPERATOR_PRECEDENCE[prec].contains(&T::EqualsEquals) {
                    let s = self.lexer.start;
                    self.report_error(
                        s,
                        format!(
                            "Operator '{last}' is not associative with operator '{op}'. Use parens."
                        ),
                    );
                }
            }
            let op_offset = self.next_token();
            let y = self.parse_test_prec(prec + 1);
            x = match (op, x, y) {
                // Java optimizeBinOpExpression: string literal + string literal.
                (T::Plus, Expr::Str { start, value: a }, Expr::Str { value: b, .. }) => Expr::Str {
                    start,
                    value: a + &b,
                },
                (op, x, y) => Expr::Binary {
                    x: Box::new(x),
                    op,
                    op_offset,
                    y: Box::new(y),
                },
            };
            last_op = Some(op);
        }
    }

    fn maybe_parse_type_annotation_after(&mut self, expected: TokenKind) -> Option<Expr> {
        if self.kind() == expected {
            let (s, k, v) = (self.lexer.start, self.kind(), self.lexer.value.clone());
            if self.check_allow_type_syntax(s, k, &v) {
                self.next_token();
                return Some(self.parse_type_expr_with_fallback());
            }
        }
        None
    }

    fn parse_type_expr_with_fallback(&mut self) -> Expr {
        self.inside_type_expr = true;
        let r = if self.options.tolerate_invalid_type_expressions {
            self.parse_test()
        } else {
            self.parse_type_expr()
        };
        self.inside_type_expr = false;
        r
    }

    fn parse_type_expr(&mut self) -> Expr {
        if self.kind() != T::Identifier {
            let start = self.lexer.start;
            self.syntax_error("expected a type");
            let end = self.sync_to(EXPR_TERMINATOR_SET);
            return self.make_error_expression(start, end);
        }
        let first = self.parse_ident();
        let mut expr = if self.kind() == T::LBracket {
            self.parse_type_application(first)
        } else {
            Expr::Ident(first)
        };
        while self.kind() == T::Pipe {
            let op_offset = self.next_token();
            let second = self.parse_ident();
            let y = if self.kind() == T::LBracket {
                self.parse_type_application(second)
            } else {
                Expr::Ident(second)
            };
            expr = Expr::Binary {
                x: Box::new(expr),
                op: T::Pipe,
                op_offset,
                y: Box::new(y),
            };
        }
        expr
    }

    fn parse_type_argument(&mut self) -> Expr {
        match self.kind() {
            T::LBracket => return self.parse_type_list(),
            T::LBrace => return self.parse_type_dict(),
            T::LParen => {
                let l = self.expect(T::LParen);
                self.expect(T::RParen);
                return Expr::List {
                    is_tuple: true,
                    start: Some(l),
                    elems: Vec::new(),
                };
            }
            T::String => return self.parse_string_literal(),
            T::Ellipsis => return self.parse_primary(),
            _ => {}
        }
        if self.kind() != T::Identifier {
            let start = self.lexer.start;
            self.syntax_error("expected a type argument");
            let end = self.sync_to(EXPR_TERMINATOR_SET);
            return self.make_error_expression(start, end);
        }
        self.parse_type_expr()
    }

    fn parse_type_list(&mut self) -> Expr {
        let l = self.expect(T::LBracket);
        let mut elems = Vec::new();
        if self.kind() != T::RBracket {
            elems.push(self.parse_type_argument());
        }
        while self.kind() != T::RBracket && self.kind() != T::Eof {
            self.expect(T::Comma);
            if self.kind() == T::RBracket {
                break;
            }
            elems.push(self.parse_type_argument());
        }
        self.next_token();
        Expr::List {
            is_tuple: false,
            start: Some(l),
            elems,
        }
    }

    fn parse_type_dict_entry(&mut self) -> (Expr, Expr) {
        let key = if self.kind() == T::String {
            self.parse_string_literal()
        } else {
            let start = self.lexer.start;
            self.syntax_error(&format!("expected {}", T::String));
            let end = self.sync_to(EXPR_TERMINATOR_SET);
            self.make_error_expression(start, end)
        };
        self.expect(T::Colon);
        let value = self.parse_type_argument();
        (key, value)
    }

    fn parse_type_dict(&mut self) -> Expr {
        let l = self.expect(T::LBrace);
        let mut entries = Vec::new();
        if self.kind() != T::RBrace {
            entries.push(self.parse_type_dict_entry());
        }
        while self.kind() != T::RBrace && self.kind() != T::Eof {
            self.expect(T::Comma);
            if self.kind() == T::RBrace {
                break;
            }
            entries.push(self.parse_type_dict_entry());
        }
        self.next_token();
        Expr::Dict { start: l, entries }
    }

    fn parse_type_application(&mut self, constructor: Ident) -> Expr {
        self.expect(T::LBracket);
        let mut args = vec![self.parse_type_argument()];
        while self.kind() != T::RBracket && self.kind() != T::Eof {
            self.expect(T::Comma);
            args.push(self.parse_type_argument());
        }
        self.expect(T::RBracket);
        Expr::TypeApplication { constructor, args }
    }

    fn parse_type_alias_statement_tail(&mut self, type_kw_start: usize) -> Stmt {
        self.check_allow_type_syntax(
            type_kw_start,
            T::Identifier,
            &TokenValue::Str("type".to_owned()),
        );
        let name = self.parse_ident();
        let params = self.parse_optional_type_parameters();
        self.expect(T::Equals);
        let def = self.parse_type_expr_with_fallback();
        Stmt::TypeAlias {
            start: type_kw_start,
            name,
            params,
            def,
        }
    }

    fn parse_optional_type_parameters(&mut self) -> Vec<Ident> {
        if self.kind() != T::LBracket {
            return Vec::new();
        }
        let (s, k, v) = (self.lexer.start, self.kind(), self.lexer.value.clone());
        self.check_allow_type_syntax(s, k, &v);
        self.next_token();
        let mut seen = Vec::<String>::new();
        let mut params = vec![self.parse_type_parameter(&mut seen)];
        while self.kind() != T::RBracket && self.kind() != T::Eof {
            self.expect(T::Comma);
            if self.kind() == T::RBracket {
                break;
            }
            params.push(self.parse_type_parameter(&mut seen));
        }
        self.expect(T::RBracket);
        params
    }

    fn parse_type_parameter(&mut self, seen: &mut Vec<String>) -> Ident {
        let (s, k, v) = (self.lexer.start, self.kind(), self.lexer.value.clone());
        let id = self.parse_ident();
        if is_valid_identifier(&id.name) {
            if seen.contains(&id.name) {
                self.syntax_error_at(s, k, &v, "duplicate type parameter");
            } else {
                seen.push(id.name.clone());
            }
        }
        id
    }

    // Java: net/starlark/java/syntax/Parser.java:1295 Parser.parseTest
    fn parse_test(&mut self) -> Expr {
        let start = self.lexer.start;
        if self.kind() == T::Lambda {
            return self.parse_lambda(true);
        }
        let expr = self.parse_test_prec(0);
        if self.kind() == T::If {
            self.next_token();
            let cond = self.parse_test_prec(0);
            if self.kind() == T::Else {
                self.next_token();
                let els = self.parse_test();
                return Expr::Conditional {
                    then: Box::new(expr),
                    cond: Box::new(cond),
                    els: Box::new(els),
                };
            }
            self.report_error(
                start,
                "missing else clause in conditional expression or semicolon before if".to_owned(),
            );
            return expr;
        }
        expr
    }

    fn parse_test_prec(&mut self, prec: usize) -> Expr {
        if prec >= OPERATOR_PRECEDENCE.len() {
            return self.parse_primary_with_suffix();
        }
        if self.kind() == T::Not && OPERATOR_PRECEDENCE[prec].contains(&T::Not) {
            let offset = self.expect(T::Not);
            let x = self.parse_test_prec(prec);
            return Expr::Unary {
                op: T::Not,
                offset,
                x: Box::new(x),
            };
        }
        self.parse_bin_op_expression(prec)
    }

    fn parse_lambda(&mut self, allow_cond: bool) -> Expr {
        let start = self.expect(T::Lambda);
        let params = self.parse_parameters(false);
        self.expect(T::Colon);
        let body = if allow_cond {
            self.parse_test()
        } else {
            self.parse_test_no_cond()
        };
        Expr::Lambda {
            start,
            params,
            body: Box::new(body),
        }
    }

    fn parse_test_no_cond(&mut self) -> Expr {
        if self.kind() == T::Lambda {
            return self.parse_lambda(false);
        }
        self.parse_test_prec(0)
    }

    fn parse_expr(&mut self) -> Expr {
        let e = self.parse_test();
        if self.kind() != T::Comma {
            return e;
        }
        let mut elems = vec![e];
        self.parse_expr_list(&mut elems, false);
        Expr::List {
            is_tuple: true,
            start: None,
            elems,
        }
    }

    // Java: net/starlark/java/syntax/Parser.java:1350 Parser.parseFileInput
    pub fn parse_file_input(&mut self) -> Vec<Stmt> {
        let mut list = Vec::new();
        while self.kind() != T::Eof {
            if self.kind() == T::Newline {
                self.expect_and_recover(T::Newline);
            } else if self.recovery_mode {
                self.sync_to(STATEMENT_TERMINATOR_SET);
                self.recovery_mode = false;
            } else {
                self.maybe_parse_doc_comment_block();
                if self.kind() == T::Eof {
                    break;
                }
                self.parse_statement(&mut list);
            }
        }
        list
    }

    fn parse_load_statement(&mut self) -> Stmt {
        let start = self.expect(T::Load);
        self.expect(T::LParen);
        if self.kind() != T::String {
            self.expect(T::String);
            return Stmt::Load {
                start,
                bindings: Vec::new(),
            };
        }
        self.parse_string_literal();
        if self.kind() == T::RParen {
            self.syntax_error("expected at least one symbol to load");
            return Stmt::Load {
                start,
                bindings: Vec::new(),
            };
        }
        self.expect(T::Comma);
        let mut bindings = Vec::new();
        self.parse_load_symbol(&mut bindings);
        while self.kind() != T::RParen && self.kind() != T::Eof {
            self.expect(T::Comma);
            if self.kind() == T::RParen {
                break;
            }
            self.parse_load_symbol(&mut bindings);
        }
        self.expect(T::RParen);
        Stmt::Load { start, bindings }
    }

    fn parse_load_symbol(&mut self, symbols: &mut Vec<(Ident, Ident)>) {
        if self.kind() != T::String && self.kind() != T::Identifier {
            self.syntax_error("expected either a literal string or an identifier");
            return;
        }
        let name = match &self.lexer.value {
            TokenValue::Str(s) => s.clone(),
            _ => String::new(),
        };
        let is_string = self.kind() == T::String;
        let name_offset = self.lexer.start + usize::from(is_string);
        let local = Ident {
            name,
            start: name_offset,
        };
        let original = if is_string {
            local.clone()
        } else {
            self.expect(T::Identifier);
            self.expect(T::Equals);
            if self.kind() != T::String {
                self.syntax_error("expected string");
                return;
            }
            let n = match &self.lexer.value {
                TokenValue::Str(s) => s.clone(),
                _ => String::new(),
            };
            Ident {
                name: n,
                start: self.lexer.start + 1,
            }
        };
        self.next_token();
        symbols.push((local, original));
    }

    fn parse_simple_statement(&mut self, list: &mut Vec<Stmt>) {
        let s = self.parse_small_statement();
        list.push(s);
        while self.kind() == T::Semi {
            self.next_token();
            if self.kind() == T::Newline || self.kind() == T::DocCommentTrailing {
                break;
            }
            let s = self.parse_small_statement();
            list.push(s);
        }
        if self.kind() == T::DocCommentTrailing {
            self.next_token();
        }
        self.expect_and_recover(T::Newline);
    }

    fn maybe_parse_trailing_doc_comment(&mut self) {
        if self.kind() == T::DocCommentTrailing {
            self.next_token();
        }
    }

    // Java: net/starlark/java/syntax/Parser.java:1482 Parser.parseSmallStatement
    fn parse_small_statement(&mut self) -> Stmt {
        match self.kind() {
            T::Return => return self.parse_return_statement(),
            T::Break | T::Continue | T::Pass => {
                let kind = self.kind();
                let start = self.next_token();
                return Stmt::Flow { kind, start };
            }
            T::Load => return self.parse_load_statement(),
            _ => {}
        }
        let lhs = self.parse_expr();
        if self.kind() == T::Identifier && matches!(&lhs, Expr::Ident(id) if id.name == "type") {
            return self.parse_type_alias_statement_tail(lhs.start());
        }
        let colon_offset = self.lexer.start;
        let mut ty = self.maybe_parse_type_annotation_after(T::Colon);
        let op = augmented(self.kind());
        if self.kind() == T::Equals || op.is_some() {
            let op_offset = self.next_token();
            let rhs = self.parse_expr();
            self.maybe_parse_trailing_doc_comment();
            if ty.is_some() {
                if !matches!(lhs, Expr::Ident(_)) {
                    self.syntax_error_at(
                        colon_offset,
                        T::Colon,
                        &TokenValue::None,
                        "type annotations must have a single identifier on the left-hand side",
                    );
                    ty = None;
                }
                if op.is_some() {
                    self.syntax_error_at(
                        colon_offset,
                        T::Colon,
                        &TokenValue::None,
                        "type annotations not allowed on augmented assignment statements",
                    );
                    ty = None;
                }
            }
            Stmt::Assign {
                lhs,
                ty,
                op,
                op_offset,
                rhs,
            }
        } else if let Some(t) = ty {
            self.maybe_parse_trailing_doc_comment();
            match lhs {
                Expr::Ident(name) => Stmt::Var { name, ty: t },
                lhs => {
                    self.syntax_error_at(
                        colon_offset,
                        T::Colon,
                        &TokenValue::None,
                        "type annotations must have a single identifier on the left-hand side",
                    );
                    // Java: makeErrorExpression(lhs start, type end); the end offset only
                    // shapes the error identifier's text, which no error message prints.
                    let start = lhs.start();
                    Stmt::Expr(Expr::Ident(Ident {
                        name: String::from("<error>"),
                        start,
                    }))
                }
            }
        } else {
            Stmt::Expr(lhs)
        }
    }

    fn parse_if_statement(&mut self) -> Stmt {
        let start = self.expect(T::If);
        let cond = self.parse_test();
        self.expect(T::Colon);
        let then = self.parse_suite();
        let mut elifs: Vec<(usize, Expr, Vec<Stmt>)> = Vec::new();
        while self.kind() == T::Elif {
            let o = self.expect(T::Elif);
            let c = self.parse_test();
            self.expect(T::Colon);
            let b = self.parse_suite();
            elifs.push((o, c, b));
        }
        let mut else_block = None;
        if self.kind() == T::Else {
            self.expect(T::Else);
            self.expect(T::Colon);
            else_block = Some(self.parse_suite());
        }
        // Rebuild the elif chain from the tail.
        for (o, c, b) in elifs.into_iter().rev() {
            else_block = Some(vec![Stmt::If {
                start: o,
                cond: c,
                then: b,
                els: else_block,
            }]);
        }
        Stmt::If {
            start,
            cond,
            then,
            els: else_block,
        }
    }

    fn parse_for_statement(&mut self) -> Stmt {
        let start = self.expect(T::For);
        let vars = self.parse_for_loop_variables();
        self.expect(T::In);
        let collection = self.parse_expr();
        self.expect(T::Colon);
        let body = self.parse_suite();
        Stmt::For {
            start,
            vars,
            collection,
            body,
        }
    }

    fn parse_def_statement(&mut self) -> Stmt {
        let start = self.expect(T::Def);
        let name = self.parse_ident();
        let type_params = self.parse_optional_type_parameters();
        self.expect(T::LParen);
        let params = self.parse_parameters(true);
        self.expect(T::RParen);
        let ret = self.maybe_parse_type_annotation_after(T::RArrow);
        self.expect(T::Colon);
        let body = self.parse_suite();
        Stmt::Def {
            start,
            name,
            type_params,
            params,
            ret,
            body,
        }
    }

    fn parse_parameters(&mut self, def_statement: bool) -> Vec<Parameter> {
        let mut has = false;
        let mut list = Vec::new();
        while self.kind() != T::RParen && self.kind() != T::Colon && self.kind() != T::Eof {
            if has {
                self.expect(T::Comma);
                if self.kind() == T::RParen {
                    break;
                }
            }
            let p = self.parse_parameter(def_statement);
            has = true;
            list.push(p);
        }
        list
    }

    // Java: net/starlark/java/syntax/Parser.java:1675 Parser.parseSuite
    fn parse_suite(&mut self) -> Vec<Stmt> {
        let mut list = Vec::new();
        if self.kind() == T::DocCommentTrailing {
            self.next_token();
        }
        if self.kind() == T::Newline {
            self.expect(T::Newline);
            self.maybe_parse_doc_comment_block();
            if self.kind() != T::Indent {
                let s = self.lexer.start;
                self.report_error(s, "expected an indented block".to_owned());
                return list;
            }
            self.expect(T::Indent);
            while self.kind() != T::Outdent && self.kind() != T::Eof {
                self.parse_statement(&mut list);
                self.maybe_parse_doc_comment_block();
            }
            self.expect_and_recover(T::Outdent);
        } else {
            self.parse_simple_statement(&mut list);
        }
        list
    }

    fn parse_return_statement(&mut self) -> Stmt {
        let start = self.expect(T::Return);
        let mut result = None;
        if !STATEMENT_TERMINATOR_SET.contains(&self.kind()) {
            result = Some(self.parse_expr());
        }
        Stmt::Return { start, result }
    }

    fn parse_statement(&mut self, list: &mut Vec<Stmt>) {
        match self.kind() {
            T::Def => {
                let s = self.parse_def_statement();
                list.push(s);
            }
            T::If => {
                let s = self.parse_if_statement();
                list.push(s);
            }
            T::For => {
                let s = self.parse_for_statement();
                list.push(s);
            }
            _ => self.parse_simple_statement(list),
        }
    }

    fn maybe_parse_doc_comment_block(&mut self) {
        while self.kind() == T::DocCommentBlock {
            self.next_token();
        }
    }
}
