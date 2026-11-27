//! Port of Bazel's Starlark scanner, used to report a file's syntax errors exactly as Java does
//! (every error, in Java's order, with Java's recovery).
//!
//! Java: net/starlark/java/syntax/Lexer.java, IntLiteral.java (scan), TokenKind.java.

use super::JavaOptions;

/// Java `TokenKind`; `Display` is its `toString` (TokenKind.java).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Ampersand,
    AmpersandEquals,
    And,
    As,
    Assert,
    Break,
    Caret,
    CaretEquals,
    Cast,
    Class,
    Colon,
    Comma,
    Continue,
    Def,
    Del,
    DocCommentBlock,
    DocCommentTrailing,
    Dot,
    Elif,
    Ellipsis,
    Else,
    Eof,
    Equals,
    EqualsEquals,
    Except,
    Finally,
    Float,
    For,
    From,
    Global,
    Greater,
    GreaterEquals,
    GreaterGreater,
    GreaterGreaterEquals,
    Identifier,
    If,
    Illegal,
    Import,
    In,
    Indent,
    Int,
    Is,
    IsInstance,
    Lambda,
    LBrace,
    LBracket,
    Less,
    LessEquals,
    LessLess,
    LessLessEquals,
    Load,
    LParen,
    Minus,
    MinusEquals,
    Newline,
    Nonlocal,
    Not,
    NotEquals,
    NotIn,
    Or,
    Outdent,
    Pass,
    Percent,
    PercentEquals,
    Pipe,
    PipeEquals,
    Plus,
    PlusEquals,
    Raise,
    RArrow,
    RBrace,
    RBracket,
    Return,
    RParen,
    Semi,
    Slash,
    SlashEquals,
    SlashSlash,
    SlashSlashEquals,
    Star,
    StarEquals,
    StarStar,
    String,
    Tilde,
    Try,
    While,
    With,
    Yield,
}

impl TokenKind {
    // Java: net/starlark/java/syntax/TokenKind.java (the constructor argument of each constant)
    pub fn text(self) -> &'static str {
        use TokenKind::*;
        match self {
            Ampersand => "&",
            AmpersandEquals => "&=",
            And => "and",
            As => "as",
            Assert => "assert",
            Break => "break",
            Caret => "^",
            CaretEquals => "^=",
            Cast => "cast",
            Class => "class",
            Colon => ":",
            Comma => ",",
            Continue => "continue",
            Def => "def",
            Del => "del",
            DocCommentBlock => "#:",
            DocCommentTrailing => "trailing #: ",
            Dot => ".",
            Elif => "elif",
            Ellipsis => "...",
            Else => "else",
            Eof => "EOF",
            Equals => "=",
            EqualsEquals => "==",
            Except => "except",
            Finally => "finally",
            Float => "float literal",
            For => "for",
            From => "from",
            Global => "global",
            Greater => ">",
            GreaterEquals => ">=",
            GreaterGreater => ">>",
            GreaterGreaterEquals => ">>=",
            Identifier => "identifier",
            If => "if",
            Illegal => "illegal character",
            Import => "import",
            In => "in",
            Indent => "indent",
            Int => "integer literal",
            Is => "is",
            IsInstance => "isinstance",
            Lambda => "lambda",
            LBrace => "{",
            LBracket => "[",
            Less => "<",
            LessEquals => "<=",
            LessLess => "<<",
            LessLessEquals => "<<=",
            Load => "load",
            LParen => "(",
            Minus => "-",
            MinusEquals => "-=",
            Newline => "newline",
            Nonlocal => "nonlocal",
            Not => "not",
            NotEquals => "!=",
            NotIn => "not in",
            Or => "or",
            Outdent => "outdent",
            Pass => "pass",
            Percent => "%",
            PercentEquals => "%=",
            Pipe => "|",
            PipeEquals => "|=",
            Plus => "+",
            PlusEquals => "+=",
            Raise => "raise",
            RArrow => "->",
            RBrace => "}",
            RBracket => "]",
            Return => "return",
            RParen => ")",
            Semi => ";",
            Slash => "/",
            SlashEquals => "/=",
            SlashSlash => "//",
            SlashSlashEquals => "//=",
            Star => "*",
            StarEquals => "*=",
            StarStar => "**",
            String => "string literal",
            Tilde => "~",
            Try => "try",
            While => "while",
            With => "with",
            Yield => "yield",
        }
    }
}

