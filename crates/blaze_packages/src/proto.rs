//! The `proto` module of utilities for protocol message processing.
//!
//! Java: com/google/devtools/build/lib/packages/Proto.java (`proto.encode_text`), ported with
//! its error texts. `Proto.INSTANCE` is put in the BUILD, .bzl and cquery environments by
//! `StarlarkGlobalsImpl.addCommonUtilToplevels`
//! (com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:49).
//!
//! Java `Structure` and `NativeInfo` values are the values providing a
//! [`StructureMarker`]: `native_info: false` is a `Structure` (`StructImpl` and the other
//! implementations), `native_info: true` a `NativeInfo`, which at the pin is *not* a `Structure`
//! (com/google/devtools/build/lib/packages/NativeInfo.java:39 `implements Info`).

use blaze_starlark::error::bare_message;
use blaze_starlark::printer::quote;
use blaze_starlark::structs::StructureMarker;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::Heap;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::float::StarlarkFloat;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;
use starlark::values::typing::StarlarkCallable;

use crate::starlark_defs::util::errorf;

/// Nesting limit of struct values in `proto.encode_text`.
// PORT: Java's TextEncoder recurses without a limit, so a cyclic value (a struct reachable from
// a list it holds) ends in a StackOverflowError. A native stack overflow cannot be reported in
// Rust, so nesting deeper than this fails loudly with `unported!` instead.
const MAX_DEPTH: usize = blaze_starlark::json::MAX_DEPTH;

/// Java `Starlark.type(x)`. starlark-rust reports "function" for builtins and for `def`s;
/// Bazel's builtins are `builtin_function_or_method`. A `def` or `lambda` reprs as
/// `<file>.<name>`, a builtin as its bare name (until patch 0009 fixes types).
// PORT: the same rule as `blaze_starlark::json` (`json_type_name`, private there).
fn starlark_type(x: Value) -> String {
    let t = x.get_type();
    if t == "function" {
        let r = x.to_repr();
        if r.starts_with("<built-in") || !r.contains('.') {
            return "builtin_function_or_method".to_owned();
        }
    }
    t.to_owned()
}

/// Java `x instanceof Structure`.
fn is_structure(x: Value) -> bool {
    x.request_value::<StructureMarker>()
        .is_some_and(|m| !m.native_info)
}

/// Java `x instanceof NativeInfo`.
fn is_native_info(x: Value) -> bool {
    x.request_value::<StructureMarker>()
        .is_some_and(|m| m.native_info)
}

/// Java `value instanceof StarlarkCallable`.
fn is_starlark_callable<'v>(value: Value<'v>) -> bool {
    matches!(
        <StarlarkCallable<'v> as UnpackValue<'v>>::unpack_value_impl(value),
        Ok(Some(_))
    )
}

/// The elements of `v` if it is a Java `Sequence` (list, tuple or range), in order.
fn sequence_elements<'v>(v: Value<'v>, heap: Heap<'v>) -> Option<Vec<Value<'v>>> {
    if let Some(l) = ListRef::from_value(v) {
        return Some(l.content().to_vec());
    }
    if let Some(t) = TupleRef::from_value(v) {
        return Some(t.content().to_vec());
    }
    // Java: net/starlark/java/eval/RangeList.java (`implements Sequence<StarlarkInt>`)
    if v.get_type() == "range" {
        return match v.iterate(heap) {
            Ok(it) => Some(it.collect()),
            Err(e) => blaze_util::unported!(
                "Proto.TextEncoder.field",
                "a range that cannot be iterated: {}",
                bare_message(&e)
            ),
        };
    }
    None
}

