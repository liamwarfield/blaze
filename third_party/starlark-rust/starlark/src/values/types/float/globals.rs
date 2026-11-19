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

use either::Either;
use starlark_derive::starlark_module;

use crate as starlark;
use crate::environment::GlobalsBuilder;
use crate::values::float::StarlarkFloat;
use crate::values::string::repr::string_repr;
use crate::values::types::num::value::NumRef;

#[starlark_module]
pub(crate) fn register_float(globals: &mut GlobalsBuilder) {
    /// [float](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#float
    /// ): interprets its argument as a floating-point number.
    ///
    /// If x is a `float`, the result is x.
    /// if x is an `int`, the result is the nearest floating point value to x.
    /// If x is a string, the string is interpreted as a floating-point literal.
    /// With no arguments, `float()` returns `0.0`.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// float() == 0.0
    /// float(1) == 1.0
    /// float('1') == 1.0
    /// float('1.0') == 1.0
    /// float('.25') == 0.25
    /// float('1e2') == 100.0
    /// float(False) == 0.0
    /// float(True) == 1.0
    /// # "#);
    /// # starlark::assert::fail(r#"
    /// float("hello")   # error: not a valid number
    /// # "#, "not a valid number");
    /// # starlark::assert::fail(r#"
    /// float([])   # error
    /// # "#, "doesn't match, expected");
    /// ```
    #[starlark(as_type = StarlarkFloat, speculative_exec_safe)]
    fn float(
        #[starlark(require = pos)] x: Option<Either<Either<NumRef, bool>, &str>>, // BLAZE PATCH 0052
    ) -> anyhow::Result<f64> {
        let a = x; // BLAZE PATCH 0052: Java parameter name `x`
        if a.is_none() {
            return Ok(0.0);
        }
        let a = a.unwrap();
        match a {
            // Java MethodLibrary.float: StarlarkInt.toFiniteDouble.
            Either::Left(Either::Left(f)) => Ok(f.as_finite_float()?), // BLAZE PATCH 0008
            Either::Left(Either::Right(b)) => Ok(if b { 1.0 } else { 0.0 }),
            // BLAZE PATCH 0051: Java MethodLibrary.floatForStarlark (MethodLibrary.java:491):
            // the non-finite words, then `Double.parseDouble`, with Java's texts.
            Either::Right(s) => { // BLAZE PATCH 0051
                let _ = string_repr; // BLAZE PATCH 0051
                java_float_from_string(s) // BLAZE PATCH 0051
            } // BLAZE PATCH 0051
        }
    }
}

/// BLAZE PATCH 0051: Java `MethodLibrary.floatForStarlark` for a string.
// Java: net/starlark/java/eval/MethodLibrary.java:491 MethodLibrary.floatForStarlark
fn java_float_from_string(s: &str) -> anyhow::Result<f64> {
    if s.is_empty() {
        return Err(anyhow::anyhow!("empty string"));
    }
    let eq = |w: &str| s.eq_ignore_ascii_case(w);
    match s.chars().last().unwrap().to_ascii_lowercase() {
        'n' | 'f' | 'y' => {
            if eq("nan") || eq("+nan") || eq("-nan") {
                Ok(f64::NAN)
            } else if eq("inf") || eq("+inf") || eq("+infinity") {
                Ok(f64::INFINITY)
            } else if eq("-inf") || eq("-infinity") {
                Ok(f64::NEG_INFINITY)
            } else {
                Err(anyhow::anyhow!("invalid float literal: {s}"))
            }
        }
        _ => match java_parse_double(s) {
            Some(d) if d.is_finite() => Ok(d),
            // parseDouble accepts signed "NaN" and "Infinity" (case sensitive), which the
            // switch above has not seen only with trailing whitespace; Java says "too large".
            Some(_) => Err(anyhow::anyhow!("floating-point number too large")),
            None => Err(anyhow::anyhow!("invalid float literal: {s}")),
        },
    }
}