impl std::fmt::Display for TokenKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.text())
    }
}

/// The value of a token (Java `Lexer.value`).
#[derive(Debug, Clone, PartialEq)]
pub enum TokenValue {
    None,
    /// STRING, IDENTIFIER, ILLEGAL.
    Str(String),
    /// INT: its decimal digits (Java Integer/Long/BigInteger `toString`).
    Int(String),
    Float(f64),
    /// DOC_COMMENT_BLOCK / DOC_COMMENT_TRAILING: the line of the last comment (1-based) and
    /// Java's `DocComments.toString()` (the comments' texts, each from its `#` to the end of
    /// its line, joined by newlines), which `Parser.tokenString` prints.
    DocComments {
        end_line: usize,
        text: String,
        /// The comment lines (Java `DocComments.getLines()`).
        lines: Vec<super::ast::DocCommentLine>,
    },
}

/// A syntax error at a char offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetError {
    pub offset: usize,
    pub message: String,
}

// Java: net/starlark/java/syntax/Lexer.java:29 Lexer
pub struct Lexer<'a> {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
    pub value: TokenValue,
    buffer: &'a [char],
    pos: usize,
    options: JavaOptions,
    indent_stack: Vec<usize>,
    open_paren_stack_depth: usize,
    check_indentation: bool,
    dents: i32,
    line_only_whitespace_or_comments: bool,
    started: bool,
    line_starts: &'a [usize],
    /// Every comment with a `#:` prefix, in file order (the doc comment lines of Java's
    /// `StarlarkFile.getComments()`).
    pub doc_comment_lines: Vec<super::ast::DocCommentLine>,
}

fn equal_token(c: char) -> Option<TokenKind> {
    Some(match c {
        '=' => TokenKind::EqualsEquals,
        '!' => TokenKind::NotEquals,
        '>' => TokenKind::GreaterEquals,
        '<' => TokenKind::LessEquals,
        '+' => TokenKind::PlusEquals,
        '-' => TokenKind::MinusEquals,
        '*' => TokenKind::StarEquals,
        '/' => TokenKind::SlashEquals,
        '%' => TokenKind::PercentEquals,
        '^' => TokenKind::CaretEquals,
        '&' => TokenKind::AmpersandEquals,
        '|' => TokenKind::PipeEquals,
        _ => return None,
    })
}

fn keyword(id: &str, type_syntax: bool) -> Option<TokenKind> {
    use TokenKind::*;
    Some(match id {
        "and" => And,
        "as" => As,
        "assert" => Assert,
        "break" => Break,
        "class" => Class,
        "continue" => Continue,
        "def" => Def,
        "del" => Del,
        "elif" => Elif,
        "else" => Else,
        "except" => Except,
        "finally" => Finally,
        "for" => For,
        "from" => From,
        "global" => Global,
        "if" => If,
        "import" => Import,
        "in" => In,
        "is" => Is,
        "lambda" => Lambda,
        "load" => Load,
        "nonlocal" => Nonlocal,
        "not" => Not,
        "or" => Or,
        "pass" => Pass,
        "raise" => Raise,
        "return" => Return,
        "try" => Try,
        "while" => While,
        "with" => With,
        "yield" => Yield,
        "cast" if type_syntax => Cast,
        "isinstance" if type_syntax => IsInstance,
        _ => return None,
    })
}

fn isdigit(c: Option<char>) -> bool {
    matches!(c, Some('0'..='9'))
}

fn isxdigit(c: Option<char>) -> bool {
    matches!(c, Some('0'..='9' | 'a'..='f' | 'A'..='F'))
}

fn isbdigit(c: Option<char>) -> bool {
    matches!(c, Some('0' | '1'))
}