/// Java `String.format(Locale.US, "%.17g", v)` for a finite `v`.
// PORT: java.util.Formatter is not ported. Its `%g` of a double (JDK 25, the oracle's embedded
// runtime: jdk.internal.math.FormattedFPDecimal) starts from the decimal that
// `Double.toString` renders, which has at most 17 digits, so precision 17 never rounds. The
// result is in decimal format when 10^-4 <= |v| < 10^17, with `17 - (exponent + 1)` digits
// after the point (none, and then no point, for exponent 16), and otherwise in computerized
// scientific notation with 16 digits after the point and an exponent of at least two digits.
// Zero is "0" followed by 16 zeros after the point. The sign of -0.0 is printed.
fn java_format_17g(v: f64) -> String {
    const PRECISION: usize = 17;
    let mut s = String::new();
    // Java: `Double.compare(value, 0.0) == -1`, true for -0.0 too.
    if v.is_sign_negative() {
        s.push('-');
    }
    let abs = v.abs();
    if abs == 0.0 {
        s.push_str("0.");
        s.push_str(&"0".repeat(PRECISION - 1));
        return s;
    }
    let (digits, exponent) = java_double_to_decimal(abs);
    let n = digits.len();
    if (-4..PRECISION as i32).contains(&exponent) {
        let fraction_length = (PRECISION as i32 - (exponent + 1)) as usize;
        let mut fraction = String::new();
        if exponent >= 0 {
            let integer_length = (exponent + 1) as usize;
            if n <= integer_length {
                s.push_str(&digits);
                s.push_str(&"0".repeat(integer_length - n));
            } else {
                s.push_str(&digits[..integer_length]);
                fraction.push_str(&digits[integer_length..]);
            }
        } else {
            s.push('0');
            fraction.push_str(&"0".repeat((-exponent - 1) as usize));
            fraction.push_str(&digits);
        }
        if fraction_length > 0 {
            while fraction.len() < fraction_length {
                fraction.push('0');
            }
            s.push('.');
            s.push_str(&fraction);
        }
    } else {
        s.push_str(&digits[..1]);
        let mut fraction = digits[1..].to_owned();
        while fraction.len() < PRECISION - 1 {
            fraction.push('0');
        }
        s.push('.');
        s.push_str(&fraction);
        s.push('e');
        s.push(if exponent < 0 { '-' } else { '+' });
        s.push_str(&format!("{:02}", exponent.unsigned_abs()));
    }
    s
}

/// The digits `d1 d2 ... dn` and the exponent `e` of the decimal `d1.d2...dn x 10^e` that Java's
/// `Double.toString` renders for a finite `abs > 0`.
// PORT: Java (java/lang/Double.java `toString`, JDK 19 and later) picks, among the decimals
// that round to the double, those of the shortest length, but of length 2 when the shortest
// length is 1, and of these the one closest to the double. Rust's `{:e}` gives the closest
// decimal of the shortest length. When that has one digit, the closest decimal of two digits is
// the double correctly rounded to two digits (`{:.1e}`): it is at least as close to the double
// as the one-digit decimal, so it rounds to the double as well. The two differ only for
// subnormals of very few bits (Double.MIN_VALUE is 4.9E-324 in Java, 5e-324 in Rust).
fn java_double_to_decimal(abs: f64) -> (String, i32) {
    let mut e_form = format!("{abs:e}");
    if !e_form.contains('.') {
        e_form = format!("{abs:.1e}");
    }
    let Some((mantissa, exponent)) = e_form.split_once('e') else {
        blaze_util::unported!("StarlarkFloat.format", "no exponent in {}", e_form)
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        blaze_util::unported!("StarlarkFloat.format", "exponent of {}", e_form)
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    (digits, exponent)
}

/// Java `StarlarkFloat.toString`: `format(v, 'g')`.
// PORT: starlark-rust's float repr differs from Java's (patch 0008 is queued), so the 'g'
// conversion is ported here.
// Java: net/starlark/java/eval/StarlarkFloat.java:51 StarlarkFloat.toString
// Java: net/starlark/java/eval/StarlarkFloat.java:120 StarlarkFloat.format
fn starlark_float_to_string(v: f64) -> String {
    if !v.is_finite() {
        if v == f64::INFINITY {
            return "+inf".to_owned();
        } else if v == f64::NEG_INFINITY {
            return "-inf".to_owned();
        } else {
            return "nan".to_owned();
        }
    }

    let mut s = java_format_17g(v); // use DBL_DECIMAL_DIG places

    // %g is the default format used by str.
    // It always includes a '.' or an 'e', to make clear that
    // the value is a float, not an int.
    //
    // For now, we just clean up the output of Java's %.17g implementation,
    // which is unambiguous, but may yield unnecessarily long digit strings
    // such as 1000000000000.0.
    match s.find('e') {
        None => match s.find('.') {
            None => {
                // Ensure result always has a decimal point if no exponent.
                // "123" -> "123.0"
                s.push_str(".0");
            }
            Some(dot) => {
                // Remove trailing zeros after decimal point.
                // "1.110" => "1.11"
                // "1.000" => "1.0"
                let bytes = s.as_bytes();
                let mut i = s.len() - 1;
                while i > dot + 1 && bytes[i] == b'0' {
                    i -= 1;
                }
                s.truncate(i + 1);
            }
        },
        Some(e) => {
            // Remove trailing zeros from mantissa.
            // "1.23000e+45" => "1.23e+45"
            // "1.00000e+45" => "1e+45"
            let bytes = s.as_bytes();
            let mut i = e - 1;
            while bytes[i] == b'0' {
                i -= 1;
            }
            if bytes[i] == b'.' {
                i -= 1;
            }
            if i < e - 1 {
                s = format!("{}{}", &s[..i + 1], &s[e..]);
            }
        }
    }

    s
}

// Java: com/google/devtools/build/lib/packages/Proto.java:117 Proto.TextEncoder
struct TextEncoder<'v> {
    out: String,
    indent: i32,
    heap: Heap<'v>,
    /// Nesting of `message` calls (see [`MAX_DEPTH`]).
    depth: usize,
}

