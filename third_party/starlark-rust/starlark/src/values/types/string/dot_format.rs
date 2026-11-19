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

use std::str::FromStr;

use starlark_syntax::dot_format_parser::FormatConv;
use starlark_syntax::dot_format_parser::FormatParser;
use starlark_syntax::dot_format_parser::FormatToken;

use crate::collections::string_pool::StringPool;
use crate::values::Heap;
use crate::values::StringValue;
use crate::values::Value;
use crate::values::ValueError;
use crate::values::ValueLike;
use crate::values::dict::Dict;

/// Try parse `"aaa{}bbb"` and return `("aaa", "bbb")`.
pub(crate) fn parse_format_one(s: &str) -> Option<(String, String)> {
    let mut parser = FormatParser::new(s);
    let mut before = String::with_capacity(s.len());
    loop {
        match parser.next().ok()?? {
            FormatToken::Text(text) => before.push_str(text),
            FormatToken::Escape(e) => before.push_str(e.as_str()),
            FormatToken::Capture {
                capture: "",
                conv: FormatConv::Str,
                pos: _,
            } => break,
            FormatToken::Capture { .. } => return None,
        }
    }

    let mut after = String::with_capacity(s.len() - before.len());
    loop {
        match parser.next().ok()? {
            Some(FormatToken::Text(text)) => after.push_str(text),
            Some(FormatToken::Escape(e)) => after.push_str(e.as_str()),
            Some(FormatToken::Capture { .. }) => return None,
            None => break,
        }
    }

    Some((before, after))
}

/// Evaluate `"<before>{}<after>".format(arg)`.
pub(crate) fn format_one<'v>(
    before: &str,
    arg: Value<'v>,
    after: &str,
    heap: Heap<'v>,
) -> StringValue<'v> {
    match StringValue::new(arg) {
        Some(arg) => heap.alloc_str_concat3(before, &arg, after),
        None => {
            let mut result = String::with_capacity(before.len() + after.len() + 10);
            result.push_str(before);
            arg.collect_str(&mut result); // BLAZE PATCH 0011
            result.push_str(after);
            heap.alloc_str(&result)
        }
    }
}

/// The format string can either have explicit indices,
/// or grab things sequentially, but not both.
/// FormatArgs knows which we are doing and keeps them in mind.
struct FormatArgs<'v, T: Iterator<Item = Value<'v>>> {
    // Initially we have the iterator set and the args empty.
    // If we ever ask by index, we decant the iterator into args.
    iterator: T,
    args: Vec<Value<'v>>,
    by_index: bool,
    by_order: bool,
}

impl<'v, T: Iterator<Item = Value<'v>>> FormatArgs<'v, T> {
    fn new(iterator: T) -> Self {
        Self {
            iterator,
            args: Vec::new(),
            by_index: false,
            by_order: false,
        }
    }

    fn next_ordered(&mut self) -> anyhow::Result<Value<'v>> {
        if self.by_index {
            Err(anyhow::anyhow!(
                "Cannot mix manual field specification and automatic field numbering in format string",
            ))
        } else {
            self.by_order = true;
            match self.iterator.next() {
                None => Err(anyhow::anyhow!("Not enough parameters in format string")),
                Some(x) => Ok(x),
            }
        }
    }

    // BLAZE PATCH 0045: a Java `int` index (FormatParser.java:89-96: negative is "No replacement
    // found for index -1", after the numbering check).
    fn by_index(&mut self, index: i32) -> anyhow::Result<Value<'v>> { // BLAZE PATCH 0045
        if self.by_order {
            Err(anyhow::anyhow!(
                "Cannot mix manual field specification and automatic field numbering in format string",
            ))
        } else {
            if !self.by_index {
                self.args.extend(&mut self.iterator);
                self.by_index = true;
            }
            match usize::try_from(index).ok().and_then(|i| self.args.get(i)) { // BLAZE PATCH 0045
                None => Err(ValueError::IndexOutOfBound(index).into()), // BLAZE PATCH 0045
                Some(v) => Ok(*v),
            }
        }
    }
}