/// BLAZE PATCH 0051: Java `Double.parseDouble` (java/lang/FloatingDecimal.readJavaFormatString):
/// leading and trailing chars <= U+0020 ignored; optional sign; `NaN`, `Infinity`; a decimal
/// literal with an optional `f`/`F`/`d`/`D` suffix; a hexadecimal literal `0x...p...`.
/// `None` where Java throws NumberFormatException. Correctly rounded, as Java.
fn java_parse_double(s: &str) -> Option<f64> {
    let t = s.trim_matches(|c: char| c <= ' ');
    let (neg, body) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let sign = |v: f64| if neg { -v } else { v };
    if body == "NaN" {
        return Some(f64::NAN);
    }
    if body == "Infinity" {
        return Some(sign(f64::INFINITY));
    }
    let b = body.as_bytes();
    if b.len() > 1 && b[0] == b'0' && (b[1] == b'x' || b[1] == b'X') {
        return java_parse_hex_double(&body[2..]).map(sign);
    }
    let body = match b.last() {
        Some(b'f' | b'F' | b'd' | b'D') => &body[..body.len() - 1],
        _ => body,
    };
    // Digits [. Digits] [e Signed] | . Digits [e Signed]
    let b = body.as_bytes();
    let mut i = 0;
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let f = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - f;
    }
    if int_digits + frac_digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let e = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == e {
            return None;
        }
    }
    if i != b.len() {
        return None;
    }
    body.parse::<f64>().ok().map(sign)
}

/// The part of a Java hexadecimal floating-point literal after `0x`: hex digits with an optional
/// `.`, then a mandatory binary exponent `p`, then an optional `f`/`d` suffix.
fn java_parse_hex_double(s: &str) -> Option<f64> {
    let s = match s.as_bytes().last() {
        Some(b'f' | b'F' | b'd' | b'D') => &s[..s.len() - 1],
        _ => s,
    };
    let p = s.find(['p', 'P'])?;
    let (mant, exp) = (&s[..p], &s[p + 1..]);
    let exp: i64 = {
        let e = exp.strip_prefix('+').unwrap_or(exp);
        if e.is_empty() || !e.trim_start_matches('-').bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        e.parse::<i64>().unwrap_or(if e.starts_with('-') { i64::MIN / 2 } else { i64::MAX / 2 })
    };
    let (ip, fp) = match mant.find('.') {
        Some(d) => (&mant[..d], &mant[d + 1..]),
        None => (mant, ""),
    };
    if ip.is_empty() && fp.is_empty() {
        return None;
    }
    if !ip.bytes().chain(fp.bytes()).all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    // Exact value = digits * 2^(exp - 4*len(fp)); compute with big integers, then round.
    let digits = format!("{ip}{fp}");
    let m = num_bigint::BigUint::parse_bytes(digits.as_bytes(), 16)?;
    let e2 = exp.saturating_sub(4 * fp.len() as i64);
    Some(big_times_pow2_to_f64(&m, e2))
}

/// `m * 2^e` rounded to the nearest double (ties to even).
fn big_times_pow2_to_f64(m: &num_bigint::BigUint, e: i64) -> f64 {
    use num_traits::ToPrimitive;
    use num_traits::Zero;
    if m.is_zero() {
        return 0.0;
    }
    let bits = m.bits() as i64;
    // Normalize to 54 significant bits (one guard bit) plus sticky.
    let shift = bits - 54;
    let (top, sticky) = if shift > 0 {
        let top = m >> (shift as usize);
        let rem = m - (&top << (shift as usize));
        (top, !rem.is_zero())
    } else {
        (m << ((-shift) as usize), false)
    };
    let mut top = top.to_u64().unwrap();
    let mut exp = e + shift; // value = top * 2^exp, top has 54 bits
    // Subnormal handling: the smallest exponent for a 53-bit mantissa is -1074.
    let mut sticky = sticky;
    let min_exp = -1074 - 1; // top includes one guard bit
    if exp < min_exp {
        let d = (min_exp - exp) as u32;
        if d >= 64 {
            sticky |= top != 0;
            top = 0;
        } else {
            sticky |= top & ((1u64 << d) - 1) != 0;
            top >>= d;
        }
        exp = min_exp;
    }
    let guard = top & 1;
    let mut mant = top >> 1;
    if guard == 1 && (sticky || mant & 1 == 1) {
        mant += 1;
    }
    let exp = exp + 1;
    (mant as f64) * 2f64.powi(0).max(1.0) * pow2(exp)
}

fn pow2(e: i64) -> f64 {
    if e > 2000 {
        return f64::INFINITY;
    }
    if e < -2000 {
        return 0.0;
    }
    // Split to stay within the range of powi.
    let mut r = 1.0f64;
    let mut e = e;
    while e > 1000 {
        r *= 2f64.powi(1000);
        e -= 1000;
    }
    while e < -1000 {
        r *= 2f64.powi(-1000);
        e += 1000;
    }
    r * 2f64.powi(e as i32)
}