/// Java `EvalException`, as far as the encoder uses it: `ex.getMessage()`.
type EvalResult = Result<(), String>;

impl<'v> TextEncoder<'v> {
    // Encodes Structure x as a protocol message.
    // Java: com/google/devtools/build/lib/packages/Proto.java:123 Proto.TextEncoder.message
    fn message(&mut self, x: Value<'v>) -> EvalResult {
        if self.depth >= MAX_DEPTH {
            blaze_util::unported!(
                "Proto.TextEncoder.message",
                "structs nested deeper than {} (Java recurses until StackOverflowError)",
                MAX_DEPTH
            );
        }
        self.depth += 1;
        let r = self.message_inner(x);
        self.depth -= 1;
        r
    }

    fn message_inner(&mut self, x: Value<'v>) -> EvalResult {
        // For determinism, sort fields.
        // Java: `Ordering.natural().sortedCopy(Starlark.dir(...))`, String.compareTo. Bazel's
        // strings hold UTF-8 bytes in Latin-1 chars, so that is the byte order of `str`.
        let mut fields = x.dir_attr();
        fields.sort();
        for field in fields {
            let r = match x.get_attr_error(&field, self.heap) {
                Err(e) => Err(bare_message(&e)),
                Ok(value) => {
                    // This preserves legacy behavior where only struct_fields from NativeInfo
                    // were emitted
                    // When serializing non NativeInfo, just let it fail on functions
                    if is_native_info(x) && is_starlark_callable(value) {
                        continue;
                    }
                    self.field(&field, value)
                }
            };
            if let Err(message) = r {
                return Err(format!(
                    "in {} field .{}: {}",
                    starlark_type(x),
                    field,
                    message
                ));
            }
        }
        Ok(())
    }

    // Encodes Structure field (name, v) as a message field
    // (a repeated field, if v is a dict or sequence.)
    // Java: com/google/devtools/build/lib/packages/Proto.java:146 Proto.TextEncoder.field
    fn field(&mut self, name: &str, v: Value<'v>) -> EvalResult {
        // dict?
        // A key-tracking dict (`ctx.var`) records every key: Java Proto.TextEncoder.field of a Map: `entrySet()`.
        starlark::values::dict::dict_record_all_keys_accessed(v);
        if let Some(dict) = DictRef::from_value(v) {
            let entries: Vec<(Value<'v>, Value<'v>)> = dict.iter().collect();
            drop(dict);
            for (key, value) in entries {
                let key_str = key.unpack_str();
                if !(key_str.is_some() || key.get_type() == "int") {
                    return Err(format!(
                        "invalid dict key: got {}, want int or string",
                        starlark_type(key)
                    ));
                }
                self.emit_line(&[name, " {"]);
                self.indent += 1;
                self.field_element("key", key)?; // can't fail
                if let Err(message) = self.field_element("value", value) {
                    // Java `Starlark.repr(key)` of a string or an int.
                    let key_repr = match key_str {
                        Some(s) => quote(s),
                        None => key.to_str(),
                    };
                    return Err(format!("in value for dict key {}: {}", key_repr, message));
                }
                self.indent -= 1;
                self.emit_line(&["}"]);
            }
            return Ok(());
        }

        // list or tuple?
        if let Some(items) = sequence_elements(v, self.heap) {
            for (i, item) in items.into_iter().enumerate() {
                if let Err(message) = self.field_element(name, item) {
                    return Err(format!("at {} index {}: {}", starlark_type(v), i, message));
                }
            }
            return Ok(());
        }

        // non-repeated field
        if v.is_none() {
            return Ok(());
        }
        self.field_element(name, v)
    }

