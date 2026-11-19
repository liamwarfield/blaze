/*
 * Copyright 2019 The Starlark in Rust Authors.
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

use either::Either;
use starlark_derive::starlark_module;

use crate as starlark;
use crate::environment::GlobalsBuilder;
use crate::values::Heap;
use crate::values::ValueOf;
use crate::values::ValueOfUnchecked;
use crate::values::int::int_or_big::StarlarkInt;
use crate::values::int::pointer_i32::PointerI32;
use crate::values::types::num::value::NumRef;

#[starlark_module]
pub(crate) fn register_int(globals: &mut GlobalsBuilder) {
    /// [int](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#int
    /// ): convert a value to integer.
    ///
    /// `int(x[, base])` interprets its argument as an integer.
    ///
    /// If x is an `int`, the result is x.
    /// If x is a `float`, the result is the integer value nearest to x,
    /// truncating towards zero; it is an error if x is not finite (`NaN`,
    /// `+Inf`, `-Inf`).
    /// If x is a `bool`, the result is 0 for `False` or 1 for `True`.
    ///
    /// If x is a string, it is parsed as Bazel's `StarlarkInt.parse` does (BLAZE PATCH 0044):
    /// `base` defaults to 10; with `base = 0` a `0b`/`0o`/`0x` prefix determines the base, and a
    /// prefix matching an explicit base is allowed. The string may specify an arbitrarily large
    /// integer. `x` is required.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// int(1) == 1
    /// int(False) == 0
    /// int(True) == 1
    /// int('1') == 1
    /// int('16') == 16
    /// int('16', 10) == 16
    /// int('16', 8) == 14
    /// int('16', 16) == 22
    /// int(0.0) == 0
    /// int(3.14) == 3
    /// int(-12345.6789) == -12345
    /// int(2e9) == 2000000000
    /// # "#);
    /// # starlark::assert::fail(r#"
    /// int("hello")   # error: invalid base-10 literal
    /// # "#, "invalid base-10 literal");
    /// # starlark::assert::fail(r#"
    /// int(float("nan"))   # error: cannot be represented as exact integer
    /// # "#, "cannot be represented as exact integer");
    /// # starlark::assert::fail(r#"
    /// int(float("inf"))   # error: cannot be represented as exact integer
    /// # "#, "cannot be represented as exact integer");
    /// ```
    #[starlark(as_type = PointerI32, speculative_exec_safe)]
    fn int<'v>(
        // BLAZE PATCH 0044: `x` is required (Java MethodLibrary.java:551-570: no default).
        #[starlark(require = pos)] a: ValueOf<'v, Either<Either<NumRef<'v>, bool>, &'v str>>, // BLAZE PATCH 0044
        base: Option<i32>,
        heap: Heap<'v>,
    ) -> starlark::Result<ValueOfUnchecked<'v, StarlarkInt>> {
        let num_or_bool = match a.typed {
            Either::Left(num_or_bool) => num_or_bool,
            // BLAZE PATCH 0044: Java MethodLibrary.intForStarlark (MethodLibrary.java:613):
            // `StarlarkInt.parse(x, base)` with base 10 when unbound.
            Either::Right(s) => { // BLAZE PATCH 0044
                let x = java_parse_int(s, base.unwrap_or(10)) // BLAZE PATCH 0044
                    .map_err(crate::Error::new_value)?; // BLAZE PATCH 0044
                return Ok(ValueOfUnchecked::new(heap.alloc(x))); // BLAZE PATCH 0044
            } // BLAZE PATCH 0044
        };

        if base.is_some() {
            // BLAZE PATCH 0044: Java's text (MethodLibrary.java:624).
            return Err(anyhow::anyhow!("can't convert non-string with explicit base").into()); // BLAZE PATCH 0044
        }

        match num_or_bool {
            Either::Left(NumRef::Int(_)) => Ok(ValueOfUnchecked::new(a.value)),
            Either::Left(NumRef::Float(f)) => Ok(ValueOfUnchecked::new(
                heap.alloc(StarlarkInt::from_f64_exact(f.0.trunc())?),
            )),
            Either::Right(b) => Ok(ValueOfUnchecked::new(heap.alloc(b as i32))),
        }
    }
}

/// BLAZE PATCH 0044: Java `StarlarkInt.parse` (net/starlark/java/eval/StarlarkInt.java:99-168).
/// Digits are Java's `Character.digit(c, base)`, which for chars up to U+00FF (every char of a
/// Blaze internal string) is ASCII `0-9`, `a-z`, `A-Z` only; chars above U+00FF (other Unicode
/// digits, which Java accepts) are rejected here.
fn java_parse_int(s: &str, base: i32) -> anyhow::Result<StarlarkInt> { // BLAZE PATCH 0044
    use num_bigint::BigInt; // BLAZE PATCH 0044
    use num_traits::Num; // BLAZE PATCH 0044
    let repr = |x: &str| { // BLAZE PATCH 0044
        let mut r = String::new(); // BLAZE PATCH 0044
        crate::values::types::string::repr::string_repr(x, &mut r); // BLAZE PATCH 0044
        r // BLAZE PATCH 0044
    }; // BLAZE PATCH 0044
    let string_for_errors = s; // BLAZE PATCH 0044
    if s.is_empty() { // BLAZE PATCH 0044
        return Err(anyhow::anyhow!("empty string")); // BLAZE PATCH 0044
    } // BLAZE PATCH 0044
    let mut base = base; // BLAZE PATCH 0044
    let (is_negative, s) = match s.as_bytes()[0] { // BLAZE PATCH 0044
        b'+' => (false, &s[1..]), // BLAZE PATCH 0044
        b'-' => (true, &s[1..]), // BLAZE PATCH 0044
        _ => (false, s), // BLAZE PATCH 0044
    }; // BLAZE PATCH 0044
    let mut digits = s; // BLAZE PATCH 0044
    let mut stripped = false; // BLAZE PATCH 0044
    let mut chars = s.chars(); // BLAZE PATCH 0044
    if let (Some('0'), Some(c)) = (chars.next(), chars.next()) { // BLAZE PATCH 0044
        let prefix_base = match c { // BLAZE PATCH 0044
            'b' | 'B' => 2, // BLAZE PATCH 0044
            'o' | 'O' => 8, // BLAZE PATCH 0044
            'x' | 'X' => 16, // BLAZE PATCH 0044
            _ => 0, // BLAZE PATCH 0044
        }; // BLAZE PATCH 0044
        if prefix_base != 0 && (base == 0 || base == prefix_base) { // BLAZE PATCH 0044
            base = prefix_base; // BLAZE PATCH 0044
            digits = &s[2..]; // BLAZE PATCH 0044
            stripped = true; // BLAZE PATCH 0044
        } // BLAZE PATCH 0044
    } // BLAZE PATCH 0044
    if !stripped && base == 0 { // BLAZE PATCH 0044
        // Don't infer base when input starts with '0' due to octal/decimal ambiguity.
        if s.chars().count() > 1 && s.starts_with('0') { // BLAZE PATCH 0044
            return Err(anyhow::anyhow!( // BLAZE PATCH 0044
                "cannot infer base when string begins with a 0: {}", // BLAZE PATCH 0044
                repr(string_for_errors) // BLAZE PATCH 0044
            )); // BLAZE PATCH 0044
        } // BLAZE PATCH 0044
        base = 10; // BLAZE PATCH 0044
    } // BLAZE PATCH 0044
    if !(2..=36).contains(&base) { // BLAZE PATCH 0044
        return Err(anyhow::anyhow!( // BLAZE PATCH 0044
            "invalid base {base} (want 2 <= base <= 36)" // BLAZE PATCH 0044
        )); // BLAZE PATCH 0044
    } // BLAZE PATCH 0044
    let invalid = || { // BLAZE PATCH 0044
        anyhow::anyhow!("invalid base-{base} literal: {}", repr(string_for_errors)) // BLAZE PATCH 0044
    }; // BLAZE PATCH 0044
    // Java: Long.parseLong / new BigInteger: at least one digit, each `Character.digit >= 0`
    // (their own sign was ruled out above).
    if digits.is_empty() // BLAZE PATCH 0044
        || !digits.chars().all(|c| c.is_ascii() && c.to_digit(base as u32).is_some()) // BLAZE PATCH 0044
    { // BLAZE PATCH 0044
        return Err(invalid()); // BLAZE PATCH 0044
    } // BLAZE PATCH 0044
    let value = BigInt::from_str_radix(digits, base as u32).map_err(|_| invalid())?; // BLAZE PATCH 0044
    let value = if is_negative { -value } else { value }; // BLAZE PATCH 0044
    Ok(StarlarkInt::from(value)) // BLAZE PATCH 0044
} // BLAZE PATCH 0044
