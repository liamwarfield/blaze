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

//! Methods for the `string` type.

use std::cmp;

use starlark_derive::starlark_module;
use starlark_syntax::fast_string;
use starlark_syntax::fast_string::StrIndices;
use starlark_syntax::fast_string::convert_str_indices;

use crate as starlark;
use crate::environment::MethodsBuilder;
use crate::eval::Arguments;
use crate::eval::Evaluator;
use crate::values::Heap;
use crate::values::StringValue;
use crate::values::UnpackValue;
use crate::values::Value;
use crate::values::ValueOfUnchecked;
use crate::values::list::AllocList;
use crate::values::list::UnpackList;
use crate::values::none::NoneOr;
use crate::values::string::dot_format;
use crate::values::tuple::UnpackTuple;
use crate::values::type_repr::StarlarkTypeRepr;
use crate::values::typing::iter::StarlarkIter;

// This does not exists in rust, split would cut the string incorrectly and
// split_whitespace cannot take a n parameter.
#[allow(dead_code)] // BLAZE PATCH 0005: Java has no whitespace split
fn splitn_whitespace(s: &str, maxsplit: usize) -> Vec<String> {
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut split = 1;
    let mut eat_ws = true;
    for c in s.chars() {
        if split >= maxsplit && !eat_ws {
            cur.push(c)
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                v.push(cur);
                cur = String::new();
                split += 1;
                eat_ws = true;
            }
        } else {
            eat_ws = false;
            cur.push(c)
        }
    }
    if !cur.is_empty() {
        v.push(cur)
    }
    v
}

#[allow(dead_code)] // BLAZE PATCH 0005: Java has no whitespace split
fn rsplitn_whitespace(s: &str, maxsplit: usize) -> Vec<String> {
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut split = 1;
    let mut eat_ws = true;
    for c in s.chars().rev() {
        if split >= maxsplit && !eat_ws {
            cur.push(c)
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                v.push(cur.chars().rev().collect());
                cur = String::new();
                split += 1;
                eat_ws = true;
            }
        } else {
            eat_ws = false;
            cur.push(c)
        }
    }
    if !cur.is_empty() {
        v.push(cur.chars().rev().collect());
    }
    v.reverse();
    v
}

#[derive(StarlarkTypeRepr, UnpackValue)]
enum StringOrTuple<'v> {
    String(&'v str),
    Tuple(UnpackTuple<&'v str>),
}

/// BLAZE PATCH 0052: the `sub` parameter of `startswith`/`endswith` checked as Java does: a
/// string or tuple (BuiltinFunction.checkParamValue), each tuple element a string
/// (Sequence.cast, Sequence.java:141).
fn java_string_or_tuple<'v>(function: &str, sub: Value<'v>) -> anyhow::Result<StringOrTuple<'v>> {
    if let Some(s) = sub.unpack_str() {
        return Ok(StringOrTuple::String(s));
    }
    if let Some(t) = crate::values::tuple::TupleRef::from_value(sub) {
        let mut items = Vec::new();
        for (i, x) in t.content().iter().enumerate() {
            match x.unpack_str() {
                Some(s) => items.push(s),
                None => {
                    return Err(anyhow::anyhow!(
                        "at index {i} of sub, got element of type {}, want string",
                        x.get_type()
                    ));
                }
            }
        }
        return Ok(StringOrTuple::Tuple(UnpackTuple { items }));
    }
    Err(anyhow::anyhow!(
        "in call to {function}(), parameter 'sub' got value of type '{}', want 'string or tuple'",
        sub.get_type()
    ))
}

// BLAZE PATCH 0033: java.lang.Character, as net/starlark/java/eval/StringModule.java applies it to
// each UTF-16 char of a string. Blaze's strings are "internal" (every char <= U+00FF, one per
// byte), for which these are exact (JDK CharacterDataLatin1; checked against the JDK of Bazel
// 9.3.2's install base). Chars above U+00FF never occur in Blaze; for them the nearest Rust
// property stands in (U+0100..=U+FFFF), and a char above U+FFFF is a surrogate pair in Java,
// which is neither a letter nor cased and maps to itself.

/// BLAZE PATCH 0033: Java `Character.isLetter(char)`.
fn java_is_letter(c: char) -> bool { // BLAZE PATCH 0033
    match c as u32 { // BLAZE PATCH 0033
        0..=0xff => matches!( // BLAZE PATCH 0033
            c as u32, // BLAZE PATCH 0033
            0x41..=0x5a | 0x61..=0x7a | 0xaa | 0xb5 | 0xba | 0xc0..=0xd6 | 0xd8..=0xf6 | 0xf8..=0xff // BLAZE PATCH 0033
        ), // BLAZE PATCH 0033
        0x100..=0xffff => c.is_alphabetic(), // BLAZE PATCH 0033
        _ => false, // BLAZE PATCH 0033
    } // BLAZE PATCH 0033
} // BLAZE PATCH 0033

/// BLAZE PATCH 0033: Java `Character.isUpperCase(char)`.
fn java_is_upper_case(c: char) -> bool { // BLAZE PATCH 0033
    match c as u32 { // BLAZE PATCH 0033
        0..=0xff => matches!(c as u32, 0x41..=0x5a | 0xc0..=0xd6 | 0xd8..=0xde), // BLAZE PATCH 0033
        0x100..=0xffff => c.is_uppercase(), // BLAZE PATCH 0033
        _ => false, // BLAZE PATCH 0033
    } // BLAZE PATCH 0033
} // BLAZE PATCH 0033

/// BLAZE PATCH 0033: Java `Character.isLowerCase(char)` (`ª`, `µ`, `º` and `ß` are lower case).
fn java_is_lower_case(c: char) -> bool { // BLAZE PATCH 0033
    match c as u32 { // BLAZE PATCH 0033
        0..=0xff => matches!( // BLAZE PATCH 0033
            c as u32, // BLAZE PATCH 0033
            0x61..=0x7a | 0xaa | 0xb5 | 0xba | 0xdf..=0xf6 | 0xf8..=0xff // BLAZE PATCH 0033
        ), // BLAZE PATCH 0033
        0x100..=0xffff => c.is_lowercase(), // BLAZE PATCH 0033
        _ => false, // BLAZE PATCH 0033
    } // BLAZE PATCH 0033
} // BLAZE PATCH 0033

/// BLAZE PATCH 0033: Java `Character.toUpperCase(char)`: one char, so `ß` stays `ß`; `µ` becomes
/// U+039C and `ÿ` U+0178, leaving Latin-1 exactly as in Java.
fn java_to_upper_case(c: char) -> char { // BLAZE PATCH 0033
    match c as u32 { // BLAZE PATCH 0033
        0x61..=0x7a | 0xe0..=0xf6 | 0xf8..=0xfe => char::from_u32(c as u32 - 0x20).unwrap(), // BLAZE PATCH 0033
        0xb5 => '\u{39c}', // BLAZE PATCH 0033
        0xff => '\u{178}', // BLAZE PATCH 0033
        0..=0xff => c, // BLAZE PATCH 0033
        0x100..=0xffff => single_char(c.to_uppercase()).unwrap_or(c), // BLAZE PATCH 0033
        _ => c, // BLAZE PATCH 0033
    } // BLAZE PATCH 0033
} // BLAZE PATCH 0033