/// Java `IntLiteral.scan`: the decimal value of an int literal, or Java's message.
// Java: net/starlark/java/syntax/IntLiteral.java:66 IntLiteral.scan
pub fn int_literal_scan(orig: &str) -> Result<String, String> {
    let mut s = orig;
    let mut radix = 10;
    let b: Vec<char> = s.chars().collect();
    if b.len() > 1 && b[0] == '0' {
        match b[1] {
            'x' | 'X' => {
                radix = 16;
                s = &s[2..];
            }
            'o' | 'O' => {
                radix = 8;
                s = &s[2..];
            }
            _ => {
                return Err(format!("invalid octal literal: {s} (use '0o{}')", &s[1..]));
            }
        }
    }
    // Long.parseLong / BigInteger(str, radix): an optional sign, then at least one digit.
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if !digits.is_empty() && digits.chars().all(|c| c.is_digit(radix)) {
        if let Some(v) = num_bigint::BigInt::parse_bytes(s.as_bytes(), radix) {
            return Ok(v.to_string());
        }
    }
    Err(format!("invalid base-{radix} integer literal: {orig}"))
}

impl<'a> Lexer<'a> {
    pub fn new(buffer: &'a [char], line_starts: &'a [usize], options: JavaOptions) -> Self {
        Lexer {
            kind: TokenKind::Newline,
            start: 0,
            end: 0,
            value: TokenValue::None,
            buffer,
            pos: 0,
            options,
            indent_stack: vec![0],
            open_paren_stack_depth: 0,
            check_indentation: true,
            dents: 0,
            line_only_whitespace_or_comments: true,
            started: false,
            line_starts,
            doc_comment_lines: Vec::new(),
        }
    }

