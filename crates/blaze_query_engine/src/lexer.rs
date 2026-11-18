//! Port of `query2/engine/Lexer.java`: the tokenizer of the query language.

use crate::query_syntax_exception::QuerySyntaxException;

/// Discriminator for different kinds of tokens (Java `Lexer.TokenKind`).
///
/// The declaration order is Java's (`ordinal()`): keywords follow `AllIdentifiersFollow`.
// Java: com/google/devtools/build/lib/query2/engine/Lexer.java:34 Lexer.TokenKind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TokenKind {
    Word,
    Eof,

    Comma,
    Equals,
    Lparen,
    Minus,
    Plus,
    Rparen,
    Caret,

    /// See `keyword_map`.
    AllIdentifiersFollow,

    In,
    Let,
    Set,

    Intersect,
    Except,
    Union,
}

impl TokenKind {
    /// All constants in declaration order (Java `EnumSet.allOf(TokenKind.class)`).
    pub const VALUES: [TokenKind; 16] = [
        TokenKind::Word,
        TokenKind::Eof,
        TokenKind::Comma,
        TokenKind::Equals,
        TokenKind::Lparen,
        TokenKind::Minus,
        TokenKind::Plus,
        TokenKind::Rparen,
        TokenKind::Caret,
        TokenKind::AllIdentifiersFollow,
        TokenKind::In,
        TokenKind::Let,
        TokenKind::Set,
        TokenKind::Intersect,
        TokenKind::Except,
        TokenKind::Union,
    ];

    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:62 TokenKind.getPrettyName
    pub fn get_pretty_name(self) -> &'static str {
        match self {
            TokenKind::Word => "word",
            TokenKind::Eof => "EOF",
            TokenKind::Comma => ",",
            TokenKind::Equals => "=",
            TokenKind::Lparen => "(",
            TokenKind::Minus => "-",
            TokenKind::Plus => "+",
            TokenKind::Rparen => ")",
            TokenKind::Caret => "^",
            TokenKind::AllIdentifiersFollow => "",
            TokenKind::In => "in",
            TokenKind::Let => "let",
            TokenKind::Set => "set",
            TokenKind::Intersect => "intersect",
            TokenKind::Except => "except",
            TokenKind::Union => "union",
        }
    }

    /// Java `Enum.name()` (what `TokenKind.toString()` prints).
    pub fn name(self) -> &'static str {
        match self {
            TokenKind::Word => "WORD",
            TokenKind::Eof => "EOF",
            TokenKind::Comma => "COMMA",
            TokenKind::Equals => "EQUALS",
            TokenKind::Lparen => "LPAREN",
            TokenKind::Minus => "MINUS",
            TokenKind::Plus => "PLUS",
            TokenKind::Rparen => "RPAREN",
            TokenKind::Caret => "CARET",
            TokenKind::AllIdentifiersFollow => "__ALL_IDENTIFIERS_FOLLOW",
            TokenKind::In => "IN",
            TokenKind::Let => "LET",
            TokenKind::Set => "SET",
            TokenKind::Intersect => "INTERSECT",
            TokenKind::Except => "EXCEPT",
            TokenKind::Union => "UNION",
        }
    }

    /// Java `ordinal()`.
    pub fn ordinal(self) -> i32 {
        TokenKind::VALUES.iter().position(|k| *k == self).unwrap() as i32
    }
}

/// Java `Lexer.BINARY_OPERATORS`.
// Java: com/google/devtools/build/lib/query2/engine/Lexer.java:67 Lexer.BINARY_OPERATORS
pub const BINARY_OPERATORS: [TokenKind; 6] = [
    TokenKind::Intersect,
    TokenKind::Caret,
    TokenKind::Union,
    TokenKind::Plus,
    TokenKind::Except,
    TokenKind::Minus,
];

/// `BINARY_OPERATORS.contains(kind)`.
pub fn is_binary_operator(kind: TokenKind) -> bool {
    BINARY_OPERATORS.contains(&kind)
}

