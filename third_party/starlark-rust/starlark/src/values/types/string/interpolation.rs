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

//! String interpolation-related code.
//! Based on <https://docs.python.org/3/library/stdtypes.html#printf-style-string-formatting>

use std::fmt::Write;

use dupe::Dupe;
use num_traits::FromPrimitive; // BLAZE PATCH 0008
use num_traits::Signed;
use num_traits::ToPrimitive; // BLAZE PATCH 0008
use thiserror::Error;

use crate::values::Heap;
use crate::values::string::repr::string_repr; // BLAZE PATCH 0051
use crate::values::StringValue;
#[allow(unused_imports)] // BLAZE PATCH 0008
use crate::values::UnpackValue;
use crate::values::Value;
#[allow(unused_imports)] // BLAZE PATCH 0008
use crate::values::ValueError;
use crate::values::ValueLike;
#[allow(unused_imports)] // BLAZE PATCH 0008
use crate::values::float::StarlarkFloat;
use crate::values::float::float;
use crate::values::string::dot_format::format_one;
#[allow(unused_imports)] // BLAZE PATCH 0008
use crate::values::types::int::int_or_big::StarlarkIntRef;
use crate::values::types::num::value::NumRef;
use crate::values::types::tuple::value::Tuple;

// `i32::abs(i32::MIN)` panics as `i32::MIN` has no corresponding
// positive value that fits inside `i32`. For this edge case,
// let's just hardcode the results.
#[allow(dead_code)] // BLAZE PATCH 0008
const I32_MIN_OCTAL: &str = "-20000000000";
#[allow(dead_code)] // BLAZE PATCH 0008
const I32_MIN_HEX: &str = "-80000000";

/// Operator `%` format or evaluation errors
#[derive(Clone, Dupe, Debug, Error)]
enum StringInterpolationError {
    #[error("Too many arguments for format string")]
    TooManyParameters,
    #[error("Not enough arguments for format string")]
    NotEnoughParameters,
    #[error("Incomplete format")]
    IncompleteFormat,
    #[error("Unsupported format character: {0:?}")]
    UnsupportedFormatCharacter(char),
    #[error("Expecting format character (internal error)")]
    ExpectingFormatCharacter,
}

enum PercentSFormat {
    /// `%s`.
    Str,
    /// `%r`.
    Repr,
    /// `%d`.
    Dec,
    /// `%o`.
    Oct,
    /// `%x`.
    Hex,
    /// `%X`.
    HexUpper,
    /// `%e`.
    Exp,
    /// `%E`.
    ExpUpper,
    /// `%f` or `%F`.
    Float,
    /// `%g`.
    FloatCompact,
    /// `%G`.
    FloatCompactUpper,
}

struct PercentFormatParser<'a> {
    rem: &'a str,
}

struct Item<'a> {
    literal: &'a str,
    format: Option<PercentSFormat>,
}

impl<'a> Iterator for PercentFormatParser<'a> {
    type Item = anyhow::Result<Item<'a>>;

    #[allow(clippy::collapsible_else_if)] // Makes code more readable.
    fn next(&mut self) -> Option<Self::Item> {
        let index_of_percent = self.rem.bytes().position(|c| c == b'%');
        if let Some(index_of_percent) = index_of_percent {
            let prev_rem = self.rem;
            let (literal, rem) = self.rem.split_at(index_of_percent);
            match rem.as_bytes().get(1) {
                None => return Some(Err(StringInterpolationError::IncompleteFormat.into())),
                Some(f) => {
                    let res = match f {
                        b'%' => {
                            // Include the percent in the literal.
                            let literal = &prev_rem[..index_of_percent + 1];
                            Item {
                                literal,
                                format: None,
                            }
                        }
                        b's' => Item {
                            literal,
                            format: Some(PercentSFormat::Str),
                        },
                        b'r' => Item {
                            literal,
                            format: Some(PercentSFormat::Repr),
                        },
                        b'd' => Item {
                            literal,
                            format: Some(PercentSFormat::Dec),
                        },
                        b'o' => Item {
                            literal,
                            format: Some(PercentSFormat::Oct),
                        },
                        b'x' => Item {
                            literal,
                            format: Some(PercentSFormat::Hex),
                        },
                        b'X' => Item {
                            literal,
                            format: Some(PercentSFormat::HexUpper),
                        },
                        b'e' => Item {
                            literal,
                            format: Some(PercentSFormat::Exp),
                        },
                        b'E' => Item {
                            literal,
                            format: Some(PercentSFormat::ExpUpper),
                        },
                        b'f' | b'F' => Item {
                            literal,
                            format: Some(PercentSFormat::Float),
                        },
                        b'g' => Item {
                            literal,
                            format: Some(PercentSFormat::FloatCompact),
                        },
                        b'G' => Item {
                            literal,
                            format: Some(PercentSFormat::FloatCompactUpper),
                        },
                        _ => {
                            // Note we need to find the second character, not the second byte.
                            let Some(c) = rem.chars().nth(1) else {
                                return Some(Err(
                                    StringInterpolationError::ExpectingFormatCharacter.into(),
                                ));
                            };
                            return Some(Err(
                                StringInterpolationError::UnsupportedFormatCharacter(c).into(),
                            ));
                        }
                    };
                    // We reach here only if format character is ASCII,
                    // so we can safely skip 2 bytes.
                    self.rem = &rem[2..];
                    Some(Ok(res))
                }
            }
        } else {
            if self.rem.is_empty() {
                None
            } else {
                let literal = self.rem;
                self.rem = "";
                Some(Ok(Item {
                    literal,
                    format: None,
                }))
            }
        }
    }
}