/// BLAZE PATCH 0051: Java's `FormatParser.format` (the algorithm and its texts), replacing
/// starlark-rust's parser for `str.format` (f-strings keep it).
// Java: net/starlark/java/eval/FormatParser.java:51 FormatParser.format
pub(crate) fn format<'v>(
    this: &str,
    args: impl Iterator<Item = Value<'v>>,
    kwargs: Dict<'v>,
    string_pool: &mut StringPool,
    heap: Heap<'v>,
) -> anyhow::Result<StringValue<'v>> {
    let args: Vec<Value<'v>> = args.collect();
    let chars: Vec<char> = this.chars().collect();
    let mut output = string_pool.alloc();
    // Java FormatParser.History: NONE (0), MANUAL (1), AUTOMATIC (2); last automatic index.
    let mut history_type = 0u8;
    let mut position: i32 = -1;
    let mixed = || anyhow::anyhow!("Cannot mix manual and automatic numbering of positional fields");
    let get_kwarg = |key: &str| -> anyhow::Result<Value<'v>> {
        match kwargs.get_str(key) {
            Some(v) => Ok(v),
            None => Err(anyhow::anyhow!("Missing argument '{key}'")),
        }
    };
    let mut pos = 0;
    while pos < chars.len() {
        let current = chars[pos];
        let mut advance = 0;
        if current == '{' {
            // Java: FormatParser.java:85 processOpeningBrace
            if chars.get(pos + 1) == Some(&'{') {
                output.push('{');
                advance = 1;
            } else {
                // Java: FormatParser.java:181 getFieldName
                let mut key = String::new();
                let mut found_closing = false;
                for &c in &chars[pos + 1..] {
                    if c == '}' {
                        found_closing = true;
                        break;
                    }
                    if c == '{' {
                        return Err(anyhow::anyhow!(
                            "Nested replacement fields are not supported"
                        ));
                    } else if matches!(c, '.' | '[' | ']' | ',') {
                        return Err(anyhow::anyhow!(
                            "Invalid character '{c}' inside replacement field"
                        ));
                    }
                    key.push(c);
                }
                if !found_closing {
                    return Err(anyhow::anyhow!("Found '{{' without matching '}}'"));
                }
                let value = if key.is_empty()
                    || key.chars().all(|c| c.is_ascii_digit() || c == '-')
                {
                    // Java: FormatParser.java:218 parsePositional (Integer.parseInt).
                    let index = if key.is_empty() {
                        if history_type == 1 {
                            return Err(mixed());
                        }
                        history_type = 2;
                        position += 1;
                        Some(position)
                    } else {
                        match java_parse_int(&key) {
                            Some(i) => {
                                if history_type == 2 {
                                    return Err(mixed());
                                }
                                history_type = 1;
                                Some(i)
                            }
                            None => None,
                        }
                    };
                    match index {
                        Some(index) => {
                            if index < 0 || index as usize >= args.len() {
                                return Err(anyhow::anyhow!(
                                    "No replacement found for index {index}"
                                ));
                            }
                            args[index as usize]
                        }
                        // NumberFormatException: a keyword.
                        None => get_kwarg(&key)?,
                    }
                } else {
                    get_kwarg(&key)?
                };
                value.collect_str(&mut output);
                advance = key.chars().count() + 1;
            }
        } else if current == '}' {
            // Java: FormatParser.java:150 processClosingBrace
            if chars.get(pos + 1) != Some(&'}') {
                return Err(anyhow::anyhow!("Found '}}' without matching '{{'"));
            }
            output.push('}');
            advance = 1;
        } else {
            output.push(current);
        }
        pos += advance + 1;
    }
    let r = heap.alloc_str(&output);
    string_pool.release(output);
    Ok(r)
}

/// Java `Integer.parseInt` on a string of ASCII digits and `-` (BLAZE PATCH 0051).
fn java_parse_int(s: &str) -> Option<i32> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let v: i64 = if digits.len() > 11 {
        return None;
    } else {
        digits.parse().ok()?
    };
    let v = if neg { -v } else { v };
    i32::try_from(v).ok()
}

#[allow(dead_code)] // BLAZE PATCH 0051: starlark-rust's `str.format`, replaced by `format`.
fn format_upstream<'v>(
    this: &str,
    args: impl Iterator<Item = Value<'v>>,
    kwargs: Dict<'v>,
    string_pool: &mut StringPool,
    heap: Heap<'v>,
) -> anyhow::Result<StringValue<'v>> {
    let mut parser = FormatParser::new(this);
    let mut result = string_pool.alloc();
    let mut args = FormatArgs::new(args);
    while let Some(token) = parser.next()? {
        match token {
            FormatToken::Text(text) => result.push_str(text),
            FormatToken::Escape(e) => result.push_str(e.as_str()),
            FormatToken::Capture {
                capture,
                conv,
                pos: _,
            } => format_capture(capture, conv, &mut args, &kwargs, &mut result)?,
        }
    }
    let r = heap.alloc_str(&result);
    string_pool.release(result);
    Ok(r)
}