/// Java `Lexer.keywordMap.get(word)`: the keyword kinds are those after `__ALL_IDENTIFIERS_FOLLOW`.
// Java: com/google/devtools/build/lib/query2/engine/Lexer.java:75 Lexer.keywordMap
fn keyword_map_get(word: &str) -> Option<TokenKind> {
    TokenKind::VALUES
        .iter()
        .copied()
        .filter(|kind| kind.ordinal() > TokenKind::AllIdentifiersFollow.ordinal())
        .find(|kind| kind.get_pretty_name() == word)
}

/// Returns true iff 'word' is a reserved word of the language.
// Java: com/google/devtools/build/lib/query2/engine/Lexer.java:87 Lexer.isReservedWord
pub fn is_reserved_word(word: &str) -> bool {
    keyword_map_get(word).is_some()
}

/// Tokens returned by the Lexer (Java `Lexer.Token`).
///
/// Java compares tokens by identity (`tokens.indexOf(token)`); the parser here tracks token
/// indices instead.
// Java: com/google/devtools/build/lib/query2/engine/Lexer.java:94 Lexer.Token
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub word: Option<String>,
}

impl Token {
    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:99 Token.<init>
    pub fn of_kind(kind: TokenKind) -> Token {
        Token { kind, word: None }
    }

    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:104 Token.<init>
    pub fn of_word(word: String) -> Token {
        Token {
            kind: TokenKind::Word,
            word: Some(word),
        }
    }
}

impl std::fmt::Display for Token {
    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:110 Token.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.kind == TokenKind::Word {
            // Java prints "null" for a null word; WORD tokens always carry one.
            f.write_str(self.word.as_deref().unwrap_or("null"))
        } else {
            f.write_str(self.kind.get_pretty_name())
        }
    }
}

