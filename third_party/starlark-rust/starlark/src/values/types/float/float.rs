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

use std::cmp::Ordering;
use std::convert::Infallible;
use std::fmt;
use std::fmt::Display;
use std::fmt::Write;
use std::hash::Hasher;

use allocative::Allocative;
use dupe::Dupe;
use serde::Serialize;
use starlark_derive::StarlarkPagable;
use starlark_derive::starlark_value;
use starlark_map::StarlarkHashValue;

use crate as starlark;
use crate::any::ProvidesStaticType;
use crate::collections::StarlarkHasher;
use crate::private::Private;
use crate::typing::Ty;
use crate::typing::TyBasic;
use crate::typing::TypingBinOp;
use crate::values::AllocFrozenValue;
use crate::values::AllocValue;
use crate::values::FrozenHeap;
use crate::values::FrozenValue;
use crate::values::Heap;
use crate::values::StarlarkValue;
use crate::values::UnpackValue;
use crate::values::Value;
use crate::values::ValueError;
use crate::values::ValueLike;
use crate::values::type_repr::StarlarkTypeRepr;
use crate::values::types::num::typecheck::NumTy;
use crate::values::types::num::typecheck::typecheck_num_bin_op;
use crate::values::types::num::value::NumRef;

const WRITE_PRECISION: usize = 6;

fn write_non_finite<W: fmt::Write>(output: &mut W, f: f64) -> fmt::Result {
    debug_assert!(f.is_nan() || f.is_infinite());
    if f.is_nan() {
        write!(output, "nan")
    } else {
        write!(
            output,
            "{}inf",
            if f.is_sign_positive() { "+" } else { "-" }
        )
    }
}


// BLAZE PATCH 0008: Java formats a double from its shortest round-trip decimal digits
// (java.util.Formatter / FormattedFloatingDecimal, then StarlarkFloat.format's clean-up,
// StarlarkFloat.java:120), rounding half-up on those digits.

/// The shortest round-trip digits of finite `f`: (negative, digits without leading zeros
/// (`[0]` for zero), decimal exponent of the first digit).
fn java_digits(f: f64) -> (bool, Vec<u8>, i32) { // BLAZE PATCH 0008
    let s = format!("{:e}", f.abs()); // BLAZE PATCH 0008
    let (mantissa, exponent) = s.split_once('e').unwrap(); // BLAZE PATCH 0008
    let digits: Vec<u8> = mantissa.bytes().filter(|b| b.is_ascii_digit()).map(|b| b - b'0').collect(); // BLAZE PATCH 0008
    (f.is_sign_negative(), digits, exponent.parse().unwrap()) // BLAZE PATCH 0008
} // BLAZE PATCH 0008

