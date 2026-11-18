//! Port of `query2/engine/QueryParser.java`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::binary_operator_expression::BinaryOperatorExpression;
use crate::function_expression::FunctionExpression;
use crate::let_expression::LetExpression;
use crate::lexer::{Lexer, Token, TokenKind, is_binary_operator};
use crate::query_environment::{Argument, ArgumentType, QueryEnvironment, QueryFunction};
use crate::query_expression::QueryExpression;
use crate::query_syntax_exception::QuerySyntaxException;
use crate::set_expression::SetExpression;
use crate::target_literal::TargetLiteral;

/// LL(1) recursive descent parser for the Blaze query language, revision 2.
///
/// In the grammar below, non-terminals are lowercase and terminals are uppercase, or character
/// literals.
///
/// ```text
/// expr ::= WORD
///        | LET WORD = expr IN expr
///        | '(' expr ')'
///        | WORD '(' expr ( ',' expr ) * ')'
///        | expr INTERSECT expr
///        | expr '^' expr
///        | expr UNION expr
///        | expr '+' expr
///        | expr EXCEPT expr
///        | expr '-' expr
///        | SET '(' WORD * ')'
/// ```
///
/// PORT: Java finds a token's position with `tokens.indexOf(token)` (identity); this parser keeps
/// the index of the current token.
// Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:52 QueryParser
pub struct QueryParser<'a, T> {
    /// Index of the current lookahead token.
    token: usize,
    tokens: Vec<Token>,
    functions: &'a HashMap<String, Arc<dyn QueryFunction<T>>>,
}

/// Java `Integer.parseInt(s)`.
///
/// PORT: Java's `Character.digit` also accepts non-ASCII Unicode decimal digits; this accepts
/// ASCII digits only.
fn java_parse_int(s: &str) -> Option<i32> {
    let (negative, digits) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() {
        return None;
    }
    let mut result: i64 = 0;
    for c in digits.chars() {
        let digit = c.to_digit(10)? as i64;
        result = result * 10 + digit;
        if result > i32::MAX as i64 + 1 {
            return None;
        }
    }
    let result = if negative { -result } else { result };
    i32::try_from(result).ok()
}