/// A tokenizer for the Blaze query language, revision 2.
///
/// No string escapes are allowed ("\"). Given the domain, that's not currently a problem.
///
/// PORT: Java scans UTF-16 code units; this scans UTF-8 bytes. Every character the lexer
/// distinguishes is ASCII, and every byte of a non-ASCII character is >= 0x80 just as every
/// non-ASCII UTF-16 unit is > 0x7F, so the token boundaries are identical.
// Java: com/google/devtools/build/lib/query2/engine/Lexer.java:29 Lexer
pub struct Lexer<'a> {
    input: &'a str,
    pos: usize,
    tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    /// Entry point to the lexer. Returns the list of tokens for the specified input, or throws
    /// QueryException.
    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:119 Lexer.scan
    pub fn scan(input: &str) -> Result<Vec<Token>, QuerySyntaxException> {
        let mut lexer = Lexer {
            input,
            pos: 0,
            tokens: Vec::new(),
        };
        lexer.tokenize()?;
        Ok(lexer.tokens)
    }

    fn add_token(&mut self, s: Token) {
        self.tokens.push(s);
    }

    fn byte_at(&self, i: usize) -> u8 {
        self.input.as_bytes()[i]
    }

    /// Scans a quoted word delimited by 'quot'.
    ///
    /// ON ENTRY: 'pos' is 1 + the index of the first delimiter ON EXIT: 'pos' is 1 + the index of
    /// the last delimiter.
    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:148 Lexer.quotedWord
    fn quoted_word(&mut self, quot: u8) -> Result<Token, QuerySyntaxException> {
        let old_pos = self.pos - 1;
        while self.pos < self.input.len() {
            let c = self.byte_at(self.pos);
            self.pos += 1;
            match c {
                b'\'' | b'"' => {
                    if c == quot {
                        // close-quote, all done.
                        return Ok(Token::of_word(
                            self.buffer_slice(old_pos + 1, self.pos - 1).to_owned(),
                        ));
                    }
                }
                _ => {}
            }
        }
        Err(QuerySyntaxException::new("unclosed quotation"))
    }

    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:165 Lexer.getTokenKindForWord
    fn get_token_kind_for_word(word: &str) -> TokenKind {
        keyword_map_get(word).unwrap_or(TokenKind::Word)
    }

    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:170 Lexer.scanWord
    fn scan_word(&mut self, first_char: u8) -> String {
        let old_pos = self.pos - 1;
        let starts_with_double_at =
            first_char == b'@' && self.pos < self.input.len() && self.byte_at(self.pos) == b'@';
        while self.pos < self.input.len() {
            let c = self.byte_at(self.pos);
            match c {
                b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'*'
                | b'/'
                | b'@'
                | b'.'
                | b'-'
                | b'_'
                | b':'
                | b'$'
                | b'~'
                | b'['
                | b']' => self.pos += 1,
                b'+' => {
                    if starts_with_double_at {
                        // Allow unquoted canonical labels such as
                        // @@rules_jvm_external++maven+maven//:bar, but still parse @foo+@bar as two
                        // separate labels (here @foo refers to the @foo//:foo target).
                        // If @@foo+bar is intended to mean @@foo + bar, it can be written as such
                        // with spaces.
                        self.pos += 1;
                    } else {
                        return self.buffer_slice(old_pos, self.pos).to_owned();
                    }
                }
                _ => {
                    // All remaining ASCII characters (and only ASCII characters) are query
                    // operators.
                    if c <= 0x7F {
                        return self.buffer_slice(old_pos, self.pos).to_owned();
                    }
                    // Unicode characters are allowed in words, so we continue scanning.
                    self.pos += 1;
                }
            }
        }
        self.buffer_slice(old_pos, self.pos).to_owned()
    }

    /// Scans a word or keyword.
    ///
    /// ON ENTRY: 'pos' is 1 + the index of the first char in the word. ON EXIT: 'pos' is 1 + the
    /// index of the last char in the word.
    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:283 Lexer.wordOrKeyword
    fn word_or_keyword(&mut self, first_char: u8) -> Token {
        let word = self.scan_word(first_char);
        let kind = Self::get_token_kind_for_word(&word);
        if kind == TokenKind::Word {
            Token::of_word(word)
        } else {
            Token::of_kind(kind)
        }
    }

    /// Performs tokenization of the character buffer of file contents provided to the
    /// constructor.
    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:290 Lexer.tokenize
    fn tokenize(&mut self) -> Result<(), QuerySyntaxException> {
        while self.pos < self.input.len() {
            let c = self.byte_at(self.pos);
            self.pos += 1;
            match c {
                b'(' => self.add_token(Token::of_kind(TokenKind::Lparen)),
                b')' => self.add_token(Token::of_kind(TokenKind::Rparen)),
                b',' => self.add_token(Token::of_kind(TokenKind::Comma)),
                b'+' => self.add_token(Token::of_kind(TokenKind::Plus)),
                b'-' => self.add_token(Token::of_kind(TokenKind::Minus)),
                b'=' => self.add_token(Token::of_kind(TokenKind::Equals)),
                b'^' => self.add_token(Token::of_kind(TokenKind::Caret)),
                b'\n' | b' ' | b'\t' | b'\r' => { /* ignore */ }
                b'\'' | b'"' => {
                    let token = self.quoted_word(c)?;
                    self.add_token(token);
                }
                _ => {
                    // PORT: a word that starts with a non-ASCII character starts at its first
                    // byte; scan_word then consumes the remaining bytes (all >= 0x80), exactly as
                    // Java consumes the rest of the character.
                    let token = self.word_or_keyword(c);
                    self.add_token(token);
                }
            }
        }

        self.add_token(Token::of_kind(TokenKind::Eof));
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/engine/Lexer.java:315 Lexer.bufferSlice
    fn buffer_slice(&self, start: usize, end: usize) -> &'a str {
        &self.input[start..end]
    }
}