/// The first `keep` digits rounded half-up (zeros added when there are fewer); `true` when
/// rounding carried into a new leading digit (the digits are then all zero, read as `1` before).
fn round_half_up(digits: &[u8], keep: i64) -> (Vec<u8>, bool) { // BLAZE PATCH 0008
    if keep < 0 { // BLAZE PATCH 0008
        return (Vec::new(), false); // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    let keep = keep as usize; // BLAZE PATCH 0008
    if keep >= digits.len() { // BLAZE PATCH 0008
        let mut r = digits.to_vec(); // BLAZE PATCH 0008
        r.resize(keep, 0); // BLAZE PATCH 0008
        return (r, false); // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    let mut r = digits[..keep].to_vec(); // BLAZE PATCH 0008
    if digits[keep] >= 5 { // BLAZE PATCH 0008
        for d in r.iter_mut().rev() { // BLAZE PATCH 0008
            if *d == 9 { // BLAZE PATCH 0008
                *d = 0; // BLAZE PATCH 0008
            } else { // BLAZE PATCH 0008
                *d += 1; // BLAZE PATCH 0008
                return (r, false); // BLAZE PATCH 0008
            } // BLAZE PATCH 0008
        } // BLAZE PATCH 0008
        return (r, true); // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    (r, false) // BLAZE PATCH 0008
} // BLAZE PATCH 0008

fn push_digits(out: &mut String, digits: &[u8]) { // BLAZE PATCH 0008
    out.extend(digits.iter().map(|d| (b'0' + d) as char)); // BLAZE PATCH 0008
} // BLAZE PATCH 0008

/// Java `String.format("%.<p>e", f)` for finite `f`.
fn java_e(f: f64, p: usize, exponent_char: char) -> String { // BLAZE PATCH 0008
    let (neg, digits, mut exp) = java_digits(f); // BLAZE PATCH 0008
    let (mut r, carry) = round_half_up(&digits, p as i64 + 1); // BLAZE PATCH 0008
    if carry { // BLAZE PATCH 0008
        r.insert(0, 1); // BLAZE PATCH 0008
        r.pop(); // BLAZE PATCH 0008
        exp += 1; // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    if f == 0.0 { // BLAZE PATCH 0008
        exp = 0; // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    let mut out = String::new(); // BLAZE PATCH 0008
    if neg { // BLAZE PATCH 0008
        out.push('-'); // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    push_digits(&mut out, &r[..1]); // BLAZE PATCH 0008
    if p > 0 { // BLAZE PATCH 0008
        out.push('.'); // BLAZE PATCH 0008
        push_digits(&mut out, &r[1..]); // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    out.push(exponent_char); // BLAZE PATCH 0008
    out.push(if exp < 0 { '-' } else { '+' }); // BLAZE PATCH 0008
    out.push_str(&format!("{:02}", exp.abs())); // BLAZE PATCH 0008
    out // BLAZE PATCH 0008
} // BLAZE PATCH 0008

/// Java `String.format("%.<p>f", f)` for finite `f`.
fn java_f(f: f64, p: usize) -> String { // BLAZE PATCH 0008
    let (neg, digits, exp) = java_digits(f); // BLAZE PATCH 0008
    let mut int_len = if f == 0.0 { 1 } else { exp as i64 + 1 }; // BLAZE PATCH 0008
    let (mut r, carry) = round_half_up(&digits, int_len + p as i64); // BLAZE PATCH 0008
    if carry { // BLAZE PATCH 0008
        r.insert(0, 1); // BLAZE PATCH 0008
        int_len += 1; // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    let mut out = String::new(); // BLAZE PATCH 0008
    if neg { // BLAZE PATCH 0008
        out.push('-'); // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    if int_len > 0 { // BLAZE PATCH 0008
        push_digits(&mut out, &r[..int_len as usize]); // BLAZE PATCH 0008
    } else { // BLAZE PATCH 0008
        out.push('0'); // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    if p > 0 { // BLAZE PATCH 0008
        out.push('.'); // BLAZE PATCH 0008
        let mut frac: Vec<u8> = Vec::new(); // BLAZE PATCH 0008
        if int_len < 0 { // BLAZE PATCH 0008
            frac.resize((-int_len) as usize, 0); // BLAZE PATCH 0008
        } // BLAZE PATCH 0008
        frac.extend_from_slice(&r[int_len.max(0) as usize..]); // BLAZE PATCH 0008
        frac.resize(p, 0); // BLAZE PATCH 0008
        push_digits(&mut out, &frac); // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
    out // BLAZE PATCH 0008
} // BLAZE PATCH 0008

/// Java StarlarkFloat.format(f, 'g'/'G') for finite `f`: `%.17g`, then trailing zeros removed
/// (a fixed-point result keeps one digit after the point).
fn java_g(f: f64, exponent_char: char) -> String { // BLAZE PATCH 0008
    let (neg, digits, exp) = java_digits(f); // BLAZE PATCH 0008
    let exp = if f == 0.0 { 0 } else { exp }; // BLAZE PATCH 0008
    if (-4..17).contains(&exp) { // BLAZE PATCH 0008
        let mut s = java_f(f, (16 - exp) as usize); // BLAZE PATCH 0008
        if !s.contains('.') { // BLAZE PATCH 0008
            // No digit after the point (17 integer digits): StarlarkFloat.format adds ".0".
            return s + ".0"; // BLAZE PATCH 0008
        } // BLAZE PATCH 0008
        let dot = s.find('.').unwrap(); // BLAZE PATCH 0008
        let trimmed = s.trim_end_matches('0').len().max(dot + 2); // BLAZE PATCH 0008
        s.truncate(trimmed); // BLAZE PATCH 0008
        s // BLAZE PATCH 0008
    } else { // BLAZE PATCH 0008
        let mut out = String::new(); // BLAZE PATCH 0008
        if neg { // BLAZE PATCH 0008
            out.push('-'); // BLAZE PATCH 0008
        } // BLAZE PATCH 0008
        push_digits(&mut out, &digits[..1]); // BLAZE PATCH 0008
        if digits.len() > 1 { // BLAZE PATCH 0008
            out.push('.'); // BLAZE PATCH 0008
            push_digits(&mut out, &digits[1..]); // BLAZE PATCH 0008
        } // BLAZE PATCH 0008
        out.push(exponent_char); // BLAZE PATCH 0008
        out.push(if exp < 0 { '-' } else { '+' }); // BLAZE PATCH 0008
        out.push_str(&format!("{:02}", exp.abs())); // BLAZE PATCH 0008
        out // BLAZE PATCH 0008
    } // BLAZE PATCH 0008
} // BLAZE PATCH 0008

pub(crate) fn write_decimal<W: fmt::Write>(output: &mut W, f: f64) -> fmt::Result {
    if !f.is_finite() {
        write_non_finite(output, f)
    } else {
        output.write_str(&java_f(f, WRITE_PRECISION)) // BLAZE PATCH 0008
    }
}

pub(crate) fn write_scientific<W: fmt::Write>(
    output: &mut W,
    f: f64,
    exponent_char: char,
    strip_trailing_zeros: bool,
) -> fmt::Result {
    if !f.is_finite() {
        write_non_finite(output, f)
    } else if !strip_trailing_zeros { // BLAZE PATCH 0008
        output.write_str(&java_e(f, WRITE_PRECISION, exponent_char)) // BLAZE PATCH 0008
    } else {
        let abs = f.abs();
        let exponent = if f == 0.0 {
            0
        } else {
            abs.log10().floor() as i32
        };
        let normal = if f == 0.0 {
            0.0
        } else {
            abs / 10f64.powf(exponent as f64)
        };

        // start with "-" for a negative number
        if f.is_sign_negative() {
            output.write_char('-')?
        }

        // use the whole integral part of normal (a single digit)
        output.write_fmt(format_args!("{}", normal.trunc()))?;

        // calculate the fractional tail for given precision
        let mut tail = (normal.fract() * 10f64.powf(WRITE_PRECISION as f64)).round() as u64;
        let mut rev_tail = [0u8; WRITE_PRECISION];
        let mut rev_tail_len = 0;
        let mut removing_trailing_zeros = strip_trailing_zeros;
        for _ in 0..WRITE_PRECISION {
            let tail_digit = tail % 10;
            if tail_digit != 0 || !removing_trailing_zeros {
                removing_trailing_zeros = false;
                rev_tail[rev_tail_len] = tail_digit as u8;
                rev_tail_len += 1;
            }
            tail /= 10;
        }

        // write fractional part
        if rev_tail_len != 0 {
            output.write_char('.')?;
        }
        for digit in rev_tail[0..rev_tail_len].iter().rev() {
            output.write_char((b'0' + digit) as char)?;
        }

        // add exponent part
        output.write_char(exponent_char)?;
        output.write_fmt(format_args!("{exponent:+03}"))
    }
}

pub(crate) fn write_compact<W: fmt::Write>(
    output: &mut W,
    f: f64,
    exponent_char: char,
) -> fmt::Result {
    if !f.is_finite() {
        write_non_finite(output, f)
    } else if true { // BLAZE PATCH 0008
        output.write_str(&java_g(f, exponent_char)) // BLAZE PATCH 0008
    } else {
        let abs = f.abs();
        let exponent = if f == 0.0 {
            0
        } else {
            abs.log10().floor() as i32
        };

        if exponent.abs() >= WRITE_PRECISION as i32 {
            // use scientific notation if exponent is outside of our precision (but strip 0s)
            write_scientific(output, f, exponent_char, true)
        } else if f.fract() == 0.0 {
            // make sure there's a fractional part even if the number doesn't have it
            output.write_fmt(format_args!("{f:.1}"))
        } else {
            // rely on the built-in formatting otherwise
            output.write_fmt(format_args!("{f}"))
        }
    }
}

/// Runtime representation of Starlark `float` type.
#[derive(
    Clone,
    Dupe,
    Copy,
    Debug,
    ProvidesStaticType,
    Serialize,
    Allocative,
    StarlarkPagable
)]
#[serde(transparent)]
pub struct StarlarkFloat(pub f64);

impl StarlarkFloat {
    /// The result of calling `type()` on floats.
    pub const TYPE: &'static str = "float";

    pub(crate) fn compare_impl(a: f64, b: f64) -> Ordering {
        // According to the spec (https://github.com/bazelbuild/starlark/blob/689f54426951638ef5b7c41a14d8fc48e65c5f77/spec.md#floating-point-numbers)
        // All NaN values compare equal to each other, but greater than any non-NaN float value.
        if let Some(ord) = a.partial_cmp(&b) {
            ord
        } else {
            a.is_nan().cmp(&b.is_nan())
        }
    }

    pub(crate) fn floor_div_impl(a: f64, b: f64) -> anyhow::Result<f64> {
        if b == 0.0 {
            // Java StarlarkFloat.floordiv (StarlarkFloat.java:213).
            Err(anyhow::anyhow!("integer division by zero")) // BLAZE PATCH 0008
        } else {
            Ok((a / b).floor())
        }
    }

    pub(crate) fn percent_impl(a: f64, b: f64) -> anyhow::Result<f64> {
        if b == 0.0 {
            // Java StarlarkFloat.mod (StarlarkFloat.java:229).
            Err(anyhow::anyhow!("floating-point modulo by zero")) // BLAZE PATCH 0008
        } else {
            let r = a % b;
            if r == 0.0 {
                Ok(0.0)
            } else {
                Ok(if b.signum() != r.signum() { r + b } else { r })
            }
        }
    }
}

impl StarlarkTypeRepr for f64 {
    type Canonical = <StarlarkFloat as StarlarkTypeRepr>::Canonical;

    fn starlark_type_repr() -> Ty {
        StarlarkFloat::starlark_type_repr()
    }
}

impl<'v> AllocValue<'v> for StarlarkFloat {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_simple(self)
    }
}

impl AllocFrozenValue for StarlarkFloat {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc_simple(self)
    }
}

impl<'v> AllocValue<'v> for f64 {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc(StarlarkFloat(self))
    }
}

impl AllocFrozenValue for f64 {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        heap.alloc(StarlarkFloat(self))
    }
}

/// Allows only a float - an int will not be accepted.
impl<'v> UnpackValue<'v> for StarlarkFloat {
    type Error = Infallible;

    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        let Some(value) = value.downcast_ref::<StarlarkFloat>() else {
            return Ok(None);
        };
        Ok(Some(*value))
    }
}

impl Display for StarlarkFloat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_compact(f, self.0, 'e')
    }
}