    pub(crate) fn line_of(&self, offset: usize) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }

    /// Reads the next token (Java `nextToken`).
    // Java: net/starlark/java/syntax/Lexer.java:124 Lexer.nextToken
    pub fn next_token(&mut self, errors: &mut Vec<OffsetError>) {
        let after_newline = self.started
            && (self.kind == TokenKind::Newline || self.kind == TokenKind::DocCommentBlock);
        self.started = true;
        self.tokenize(errors);
        if self.kind == TokenKind::Eof && !after_newline {
            self.kind = TokenKind::Newline;
        }
        if !matches!(
            self.kind,
            TokenKind::Newline
                | TokenKind::Indent
                | TokenKind::Outdent
                | TokenKind::DocCommentBlock
                | TokenKind::DocCommentTrailing
        ) {
            self.line_only_whitespace_or_comments = false;
        }
    }

    fn error(&self, errors: &mut Vec<OffsetError>, message: impl Into<String>, pos: usize) {
        errors.push(OffsetError {
            offset: pos,
            message: message.into(),
        });
    }

    fn pop_paren(&mut self, errors: &mut Vec<OffsetError>) {
        if self.open_paren_stack_depth == 0 {
            self.error(errors, "indentation error", self.pos - 1);
        } else {
            self.open_paren_stack_depth -= 1;
        }
    }

    fn set_token(&mut self, kind: TokenKind, start: usize, end: usize) {
        self.kind = kind;
        self.start = start;
        self.end = end;
        self.value = TokenValue::None;
    }

    pub fn buffer_slice(&self, start: usize, end: usize) -> String {
        self.buffer[start.min(self.buffer.len())..end.min(self.buffer.len())]
            .iter()
            .collect()
    }

    fn peek(&self, i: usize) -> Option<char> {
        self.buffer.get(self.pos + i).copied()
    }

    fn next(&mut self) -> Option<char> {
        self.pos += 1;
        self.peek(0)
    }

    fn newline(&mut self) {
        self.line_only_whitespace_or_comments = true;
        if self.open_paren_stack_depth > 0 {
            while self.pos < self.buffer.len() {
                match self.buffer[self.pos] {
                    ' ' | '\t' | '\r' => self.pos += 1,
                    _ => return,
                }
            }
        } else {
            self.check_indentation = true;
            self.set_token(TokenKind::Newline, self.pos - 1, self.pos);
        }
    }

    // Java: net/starlark/java/syntax/Lexer.java:208 Lexer.computeIndentation
    fn compute_indentation(&mut self, errors: &mut Vec<OffsetError>) {
        let mut indent_len = 0;
        while self.pos < self.buffer.len() {
            let c = self.buffer[self.pos];
            if c == ' ' {
                indent_len += 1;
                self.pos += 1;
            } else if c == '\r' {
                self.pos += 1;
            } else if c == '\t' {
                indent_len += 1;
                self.pos += 1;
                self.error(
                    errors,
                    "Tab characters are not allowed for indentation. Use spaces instead.",
                    self.pos,
                );
            } else if c == '\n' {
                indent_len = 0;
                self.pos += 1;
            } else if c == '#' {
                if self.peek(1) == Some(':') && self.open_paren_stack_depth == 0 {
                    return;
                }
                let comment_start = self.pos;
                self.scan_to_newline();
                self.record_doc_comment_line(comment_start);
                indent_len = 0;
            } else {
                break;
            }
        }
        if self.pos == self.buffer.len() {
            indent_len = 0;
        }
        let mut peeked = *self.indent_stack.last().unwrap();
        if peeked < indent_len {
            self.indent_stack.push(indent_len);
            self.dents += 1;
        } else if peeked > indent_len {
            while peeked > indent_len {
                self.indent_stack.pop();
                self.dents -= 1;
                peeked = *self.indent_stack.last().unwrap();
            }
            if peeked < indent_len {
                self.error(errors, "indentation error", self.pos - 1);
            }
        }
    }

    fn skip_triple_quote(&mut self, quot: char) -> bool {
        if self.peek(0) == Some(quot) && self.peek(1) == Some(quot) {
            self.pos += 2;
            true
        } else {
            false
        }
    }

    // Java: net/starlark/java/syntax/Lexer.java:278 Lexer.escapedStringLiteral
    fn escaped_string_literal(&mut self, quot: char, is_raw: bool, errors: &mut Vec<OffsetError>) {
        let literal_start = if is_raw { self.pos - 2 } else { self.pos - 1 };
        let in_triple = self.skip_triple_quote(quot);
        let mut literal = String::new();
        let n = self.buffer.len();
        while self.pos < n {
            let c = self.buffer[self.pos];
            self.pos += 1;
            match c {
                '\n' => {
                    if in_triple {
                        literal.push(c);
                    } else {
                        self.error(errors, "unclosed string literal", literal_start);
                        self.set_token(TokenKind::String, literal_start, self.pos);
                        self.value = TokenValue::Str(literal);
                        return;
                    }
                }
                '\\' => {
                    if self.pos == n {
                        self.error(errors, "unclosed string literal", literal_start);
                        self.set_token(TokenKind::String, literal_start, self.pos);
                        self.value = TokenValue::Str(literal);
                        return;
                    }
                    if is_raw {
                        literal.push('\\');
                        if self.peek(0) == Some('\r') && self.peek(1) == Some('\n') {
                            literal.push('\n');
                            self.pos += 2;
                        } else if self.buffer[self.pos] == '\r' || self.buffer[self.pos] == '\n' {
                            literal.push('\n');
                            self.pos += 1;
                        } else {
                            literal.push(self.buffer[self.pos]);
                            self.pos += 1;
                        }
                        continue;
                    }
                    let c = self.buffer[self.pos];
                    self.pos += 1;
                    match c {
                        '\r' => {
                            if self.peek(0) == Some('\n') {
                                self.pos += 1;
                            }
                        }
                        '\n' => {}
                        'a' => literal.push('\u{7}'),
                        'b' => literal.push('\u{8}'),
                        'f' => literal.push('\u{c}'),
                        'n' => literal.push('\n'),
                        'r' => literal.push('\r'),
                        't' => literal.push('\t'),
                        'v' => literal.push('\u{b}'),
                        '\\' => literal.push('\\'),
                        '\'' => literal.push('\''),
                        '"' => literal.push('"'),
                        '0'..='7' => {
                            let mut octal = c as u32 - '0' as u32;
                            if self.pos < n {
                                let c = self.buffer[self.pos];
                                if ('0'..='7').contains(&c) {
                                    self.pos += 1;
                                    octal = (octal << 3) | (c as u32 - '0' as u32);
                                    if self.pos < n {
                                        let c = self.buffer[self.pos];
                                        if ('0'..='7').contains(&c) {
                                            self.pos += 1;
                                            octal = (octal << 3) | (c as u32 - '0' as u32);
                                        }
                                    }
                                }
                            }
                            if octal > 0xff {
                                self.error(
                                    errors,
                                    "octal escape sequence out of range (maximum is \\377)",
                                    self.pos - 1,
                                );
                            } else if self.options.string_literals_are_ascii_only && octal >= 0x80 {
                                self.error(
                                    errors,
                                    "octal escape sequence denotes non-ASCII character",
                                    self.pos - 1,
                                );
                            }
                            literal.push(char::from_u32(octal & 0xff).unwrap());
                        }
                        c => {
                            self.error(
                                errors,
                                format!(
                                    "invalid escape sequence: \\{c}. Use '\\\\' to insert '\\'."
                                ),
                                self.pos - 1,
                            );
                            literal.push('\\');
                            literal.push(c);
                        }
                    }
                }
                '\'' | '"' => {
                    if c != quot || (in_triple && !self.skip_triple_quote(quot)) {
                        literal.push(c);
                    } else {
                        self.set_token(TokenKind::String, literal_start, self.pos);
                        self.value = TokenValue::Str(literal);
                        return;
                    }
                }
                c => {
                    literal.push(c);
                    if self.options.string_literals_are_ascii_only && c as u32 >= 0x80 {
                        self.error(
                            errors,
                            "string literal contains non-ASCII character",
                            self.pos - 1,
                        );
                    }
                }
            }
        }
        self.error(errors, "unclosed string literal", literal_start);
        self.set_token(TokenKind::String, literal_start, self.pos);
        self.value = TokenValue::Str(literal);
    }

    // Java: net/starlark/java/syntax/Lexer.java:441 Lexer.stringLiteral
    fn string_literal(&mut self, quot: char, is_raw: bool, errors: &mut Vec<OffsetError>) {
        let literal_start = if is_raw { self.pos - 2 } else { self.pos - 1 };
        let content_start = self.pos;
        if self.skip_triple_quote(quot) {
            self.pos -= 2;
            self.escaped_string_literal(quot, is_raw, errors);
            return;
        }
        let n = self.buffer.len();
        while self.pos < n {
            let c = self.buffer[self.pos];
            self.pos += 1;
            match c {
                '\n' => {
                    self.error(errors, "unclosed string literal", literal_start);
                    self.set_token(TokenKind::String, literal_start, self.pos);
                    self.value = TokenValue::Str(self.buffer_slice(content_start, self.pos - 1));
                    return;
                }
                '\\' => {
                    if is_raw {
                        if self.peek(0) == Some('\r') && self.peek(1) == Some('\n') {
                            self.pos = content_start;
                            self.escaped_string_literal(quot, true, errors);
                            return;
                        } else {
                            self.pos += 1;
                            continue;
                        }
                    }
                    self.pos = content_start;
                    self.escaped_string_literal(quot, false, errors);
                    return;
                }
                '\'' | '"' if c == quot => {
                    self.set_token(TokenKind::String, literal_start, self.pos);
                    self.value = TokenValue::Str(self.buffer_slice(content_start, self.pos - 1));
                    if self.options.string_literals_are_ascii_only {
                        for i in content_start..self.pos - 1 {
                            if self.buffer[i] as u32 >= 0x80 {
                                self.error(
                                    errors,
                                    "string literal contains non-ASCII character",
                                    i,
                                );
                            }
                        }
                    }
                    return;
                }
                _ => {}
            }
        }
        if self.pos > n {
            self.pos = n;
        }
        self.error(errors, "unclosed string literal", literal_start);
        self.set_token(TokenKind::String, literal_start, self.pos);
        self.value = TokenValue::Str(self.buffer_slice(content_start, self.pos));
    }

    // Java: net/starlark/java/syntax/Lexer.java:516 Lexer.docComments
    fn doc_comments(&mut self, first_start: usize, is_block: bool, errors: &mut Vec<OffsetError>) {
        let mut last_end = self.pos;
        // Java: DocComments.java:53 DocComments.toString (Comment.toString is its text).
        let mut texts = vec![self.buffer_slice(first_start, self.pos)];
        let mut lines = vec![super::ast::DocCommentLine {
            line: self.line_of(first_start),
            text: self.buffer_slice(first_start, self.pos),
        }];
        if is_block {
            let mut prev_line = self.line_of(first_start);
            while self.peek(0) == Some('\n') {
                self.check_indentation = false;
                self.compute_indentation(errors);
                if self.peek(0) != Some('#') || self.peek(1) != Some(':') {
                    break;
                }
                let line = self.line_of(self.pos);
                if line != prev_line + 1 {
                    break;
                }
                prev_line = line;
                let start_pos = self.pos;
                self.scan_to_newline();
                self.record_doc_comment_line(start_pos);
                texts.push(self.buffer_slice(start_pos, self.pos));
                lines.push(super::ast::DocCommentLine {
                    line,
                    text: self.buffer_slice(start_pos, self.pos),
                });
                last_end = self.pos;
            }
            self.set_token(TokenKind::DocCommentBlock, first_start, last_end);
            self.value = TokenValue::DocComments {
                end_line: prev_line,
                text: texts.join("\n"),
                lines,
            };
        } else {
            self.set_token(TokenKind::DocCommentTrailing, first_start, last_end);
            self.value = TokenValue::DocComments {
                end_line: self.line_of(first_start),
                text: texts.join("\n"),
                lines,
            };
        }
    }

    /// Records the comment scanned from `start` to the current position when it has the `#:`
    /// prefix.
    fn record_doc_comment_line(&mut self, start: usize) {
        if self.buffer.get(start + 1) == Some(&':') {
            let line = super::ast::DocCommentLine {
                line: self.line_of(start),
                text: self.buffer_slice(start, self.pos),
            };
            self.doc_comment_lines.push(line);
        }
    }

    fn scan_to_newline(&mut self) {
        while self.pos < self.buffer.len() {
            if self.buffer[self.pos] == '\n' {
                break;
            }
            self.pos += 1;
        }
    }

    // Java: net/starlark/java/syntax/Lexer.java:609 Lexer.identifierOrKeyword
    fn identifier_or_keyword(&mut self) {
        let old = self.pos - 1;
        while self.pos < self.buffer.len() {
            match self.buffer[self.pos] {
                '_' | 'a'..='z' | 'A'..='Z' | '0'..='9' => self.pos += 1,
                _ => break,
            }
        }
        let id = self.buffer_slice(old, self.pos);
        match keyword(&id, self.options.allow_type_syntax) {
            None => {
                self.set_token(TokenKind::Identifier, old, self.pos);
                self.value = TokenValue::Str(id);
            }
            Some(k) => self.set_token(k, old, self.pos),
        }
    }

    fn tokenize_two_chars(&mut self) -> bool {
        if self.pos + 2 >= self.buffer.len() {
            return false;
        }
        let c1 = self.buffer[self.pos];
        let c2 = self.buffer[self.pos + 1];
        let tok = if c2 == '=' {
            equal_token(c1)
        } else if c2 == '*' && c1 == '*' {
            Some(TokenKind::StarStar)
        } else {
            None
        };
        match tok {
            None => false,
            Some(t) => {
                self.set_token(t, self.pos, self.pos + 2);
                true
            }
        }
    }

    // Java: net/starlark/java/syntax/Lexer.java:689 Lexer.tokenize
    fn tokenize(&mut self, errors: &mut Vec<OffsetError>) {
        if self.check_indentation {
            self.check_indentation = false;
            self.compute_indentation(errors);
        }
        if self.dents != 0 {
            if self.dents < 0 {
                self.dents += 1;
                self.set_token(TokenKind::Outdent, self.pos.saturating_sub(1), self.pos);
            } else {
                self.dents -= 1;
                self.set_token(TokenKind::Indent, self.pos.saturating_sub(1), self.pos);
            }
            return;
        }
        let mut scanned;
        while self.pos < self.buffer.len() {
            if self.tokenize_two_chars() {
                self.pos += 2;
                return;
            }
            let c = self.buffer[self.pos];
            self.pos += 1;
            let p = self.pos;
            scanned = true;
            match c {
                '{' => {
                    self.set_token(TokenKind::LBrace, p - 1, p);
                    self.open_paren_stack_depth += 1;
                }
                '}' => {
                    self.set_token(TokenKind::RBrace, p - 1, p);
                    self.pop_paren(errors);
                }
                '(' => {
                    self.set_token(TokenKind::LParen, p - 1, p);
                    self.open_paren_stack_depth += 1;
                }
                ')' => {
                    self.set_token(TokenKind::RParen, p - 1, p);
                    self.pop_paren(errors);
                }
                '[' => {
                    self.set_token(TokenKind::LBracket, p - 1, p);
                    self.open_paren_stack_depth += 1;
                }
                ']' => {
                    self.set_token(TokenKind::RBracket, p - 1, p);
                    self.pop_paren(errors);
                }
                '>' => {
                    if self.peek(0) == Some('>') && self.peek(1) == Some('=') {
                        self.set_token(TokenKind::GreaterGreaterEquals, p - 1, p + 2);
                        self.pos += 2;
                    } else if self.peek(0) == Some('>') {
                        self.set_token(TokenKind::GreaterGreater, p - 1, p + 1);
                        self.pos += 1;
                    } else {
                        self.set_token(TokenKind::Greater, p - 1, p);
                    }
                }
                '<' => {
                    if self.peek(0) == Some('<') && self.peek(1) == Some('=') {
                        self.set_token(TokenKind::LessLessEquals, p - 1, p + 2);
                        self.pos += 2;
                    } else if self.peek(0) == Some('<') {
                        self.set_token(TokenKind::LessLess, p - 1, p + 1);
                        self.pos += 1;
                    } else {
                        self.set_token(TokenKind::Less, p - 1, p);
                    }
                }
                ':' => self.set_token(TokenKind::Colon, p - 1, p),
                ',' => self.set_token(TokenKind::Comma, p - 1, p),
                '+' => self.set_token(TokenKind::Plus, p - 1, p),
                '-' => {
                    if self.peek(0) == Some('>') {
                        self.set_token(TokenKind::RArrow, p - 1, p + 1);
                        self.pos += 1;
                    } else {
                        self.set_token(TokenKind::Minus, p - 1, p);
                    }
                }
                '|' => self.set_token(TokenKind::Pipe, p - 1, p),
                '=' => self.set_token(TokenKind::Equals, p - 1, p),
                '%' => self.set_token(TokenKind::Percent, p - 1, p),
                '~' => self.set_token(TokenKind::Tilde, p - 1, p),
                '&' => self.set_token(TokenKind::Ampersand, p - 1, p),
                '^' => self.set_token(TokenKind::Caret, p - 1, p),
                '/' => {
                    if self.peek(0) == Some('/') && self.peek(1) == Some('=') {
                        self.set_token(TokenKind::SlashSlashEquals, p - 1, p + 2);
                        self.pos += 2;
                    } else if self.peek(0) == Some('/') {
                        self.set_token(TokenKind::SlashSlash, p - 1, p + 1);
                        self.pos += 1;
                    } else {
                        self.set_token(TokenKind::Slash, p - 1, p);
                    }
                }
                ';' => self.set_token(TokenKind::Semi, p - 1, p),
                '*' => self.set_token(TokenKind::Star, p - 1, p),
                ' ' | '\t' | '\r' => {
                    scanned = false;
                }
                '\\' => {
                    if self.peek(0) == Some('\n') {
                        self.pos += 1;
                        scanned = false;
                    } else if self.peek(0) == Some('\r') && self.peek(1) == Some('\n') {
                        self.pos += 2;
                        scanned = false;
                    } else {
                        self.set_token(TokenKind::Illegal, p - 1, p);
                        self.value = TokenValue::Str("\\".to_owned());
                    }
                }
                '\n' => {
                    self.newline();
                    scanned = self.open_paren_stack_depth == 0;
                }
                '#' => {
                    let old = p - 1;
                    self.scan_to_newline();
                    self.record_doc_comment_line(old);
                    let is_doc = self.buffer.get(old + 1) == Some(&':');
                    if is_doc && self.open_paren_stack_depth == 0 {
                        let block = self.line_only_whitespace_or_comments;
                        self.doc_comments(old, block, errors);
                    } else {
                        scanned = false;
                    }
                }
                '\'' | '"' => self.string_literal(c, false, errors),
                _ => {
                    if c == 'r' {
                        let c0 = self.peek(0);
                        if c0 == Some('\'') || c0 == Some('"') {
                            self.pos += 1;
                            self.string_literal(c0.unwrap(), true, errors);
                            return;
                        }
                    }
                    if c == '.' || c.is_ascii_digit() {
                        self.pos -= 1;
                        self.scan_number_or_dot_or_ellipsis(c, errors);
                        return;
                    }
                    if c.is_ascii_alphabetic() || c == '_' {
                        self.identifier_or_keyword();
                    } else {
                        self.error(errors, format!("invalid character: '{c}'"), p - 1);
                        scanned = false;
                    }
                }
            }
            if scanned {
                return;
            }
        }
        if self.indent_stack.len() > 1 {
            self.set_token(TokenKind::Newline, self.pos.saturating_sub(1), self.pos);
            while self.indent_stack.len() > 1 {
                self.indent_stack.pop();
                self.dents -= 1;
            }
            return;
        }
        self.set_token(TokenKind::Eof, self.pos, self.pos);
    }

    // Java: net/starlark/java/syntax/Lexer.java:902 Lexer.scanNumberOrDotOrEllipsis
    fn scan_number_or_dot_or_ellipsis(&mut self, c: char, errors: &mut Vec<OffsetError>) {
        let start = self.pos;
        let mut fraction = false;
        let mut exponent = false;
        let mut c = Some(c);
        if c == Some('.') {
            if !isdigit(self.peek(1)) {
                if self.peek(1) == Some('.') && self.peek(2) == Some('.') {
                    self.pos += 3;
                    self.set_token(TokenKind::Ellipsis, start, self.pos);
                } else {
                    self.pos += 1;
                    self.set_token(TokenKind::Dot, start, self.pos);
                }
                return;
            }
            fraction = true;
        } else if c == Some('0') {
            c = self.next();
            if c == Some('.') {
                fraction = true;
            } else if matches!(c, Some('x' | 'X')) {
                c = self.next();
                if !isxdigit(c) {
                    self.error(errors, "invalid hex literal", start);
                }
                while isxdigit(c) {
                    c = self.next();
                }
            } else if matches!(c, Some('o' | 'O')) {
                c = self.next();
                while isdigit(c) {
                    c = self.next();
                }
            } else if matches!(c, Some('b' | 'B')) {
                c = self.next();
                if !isbdigit(c) {
                    self.error(errors, "invalid binary literal", start);
                }
                while isbdigit(c) {
                    c = self.next();
                }
            } else {
                while isdigit(c) {
                    c = self.next();
                }
                if c == Some('.') {
                    fraction = true;
                } else if matches!(c, Some('e' | 'E')) {
                    exponent = true;
                }
            }
        } else {
            while isdigit(c) {
                c = self.next();
            }
            if c == Some('.') {
                fraction = true;
            } else if matches!(c, Some('e' | 'E')) {
                exponent = true;
            }
        }
        if fraction {
            c = self.next();
            while isdigit(c) {
                c = self.next();
            }
            if matches!(c, Some('e' | 'E')) {
                exponent = true;
            }
        }
        if exponent {
            c = self.next();
            if matches!(c, Some('+' | '-')) {
                c = self.next();
            }
            while isdigit(c) {
                c = self.next();
            }
        }
        let text = self.buffer_slice(start, self.pos);
        if fraction || exponent {
            self.set_token(TokenKind::Float, start, self.pos);
            let mut value = 0.0;
            match java_parse_float_literal(&text) {
                Some(v) => {
                    value = v;
                    if !v.is_finite() {
                        self.error(errors, "floating-point literal too large", start);
                    }
                }
                None => self.error(errors, "invalid float literal", start),
            }
            self.value = TokenValue::Float(value);
            return;
        }
        self.set_token(TokenKind::Int, start, self.pos);
        let value = match int_literal_scan(&text) {
            Ok(v) => v,
            Err(m) => {
                self.error(errors, m, start);
                "0".to_owned()
            }
        };
        self.value = TokenValue::Int(value);
    }
}

/// Java `Double.parseDouble` on the text the scanner collects (digits, `.`, an exponent): `None`
/// when the exponent has no digits.
fn java_parse_float_literal(text: &str) -> Option<f64> {
    let (mant, exp) = match text.find(['e', 'E']) {
        Some(i) => (&text[..i], Some(&text[i + 1..])),
        None => (text, None),
    };
    if let Some(e) = exp {
        let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
        if digits.is_empty() {
            return None;
        }
    }
    if !mant.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse::<f64>().ok()
}