/// BLAZE PATCH 0033: Java `Character.toLowerCase(char)`.
fn java_to_lower_case(c: char) -> char { // BLAZE PATCH 0033
    match c as u32 { // BLAZE PATCH 0033
        0x41..=0x5a | 0xc0..=0xd6 | 0xd8..=0xde => char::from_u32(c as u32 + 0x20).unwrap(), // BLAZE PATCH 0033
        0..=0xff => c, // BLAZE PATCH 0033
        0x100..=0xffff => single_char(c.to_lowercase()).unwrap_or(c), // BLAZE PATCH 0033
        _ => c, // BLAZE PATCH 0033
    } // BLAZE PATCH 0033
} // BLAZE PATCH 0033

/// BLAZE PATCH 0033: the mapping when it is a single char (Java's simple case mapping).
fn single_char(mut it: impl Iterator<Item = char>) -> Option<char> { // BLAZE PATCH 0033
    let c = it.next()?; // BLAZE PATCH 0033
    if it.next().is_some() { None } else { Some(c) } // BLAZE PATCH 0033
} // BLAZE PATCH 0033

/// BLAZE PATCH 0033: Guava `CharMatcher.javaDigit()` (`Character.isDigit`; only `0`-`9` in Latin-1).
fn java_is_digit(c: char) -> bool { // BLAZE PATCH 0033
    match c as u32 { // BLAZE PATCH 0033
        0..=0xff => c.is_ascii_digit(), // BLAZE PATCH 0033
        0x100..=0xffff => c.is_numeric(), // BLAZE PATCH 0033
        _ => false, // BLAZE PATCH 0033
    } // BLAZE PATCH 0033
} // BLAZE PATCH 0033

/// BLAZE PATCH 0033: Guava `CharMatcher.whitespace()` (StringModule.java:878 `SPACE`): Unicode
/// White_Space; in Latin-1 TAB..CR, space, U+0085 and U+00A0, not U+001C..U+001F.
fn guava_is_whitespace(c: char) -> bool { // BLAZE PATCH 0033
    matches!( // BLAZE PATCH 0033
        c as u32, // BLAZE PATCH 0033
        0x09..=0x0d // BLAZE PATCH 0033
            | 0x20 // BLAZE PATCH 0033
            | 0x85 // BLAZE PATCH 0033
            | 0xa0 // BLAZE PATCH 0033
            | 0x1680 // BLAZE PATCH 0033
            | 0x2000..=0x200a // BLAZE PATCH 0033
            | 0x2028 // BLAZE PATCH 0033
            | 0x2029 // BLAZE PATCH 0033
            | 0x202f // BLAZE PATCH 0033
            | 0x205f // BLAZE PATCH 0033
            | 0x3000 // BLAZE PATCH 0033
    ) // BLAZE PATCH 0033
} // BLAZE PATCH 0033

/// BLAZE PATCH 0033: StringModule.java:854 `matches(str, matcher, requiresAtLeastOneCasedLetter)`
/// (`CASED` is `[a-zA-Z]`).
fn java_matches(s: &str, matcher: impl Fn(char) -> bool, requires_cased: bool) -> bool { // BLAZE PATCH 0033
    if s.is_empty() { // BLAZE PATCH 0033
        return false; // BLAZE PATCH 0033
    } // BLAZE PATCH 0033
    let mut cased = false; // BLAZE PATCH 0033
    for c in s.chars() { // BLAZE PATCH 0033
        if !matcher(c) { // BLAZE PATCH 0033
            return false; // BLAZE PATCH 0033
        } // BLAZE PATCH 0033
        cased |= c.is_ascii_alphabetic(); // BLAZE PATCH 0033
    } // BLAZE PATCH 0033
    !requires_cased || cased // BLAZE PATCH 0033
} // BLAZE PATCH 0033

/// BLAZE PATCH 0034: StringModule.java:196 `LATIN1_WHITESPACE` (the flag off) and :206
/// `ASCII_WHITESPACE` (the flag on): what `strip`/`lstrip`/`rstrip` remove without `chars`.
fn java_strip_whitespace(c: char, utf8_byte_strings: bool) -> bool { // BLAZE PATCH 0034
    match c as u32 { // BLAZE PATCH 0034
        0x09..=0x0d | 0x1c..=0x1f | 0x20 => true, // BLAZE PATCH 0034
        0x85 | 0xa0 => !utf8_byte_strings, // BLAZE PATCH 0034
        _ => false, // BLAZE PATCH 0034
    } // BLAZE PATCH 0034
} // BLAZE PATCH 0034

/// BLAZE PATCH 0034: StringModule.java:326 `matcher(charsOrNone, semantics)`.
fn java_strip_matcher<'a>( // BLAZE PATCH 0034
    chars: Option<&'a str>, // BLAZE PATCH 0034
    eval: &Evaluator, // BLAZE PATCH 0034
) -> impl Fn(char) -> bool + 'a { // BLAZE PATCH 0034
    let utf8_byte_strings = eval.utf8_byte_strings(); // BLAZE PATCH 0034
    move |c| match chars { // BLAZE PATCH 0034
        Some(s) => s.contains(c), // BLAZE PATCH 0034
        None => java_strip_whitespace(c, utf8_byte_strings), // BLAZE PATCH 0034
    } // BLAZE PATCH 0034
} // BLAZE PATCH 0034