#[starlark_value(type = StarlarkFloat::TYPE)]
impl<'v> StarlarkValue<'v> for StarlarkFloat {
    fn equals(&self, other: Value) -> crate::Result<bool> {
        Ok(Some(NumRef::Float(StarlarkFloat(self.0))) == other.unpack_num())
    }

    fn collect_repr(&self, s: &mut String) {
        write!(s, "{self}").unwrap()
    }

    fn to_bool(&self) -> bool {
        self.0 != 0.0
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> crate::Result<()> {
        hasher.write_u64(NumRef::from(self.0).get_hash_64());
        Ok(())
    }

    fn get_hash(&self, _private: Private) -> crate::Result<StarlarkHashValue> {
        Ok(NumRef::Float(*self).get_hash())
    }

    fn plus(&self, heap: Heap<'v>) -> crate::Result<Value<'v>> {
        Ok(heap.alloc(*self))
    }

    fn minus(&self, heap: Heap<'v>) -> crate::Result<Value<'v>> {
        Ok(heap.alloc(StarlarkFloat(-self.0)))
    }

    fn add(&self, other: Value, heap: Heap<'v>) -> Option<crate::Result<Value<'v>>> {
        let other = other.unpack_num()?; // BLAZE PATCH 0008
        if let Err(e) = NumRef::Float(*self).check_float_operands(other) { // BLAZE PATCH 0008
            return Some(Err(e.into())); // BLAZE PATCH 0008
        } // BLAZE PATCH 0008
        Some(Ok(heap.alloc(NumRef::Float(*self) + other))) // BLAZE PATCH 0008
    }

    fn sub(&self, other: Value, heap: Heap<'v>) -> crate::Result<Value<'v>> {
        match other.unpack_num() {
            None => ValueError::unsupported_with(self, "-", other),
            Some(other) => { // BLAZE PATCH 0008
                NumRef::Float(*self).check_float_operands(other)?; // BLAZE PATCH 0008
                Ok(heap.alloc(NumRef::Float(*self) - other)) // BLAZE PATCH 0008
            } // BLAZE PATCH 0008
        }
    }

    fn mul(&self, other: Value<'v>, heap: Heap<'v>) -> Option<crate::Result<Value<'v>>> {
        let other = other.unpack_num()?; // BLAZE PATCH 0008
        if let Err(e) = NumRef::Float(*self).check_float_operands(other) { // BLAZE PATCH 0008
            return Some(Err(e.into())); // BLAZE PATCH 0008
        } // BLAZE PATCH 0008
        Some(Ok(heap.alloc(NumRef::Float(*self) * other))) // BLAZE PATCH 0008
    }

    fn div(&self, other: Value, heap: Heap<'v>) -> crate::Result<Value<'v>> {
        match other.unpack_num() {
            None => ValueError::unsupported_with(self, "/", other),
            Some(other) => Ok(heap.alloc(NumRef::Float(*self).div(other)?)),
        }
    }

    fn percent(&self, other: Value, heap: Heap<'v>) -> crate::Result<Value<'v>> {
        match other.unpack_num() {
            Some(other) => Ok(heap.alloc(NumRef::Float(*self).percent(other)?)),
            None => ValueError::unsupported_with(self, "%", other),
        }
    }

    fn floor_div(&self, other: Value, heap: Heap<'v>) -> crate::Result<Value<'v>> {
        match other.unpack_num() {
            None => ValueError::unsupported_with(self, "//", other),
            Some(other) => Ok(heap.alloc(NumRef::Float(*self).floor_div(other)?)),
        }
    }

    fn bin_op_ty(op: TypingBinOp, rhs: &TyBasic) -> Option<Ty> {
        typecheck_num_bin_op(NumTy::Float, op, rhs)
    }

    fn compare(&self, other: Value) -> crate::Result<Ordering> {
        match other.unpack_num() {
            None => ValueError::unsupported_with(self, "compare", other),
            Some(other) => Ok(NumRef::Float(*self).cmp(&other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert;
    use crate::assert::Assert;

    fn non_finite(f: f64) -> String {
        let mut buf = String::new();
        write_non_finite(&mut buf, f).unwrap();
        buf
    }

    #[test]
    fn test_write_non_finite() {
        assert_eq!(non_finite(f64::NAN), "nan");
        assert_eq!(non_finite(f64::INFINITY), "+inf");
        assert_eq!(non_finite(f64::NEG_INFINITY), "-inf");
    }

    fn decimal(f: f64) -> String {
        let mut buf = String::new();
        write_decimal(&mut buf, f).unwrap();
        buf
    }

    #[test]
    fn test_write_decimal() {
        assert_eq!(decimal(f64::NAN), "nan");
        assert_eq!(decimal(f64::INFINITY), "+inf");
        assert_eq!(decimal(f64::NEG_INFINITY), "-inf");

        assert_eq!(decimal(0f64), "0.000000");
        assert_eq!(decimal(std::f64::consts::PI), "3.141593");
        assert_eq!(decimal(-std::f64::consts::E), "-2.718282");
        assert_eq!(decimal(1e10), "10000000000.000000");
    }

    fn scientific(f: f64) -> String {
        let mut buf = String::new();
        write_scientific(&mut buf, f, 'e', false).unwrap();
        buf
    }

    #[test]
    fn test_write_scientific() {
        assert_eq!(scientific(f64::NAN), "nan");
        assert_eq!(scientific(f64::INFINITY), "+inf");
        assert_eq!(scientific(f64::NEG_INFINITY), "-inf");

        assert_eq!(scientific(0f64), "0.000000e+00");
        assert_eq!(scientific(-0f64), "-0.000000e+00");
        assert_eq!(scientific(1.23e45), "1.230000e+45");
        assert_eq!(scientific(-3.14e-145), "-3.140000e-145");
        assert_eq!(scientific(1e300), "1.000000e+300");
    }

    fn compact(f: f64) -> String {
        let mut buf = String::new();
        write_compact(&mut buf, f, 'e').unwrap();
        buf
    }

    #[test]
    fn test_write_compact() {
        assert_eq!(compact(f64::NAN), "nan");
        assert_eq!(compact(f64::INFINITY), "+inf");
        assert_eq!(compact(f64::NEG_INFINITY), "-inf");

        assert_eq!(compact(0f64), "0.0");
        assert_eq!(compact(std::f64::consts::PI), "3.141592653589793");
        assert_eq!(compact(-std::f64::consts::E), "-2.718281828459045");
        assert_eq!(compact(1e10), "1e+10");
        assert_eq!(compact(1.23e45), "1.23e+45");
        assert_eq!(compact(-3.14e-145), "-3.14e-145");
        assert_eq!(compact(1e300), "1e+300");
    }

    #[test]
    fn test_arithmetic_operators() {
        assert::all_true(
            r#"
+1.0 == 1.0
-1.0 == 0. - 1.
1.0 + 2.0 == 3.0
1.0 - 2.0 == -1.0
2.0 * 3.0 == 6.0
5.0 / 2.0 == 2.5
5.0 % 3.0 == 2.0
5.0 // 2.0 == 2.0
"#,
        );
    }

    #[test]
    fn test_dictionary_key() {
        assert::pass(
            r#"
x = {0: 123}
assert_eq(x[0], 123)
# TODO(nga): fix typechecker, and remove `noop`.
assert_eq(x[noop(0.0)], 123)
assert_eq(x[noop(-0.0)], 123)
assert_eq(1 in x, False)
        "#,
        );
    }

    #[test]
    fn test_comparisons() {
        let mut a = Assert::new();
        // TODO(nga): fix and enable.
        a.disable_static_typechecking();
        a.all_true(
            r#"
+0.0 == -0.0
0.0 == 0
0 == 0.0
0 < 1.0
0.0 < 1
1 > 0.0
1.0 > 0
0.0 < float("nan")
float("+inf") < float("nan")
"#,
        );
    }

    #[test]
    fn test_comparisons_by_sorting() {
        assert::eq(
            "sorted([float('inf'), float('-inf'), float('nan'), 1e300, -1e300, 1.0, -1.0, 1, -1, 1e-300, -1e-300, 0, 0.0, float('-0.0'), 1e-300, -1e-300])",
            "[float('-inf'), -1e+300, -1.0, -1, -1e-300, -1e-300, 0, 0.0, -0.0, 1e-300, 1e-300, 1.0, 1, 1e+300, float('+inf'), float('nan')]",
        );
    }
}