    // Emits field (name, v) as a message field, or one element of a repeated field.
    // v must be an int, float, string, bool, or Structure.
    // Java: com/google/devtools/build/lib/packages/Proto.java:194 Proto.TextEncoder.fieldElement
    fn field_element(&mut self, name: &str, v: Value<'v>) -> EvalResult {
        if is_structure(v) {
            self.emit_line(&[name, " {"]);
            self.indent += 1;
            self.message(v)?;
            self.indent -= 1;
            self.emit_line(&["}"]);
        } else if let Some(s) = v.unpack_str() {
            self.emit_line(&[
                name,
                ": \"",
                &s.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n"),
                "\"",
            ]);
        } else if let Some(b) = v.unpack_bool() {
            // Java: Boolean.toString
            self.emit_line(&[name, ": ", if b { "true" } else { "false" }]);
        } else if v.get_type() == "int" {
            // Java: StarlarkInt.toString
            self.emit_line(&[name, ": ", &v.to_str()]);
        } else if let Some(f) = v.downcast_ref::<StarlarkFloat>() {
            let mut s = starlark_float_to_string(f.0);
            // Encoding to textproto via proto.encode_text requires "inf" for "+inf".
            if s == "+inf" {
                s = "inf".to_owned();
            }
            self.emit_line(&[name, ": ", &s]);
        } else {
            return Err(format!(
                "got {}, want string, int, float, bool, or struct",
                starlark_type(v)
            ));
        }
        Ok(())
    }

    // Emits items on an indented line.
    // Java: com/google/devtools/build/lib/packages/Proto.java:221 Proto.TextEncoder.emitLine
    fn emit_line(&mut self, items: &[&str]) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
        for item in items {
            self.out.push_str(item);
        }
        self.out.push('\n');
    }
}

/// Java `Proto.encodeText`, with the check of its parameter `x`
/// (`allowedTypes = {Structure, NativeInfo}`).
// Java: com/google/devtools/build/lib/packages/Proto.java:111 Proto.encodeText
pub fn proto_encode_text<'v>(x: Value<'v>, heap: Heap<'v>) -> starlark::Result<String> {
    // Java: net/starlark/java/eval/BuiltinFunction.java:388 BuiltinFunction.checkParamValue.
    // The wanted types are `Starlark.classType(Structure.class)` ("structure") and
    // `Starlark.classType(NativeInfo.class)` (the simple class name: NativeInfo has no
    // @StarlarkBuiltin ancestor), joined by ParamDescriptor.getTypeErrorMessage.
    if !(is_structure(x) || is_native_info(x)) {
        return Err(errorf(format!(
            "in call to encode_text(), parameter 'x' got value of type '{}', want 'structure or \
             NativeInfo'",
            starlark_type(x)
        )));
    }
    let mut enc = TextEncoder {
        out: String::new(),
        indent: 0,
        heap,
        depth: 0,
    };
    enc.message(x).map_err(errorf)?;
    Ok(enc.out)
}

/// The members of the `proto` module (Java: `Proto.INSTANCE`).
// Java: com/google/devtools/build/lib/packages/Proto.java:41 Proto
#[starlark_module]
pub fn register_proto_members(builder: &mut GlobalsBuilder) {
    /// Returns the struct argument's encoding as a text-format protocol message.
    // Java: com/google/devtools/build/lib/packages/Proto.java:111 Proto.encodeText
    fn encode_text<'v>(
        #[starlark(require = pos)] x: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<String> {
        proto_encode_text(x, heap)
    }
}

/// Registers the `proto` namespace (Java: `env.put("proto", Proto.INSTANCE)`).
// PORT: like `json` (blaze_starlark::json::register_json), the module object is a starlark-rust
// namespace; Java's `type(proto)` is "proto" and its repr
// `<unknown object com.google.devtools.build.lib.packages.Proto>`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:54 StarlarkGlobalsImpl.addCommonUtilToplevels
pub fn register_proto(builder: &mut GlobalsBuilder) {
    builder.namespace("proto", register_proto_members);
}

#[cfg(test)]
mod tests {
    use super::java_format_17g;
    use super::starlark_float_to_string;