impl<'a, T: 'static> QueryParser<'a, T> {
    /// Scan and parse the specified query expression.
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:60 QueryParser.parse
    pub fn parse(
        query: &str,
        env: &dyn QueryEnvironment<T>,
    ) -> Result<QueryExpression, QuerySyntaxException> {
        let mut functions: HashMap<String, Arc<dyn QueryFunction<T>>> = HashMap::new();
        for query_function in env.get_functions() {
            functions.insert(query_function.get_name(), query_function);
        }
        QueryParser::parse_with_functions(query, &functions)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:69 QueryParser.parse
    pub fn parse_with_functions(
        query: &str,
        functions: &HashMap<String, Arc<dyn QueryFunction<T>>>,
    ) -> Result<QueryExpression, QuerySyntaxException> {
        let mut parser = QueryParser::new(Lexer::scan(query)?, functions);
        let expr = parser.parse_expression()?;
        if parser.current().kind != TokenKind::Eof {
            return Err(QuerySyntaxException::new(format!(
                "unexpected token '{}' after query expression '{}'",
                parser.current(),
                expr.to_trunctated_string()
            )));
        }
        Ok(expr)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:82 QueryParser.<init>
    pub fn new(
        tokens: Vec<Token>,
        functions: &'a HashMap<String, Arc<dyn QueryFunction<T>>>,
    ) -> QueryParser<'a, T> {
        // `nextToken()` from a null token reads the first token.
        QueryParser {
            token: 0,
            tokens,
            functions,
        }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.token]
    }

    /// Throws a syntax error exception.
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:91 QueryParser.syntaxError
    fn syntax_error(&self, token: usize) -> QuerySyntaxException {
        let mut message = String::from("premature end of input");
        if self.tokens[token].kind != TokenKind::Eof {
            let mut buf = String::from("syntax error at '");
            self.append_input_context(&mut buf, token);
            buf.push('\'');
            message = buf;
        }
        QuerySyntaxException::new(message)
    }

    /// Throws an exception indicating that the current token is an unknown function name.
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:111 QueryParser.unknownFunctionError
    fn unknown_function_error(&self, token: usize) -> QuerySyntaxException {
        blaze_util::check_argument!(self.tokens[token].kind == TokenKind::Word);
        let mut buf = String::from("unknown function '");
        buf.push_str(&self.tokens[token].to_string());
        buf.push_str("' at '");
        self.append_input_context(&mut buf, token);
        buf.push_str("'; expected one of ['");
        let mut names: Vec<&String> = self.functions.keys().collect();
        names.sort();
        buf.push_str(
            &names
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("', '"),
        );
        buf.push_str("']");
        QuerySyntaxException::new(buf)
    }

    /// Throws an exception indicating that the current function is being called with the wrong
    /// number of arguments.
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:128 QueryParser.functionArgumentCountError
    fn function_argument_count_error(
        &self,
        function: &dyn QueryFunction<T>,
        description: &str,
    ) -> QuerySyntaxException {
        let mut buf = String::from(description);
        buf.push_str(" arguments to function '");
        buf.push_str(&function.get_name());
        buf.push_str("' at '");
        self.append_input_context(&mut buf, self.token);
        buf.push('\'');
        QuerySyntaxException::new(buf)
    }

    /// Throws an exception indicating that the current function is being called with too few
    /// arguments.
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:144 QueryParser.tooFewArgumentsError
    fn too_few_arguments_error(&self, function: &dyn QueryFunction<T>) -> QuerySyntaxException {
        self.function_argument_count_error(function, "too few")
    }

    /// Throws an exception indicating that the current function is being called with too many
    /// arguments.
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:154 QueryParser.tooManyArgumentsError
    fn too_many_arguments_error(&self, function: &dyn QueryFunction<T>) -> QuerySyntaxException {
        self.function_argument_count_error(function, "too many")
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:159 QueryParser.appendInputContext
    fn append_input_context(&self, buf: &mut String, token: usize) {
        let mut sep = "";
        let max = std::cmp::min(self.tokens.len() - 1, token + 3); // 3 tokens of context
        for index in token..max {
            buf.push_str(sep);
            buf.push_str(&self.tokens[index].to_string());
            sep = " ";
        }
    }

    /// Consumes the current token. If it is not of the specified (expected) kind, throws
    /// `QuerySyntaxException`. Returns the value associated with the consumed token, if any.
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:175 QueryParser.consume
    fn consume(&mut self, kind: TokenKind) -> Result<Option<String>, QuerySyntaxException> {
        if self.current().kind != kind {
            return Err(self.syntax_error(self.token));
        }
        let word = self.current().word.clone();
        self.next_token();
        Ok(word)
    }

    /// Consumes a WORD token and returns its word.
    fn consume_word(&mut self) -> Result<String, QuerySyntaxException> {
        Ok(self
            .consume(TokenKind::Word)?
            .expect("WORD tokens carry a word"))
    }

    /// Consumes the current token, which must be a WORD containing an integer literal. Returns
    /// that integer, or throws a `QuerySyntaxException` otherwise.
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:188 QueryParser.consumeIntLiteral
    fn consume_int_literal(&mut self) -> Result<i32, QuerySyntaxException> {
        let int_string = self.consume_word()?;
        match java_parse_int(&int_string) {
            Some(i) => Ok(i),
            None => Err(QuerySyntaxException::new(format!(
                "expected an integer literal: '{int_string}'"
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:199 QueryParser.nextToken
    fn next_token(&mut self) {
        if self.current().kind != TokenKind::Eof {
            self.token += 1;
        }
    }

    /// ```text
    /// expr ::= primary
    ///        | expr INTERSECT expr
    ///        | expr '^' expr
    ///        | expr UNION expr
    ///        | expr '+' expr
    ///        | expr EXCEPT expr
    ///        | expr '-' expr
    /// ```
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:218 QueryParser.parseExpression
    fn parse_expression(&mut self) -> Result<QueryExpression, QuerySyntaxException> {
        // All operators are left-associative and of equal precedence.
        let primary = self.parse_primary()?;
        self.parse_binary_operator_tail(primary)
    }

    /// ```text
    /// tail ::= ( <op> <primary> )*
    /// ```
    ///
    /// All operators have equal precedence. This factoring is required for left-associative
    /// binary operators in LL(1).
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:233 QueryParser.parseBinaryOperatorTail
    fn parse_binary_operator_tail(
        &mut self,
        lhs: QueryExpression,
    ) -> Result<QueryExpression, QuerySyntaxException> {
        if !is_binary_operator(self.current().kind) {
            return Ok(lhs);
        }

        let mut operands = vec![lhs];
        let mut last_operator = self.current().kind;

        while is_binary_operator(self.current().kind) {
            let operator = self.current().kind;
            self.consume(operator)?;
            if operator != last_operator {
                let lhs: QueryExpression =
                    BinaryOperatorExpression::new(last_operator, std::mem::take(&mut operands))
                        .into();
                operands.push(lhs);
                last_operator = operator;
            }
            let rhs = self.parse_primary()?;
            operands.push(rhs);
        }
        Ok(BinaryOperatorExpression::new(last_operator, operands).into())
    }

    /// ```text
    /// primary ::= WORD
    ///           | WORD '(' arg ( ',' arg ) * ')'
    ///           | LET WORD = expr IN expr
    ///           | '(' expr ')'
    ///           | SET '(' WORD * ')'
    /// arg ::= expr
    ///       | WORD
    ///       | INT
    /// ```
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:270 QueryParser.parsePrimary
    fn parse_primary(&mut self) -> Result<QueryExpression, QuerySyntaxException> {
        match self.current().kind {
            TokenKind::Word => {
                let word_token = self.token;
                let word = self.consume_word()?;
                if self.current().kind == TokenKind::Lparen {
                    let function = match self.functions.get(&word) {
                        Some(function) => function.clone(),
                        None => return Err(self.unknown_function_error(word_token)),
                    };
                    let mut args = Vec::new();
                    let mut token_kind = TokenKind::Lparen;
                    let mut args_seen = 0;
                    for type_ in function.get_argument_types() {
                        if self.current().kind == TokenKind::Rparen {
                            // Got rparen instead of argument-separating comma.
                            if args_seen >= function.get_mandatory_arguments() {
                                break;
                            } else {
                                return Err(self.too_few_arguments_error(&*function));
                            }
                        }

                        // Consume lparen on first iteration, comma on subsequent iterations.
                        self.consume(token_kind)?;
                        token_kind = TokenKind::Comma;
                        if args_seen == 0 && self.current().kind == TokenKind::Rparen {
                            // Got rparen instead of mandatory first argument.
                            return Err(self.too_few_arguments_error(&*function));
                        }
                        match type_ {
                            ArgumentType::Expression => {
                                args.push(Argument::of_expression(self.parse_expression()?))
                            }
                            ArgumentType::Word => {
                                args.push(Argument::of_word(self.consume_word()?))
                            }
                            ArgumentType::Integer => {
                                args.push(Argument::of_integer(self.consume_int_literal()?))
                            }
                        }

                        args_seen += 1;
                    }

                    if self.current().kind == TokenKind::Comma && args_seen > 0 {
                        return Err(self.too_many_arguments_error(&*function));
                    }
                    self.consume(TokenKind::Rparen)?;
                    Ok(FunctionExpression::new(function, args).into())
                } else {
                    Ok(Self::validate_target_literal(word)?.into())
                }
            }
            TokenKind::Let => {
                self.consume(TokenKind::Let)?;
                let name = self.consume_word()?;
                self.consume(TokenKind::Equals)?;
                let var_expr = self.parse_expression()?;
                self.consume(TokenKind::In)?;
                let body_expr = self.parse_expression()?;
                Ok(LetExpression::new(name, var_expr, body_expr).into())
            }
            TokenKind::Lparen => {
                self.consume(TokenKind::Lparen)?;
                let expr = self.parse_expression()?;
                self.consume(TokenKind::Rparen)?;
                Ok(expr)
            }
            TokenKind::Set => {
                self.next_token();
                self.consume(TokenKind::Lparen)?;
                let mut words = Vec::new();
                while self.current().kind == TokenKind::Word {
                    let word = self.consume_word()?;
                    words.push(Arc::new(Self::validate_target_literal(word)?));
                }
                self.consume(TokenKind::Rparen)?;
                Ok(SetExpression::new(words).into())
            }
            _ => Err(self.syntax_error(self.token)),
        }
    }

    /// Unquoted words may not start with a hyphen or asterisk, even though relative target names
    /// may start with those characters.
    // Java: com/google/devtools/build/lib/query2/engine/QueryParser.java:351 QueryParser.validateTargetLiteral
    fn validate_target_literal(word: String) -> Result<TargetLiteral, QuerySyntaxException> {
        if word.starts_with('-') || word.starts_with('*') {
            return Err(QuerySyntaxException::new(format!(
                "target literal must not begin with ({}): {}",
                word.chars().next().unwrap(),
                word
            )));
        }
        Ok(TargetLiteral::new(word))
    }
}

#[cfg(test)]
mod tests {
    use super::java_parse_int;

    #[test]
    fn java_parse_int_matches_java() {
        assert_eq!(java_parse_int("3"), Some(3));
        assert_eq!(java_parse_int("+3"), Some(3));
        assert_eq!(java_parse_int("-3"), Some(-3));
        assert_eq!(java_parse_int("2147483647"), Some(i32::MAX));
        assert_eq!(java_parse_int("-2147483648"), Some(i32::MIN));
        assert_eq!(java_parse_int("2147483648"), None);
        assert_eq!(java_parse_int("-"), None);
        assert_eq!(java_parse_int(""), None);
        assert_eq!(java_parse_int("c"), None);
        assert_eq!(java_parse_int("1a"), None);
    }
}