fn format_capture<'v, T: Iterator<Item = Value<'v>>>(
    field: &str,
    conv: FormatConv,
    args: &mut FormatArgs<'v, T>,
    kwargs: &Dict,
    result: &mut String,
) -> anyhow::Result<()> {
    let conv_s = |x: Value, result: &mut String| x.collect_str(result);
    let conv_r = |x: Value, result: &mut String| x.collect_repr(result);
    let conv: &dyn Fn(Value, &mut String) = match conv {
        FormatConv::Str => &conv_s,
        FormatConv::Repr => &conv_r,
    };
    if field.is_empty() {
        conv(args.next_ordered()?, result);
        Ok(())
    // BLAZE PATCH 0045: Java FormatParser.processOpeningBrace (FormatParser.java:85-100): a
    // field of digits and `-` is an index when `Integer.parseInt` accepts it, else a keyword.
    } else if let Some(i) = field // BLAZE PATCH 0045
        .bytes() // BLAZE PATCH 0045
        .all(|c| c.is_ascii_digit() || c == b'-') // BLAZE PATCH 0045
        .then(|| i32::from_str(field).ok()) // BLAZE PATCH 0045
        .flatten() // BLAZE PATCH 0045
    {
        conv(args.by_index(i)?, result);
        Ok(())
    } else {
        if let Some(x) = field.bytes().find(|c| match c {
            b'.' | b',' | b'[' | b']' => true,
            _ => false,
        }) {
            return Err(anyhow::anyhow!(
                "Invalid character '{}' inside replacement field",
                char::from(x)
            ));
        }
        match kwargs.get_str(field) {
            None => Err(ValueError::KeyNotFound(field.to_owned()).into()),
            Some(v) => {
                conv(v, result);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use starlark_map::small_map::SmallMap;
    use starlark_syntax::dot_format_parser::FormatConv;

    use crate::assert;
    use crate::coerce::coerce;
    use crate::values::Heap;
    use crate::values::Value;
    use crate::values::dict::Dict;
    use crate::values::string::dot_format::FormatArgs;
    use crate::values::string::dot_format::format_capture;
    use crate::values::string::dot_format::parse_format_one;

    fn format_capture_for_test<'v, T: Iterator<Item = Value<'v>>>(
        capture: &str,
        conv: FormatConv,
        args: &mut FormatArgs<'v, T>,
        kwargs: &Dict,
    ) -> anyhow::Result<String> {
        let mut result = String::new();
        format_capture(capture, conv, args, kwargs, &mut result)?;
        Ok(result)
    }

    #[test]
    fn test_format_capture() {
        Heap::temp(|heap| {
            let original_args = [heap.alloc("1"), heap.alloc("2"), heap.alloc("3")];
            let mut args = FormatArgs::new(original_args.iter().copied());
            let mut kwargs = SmallMap::new();

            kwargs.insert_hashed(heap.alloc_str("a").get_hashed(), heap.alloc("x"));
            kwargs.insert_hashed(heap.alloc_str("b").get_hashed(), heap.alloc("y"));
            kwargs.insert_hashed(heap.alloc_str("c").get_hashed(), heap.alloc("z"));
            let kwargs = Dict::new(coerce(kwargs));
            assert_eq!(
                format_capture_for_test("", FormatConv::Str, &mut args, &kwargs).unwrap(),
                "1"
            );
            assert_eq!(
                format_capture_for_test("", FormatConv::Str, &mut args, &kwargs).unwrap(),
                "2"
            );
            assert_eq!(
                format_capture_for_test("", FormatConv::Repr, &mut args, &kwargs).unwrap(),
                "\"3\""
            );
            assert_eq!(
                format_capture_for_test("a", FormatConv::Repr, &mut args, &kwargs).unwrap(),
                "\"x\""
            );
            assert_eq!(
                format_capture_for_test("a", FormatConv::Str, &mut args, &kwargs).unwrap(),
                "x"
            );
            assert!(format_capture_for_test("1", FormatConv::Str, &mut args, &kwargs).is_err());
            let mut args = FormatArgs::new(original_args.iter().copied());
            assert_eq!(
                format_capture_for_test("1", FormatConv::Str, &mut args, &kwargs).unwrap(),
                "2"
            );
            assert!(format_capture_for_test("", FormatConv::Str, &mut args, &kwargs).is_err());
        });
    }

    #[test]
    fn test_format() {
        assert::eq("'a{x}b{y}c{}'.format(1, x=2, y=3)", "'a2b3c1'");
        assert::eq("'a{x}b{{y}}c{}'.format(1, x=2)", "'a2b{y}c1'");
    }

    #[test]
    fn test_parse_format_one() {
        assert_eq!(
            Some(("abc".to_owned(), "def".to_owned())),
            parse_format_one("abc{}def")
        );
        // BLAZE PATCH 0045: `{!s}` is the field `!s`, not a conversion.
        assert_eq!(None, parse_format_one("abc{!s}def"));
        assert_eq!(None, parse_format_one("abc{!r}def"));
        assert_eq!(
            Some(("a{b".to_owned(), "c}d{".to_owned())),
            parse_format_one("a{{b{}c}}d{{")
        );
        assert_eq!(None, parse_format_one("a{"));
        assert_eq!(None, parse_format_one("a{}{}"));
        assert_eq!(None, parse_format_one("{x}"));
    }
}
