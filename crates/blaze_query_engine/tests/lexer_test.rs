//! Port of `src/test/java/com/google/devtools/build/lib/query2/engine/LexerTest.java`.

use blaze_query_engine::lexer::{Lexer, Token, TokenKind};

fn as_string(tokens: &[Token]) -> String {
    let mut buffer = String::new();
    for token in tokens {
        if !buffer.is_empty() {
            buffer.push(' ');
        }
        buffer.push_str(&token.to_string());
    }
    buffer
}

fn scan(input: &str) -> Vec<Token> {
    Lexer::scan(input).unwrap()
}

#[test]
fn test_basics() {
    assert_eq!(as_string(&scan("")), "EOF");
}

#[test]
fn test_words_and_keywords() {
    let tokens = scan("foo bar wiz intersect");
    assert_eq!(as_string(&tokens), "foo bar wiz intersect EOF");
    assert_eq!(tokens[0].kind, TokenKind::Word);
    assert_eq!(tokens[1].kind, TokenKind::Word);
    assert_eq!(tokens[2].kind, TokenKind::Word);
    assert_eq!(tokens[3].kind, TokenKind::Intersect);
    assert_eq!(tokens[4].kind, TokenKind::Eof);
}

#[test]
fn test_punctuation_and_word_boundaries() {
    assert_eq!(
        as_string(&scan("foo(bar,wiz)deps=intersect")),
        "foo ( bar , wiz ) deps = intersect EOF"
    );
    assert_eq!(
        as_string(&scan("deps(//pkg:target)")),
        "deps ( //pkg:target ) EOF"
    );
}

#[test]
fn test_words_may_contain_dash_or_star_but_not_start_with_them() {
    assert_eq!(as_string(&scan("* foo*")), "* foo* EOF");
    assert_eq!(as_string(&scan("-foo foo-bar")), "- foo foo-bar EOF");
}

#[test]
fn test_dot_dot_dot() {
    assert_eq!(as_string(&scan("...")), "... EOF");
}

#[test]
fn test_quotation() {
    let tokens = scan("foo bar 'foo bar'");
    assert_eq!(as_string(&tokens), "foo bar foo bar EOF");
    assert_eq!(tokens[0].kind, TokenKind::Word);
    assert_eq!(tokens[1].kind, TokenKind::Word);
    assert_eq!(tokens[2].kind, TokenKind::Word);
    assert_eq!(tokens[2].word.as_deref(), Some("foo bar"));
}

#[test]
fn test_quoted_words_are_not_identifiers() {
    let tokens = scan("set 'set' \"set\"");
    assert_eq!(as_string(&tokens), "set set set EOF");
    assert_eq!(tokens[0].kind, TokenKind::Set);
    assert_eq!(tokens[1].kind, TokenKind::Word);
    assert_eq!(tokens[2].kind, TokenKind::Word);
}

#[test]
fn test_unterminated_quotation() {
    let e = Lexer::scan("'foo").unwrap_err();
    assert_eq!(e.get_message(), "unclosed quotation");
}

#[test]
fn test_operator_with_special_characters() {
    let tokens = scan("set(//foo_bar:.*@4)");
    assert_eq!(as_string(&tokens), "set ( //foo_bar:.*@4 ) EOF");
    assert_eq!(tokens[0].kind, TokenKind::Set);
    assert_eq!(tokens[1].kind, TokenKind::Lparen);
    assert_eq!(tokens[2].kind, TokenKind::Word);
    assert_eq!(tokens[3].kind, TokenKind::Rparen);
}

#[test]
fn test_operator_with_quoted_expr_with_special_characters() {
    let tokens = scan("set(\"//foo_bar:.*@4\")");
    assert_eq!(as_string(&tokens), "set ( //foo_bar:.*@4 ) EOF");
    assert_eq!(tokens[0].kind, TokenKind::Set);
    assert_eq!(tokens[1].kind, TokenKind::Lparen);
    assert_eq!(tokens[2].kind, TokenKind::Word);
    assert_eq!(tokens[3].kind, TokenKind::Rparen);
}

#[test]
fn test_operator_with_quoted_expr_with_more_special_characters() {
    let tokens = scan("set(\"//foo:foo=base/2~123[]+asd\")");
    assert_eq!(as_string(&tokens), "set ( //foo:foo=base/2~123[]+asd ) EOF");
    assert_eq!(tokens[0].kind, TokenKind::Set);
    assert_eq!(tokens[1].kind, TokenKind::Lparen);
    assert_eq!(tokens[2].kind, TokenKind::Word);
    assert_eq!(tokens[3].kind, TokenKind::Rparen);
}

#[test]
fn test_operator_with_unquoted_expr_with_special_characters() {
    let tokens = scan("set(//a:b=bar./@_:~-*$123[]+asd)");
    assert_eq!(
        as_string(&tokens),
        "set ( //a:b = bar./@_:~-*$123[] + asd ) EOF"
    );
    assert_eq!(tokens[0].kind, TokenKind::Set);
    assert_eq!(tokens[1].kind, TokenKind::Lparen);
    assert_eq!(tokens[2].kind, TokenKind::Word);
    assert_eq!(tokens[3].kind, TokenKind::Equals);
    assert_eq!(tokens[4].kind, TokenKind::Word);
    assert_eq!(tokens[5].kind, TokenKind::Plus);
    assert_eq!(tokens[6].kind, TokenKind::Word);
    assert_eq!(tokens[7].kind, TokenKind::Rparen);
}

#[test]
fn test_unquoted_canonical_labels() {
    let tokens =
        scan("somepath(@foo+@bar+//baz+@@foo +bar,  @@rules_jvm_external++maven+maven//:bar)");
    assert_eq!(
        as_string(&tokens),
        "somepath ( @foo + @bar + //baz + @@foo + bar , @@rules_jvm_external++maven+maven//:bar ) EOF"
    );
    assert_eq!(tokens[0].kind, TokenKind::Word);
    assert_eq!(tokens[1].kind, TokenKind::Lparen);
    assert_eq!(tokens[2].kind, TokenKind::Word);
    assert_eq!(tokens[3].kind, TokenKind::Plus);
    assert_eq!(tokens[4].kind, TokenKind::Word);
    assert_eq!(tokens[5].kind, TokenKind::Plus);
    assert_eq!(tokens[6].kind, TokenKind::Word);
    assert_eq!(tokens[7].kind, TokenKind::Plus);
    assert_eq!(tokens[8].kind, TokenKind::Word);
    assert_eq!(tokens[9].kind, TokenKind::Plus);
    assert_eq!(tokens[10].kind, TokenKind::Word);
    assert_eq!(tokens[11].kind, TokenKind::Comma);
    assert_eq!(tokens[12].kind, TokenKind::Word);
    assert_eq!(tokens[13].kind, TokenKind::Rparen);
}