#[starlark_module]
pub(crate) fn string_methods(builder: &mut MethodsBuilder) {
    /// [string.elems](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·elems
    /// ): returns an iterable of the bytes values of a string.
    ///
    /// `S.elems()` returns an iterable value containing the
    /// sequence of numeric bytes values in the string S.
    ///
    /// To materialize the entire sequence of bytes, apply `list(...)` to the
    /// result.
    ///
    /// ```
    /// # starlark::assert::is_true(r#"
    /// list("Hello, 世界".elems()) == ["H", "e", "l", "l", "o", ",", " ", "世", "界"]
    /// # "#);
    /// ```
    // BLAZE PATCH 0005: an immutable list of 1-element strings, not a lazy iterator (Java
    // net/starlark/java/eval/StringModule.java:932 `elems`: `StarlarkList.wrap(null, strings)`),
    // so `s.elems()[:]`, `len(s.elems())` and indexing work as in Bazel.
    fn elems<'v>(this: StringValue<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let chars: Vec<Value<'v>> = this // BLAZE PATCH 0005
            .as_str()
            .chars()
            .map(|c| heap.alloc_char(c).to_value())
            .collect();
        Ok(crate::values::list::alloc_immutable_list(heap, &chars)) // BLAZE PATCH 0005
    }

    /// [string.capitalize](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string%C2%B7capitalize
    /// ): returns a copy of string S, where the first character (if any) is converted to uppercase;
    /// all other characters are converted to lowercase.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "hello, world!".capitalize() == "Hello, world!"
    /// "Hello, World!".capitalize() == "Hello, world!"
    /// "".capitalize() == ""
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:526 capitalize:
    // `Character.toUpperCase(self.charAt(0)) + Ascii.toLowerCase(self.substring(1))`.
    #[starlark(speculative_exec_safe)]
    fn capitalize(this: &str) -> anyhow::Result<String> {
        let mut chars = this.chars(); // BLAZE PATCH 0033
        let mut result = String::with_capacity(this.len()); // BLAZE PATCH 0033
        if let Some(first) = chars.next() { // BLAZE PATCH 0033
            result.push(java_to_upper_case(first)); // BLAZE PATCH 0033
            result.push_str(&chars.as_str().to_ascii_lowercase()); // BLAZE PATCH 0033
        } // BLAZE PATCH 0033
        Ok(result)
    }

    // BLAZE PATCH 0043: no `codepoints` method (Java StringModule has none; `"abc".codepoints()`
    // fails "'string' value has no field or method 'codepoints'").

    /// [string.count](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·count
    /// ): count the number of occurrences of a string in another string.
    ///
    /// `S.count(sub[, start[, end]])` returns the number of occurrences of
    /// `sub` within the string S, or, if the optional substring indices
    /// `start` and `end` are provided, within the designated substring of S.
    /// They are interpreted according to Skylark's [indexing conventions](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#indexing).
    ///
    /// This implementation does not count occurrence of `sub` in the string `S`
    /// that overlap other occurrence of S (which can happen if some suffix of S
    /// is a prefix of S). For instance, `"abababa".count("aba")` returns 2
    /// for `[aba]a[aba]`, not counting the middle occurrence: `ab[aba]ba`
    /// (this is following Python behavior).
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "hello, world!".count("o") == 2
    /// "abababa".count("aba") == 2
    /// "hello, world!".count("o", 7, 12) == 1  # in "world"
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn count(
        this: &str,
        #[starlark(require = pos)] sub: &str, // BLAZE PATCH 0052
        #[starlark(require = pos, default = NoneOr::None)] start: NoneOr<i32>,
        #[starlark(require = pos, default = NoneOr::None)] end: NoneOr<i32>,
    ) -> anyhow::Result<i32> {
        let needle = sub; // BLAZE PATCH 0052: Java parameter name `sub`
        if let Some(StrIndices { haystack, .. }) =
            convert_str_indices(this, start.into_option(), end.into_option())
        {
            Ok(fast_string::count_matches(haystack, needle) as i32)
        } else {
            Ok(0)
        }
    }

    /// [string.endswith](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·endswith
    /// ): determine if a string ends with a given suffix.
    ///
    /// `S.endswith(suffix[, start[, end]])` reports whether the string
    /// `S[start:end]` has the specified suffix.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "filename.sky".endswith(".sky") == True
    /// "hello".endswith("ell", 0, 4) == True
    /// "hello".endswith("lo", 3) == True
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn endswith<'v>( // BLAZE PATCH 0052
        this: &str,
        #[starlark(require = pos)] sub: Value<'v>, // BLAZE PATCH 0052
        #[starlark(require = pos, default = NoneOr::None)] start: NoneOr<i32>,
        #[starlark(require = pos, default = NoneOr::None)] end: NoneOr<i32>,
    ) -> anyhow::Result<bool> {
        let suffix = java_string_or_tuple("endswith", sub)?; // BLAZE PATCH 0052: Java parameter name `sub`
        let haystack = match convert_str_indices(this, start.into_option(), end.into_option()) {
            Some(StrIndices { haystack, .. }) => haystack,
            None => return Ok(false),
        };
        match suffix {
            StringOrTuple::String(x) => Ok(haystack.ends_with(x)),
            StringOrTuple::Tuple(xs) => Ok(xs.items.iter().any(|x| haystack.ends_with(x))),
        }
    }

    /// [string.find](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·find
    /// ): find a substring in a string.
    ///
    /// `S.find(sub[, start[, end]])` returns the index of the first
    /// occurrence of the substring `sub` within S.
    ///
    /// If either or both of `start` or `end` are specified,
    /// they specify a subrange of S to which the search should be restricted.
    /// They are interpreted according to Skylark's [indexing
    /// conventions](https://github.com/bazelbuild/starlark/blob/master/spec.md#indexing).
    ///
    /// If no occurrence is found, `found` returns -1.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "bonbon".find("on") == 1
    /// "bonbon".find("on", 2) == 4
    /// "bonbon".find("on", 2, 5) == -1
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn find(
        this: &str,
        #[starlark(require = pos)] sub: &str, // BLAZE PATCH 0052
        #[starlark(require = pos, default = NoneOr::None)] start: NoneOr<i32>,
        #[starlark(require = pos, default = NoneOr::None)] end: NoneOr<i32>,
    ) -> anyhow::Result<i32> {
        let needle = sub; // BLAZE PATCH 0052: Java parameter name `sub`
        if let Some(StrIndices { start, haystack }) =
            convert_str_indices(this, start.into_option(), end.into_option())
        {
            if let Some(index) = haystack.find(needle) {
                let index = fast_string::len(&haystack[..index]);
                return Ok((start + index).0 as i32);
            }
        }
        Ok(-1)
    }

    /// [string.format](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·format
    /// ): format a string.
    ///
    /// `S.format(*args, **kwargs)` returns a version of the format string S
    /// in which bracketed portions `{...}` are replaced
    /// by arguments from `args` and `kwargs`.
    ///
    /// Within the format string, a pair of braces `{{` or `}}` is treated as
    /// a literal open or close brace.
    /// Each unpaired open brace must be matched by a close brace `}`.
    /// The optional text between corresponding open and close braces
    /// specifies which argument to use and how to format it, and consists of
    /// three components, all optional:
    /// a field name, a conversion preceded by '`!`', and a format specifier
    /// preceded by '`:`'.
    ///
    /// ```text
    /// {field}
    /// {field:spec}
    /// {field!conv}
    /// {field!conv:spec}
    /// ```
    ///
    /// The *field name* may be either a decimal number or a keyword.
    /// A number is interpreted as the index of a positional argument;
    /// a keyword specifies the value of a keyword argument.
    /// If all the numeric field names form the sequence 0, 1, 2, and so on,
    /// they may be omitted and those values will be implied; however,
    /// the explicit and implicit forms may not be mixed.
    ///
    /// The *conversion* specifies how to convert an argument value `x` to a
    /// string. It may be either `!r`, which converts the value using
    /// `repr(x)`, or `!s`, which converts the value using `str(x)` and is
    /// the default. BLAZE PATCH 0045: not in Bazel, whose `format` has no conversions: the
    /// whole field (`0!r`) is the argument name (net/starlark/java/eval/FormatParser.java).
    ///
    /// The *format specifier*, after a colon, specifies field width,
    /// alignment, padding, and numeric precision.
    /// Currently it must be empty, but it is reserved for future use.
    ///
    /// ```rust
    /// # starlark::assert::all_true(r#"
    /// "a {} c".format(3) == "a 3 c"
    /// "a{x}b{y}c{}".format(1, x=2, y=3) == "a2b3c1"
    /// "a{}b{}c".format(1, 2) == "a1b2c"
    /// "({1}, {0})".format("zero", "one") == "(one, zero)"
    /// "{0!r}".format(**{"0!r": 1}) == "1"
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn format<'v>(
        this: &str,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StringValue<'v>> {
        let iter = args.positions(eval.heap())?;
        dot_format::format(
            this,
            iter,
            args.names()?,
            &mut eval.string_pool,
            eval.module_env.heap(),
        )
        .map_err(Into::into)
    }

    /// [string.index](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·index
    /// ): search a substring inside a string, failing on not found.
    ///
    /// `S.index(sub[, start[, end]])` returns the index of the first
    /// occurrence of the substring `sub` within S, like `S.find`, except
    /// that if the substring is not found, the operation fails.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "bonbon".index("on") == 1
    /// "bonbon".index("on", 2) == 4
    /// # "#);
    /// # starlark::assert::fail(r#"
    /// "bonbon".index("on", 2, 5)    # error: not found
    /// # "#, "not found");
    /// ```
    #[starlark(speculative_exec_safe)]
    fn index(
        this: &str,
        #[starlark(require = pos)] sub: &str, // BLAZE PATCH 0052
        #[starlark(require = pos, default = NoneOr::None)] start: NoneOr<i32>,
        #[starlark(require = pos, default = NoneOr::None)] end: NoneOr<i32>,
    ) -> anyhow::Result<i32> {
        let needle = sub; // BLAZE PATCH 0052: Java parameter name `sub`
        if let Some(StrIndices { start, haystack }) =
            convert_str_indices(this, start.into_option(), end.into_option())
        {
            if let Some(index) = haystack.find(needle) {
                let index = fast_string::len(&haystack[..index]);
                return Ok((start + index).0 as i32);
            }
        }
        // BLAZE PATCH 0051: Java's text (StringModule.java:676, :710).
        Err(anyhow::anyhow!("substring not found"))
    }

    /// [string.isalnum](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·isalnum
    /// ): test if a string is composed only of letters and digits.
    ///
    /// `S.isalnum()` reports whether the string S is non-empty and consists
    /// only Unicode letters and digits.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "base64".isalnum() == True
    /// "Catch-22".isalnum() == False
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:767 isAlnum: `[a-zA-Z]` or javaDigit.
    #[starlark(speculative_exec_safe)]
    fn isalnum(this: &str) -> anyhow::Result<bool> {
        Ok(java_matches(this, |c| c.is_ascii_alphabetic() || java_is_digit(c), false)) // BLAZE PATCH 0033
    }

    /// [string.isalpha](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·isalpha
    /// ): test if a string is composed only of letters.
    ///
    /// `S.isalpha()` reports whether the string S is non-empty and consists
    /// only of Unicode letters.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "ABC".isalpha() == True
    /// "Catch-22".isalpha() == False
    /// "".isalpha() == False
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:757 isAlpha: `[a-zA-Z]`.
    #[starlark(speculative_exec_safe)]
    fn isalpha(this: &str) -> anyhow::Result<bool> {
        Ok(java_matches(this, |c| c.is_ascii_alphabetic(), false)) // BLAZE PATCH 0033
    }

    /// [string.isdigit](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·isdigit
    /// ): test if a string is composed only of digits.
    ///
    /// `S.isdigit()` reports whether the string S is non-empty and consists
    /// only of Unicode digits.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "123".isdigit() == True
    /// "Catch-22".isdigit() == False
    /// "".isdigit() == False
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:777 isDigit: javaDigit.
    #[starlark(speculative_exec_safe)]
    fn isdigit(this: &str) -> anyhow::Result<bool> {
        Ok(java_matches(this, java_is_digit, false)) // BLAZE PATCH 0033
    }

    /// [string.islower](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·islower
    /// ): test if all letters of a string are lowercase.
    ///
    /// `S.islower()` reports whether the string S contains at least one cased
    /// Unicode letter, and all such letters are lowercase.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "hello, world".islower() == True
    /// "Catch-22".islower() == False
    /// "123".islower() == False
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:797 isLower: no `[A-Z]`, at least one `[a-zA-Z]`.
    #[starlark(speculative_exec_safe)]
    fn islower(this: &str) -> anyhow::Result<bool> {
        Ok(java_matches(this, |c| !c.is_ascii_uppercase(), true)) // BLAZE PATCH 0033
    }

    /// [string.isspace](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·isspace
    /// ): test if all characters of a string are whitespaces.
    ///
    /// `S.isspace()` reports whether the string S is non-empty and consists
    /// only of Unicode spaces.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "    ".isspace() == True
    /// "\r\t\n".isspace() == True
    /// "".isspace() == False
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:787 isSpace: Guava `CharMatcher.whitespace()`.
    #[starlark(speculative_exec_safe)]
    fn isspace(this: &str) -> anyhow::Result<bool> {
        Ok(java_matches(this, guava_is_whitespace, false)) // BLAZE PATCH 0033
    }

    /// [string.istitle](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·istitle
    /// ): test if the string is title cased.
    ///
    /// `S.istitle()` reports whether the string S contains at least one cased
    /// Unicode letter, and all such letters that begin a word are in title
    /// case.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "Hello, World!".istitle() == True
    /// "Catch-22".istitle() == True
    /// "HAL-9000".istitle() == False
    /// "123".istitle() == False
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:821 isTitle, right to left with the `[a-z]` /
    // `[A-Z]` matchers.
    #[starlark(speculative_exec_safe)]
    fn istitle(this: &str) -> anyhow::Result<bool> {
        if this.is_empty() { // BLAZE PATCH 0033
            return Ok(false); // BLAZE PATCH 0033
        } // BLAZE PATCH 0033
        // 0: any char; 1: CASED (`[a-zA-Z]`); 2: not CASED.
        let mut matcher = 0; // BLAZE PATCH 0033
        let mut left_most_cased = ' '; // BLAZE PATCH 0033
        for c in this.chars().rev() { // BLAZE PATCH 0033
            let ok = match matcher { // BLAZE PATCH 0033
                1 => c.is_ascii_alphabetic(), // BLAZE PATCH 0033
                2 => !c.is_ascii_alphabetic(), // BLAZE PATCH 0033
                _ => true, // BLAZE PATCH 0033
            }; // BLAZE PATCH 0033
            if !ok { // BLAZE PATCH 0033
                return Ok(false); // BLAZE PATCH 0033
            } // BLAZE PATCH 0033
            matcher = if c.is_ascii_lowercase() { // BLAZE PATCH 0033
                1 // BLAZE PATCH 0033
            } else if c.is_ascii_uppercase() { // BLAZE PATCH 0033
                2 // BLAZE PATCH 0033
            } else { // BLAZE PATCH 0033
                0 // BLAZE PATCH 0033
            }; // BLAZE PATCH 0033
            if c.is_ascii_alphabetic() { // BLAZE PATCH 0033
                left_most_cased = c; // BLAZE PATCH 0033
            } // BLAZE PATCH 0033
        } // BLAZE PATCH 0033
        Ok(left_most_cased.is_ascii_uppercase()) // BLAZE PATCH 0033
    }

    /// [string.isupper](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·isupper
    /// ): test if all letters of a string are uppercase.
    ///
    /// `S.isupper()` reports whether the string S contains at least one cased
    /// Unicode letter, and all such letters are uppercase.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "HAL-9000".isupper() == True
    /// "Catch-22".isupper() == False
    /// "123".isupper() == False
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:808 isUpper: no `[a-z]`, at least one `[a-zA-Z]`.
    #[starlark(speculative_exec_safe)]
    fn isupper(this: &str) -> anyhow::Result<bool> {
        Ok(java_matches(this, |c| !c.is_ascii_lowercase(), true)) // BLAZE PATCH 0033
    }

    /// [string.lower](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·lower
    /// ): convert a string to all lowercase.
    ///
    /// `S.lower()` returns a copy of the string S with letters converted to
    /// lowercase.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "Hello, World!".lower() == "hello, world!"
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:174 lower: Guava `Ascii.toLowerCase`.
    #[starlark(speculative_exec_safe)]
    fn lower(this: &str) -> anyhow::Result<String> {
        Ok(this.to_ascii_lowercase()) // BLAZE PATCH 0033
    }

    /// [string.join](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·join
    /// ): join elements with a separator.
    ///
    /// `S.join(iterable)` returns the string formed by concatenating each
    /// element of its argument, with a copy of the string S between
    /// successive elements. The argument must be an iterable whose elements
    /// are strings.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// ", ".join([]) == ""
    /// ", ".join(("x", )) == "x"
    /// ", ".join(["one", "two", "three"]) == "one, two, three"
    /// "a".join("ctmrn".elems()) == "catamaran"
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn join<'v>(
        this: &str,
        #[starlark(require = pos)] elements: ValueOfUnchecked<'v, StarlarkIter<String>>, // BLAZE PATCH 0052
        heap: Heap<'v>,
    ) -> starlark::Result<ValueOfUnchecked<'v, String>> {
        let to_join = elements; // BLAZE PATCH 0052: Java parameter name `elements`
        crate::values::unpack::java_check_iterable("join", "elements", to_join.get(), heap)?; // BLAZE PATCH 0052
        // BLAZE PATCH 0052: Java's element check (StringModule.java:161 join).
        let mut index = 0usize; // BLAZE PATCH 0052
        let mut as_str = |x: Value<'v>| -> crate::Result<StringValue<'v>> { // BLAZE PATCH 0052
            let i = index; // BLAZE PATCH 0052
            index += 1; // BLAZE PATCH 0052
            StringValue::new(x).ok_or_else(|| { // BLAZE PATCH 0052
                crate::Error::new_other(anyhow::anyhow!( // BLAZE PATCH 0052
                    "expected string for sequence element {i}, got '{}' of type {}", // BLAZE PATCH 0052
                    x.to_str(), // BLAZE PATCH 0052
                    x.get_type() // BLAZE PATCH 0052
                )) // BLAZE PATCH 0052
            }) // BLAZE PATCH 0052
        }; // BLAZE PATCH 0052
        #[allow(dead_code)] // BLAZE PATCH 0052
        #[inline(always)]
        fn as_str_upstream<'v>(x: Value<'v>) -> crate::Result<StringValue<'v>> { // BLAZE PATCH 0052
            StringValue::unpack_named_param(x, "to_join")
        }

        let mut it = to_join.get().iterate(heap)?;
        match it.next() {
            None => Ok(ValueOfUnchecked::new(Value::new_empty_string())),
            Some(x1) => {
                match it.next() {
                    None => {
                        // If there is a singleton we can avoid reallocation
                        Ok(as_str(x1)?.to_value_of_unchecked().cast())
                    }
                    Some(x2) => {
                        let s1 = as_str(x1)?.as_str();
                        let s2 = as_str(x2)?.as_str();
                        // guess towards the upper bound, since we throw away over-allocations quickly
                        // include a buffer (20 bytes)
                        let n = it.size_hint().0 + 2;
                        let guess =
                            (cmp::max(s1.len(), s2.len()) * n) + (this.len() * (n - 1)) + 20;
                        let mut r = String::with_capacity(guess);
                        r.push_str(s1);
                        r.push_str(this);
                        r.push_str(s2);
                        for x in it {
                            r.push_str(this);
                            r.push_str(as_str(x)?.as_str());
                        }
                        Ok(heap.alloc_typed_unchecked(r))
                    }
                }
            }
        }
    }

    /// [string.lstrip](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·lstrip
    /// ): trim leading whitespaces.
    ///
    /// `S.lstrip()` returns a copy of the string S with leading whitespace removed.
    /// In most cases instead of passing an argument you should use `removeprefix`.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "  hello  ".lstrip() == "hello  "
    /// "x!hello  ".lstrip("!x ") == "hello  "
    /// # "#);
    /// ```
    // BLAZE PATCH 0034: Java StringModule.java:256 lstrip: without `chars`, the whitespace of
    // `StringModule.matcher` (ASCII when the byte-strings flag is on, Latin-1 when off). Not
    // speculative: the result depends on the evaluator's flag.
    fn lstrip<'v>(
        this: StringValue<'v>,
        // BLAZE PATCH 0046: `chars` may be `None` (Java StringModule.java:243-249: String or
        // NoneType, default None).
        #[starlark(require = pos, default = NoneOr::None)] chars: NoneOr<&str>, // BLAZE PATCH 0046
        eval: &mut Evaluator<'v, '_, '_>, // BLAZE PATCH 0034
    ) -> anyhow::Result<StringValue<'v>> {
        let chars = chars.into_option(); // BLAZE PATCH 0046
        let res = this.trim_start_matches(java_strip_matcher(chars, eval)); // BLAZE PATCH 0034
        if res.len() == this.len() {
            Ok(this)
        } else {
            Ok(eval.heap().alloc_str(res)) // BLAZE PATCH 0034
        }
    }

    /// [string.partition](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·partition
    /// ): partition a string in 3 components
    ///
    /// `S.partition(x = " ")` splits string S into three parts and returns them
    /// as a tuple: the portion before the first occurrence of string `x`,
    /// `x` itself, and the portion following it.
    /// If S does not contain `x`, `partition` returns `(S, "", "")`.
    ///
    /// `partition` fails if `x` is not a string, or is the empty string.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "one/two/three".partition("/") == ("one", "/", "two/three")
    /// "one".partition("/") == ("one", "", "")
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn partition<'v>(
        this: StringValue<'v>,
        #[starlark(require = pos)] sep: StringValue<'v>, // BLAZE PATCH 0052
        heap: Heap<'v>,
    ) -> anyhow::Result<(StringValue<'v>, StringValue<'v>, StringValue<'v>)> {
        let needle = sep; // BLAZE PATCH 0052: Java parameter name `sep`
        if needle.is_empty() {
            // BLAZE PATCH 0051: Java's text (StringModule.java:497 partitionCommon).
            return Err(anyhow::anyhow!("empty separator"));
        }
        if let Some(offset) = this.find(needle.as_str()) {
            let offset2 = offset + needle.len();
            Ok((
                heap.alloc_str(this.get(..offset).unwrap()),
                needle,
                heap.alloc_str(this.get(offset2..).unwrap()),
            ))
        } else {
            let empty = StringValue::default();
            Ok((this, empty, empty))
        }
    }

    /// [string.replace](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·replace
    /// ): replace all occurrences of a substring.
    ///
    /// `S.replace(old, new[, count])` returns a copy of string S with all
    /// occurrences of substring `old` replaced by `new`. If the optional
    /// argument `count`, which must be an `int`, is non-negative, it
    /// specifies a maximum number of occurrences to replace.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "banana".replace("a", "o") == "bonono"
    /// "banana".replace("a", "o", 2) == "bonona"
    /// "banana".replace("z", "x") == "banana"
    /// "banana".replace("", "x") == "xbxaxnxaxnxax"
    /// "banana".replace("", "x", 2) == "xbxanana"
    /// "".replace("", "x") == "x"
    /// # "# );
    /// # starlark::assert::fail(r#"
    /// "banana".replace("a", "o", -2)  # error: argument was negative
    /// # "#, "argument was negative");
    /// ```
    #[starlark(speculative_exec_safe)]
    fn replace<'v>(
        this: StringValue<'v>,
        #[starlark(require = pos)] old: &str,
        #[starlark(require = pos)] new: &str,
        #[starlark(require = pos)] count: Option<i32>,
        heap: Heap<'v>,
    ) -> anyhow::Result<StringValue<'v>> {
        // BLAZE PATCH 0005: a negative count means no limit (Java StringModule.replace,
        // StringModule.java:358).
        match count {
            Some(count) if count >= 0 => {
                Ok(heap.alloc_str(&this.replacen(old, new, count as usize)))
            }
            _ => { // BLAZE PATCH 0005
                // Optimise `replace` using the Rust standard library definition,
                // but avoiding redundant allocation in the last step
                let x = this.as_str();
                let mut result = String::new();
                let mut last_end = 0;
                for (start, part) in x.match_indices(old) {
                    result.push_str(unsafe { x.get_unchecked(last_end..start) });
                    result.push_str(new);
                    last_end = start + part.len();
                }
                if result.is_empty() && last_end == 0 {
                    Ok(this)
                } else {
                    Ok(heap
                        .alloc_str_concat(&result, unsafe { x.get_unchecked(last_end..x.len()) }))
                }
            }
        }
    }

    /// [string.rfind](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·rfind
    /// ): find the last index of a substring.
    ///
    /// `S.rfind(sub[, start[, end]])` returns the index of the substring `sub`
    /// within S, like `S.find`, except that `rfind` returns the index of
    /// the substring's _last_ occurrence.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "bonbon".rfind("on") == 4
    /// "bonbon".rfind("on", None, 5) == 1
    /// "bonbon".rfind("on", 2, 5) == -1
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn rfind(
        this: &str,
        #[starlark(require = pos)] sub: &str, // BLAZE PATCH 0052
        #[starlark(require = pos, default = NoneOr::None)] start: NoneOr<i32>,
        #[starlark(require = pos, default = NoneOr::None)] end: NoneOr<i32>,
    ) -> anyhow::Result<i32> {
        let needle = sub; // BLAZE PATCH 0052: Java parameter name `sub`
        if let Some(StrIndices { start, haystack }) =
            convert_str_indices(this, start.into_option(), end.into_option())
        {
            if let Some(index) = haystack.rfind(needle) {
                let index = fast_string::len(&haystack[..index]);
                return Ok((start + index).0 as i32);
            }
        }
        Ok(-1)
    }

    /// [string.rindex](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·rindex
    /// ): find the last index of a substring, failing on not found.
    ///
    /// `S.rindex(sub[, start[, end]])` returns the index of the substring `sub`
    /// within S, like `S.index`, except that `rindex` returns the index of
    /// the substring's _last_ occurrence.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "bonbon".rindex("on") == 4
    /// "bonbon".rindex("on", None, 5) == 1  # in "bonbo"
    /// # "#);
    /// # starlark::assert::fail(r#"
    /// "bonbon".rindex("on", 2, 5) #   error: not found
    /// # "#, "not found");
    /// ```
    #[starlark(speculative_exec_safe)]
    fn rindex(
        this: &str,
        #[starlark(require = pos)] sub: &str, // BLAZE PATCH 0052
        #[starlark(require = pos, default = NoneOr::None)] start: NoneOr<i32>,
        #[starlark(require = pos, default = NoneOr::None)] end: NoneOr<i32>,
    ) -> anyhow::Result<i32> {
        let needle = sub; // BLAZE PATCH 0052: Java parameter name `sub`
        if let Some(StrIndices { start, haystack }) =
            convert_str_indices(this, start.into_option(), end.into_option())
        {
            if let Some(index) = haystack.rfind(needle) {
                let index = fast_string::len(&haystack[..index]);
                return Ok((start + index).0 as i32);
            }
        }
        // BLAZE PATCH 0051: Java's text (StringModule.java:676, :710).
        Err(anyhow::anyhow!("substring not found"))
    }

    /// [string.rpartition](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·rpartition
    /// ): partition a string in 3 elements.
    ///
    /// `S.rpartition([x = ' '])` is like `partition`, but splits `S` at the
    /// last occurrence of `x`.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "one/two/three".rpartition("/") == ("one/two", "/", "three")
    /// "one".rpartition("/") == ("", "", "one")
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn rpartition<'v>(
        this: StringValue<'v>,
        #[starlark(require = pos)] sep: StringValue<'v>, // BLAZE PATCH 0052
        heap: Heap<'v>,
    ) -> anyhow::Result<(StringValue<'v>, StringValue<'v>, StringValue<'v>)> {
        let needle = sep; // BLAZE PATCH 0052: Java parameter name `sep`
        if needle.is_empty() {
            // BLAZE PATCH 0051: Java's text (StringModule.java:497 partitionCommon).
            return Err(anyhow::anyhow!("empty separator"));
        }
        if let Some(offset) = this.rfind(needle.as_str()) {
            let offset2 = offset + needle.len();
            Ok((
                heap.alloc_str(this.get(..offset).unwrap()),
                needle,
                heap.alloc_str(this.get(offset2..).unwrap()),
            ))
        } else {
            let empty = StringValue::default();
            Ok((empty, empty, this))
        }
    }

    /// [string.rsplit](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·rsplit
    /// ): splits a string into substrings.
    ///
    /// `S.rsplit([sep[, maxsplit]])` splits a string into substrings like
    /// `S.split`, except that when a maximum number of splits is specified,
    /// `rsplit` chooses the rightmost splits.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "banana".rsplit("n") == ["ba", "a", "a"]
    /// "banana".rsplit("n", 1) == ["bana", "a"]
    /// "one two  three".rsplit(None, 1) == ["one two", "three"]
    /// # "#);
    /// ```
    // BLAZE PATCH 0005: Java StringModule.rsplit (StringModule.java:426): `sep` is required and
    // may be named, `maxsplit` is an int (named or positional, negative: no limit), an empty
    // separator fails with "Empty separator".
    #[starlark(speculative_exec_safe)]
    fn rsplit<'v>(
        this: &str,
        sep: &str, // BLAZE PATCH 0005
        maxsplit: Option<i32>, // BLAZE PATCH 0005
        heap: Heap<'v>,
    ) -> anyhow::Result<ValueOfUnchecked<'v, UnpackList<String>>> {
        if sep.is_empty() { // BLAZE PATCH 0005
            return Err(anyhow::anyhow!("Empty separator")); // BLAZE PATCH 0005
        } // BLAZE PATCH 0005
        let maxsplit = match maxsplit { // BLAZE PATCH 0005
            Some(v) if v >= 0 => Some(v as usize + 1), // BLAZE PATCH 0005
            _ => None, // BLAZE PATCH 0005
        }; // BLAZE PATCH 0005
        let mut v: Vec<_> = match maxsplit { // BLAZE PATCH 0005
            None => this.rsplit(sep).collect(), // BLAZE PATCH 0005
            Some(maxsplit) => this.rsplitn(maxsplit, sep).collect(), // BLAZE PATCH 0005
        }; // BLAZE PATCH 0005
        v.reverse(); // BLAZE PATCH 0005
        Ok(heap.alloc_typed_unchecked(AllocList(v)).cast()) // BLAZE PATCH 0005
    }

    /// [string.rstrip](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·rstrip
    /// ): trim trailing whitespace.
    ///
    /// `S.rstrip()` returns a copy of the string S with trailing whitespace removed.
    /// In most cases instead of passing an argument you should use `removesuffix`.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "  hello  ".rstrip() == "  hello"
    /// "  hello!x".rstrip(" x!") == "  hello"
    /// # "#);
    /// ```
    // BLAZE PATCH 0034: Java StringModule.java:286 rstrip: without `chars`, the whitespace of
    // `StringModule.matcher` (ASCII when the byte-strings flag is on, Latin-1 when off). Not
    // speculative: the result depends on the evaluator's flag.
    fn rstrip<'v>(
        this: StringValue<'v>,
        // BLAZE PATCH 0046: `chars` may be `None` (Java StringModule.java:243-249: String or
        // NoneType, default None).
        #[starlark(require = pos, default = NoneOr::None)] chars: NoneOr<&str>, // BLAZE PATCH 0046
        eval: &mut Evaluator<'v, '_, '_>, // BLAZE PATCH 0034
    ) -> anyhow::Result<StringValue<'v>> {
        let chars = chars.into_option(); // BLAZE PATCH 0046
        let res = this.trim_end_matches(java_strip_matcher(chars, eval)); // BLAZE PATCH 0034
        if res.len() == this.len() {
            Ok(this)
        } else {
            Ok(eval.heap().alloc_str(res)) // BLAZE PATCH 0034
        }
    }

    /// [string.split](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·split
    /// ): split a string in substrings.
    ///
    /// `S.split([sep [, maxsplit]])` returns the list of substrings of S,
    /// splitting at occurrences of the delimiter string `sep`.
    ///
    /// Consecutive occurrences of `sep` are considered to delimit empty
    /// strings, so `'food'.split('o')` returns `['f', '', 'd']`.
    /// Splitting an empty string with a specified separator returns `['']`.
    /// If `sep` is the empty string, `split` fails.
    ///
    /// If `sep` is not specified or is `None`, `split` uses a different
    /// algorithm: it removes all leading spaces from S
    /// (or trailing spaces in the case of `rsplit`),
    /// then splits the string around each consecutive non-empty sequence of
    /// Unicode white space characters.
    ///
    /// If S consists only of white space, `split` returns the empty list.
    ///
    /// If `maxsplit` is given and non-negative, it specifies a maximum number
    /// of splits.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "one two  three".split() == ["one", "two", "three"]
    /// "one two  three".split(" ") == ["one", "two", "", "three"]
    /// "one two  three".split(None, 1) == ["one", "two  three"]
    /// "banana".split("n") == ["ba", "a", "a"]
    /// "banana".split("n", 1) == ["ba", "ana"]
    /// # "#);
    /// ```
    // BLAZE PATCH 0005: Java StringModule.split (StringModule.java:387): `sep` is required and
    // may be named, `maxsplit` is an int (named or positional, negative: no limit), an empty
    // separator fails with "Empty separator".
    #[starlark(speculative_exec_safe)]
    fn split<'v>(
        this: &str,
        sep: &str, // BLAZE PATCH 0005
        maxsplit: Option<i32>, // BLAZE PATCH 0005
        heap: Heap<'v>,
    ) -> anyhow::Result<ValueOfUnchecked<'v, UnpackList<String>>> {
        if sep.is_empty() { // BLAZE PATCH 0005
            return Err(anyhow::anyhow!("Empty separator")); // BLAZE PATCH 0005
        } // BLAZE PATCH 0005
        let maxsplit = match maxsplit { // BLAZE PATCH 0005
            Some(v) if v >= 0 => Some(v as usize + 1), // BLAZE PATCH 0005
            _ => None, // BLAZE PATCH 0005
        }; // BLAZE PATCH 0005
        Ok(match maxsplit { // BLAZE PATCH 0005
            None => { // BLAZE PATCH 0005
                if sep.len() == 1 { // BLAZE PATCH 0005
                    // If we are searching for a 1-byte string, we can provide a much faster path.
                    // Since it is one byte, given how UTF8 works, all the resultant slices must be UTF8 too.
                    let b = sep.as_bytes()[0]; // BLAZE PATCH 0005
                    let count = fast_string::count_matches_byte(this, b); // BLAZE PATCH 0005
                    let mut res = Vec::with_capacity(count + 1); // BLAZE PATCH 0005
                    res.extend( // BLAZE PATCH 0005
                        this.as_bytes() // BLAZE PATCH 0005
                            .split(|x| *x == b) // BLAZE PATCH 0005
                            .map(|x| unsafe { std::str::from_utf8_unchecked(x) }), // BLAZE PATCH 0005
                    ); // BLAZE PATCH 0005
                    debug_assert_eq!(res.len(), count + 1); // BLAZE PATCH 0005
                    heap.alloc_typed_unchecked(AllocList(res)).cast() // BLAZE PATCH 0005
                } else { // BLAZE PATCH 0005
                    heap.alloc_typed_unchecked(AllocList(this.split(sep))) // BLAZE PATCH 0005
                        .cast() // BLAZE PATCH 0005
                } // BLAZE PATCH 0005
            } // BLAZE PATCH 0005
            Some(maxsplit) => heap // BLAZE PATCH 0005
                .alloc_typed_unchecked(AllocList(this.splitn(maxsplit, sep))) // BLAZE PATCH 0005
                .cast(), // BLAZE PATCH 0005
        }) // BLAZE PATCH 0005
    } // BLAZE PATCH 0005

    /// [string.splitlines](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·splitlines
    /// ): return the list of lines of a string.
    ///
    /// `S.splitlines([keepends])` returns a list whose elements are the
    /// successive lines of S, that is, the strings formed by splitting S at
    /// line terminators ('\n', '\r' or '\r\n').
    ///
    /// The optional argument, `keepends`, is interpreted as a Boolean.
    /// If true, line terminators are preserved in the result, though
    /// the final element does not necessarily end with a line terminator.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "one\n\ntwo".splitlines() == ["one", "", "two"]
    /// "one\n\ntwo".splitlines(True) == ["one\n", "\n", "two"]
    /// "a\nb".splitlines() == ["a", "b"]
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn splitlines<'v>(
        this: &str,
        #[starlark(require = pos, default = false)] keepends: bool,
        heap: Heap<'v>,
    ) -> anyhow::Result<Vec<StringValue<'v>>> {
        let mut s = this;
        let mut lines: Vec<StringValue> = Vec::new();
        loop {
            if let Some(x) = s.find(['\n', '\r']) {
                let y = x;
                let x = match s.get(y..y + 2) {
                    Some("\r\n") => y + 2,
                    _ => y + 1,
                };
                if keepends {
                    lines.push(heap.alloc_str(s.get(..x).unwrap()))
                } else {
                    lines.push(heap.alloc_str(s.get(..y).unwrap()))
                }
                if x == s.len() {
                    return Ok(lines);
                }
                s = s.get(x..).unwrap();
            } else {
                if !s.is_empty() {
                    lines.push(heap.alloc_str(s));
                }
                return Ok(lines);
            }
        }
    }

    /// [string.startswith](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·startswith
    /// ): test whether a string starts with a given prefix.
    ///
    /// `S.startswith(prefix[, start[, end]])` reports whether the string
    /// `S[start:end]` has the specified prefix.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "filename.sky".startswith("filename") == True
    /// "filename.sky".startswith("sky") == False
    /// 'abc'.startswith(('a', 'A')) == True
    /// 'ABC'.startswith(('a', 'A')) == True
    /// 'def'.startswith(('a', 'A')) == False
    /// "//foo".startswith("//", 0, 2) == True
    /// "//foo".startswith("foo", 2) == True
    /// "hello".startswith("ell", 1, 4) == True
    /// "hello".startswith("ell", 2, 4) == False
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn startswith<'v>( // BLAZE PATCH 0052
        this: &str,
        #[starlark(require = pos)] sub: Value<'v>, // BLAZE PATCH 0052
        #[starlark(require = pos, default = NoneOr::None)] start: NoneOr<i32>,
        #[starlark(require = pos, default = NoneOr::None)] end: NoneOr<i32>,
    ) -> anyhow::Result<bool> {
        let prefix = java_string_or_tuple("startswith", sub)?; // BLAZE PATCH 0052: Java parameter name `sub`
        let haystack = match convert_str_indices(this, start.into_option(), end.into_option()) {
            Some(StrIndices { haystack, .. }) => haystack,
            None => return Ok(false),
        };
        match prefix {
            StringOrTuple::String(x) => Ok(haystack.starts_with(x)),
            StringOrTuple::Tuple(xs) => Ok(xs.items.iter().any(|x| haystack.starts_with(x))),
        }
    }

    /// [string.strip](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·strip
    /// ): trim leading and trailing whitespaces.
    ///
    /// `S.strip()` returns a copy of the string S with leading and trailing
    /// whitespace removed.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "  hello  ".strip() == "hello"
    /// "xxhello!!".strip("x!") == "hello"
    /// # "#);
    /// ```
    // BLAZE PATCH 0034: Java StringModule.java:317 strip: without `chars`, the whitespace of
    // `StringModule.matcher` (ASCII when the byte-strings flag is on, Latin-1 when off). Not
    // speculative: the result depends on the evaluator's flag.
    fn strip<'v>(
        this: StringValue<'v>,
        // BLAZE PATCH 0046: `chars` may be `None` (Java StringModule.java:243-249: String or
        // NoneType, default None).
        #[starlark(require = pos, default = NoneOr::None)] chars: NoneOr<&str>, // BLAZE PATCH 0046
        eval: &mut Evaluator<'v, '_, '_>, // BLAZE PATCH 0034
    ) -> anyhow::Result<StringValue<'v>> {
        let chars = chars.into_option(); // BLAZE PATCH 0046
        let res = this.trim_matches(java_strip_matcher(chars, eval)); // BLAZE PATCH 0034
        if res.len() == this.len() {
            Ok(this)
        } else {
            Ok(eval.heap().alloc_str(res)) // BLAZE PATCH 0034
        }
    }

    /// [string.title](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·title
    /// ): convert a string to title case.
    ///
    /// `S.title()` returns a copy of the string S with letters converted to
    /// titlecase.
    ///
    /// Letters are converted to uppercase at the start of words, lowercase
    /// elsewhere.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "hElLo, WoRlD!".title() == "Hello, World!"
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:542 title, with `Character` per char.
    #[starlark(speculative_exec_safe)]
    fn title(this: &str) -> anyhow::Result<String> {
        let mut previous_was_letter = false; // BLAZE PATCH 0033
        let mut result = String::with_capacity(this.len());
        for c in this.chars() {
            let current_is_letter = java_is_letter(c); // BLAZE PATCH 0033
            let mut d = c; // BLAZE PATCH 0033
            if current_is_letter { // BLAZE PATCH 0033
                if previous_was_letter && java_is_upper_case(c) { // BLAZE PATCH 0033
                    d = java_to_lower_case(c); // BLAZE PATCH 0033
                } else if !previous_was_letter && java_is_lower_case(c) { // BLAZE PATCH 0033
                    d = java_to_upper_case(c); // BLAZE PATCH 0033
                } // BLAZE PATCH 0033
            } // BLAZE PATCH 0033
            result.push(d); // BLAZE PATCH 0033
            previous_was_letter = current_is_letter; // BLAZE PATCH 0033
        }
        Ok(result)
    }

    /// [string.upper](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#string·upper
    /// ): convert a string to all uppercase.
    ///
    /// `S.upper()` returns a copy of the string S with letters converted to
    /// uppercase.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "Hello, World!".upper() == "HELLO, WORLD!"
    /// # "#);
    /// ```
    // BLAZE PATCH 0033: Java StringModule.java:182 upper: Guava `Ascii.toUpperCase`.
    #[starlark(speculative_exec_safe)]
    fn upper(this: &str) -> anyhow::Result<String> {
        Ok(this.to_ascii_uppercase()) // BLAZE PATCH 0033
    }

    /// [string.removeprefix](
    /// https://docs.python.org/3.9/library/stdtypes.html#str.removeprefix
    /// ): remove a prefix from a string. _Not part of standard Starlark._
    ///
    /// If the string starts with the prefix string, return `string[len(prefix):]`.
    /// Otherwise, return a copy of the original string:
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "Hello, World!".removeprefix("Hello") == ", World!"
    /// "Hello, World!".removeprefix("Goodbye") == "Hello, World!"
    /// "Hello".removeprefix("Hello") == ""
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn removeprefix<'v>(
        this: StringValue<'v>,
        #[starlark(require = pos)] prefix: &str,
        heap: Heap<'v>,
    ) -> anyhow::Result<StringValue<'v>> {
        let x = this.as_str();
        if x.starts_with(prefix) && !prefix.is_empty() {
            Ok(heap.alloc_str(&x[prefix.len()..]))
        } else {
            Ok(this)
        }
    }

    /// [string.removesuffix](
    /// https://docs.python.org/3.9/library/stdtypes.html#str.removesuffix
    /// ): remove a prefix from a string. _Not part of standard Starlark._
    ///
    /// If the string starts with the prefix string, return `string[len(prefix):]`.
    /// Otherwise, return a copy of the original string:
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// "Hello, World!".removesuffix("World!") == "Hello, "
    /// "Hello, World!".removesuffix("World") == "Hello, World!"
    /// "Hello".removesuffix("Hello") == ""
    /// # "#);
    /// ```
    #[starlark(speculative_exec_safe)]
    fn removesuffix<'v>(
        this: StringValue<'v>,
        #[starlark(require = pos)] suffix: &str,
        heap: Heap<'v>,
    ) -> anyhow::Result<StringValue<'v>> {
        let x = this.as_str();
        if x.ends_with(suffix) && !suffix.is_empty() {
            Ok(heap.alloc_str(&x[..x.len() - suffix.len()]))
        } else {
            Ok(this)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::assert;

    #[test]
    fn test_error_codes() {
        assert::fail(r#""bonbon".index("on", 2, 5)"#, "not found in");
        assert::fail(r#"("banana".replace("a", "o", -2))"#, "negative");
        assert::fail(r#""bonbon".rindex("on", 2, 5)"#, "not found in");
    }

    #[test]
    fn test_count() {
        assert::eq("'abc'.count('a', 10, -10)", "0");
    }

    #[test]
    fn test_find() {
        assert::eq("'Троянская война окончена'.find('война')", "10");
    }

    #[test]
    fn test_opaque_iterator() {
        assert::is_true("type('foo'.elems()) != type([])");
        // BLAZE PATCH 0043: no `codepoints`.
    }

    #[test]
    fn test_startswith_with_start_end() {
        assert::all_true(
            r#"
"//foo".startswith("//", 0, 2) == True
"//foo".startswith("foo", 2) == True
"hello".startswith("ell", 1, 4) == True
"hello".startswith("ell", 2, 4) == False
"hello".startswith("hel", -5) == True
"hello".startswith(("he", "wo"), 0, 3) == True
"#,
        );
    }

    #[test]
    fn test_endswith_with_start_end() {
        assert::all_true(
            r#"
"hello".endswith("ell", 0, 4) == True
"hello".endswith("lo", 3) == True
"hello".endswith("llo", 0, -1) == False
"hello".endswith(("lo", "la"), 3) == True
"#,
        );
    }
}
