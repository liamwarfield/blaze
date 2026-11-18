//! Port of `NetrcParser`.

use std::collections::{HashMap, VecDeque};
use std::io::Read;

use super::netrc::{Netrc, NetrcCredential};
use crate::errors::IoException;

const MACHINE: &str = "machine";
const MACDEF: &str = "macdef";
const DEFAULT: &str = "default";
const LOGIN: &str = "login";
const PASSWORD: &str = "password";
const ACCOUNT: &str = "account";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Item(String),
    Newline,
    Comment,
}

// Java: com/google/devtools/build/lib/authandtls/NetrcParser.java:73 NetrcParser.TokenStream
struct TokenStream {
    lines: VecDeque<String>,
    tokens: VecDeque<Token>,
}

impl TokenStream {
    fn new(content: &str) -> TokenStream {
        let lines = crate::fs_utils::split_lines(content).into();
        let mut s = TokenStream {
            lines,
            tokens: VecDeque::new(),
        };
        s.process_line();
        s
    }

    fn process_line(&mut self) {
        let Some(line) = self.lines.pop_front() else {
            return;
        };
        // Comments start with #
        if line.starts_with('#') {
            self.tokens.push_back(Token::Comment);
        } else {
            // Java String.split("\\s+") over Java whitespace (\t\n\x0B\f\r and space).
            for item in line.split([' ', '\t', '\n', '\x0B', '\x0C', '\r']) {
                if !item.is_empty() {
                    self.tokens.push_back(Token::Item(item.to_string()));
                }
            }
        }
        self.tokens.push_back(Token::Newline);
    }

    fn has_next(&self) -> bool {
        !self.tokens.is_empty()
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.pop_front();
        if self.tokens.is_empty() {
            self.process_line();
        }
        token
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.front()
    }
}

/// Parses `.netrc` content (read as ISO-8859-1, i.e. raw bytes) and closes the stream.
// Java: com/google/devtools/build/lib/authandtls/NetrcParser.java:121 NetrcParser.parseAndClose
pub fn parse_and_close(mut input: impl Read) -> Result<Netrc, IoException> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    // PORT: StringEncoding identity; the ISO-8859-1 reading keeps raw bytes, which Rust holds as
    // UTF-8 text (lossy only for invalid UTF-8).
    let content = String::from_utf8_lossy(&bytes);
    parse(&mut TokenStream::new(&content))
}

// Java: com/google/devtools/build/lib/authandtls/NetrcParser.java:127 NetrcParser.parse
fn parse(token_stream: &mut TokenStream) -> Result<Netrc, IoException> {
    let mut default_credential = None;
    let mut credential_map: HashMap<String, NetrcCredential> = HashMap::new();
    let mut done = false;
    while !done && token_stream.has_next() {
        if let Some(Token::Item(item)) = token_stream.next() {
            match item.as_str() {
                MACHINE => {
                    let machine = next_item(token_stream)?;
                    let credential = parse_credential_for_machine(token_stream, &machine)?;
                    credential_map.insert(machine, credential);
                }
                MACDEF => skip_macdef(token_stream),
                DEFAULT => {
                    default_credential = Some(parse_credential_for_machine(token_stream, DEFAULT)?);
                    // There can be only one default token, and it must be after all machine
                    // tokens.
                    done = true;
                }
                _ => {
                    return Err(IoException::new(format!(
                        "Unexpected token: {} (expecting {}, {} or {})",
                        item, MACHINE, MACDEF, DEFAULT
                    )));
                }
            }
        }
    }
    Ok(Netrc::create(default_credential, credential_map))
}

// Java: com/google/devtools/build/lib/authandtls/NetrcParser.java:157 NetrcParser.nextItem
fn next_item(token_stream: &mut TokenStream) -> Result<String, IoException> {
    while token_stream.has_next() {
        if let Some(Token::Item(item)) = token_stream.next() {
            return Ok(item);
        }
    }
    Err(IoException::new("Unexpected EOF"))
}

/// Parse credentials for a given machine from token stream.
// Java: com/google/devtools/build/lib/authandtls/NetrcParser.java:168 NetrcParser.parseCredentialForMachine
fn parse_credential_for_machine(
    token_stream: &mut TokenStream,
    machine: &str,
) -> Result<NetrcCredential, IoException> {
    let mut builder = NetrcCredential::builder(machine);
    let mut done = false;
    while !done && token_stream.has_next() {
        // Peek rather than taking next token since we probably won't process it
        let item = match token_stream.peek() {
            Some(Token::Item(item)) => Some(item.clone()),
            _ => None,
        };
        match item {
            Some(item) => match item.as_str() {
                LOGIN => {
                    token_stream.next();
                    builder = builder.set_login(&next_item(token_stream)?);
                }
                PASSWORD => {
                    token_stream.next();
                    builder = builder.set_password(&next_item(token_stream)?);
                }
                ACCOUNT => {
                    token_stream.next();
                    builder = builder.set_account(&next_item(token_stream)?);
                }
                MACHINE | MACDEF | DEFAULT => done = true,
                _ => {
                    return Err(IoException::new(format!(
                        "Unexpected item: {} (expecting {}, {}, {}, {}, {} or {})",
                        item, LOGIN, PASSWORD, ACCOUNT, MACHINE, MACDEF, DEFAULT
                    )));
                }
            },
            None => {
                token_stream.next();
            }
        }
    }
    Ok(builder)
}

/// Skip macdef section since we don't need that data currently.
// Java: com/google/devtools/build/lib/authandtls/NetrcParser.java:208 NetrcParser.skipMacdef
fn skip_macdef(token_stream: &mut TokenStream) {
    let mut num_newlines = 0;
    while token_stream.has_next() {
        if token_stream.next() == Some(Token::Newline) {
            num_newlines += 1;
        } else {
            num_newlines = 0;
        }
        if num_newlines >= 2 {
            break;
        }
    }
}