    // Expected values: the Java code of StarlarkFloat.format (StarlarkFloat.java:120-204) run on
    // OpenJDK 25 (the major version of the oracle's embedded runtime); the values marked
    // "oracle" were also printed by Bazel 9.3.2 through `proto.encode_text`.
    #[test]
    #[allow(clippy::excessive_precision)]
    fn starlark_float_to_string_matches_java() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0.0"),   // oracle
            (-0.0, "-0.0"), // oracle
            (1.0, "1.0"),   // oracle
            (-1.0, "-1.0"),
            (1.5, "1.5"),
            (0.1, "0.1"),                       // oracle
            (0.1 + 0.2, "0.30000000000000004"), // oracle
            (1.2e34, "1.2e+34"), // oracle; StarlarkRuleClassFunctionsTest.testSimpleTextMessages
            (-1.2e-34, "-1.2e-34"),
            (1e15, "1000000000000000.0"),  // oracle
            (1e16, "10000000000000000.0"), // oracle
            (4e16, "40000000000000000.0"),
            (1e17, "1e+17"), // oracle
            (4e17, "4e+17"),
            (12345678901234567.0, "12345678901234568.0"),
            (123456789012345678.0, "1.2345678901234568e+17"),
            (99999999999999990.0, "99999999999999980.0"),
            (1234567890123456.7, "1234567890123456.8"),
            (9007199254740993.0, "9007199254740992.0"),
            (1e-4, "0.0001"), // oracle
            (1.25e-4, "0.000125"),
            (0.0001234, "0.0001234"),
            (0.00012345678901234567, "0.00012345678901234567"),
            (1e-5, "1e-05"), // oracle
            (9.99e-5, "9.99e-05"),
            (5e-5, "5e-05"),
            (1e-7, "1e-07"),
            (1.1e-10, "1.1e-10"),
            (0.001, "0.001"),
            (0.25, "0.25"),
            (0.5, "0.5"),
            (1.0 / 3.0, "0.3333333333333333"),
            (2.0 / 3.0, "0.6666666666666666"),
            (0.1 + 0.7, "0.7999999999999999"),
            (3.0, "3.0"),
            (8.0, "8.0"),
            (33.0, "33.0"),
            (100.0, "100.0"),
            (123.0, "123.0"),
            (1024.0, "1024.0"),
            (1e5, "100000.0"),
            (1e6, "1000000.0"),
            (1e7, "10000000.0"),
            (1e9, "1000000000.0"),
            (1e10, "10000000000.0"),
            (123456.789, "123456.789"),
            (123456789.125, "123456789.125"), // oracle
            (1e21, "1e+21"),
            (1e22, "1e+22"),
            (1e23, "1e+23"),
            (2e23, "2e+23"), // oracle
            (2.82879384806159e17, "2.82879384806159e+17"),
            (1e100, "1e+100"),
            (1e-100, "1e-100"),
            (1.5e300, "1.5e+300"),
            (1e308, "1e+308"),
            (f64::MAX, "1.7976931348623157e+308"),
            (f64::MIN_POSITIVE, "2.2250738585072014e-308"),
            (2e-308, "2e-308"),
            // Subnormals of few bits: Double.toString renders at least two digits.
            (5e-324, "4.9e-324"), // oracle
            (1e-323, "9.9e-324"), // oracle
            (1.5e-323, "1.5e-323"),
            (2e-323, "2e-323"),
            (5e-323, "4.9e-323"), // oracle
            (1e-322, "9.9e-323"),
            (4.94e-322, "4.94e-322"),
            (5e-322, "5e-322"),
            (1e-320, "1e-320"),
            (f64::INFINITY, "+inf"),
            (f64::NEG_INFINITY, "-inf"),
            (f64::NAN, "nan"),
        ];
        for (v, expected) in cases {
            assert_eq!(&starlark_float_to_string(*v), expected, "{v:e}");
        }
    }

    // Expected values: `String.format(Locale.US, "%.17g", v)` on OpenJDK 25.
    #[test]
    #[allow(clippy::excessive_precision)]
    fn java_format_17g_matches_java() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0.0000000000000000"),
            (-0.0, "-0.0000000000000000"),
            (1.0, "1.0000000000000000"),
            (0.1, "0.10000000000000000"),
            (1e-4, "0.00010000000000000000"),
            (1e-5, "1.0000000000000000e-05"),
            (1.2e34, "1.2000000000000000e+34"),
            (1e15, "1000000000000000.0"),
            (1e16, "10000000000000000"),
            (12345678901234567.0, "12345678901234568"),
            (1e17, "1.0000000000000000e+17"),
            (123.0, "123.00000000000000"),
            (123456789.125, "123456789.12500000"),
            (1e100, "1.0000000000000000e+100"),
            (5e-324, "4.9000000000000000e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
        ];
        for (v, expected) in cases {
            assert_eq!(&java_format_17g(*v), expected, "{v:e}");
        }
    }
}