// BLAZE PATCH 0008: Java Printer.formatWithList's numeric conversions.
fn want_int_or_float(value: Value, conv: char) -> crate::Error { // BLAZE PATCH 0008
    crate::Error::new_other(anyhow::anyhow!( // BLAZE PATCH 0008
        "got {} for '%{}' format, want int or float", // BLAZE PATCH 0008
        value.get_type(), // BLAZE PATCH 0008
        conv // BLAZE PATCH 0008
    )) // BLAZE PATCH 0008
} // BLAZE PATCH 0008

/// `%d`, `%o`, `%x`, `%X`: Java formats an int32 or int64 with java.util.Formatter, so negative
/// values print in two's complement for `%o`/`%x`/`%X`; larger ints print sign and magnitude.
fn java_int_conv(res: &mut String, value: Value, conv: char) -> crate::Result<()> { // BLAZE PATCH 0008
    let i: num_bigint::BigInt = match value.unpack_num() { // BLAZE PATCH 0008
        Some(NumRef::Int(i)) => i.to_big(), // BLAZE PATCH 0008
        Some(NumRef::Float(f)) => { // BLAZE PATCH 0008
            if !f.0.is_finite() { // BLAZE PATCH 0008
                return Err(crate::Error::new_other(anyhow::anyhow!( // BLAZE PATCH 0008
                    "got {}, want a finite number", // BLAZE PATCH 0008
                    value.to_repr() // BLAZE PATCH 0008
                ))); // BLAZE PATCH 0008
            } // BLAZE PATCH 0008
            num_bigint::BigInt::from_f64(f.0.trunc()).unwrap() // BLAZE PATCH 0008
        } // BLAZE PATCH 0008
        None => return Err(want_int_or_float(value, conv)), // BLAZE PATCH 0008
    }; // BLAZE PATCH 0008
    let radix = |u: &dyn Fn() -> String, x: &dyn Fn() -> String, ux: &dyn Fn() -> String| match conv { // BLAZE PATCH 0008
        'o' => u(), // BLAZE PATCH 0008
        'x' => x(), // BLAZE PATCH 0008
        _ => ux(), // BLAZE PATCH 0008
    }; // BLAZE PATCH 0008
    if conv == 'd' { // BLAZE PATCH 0008
        write!(res, "{i}").unwrap(); // BLAZE PATCH 0008
    } else if let Some(v) = i.to_i32() { // BLAZE PATCH 0008
        let v = v as u32; // BLAZE PATCH 0008
        res.push_str(&radix(&|| format!("{v:o}"), &|| format!("{v:x}"), &|| format!("{v:X}"))); // BLAZE PATCH 0008
    } else if let Some(v) = i.to_i64() { // BLAZE PATCH 0008
        let v = v as u64; // BLAZE PATCH 0008
        res.push_str(&radix(&|| format!("{v:o}"), &|| format!("{v:x}"), &|| format!("{v:X}"))); // BLAZE PATCH 0008
    } else { // BLAZE PATCH 0008
        if i.is_negative() { // BLAZE PATCH 0008
            res.push('-'); // BLAZE PATCH 0008
        } // BLAZE PATCH 0008
        let m = i.magnitude(); // BLAZE PATCH 0008
        res.push_str(&radix(&|| format!("{m:o}"), &|| format!("{m:x}"), &|| format!("{m:X}"))); // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    Ok(()) // BLAZE PATCH 0008
} // BLAZE PATCH 0008

/// `%e`, `%f`, `%g` and upper case: a float, or an int converted with
/// StarlarkInt.toFiniteDouble.
fn java_float_arg(value: Value, conv: char) -> crate::Result<f64> { // BLAZE PATCH 0008
    match value.unpack_num() { // BLAZE PATCH 0008
        Some(n) => Ok(n.as_finite_float()?), // BLAZE PATCH 0008
        None => Err(want_int_or_float(value, conv)), // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
} // BLAZE PATCH 0008

/// BLAZE PATCH 0051: Java's `Printer.formatWithList` (its algorithm, order of checks and
/// texts) for `pattern % value`.
// Java: net/starlark/java/eval/Printer.java:312 Printer.formatWithList
pub(crate) fn percent(format: &str, value: Value) -> crate::Result<String> {
    let tuple = Tuple::from_value(value);
    let one = &[value];
    let values = match &tuple {
        Some(xs) => xs.content(),
        None => one,
    };
    let err = |m: String| crate::Error::new_other(anyhow::anyhow!("{m}"));
    let pattern_repr = || {
        let mut r = String::new();
        string_repr(format, &mut r);
        r
    };
    let chars: Vec<char> = format.chars().collect();
    let length = chars.len();
    let mut res: String = String::with_capacity(format.len() + 20);
    let mut a = 0;
    let mut i = 0;
    while i < length {
        let Some(p) = chars[i..].iter().position(|c| *c == '%').map(|p| p + i) else {
            res.extend(&chars[i..]);
            break;
        };
        res.extend(&chars[i..p]);
        if p == length - 1 {
            return Err(err(format!(
                "incomplete format pattern ends with %: {}",
                pattern_repr()
            )));
        }
        let conv = chars[p + 1];
        i = p + 2;
        if conv == '%' {
            res.push('%');
            continue;
        }
        if a >= values.len() {
            let args = match &tuple {
                Some(_) => value.to_repr(),
                // Java: Tuple.copyOf(arguments) of the single argument.
                None => format!("({},)", value.to_repr()),
            };
            return Err(err(format!(
                "not enough arguments for format pattern {}: {}",
                pattern_repr(),
                args
            )));
        }
        let arg = values[a];
        a += 1;
        match conv {
            'd' | 'o' | 'x' | 'X' => java_int_conv(&mut res, arg, conv)?,
            'e' | 'E' | 'f' | 'F' | 'g' | 'G' => {
                let v = java_float_arg(arg, conv)?;
                match conv {
                    'e' => float::write_scientific(&mut res, v, 'e', false).unwrap(),
                    'E' => float::write_scientific(&mut res, v, 'E', false).unwrap(),
                    // Java StarlarkFloat.format: 'F' is an alias of 'f'.
                    'f' | 'F' => float::write_decimal(&mut res, v).unwrap(),
                    'g' => float::write_compact(&mut res, v, 'e').unwrap(),
                    _ => float::write_compact(&mut res, v, 'E').unwrap(),
                }
            }
            'r' => arg.collect_repr(&mut res),
            's' => match arg.unpack_str() {
                None => arg.collect_str(&mut res),
                Some(s) => res.push_str(s),
            },
            _ => {
                return Err(err(format!(
                    "unsupported format character \"{conv}\" at index {} in {}",
                    p + 1,
                    pattern_repr()
                )));
            }
        }
    }
    if a < values.len() {
        return Err(err(
            "not all arguments converted during string formatting".to_owned()
        ));
    }
    Ok(res)
}

#[allow(dead_code)] // BLAZE PATCH 0051: starlark-rust's `%`, replaced by `percent`.
fn percent_upstream(format: &str, value: Value) -> crate::Result<String> {
    // NOTE(nga): use could reuse `Evaluator::string_pool` here, but
    //   * we don't have access to `Evaluator` in `StarlarkValue::percent`
    //   * after single %s made intrinsic, this code is not that hot now

    // random guess as a baseline capacity
    let mut res: String = String::with_capacity(format.len() + 20);

    let tuple = Tuple::from_value(value);
    let one = &[value];
    let values = match &tuple {
        Some(xs) => xs.content(),
        None => one,
    };
    let mut values = values.iter().copied();
    let mut next_value = || -> anyhow::Result<Value> {
        values
            .next()
            .ok_or_else(|| StringInterpolationError::NotEnoughParameters.into())
    };

    // because of the way format is defined, we can deal with it as bytes
    for item in (PercentFormatParser { rem: format }) {
        let item = item?;
        res.push_str(item.literal);
        match item.format {
            None => {}
            Some(PercentSFormat::Str) => {
                let arg = next_value()?;
                match arg.unpack_str() {
                    None => arg.collect_str(&mut res), // BLAZE PATCH 0011
                    Some(s) => res.push_str(s),
                }
            }
            Some(PercentSFormat::Repr) => next_value()?.collect_repr(&mut res),
            // Java Printer.formatWithList (Printer.java:353-400): %d %o %x %X take an int or a
            // finite float (truncated); %e %f %g (and upper case) take a float or an int that
            // converts to a finite float.
            Some(PercentSFormat::Dec) => java_int_conv(&mut res, next_value()?, 'd')?, // BLAZE PATCH 0008
            Some(PercentSFormat::Oct) => java_int_conv(&mut res, next_value()?, 'o')?, // BLAZE PATCH 0008
            Some(PercentSFormat::Hex) => java_int_conv(&mut res, next_value()?, 'x')?, // BLAZE PATCH 0008
            Some(PercentSFormat::HexUpper) => java_int_conv(&mut res, next_value()?, 'X')?, // BLAZE PATCH 0008
            Some(PercentSFormat::Exp) => { // BLAZE PATCH 0008
                let v = java_float_arg(next_value()?, 'e')?; // BLAZE PATCH 0008
                float::write_scientific(&mut res, v, 'e', false).unwrap() // BLAZE PATCH 0008
            } // BLAZE PATCH 0008
            Some(PercentSFormat::ExpUpper) => { // BLAZE PATCH 0008
                let v = java_float_arg(next_value()?, 'E')?; // BLAZE PATCH 0008
                float::write_scientific(&mut res, v, 'E', false).unwrap() // BLAZE PATCH 0008
            } // BLAZE PATCH 0008
            Some(PercentSFormat::Float) => { // BLAZE PATCH 0008
                let v = java_float_arg(next_value()?, 'f')?; // BLAZE PATCH 0008
                float::write_decimal(&mut res, v).unwrap() // BLAZE PATCH 0008
            } // BLAZE PATCH 0008
            Some(PercentSFormat::FloatCompact) => { // BLAZE PATCH 0008
                let v = java_float_arg(next_value()?, 'g')?; // BLAZE PATCH 0008
                float::write_compact(&mut res, v, 'e').unwrap() // BLAZE PATCH 0008
            } // BLAZE PATCH 0008
            Some(PercentSFormat::FloatCompactUpper) => { // BLAZE PATCH 0008
                let v = java_float_arg(next_value()?, 'G')?; // BLAZE PATCH 0008
                float::write_compact(&mut res, v, 'E').unwrap() // BLAZE PATCH 0008
            } // BLAZE PATCH 0008
        }
    }
    if values.next().is_some() {
        Err(crate::Error::new_other(
            StringInterpolationError::TooManyParameters,
        ))
    } else {
        Ok(res)
    }
}

/// Try parse `"aaa%sbbb"` and return `("aaa", "bbb")`.
pub(crate) fn parse_percent_s_one(format: &str) -> Option<(String, String)> {
    let mut before = String::with_capacity(format.len());
    let mut chars = format.chars();
    loop {
        match chars.next()? {
            '%' => match chars.next()? {
                '%' => before.push('%'),
                's' => break,
                _ => return None,
            },
            c => before.push(c),
        }
    }
    let mut after = String::with_capacity(format.len() - before.len());
    loop {
        match chars.next() {
            Some('%') => match chars.next()? {
                '%' => after.push('%'),
                _ => return None,
            },
            Some(c) => after.push(c),
            None => break,
        }
    }
    Some((before, after))
}

/// Evaluate `"<before>%s<after>" % arg`.
pub(crate) fn percent_s_one<'v>(
    before: &str,
    arg: Value<'v>,
    after: &str,
    heap: Heap<'v>,
) -> crate::Result<StringValue<'v>> {
    Ok(match StringValue::new(arg) {
        Some(arg) => heap.alloc_str_concat3(before, &arg, after),
        None => {
            let one = match Tuple::from_value(arg) {
                Some(tuple) => match tuple.content() {
                    // BLAZE PATCH 0051: Java's texts (Printer.formatWithList).
                    [] | [_, _, ..] => { // BLAZE PATCH 0051
                        let pattern = format!( // BLAZE PATCH 0051
                            "{}%s{}", // BLAZE PATCH 0051
                            before.replace('%', "%%"), // BLAZE PATCH 0051
                            after.replace('%', "%%") // BLAZE PATCH 0051
                        ); // BLAZE PATCH 0051
                        return Err(percent(&pattern, arg).unwrap_err()); // BLAZE PATCH 0051
                    }
                    [value] => *value,
                },
                None => arg,
            };
            format_one(before, one, after, heap)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert;

    #[test]
    fn test_incomplete_format() {
        assert::fail("'%' % ()", "Incomplete format");
    }

    #[test]
    fn test_unsupported_format_character() {
        assert::fail("'xx%qxx' % (1,)", "Unsupported format character: 'q'");
    }

    #[test]
    fn test_parse_percent_s_one() {
        assert_eq!(
            Some(("abc".to_owned(), "def".to_owned())),
            parse_percent_s_one("abc%sdef")
        );
        assert_eq!(
            Some(("a%b".to_owned(), "c%d%".to_owned())),
            parse_percent_s_one("a%%b%sc%%d%%")
        );
        assert_eq!(None, parse_percent_s_one("a%"));
        assert_eq!(None, parse_percent_s_one("a%s%"));
        assert_eq!(None, parse_percent_s_one("a%s%s"));
        assert_eq!(None, parse_percent_s_one("%d"));
    }

    #[test]
    fn test_type_support_d() {
        assert::eq("'%d' % (-123,)", "'-123'");
        assert::eq("'%d' % (-12345678901234567890,)", "'-12345678901234567890'");
        assert::eq("'%d' % (-123.0,)", "'-123'");

        assert::fail(
            "'%d' % (True,)",
            "Operation `format(%d)` not supported on type `bool`",
        );
        assert::fail(
            "'%d' % ('abc',)",
            "Operation `format(%d)` not supported on type `string`",
        );
        assert::fail(
            "'%d' % ([],)",
            "Operation `format(%d)` not supported on type `list`",
        );
    }

    #[test]
    fn test_type_support_o() {
        assert::eq("'%o' % (-123,)", "'-173'");
        assert::eq(
            "'%o' % (-12345678901234567890,)",
            "'-1255245230635307605322'",
        );

        assert::fail(
            "'%o' % (-123.0,)",
            "Operation `format(%o)` not supported on type `float`",
        );
        assert::fail(
            "'%o' % (True,)",
            "Operation `format(%o)` not supported on type `bool`",
        );
        assert::fail(
            "'%o' % ('abc',)",
            "Operation `format(%o)` not supported on type `string`",
        );
        assert::fail(
            "'%o' % ([],)",
            "Operation `format(%o)` not supported on type `list`",
        );
    }

    #[test]
    fn test_type_support_x() {
        assert::eq("'%x' % (-123,)", "'-7b'");
        assert::eq("'%x' % (-12345678901234567890,)", "'-ab54a98ceb1f0ad2'");

        assert::fail(
            "'%x' % (-123.0,)",
            "Operation `format(%x)` not supported on type `float`",
        );
        assert::fail(
            "'%x' % (True,)",
            "Operation `format(%x)` not supported on type `bool`",
        );
        assert::fail(
            "'%x' % ('abc',)",
            "Operation `format(%x)` not supported on type `string`",
        );
        assert::fail(
            "'%x' % ([],)",
            "Operation `format(%x)` not supported on type `list`",
        );
    }

    #[test]
    fn test_type_support_e() {
        assert::eq("'%e' % (-123,)", "'-1.230000e+02'");
        assert::eq("'%e' % (-12345678901234567890,)", "'-1.234568e+19'");
        assert::eq("'%e' % (-123.0,)", "'-1.230000e+02'");

        assert::fail(
            "'%e' % (True,)",
            "Type of parameters mismatch, expected `float | int`, actual `bool (repr: True)`",
        );
        assert::fail(
            "'%e' % ('abc',)",
            "Type of parameters mismatch, expected `float | int`, actual `string (repr:",
        );
        assert::fail(
            "'%e' % ([],)",
            "Type of parameters mismatch, expected `float | int`, actual `list (repr",
        );
    }

    #[test]
    fn test_int_min() {
        // 2147483647 is `i32::MIN + 1`, it has corresponding positive value in i32.
        // 2147483648 is `i32::MIN`, it has no corresponding positive value in i32.
        assert::eq("'%o' % (-2147483647,)", "'-17777777777'");
        assert::eq("'%o' % (-2147483648,)", "'-20000000000'");

        assert::eq("'%x' % (-2147483647,)", "'-7fffffff'");
        assert::eq("'%x' % (-2147483648,)", "'-80000000'");
    }
}
