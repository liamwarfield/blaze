//! The part of `lib/util/ResourceConverter.java` that `TestResourcesConverter` uses: the
//! keyword-expression parsing of `ResourceConverter.DoubleConverter`.
//!
//! PORT: ResourceConverter is a `lib/util` class shared with execution options; only the
//! `DoubleConverter` path is ported here. The `HOST_CPUS`/`HOST_RAM` suppliers need
//! `LocalHostCapacity`, which is not ported: evaluating a keyword fails loudly.

use blaze_options::{OptionsParsingException, java_parse_double};

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:47 ResourceConverter.AUTO_KEYWORD
pub const AUTO_KEYWORD: &str = "auto";
// Java: com/google/devtools/build/lib/util/ResourceConverter.java:48 ResourceConverter.HOST_CPUS_KEYWORD
pub const HOST_CPUS_KEYWORD: &str = "HOST_CPUS";
// Java: com/google/devtools/build/lib/util/ResourceConverter.java:49 ResourceConverter.HOST_RAM_KEYWORD
pub const HOST_RAM_KEYWORD: &str = "HOST_RAM";

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:51 ResourceConverter.HOST_CPUS_SUPPLIER
pub fn host_cpus_supplier() -> f64 {
    blaze_util::unported!(
        "LocalHostCapacity.getLocalHostCapacity (ResourceConverter.HOST_CPUS_SUPPLIER)"
    )
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:53 ResourceConverter.HOST_RAM_SUPPLIER
pub fn host_ram_supplier() -> f64 {
    blaze_util::unported!(
        "LocalHostCapacity.getLocalHostCapacity (ResourceConverter.HOST_RAM_SUPPLIER)"
    )
}

/// A keyword and the supplier of its value (`ImmutableMap<String, Supplier<Double>>` entry).
pub type ResourceKeyword = (&'static str, fn() -> f64);

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:113 ResourceConverter.DoubleConverter
/// Resource converter for doubles.
pub struct ResourceConverterDoubleConverter {
    /// Keyword to the supplier of its value, in map order.
    keywords: Vec<ResourceKeyword>,
    min_value: f64,
    max_value: f64,
}

impl ResourceConverterDoubleConverter {
    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:126 ResourceConverter.DoubleConverter.<init>
    pub fn new(keywords: Vec<ResourceKeyword>, min_value: f64, max_value: f64) -> Self {
        ResourceConverterDoubleConverter {
            keywords,
            min_value,
            max_value,
        }
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:132 ResourceConverter.DoubleConverter.convert
    pub fn convert(&self, input: &str) -> Result<f64, OptionsParsingException> {
        match java_parse_double(input) {
            // Converters.DoubleConverter parses the same way.
            Some(v) => self.check_and_limit(v),
            None => self.convert_keyword(input),
        }
    }

    /// `validInputPattern.matcher(input).matches()`:
    /// `(?<keyword>K1|K2...)(?<expression>[-*][0-9]?(?:.[0-9]+)?)?`. Returns the keyword index
    /// and the expression (None when the group did not participate).
    fn match_valid_input<'a>(&self, input: &'a str) -> Option<(usize, Option<&'a str>)> {
        for (i, (k, _)) in self.keywords.iter().enumerate() {
            let Some(rest) = input.strip_prefix(k) else {
                continue;
            };
            if rest.is_empty() {
                return Some((i, None));
            }
            if is_expression(rest) {
                return Some((i, Some(rest)));
            }
        }
        None
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:183 ResourceConverter.convertKeyword
    pub fn convert_keyword(&self, input: &str) -> Result<f64, OptionsParsingException> {
        if let Some((i, expression)) = self.match_valid_input(input) {
            let resource_supplier = self.keywords[i].1;
            return self.apply_operator(expression, resource_supplier);
        }
        Err(OptionsParsingException::new(format!(
            "Parameter '{}' does not follow correct syntax. This flag takes {}.",
            input,
            self.get_type_description()
        )))
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:198 ResourceConverter.applyOperator
    /// Applies function designated in `expression` (`[-|*]<float>`) to value.
    fn apply_operator(
        &self,
        expression: Option<&str>,
        first_operand_supplier: fn() -> f64,
    ) -> Result<f64, OptionsParsingException> {
        let Some(expression) = expression else {
            return Ok(first_operand_supplier());
        };
        for (op, f) in [
            ("-", (|l, r| l - r) as fn(f64, f64) -> f64),
            ("*", |l, r| l * r),
        ] {
            if let Some(operand) = expression.strip_prefix(op) {
                // Float.parseFloat trims leading/trailing whitespace.
                let second_operand: f32 = match operand.trim_matches(|c: char| c <= ' ').parse() {
                    Ok(v) => v,
                    Err(_) => {
                        return Err(OptionsParsingException::with_cause(
                            format!("'{} is not a float", operand),
                            format!("For input string: \"{}\"", operand),
                        ));
                    }
                };
                return Ok(f(first_operand_supplier(), second_operand as f64));
            }
        }
        // This should never happen because we've checked for a valid operator already.
        Err(OptionsParsingException::new(format!(
            "Parameter value '{}' does not contain a valid operator.",
            expression
        )))
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:228 ResourceConverter.checkAndLimit
    /// Checks validity of a resource value against min/max constraints.
    pub fn check_and_limit(&self, value: f64) -> Result<f64, OptionsParsingException> {
        // Double.compareTo: NaN is greater than everything.
        if value < self.min_value {
            return Err(OptionsParsingException::new(format!(
                "Value '({})' must be at least {}.",
                java_format_f(value),
                java_format_f(self.min_value)
            )));
        }
        if value > self.max_value || value.is_nan() {
            return Err(OptionsParsingException::new(format!(
                "Value '({})' cannot be greater than {}.",
                java_format_f(value),
                java_format_f(self.max_value)
            )));
        }
        Ok(value)
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:244 ResourceConverter.getTypeDescription
    pub fn get_type_description(&self) -> String {
        let first_keyword = self.keywords[0].0;
        let names: Vec<&str> = self.keywords.iter().map(|(k, _)| *k).collect();
        format!(
            "an integer, or a keyword (\"{}\"), optionally followed by an operation \
             ([-|*]<float>) eg. \"{}\", \"{}*.5\"",
            names.join("\", \""),
            first_keyword,
            HOST_CPUS_KEYWORD
        )
    }
}

/// `[-*][0-9]?(?:.[0-9]+)?` matched against the whole of `s` (Java's `.` excludes line
/// terminators).
fn is_expression(s: &str) -> bool {
    let mut chars = s.chars();
    if !matches!(chars.next(), Some('-' | '*')) {
        return false;
    }
    let t: Vec<char> = chars.collect();
    let dot_digits = |u: &[char]| {
        u.len() >= 2
            && !matches!(u[0], '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
            && u[1..].iter().all(|c| c.is_ascii_digit())
    };
    t.is_empty()
        || (t.len() == 1 && t[0].is_ascii_digit())
        || (t[0].is_ascii_digit() && dot_digits(&t[1..]))
        || dot_digits(&t)
}

/// Java `String.format("%f", d)`: Java rounds the shortest round-trip decimal digits (as
/// `Double.toString` produces them) half-up to six fraction digits.
pub fn java_format_f(d: f64) -> String {
    if d.is_nan() {
        return "NaN".to_string();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let neg = d.is_sign_negative() && d != 0.0;
    // Shortest digits and exponent: "d.ddde<exp>".
    let sci = format!("{:e}", d.abs());
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: Vec<u8> = mant
        .bytes()
        .filter(|b| *b != b'.')
        .map(|b| b - b'0')
        .collect();
    // Value = 0.d1d2d3... * 10^(exp+1). Build integer and fraction digit arrays.
    let point = exp + 1; // number of digits before the decimal point
    let mut int_part: Vec<u8> = Vec::new();
    let mut frac_part: Vec<u8> = Vec::new();
    if point <= 0 {
        int_part.push(0);
        frac_part.extend(std::iter::repeat_n(0, (-point) as usize));
        frac_part.extend(&digits);
    } else {
        for i in 0..point as usize {
            int_part.push(*digits.get(i).unwrap_or(&0));
        }
        if (point as usize) < digits.len() {
            frac_part.extend(&digits[point as usize..]);
        }
    }
    // Round half-up to 6 fraction digits.
    let round_up = frac_part.len() > 6 && frac_part[6] >= 5;
    frac_part.resize(6, 0);
    if round_up {
        let mut carry = true;
        for d in frac_part.iter_mut().rev().chain(int_part.iter_mut().rev()) {
            if !carry {
                break;
            }
            if *d == 9 {
                *d = 0;
            } else {
                *d += 1;
                carry = false;
            }
        }
        if carry {
            int_part.insert(0, 1);
        }
    }
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.extend(int_part.iter().map(|d| (b'0' + d) as char));
    s.push('.');
    s.extend(frac_part.iter().map(|d| (b'0' + d) as char));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv() -> ResourceConverterDoubleConverter {
        ResourceConverterDoubleConverter::new(
            vec![
                (HOST_CPUS_KEYWORD, host_cpus_supplier),
                (HOST_RAM_KEYWORD, host_ram_supplier),
            ],
            0.0,
            f64::MAX,
        )
    }

    #[test]
    fn java_format_f_matches_java() {
        assert_eq!(java_format_f(0.0), "0.000000");
        assert_eq!(java_format_f(-1.0), "-1.000000");
        assert_eq!(java_format_f(0.1234565), "0.123457");
        assert_eq!(java_format_f(123.5), "123.500000");
        assert_eq!(
            java_format_f(f64::MAX),
            format!("17976931348623157{}.000000", "0".repeat(292))
        );
    }

    #[test]
    fn convert_plain_numbers_and_bounds() {
        assert_eq!(conv().convert("1.5").unwrap(), 1.5);
        assert_eq!(
            conv().convert("-1").unwrap_err().get_message(),
            "Value '(-1.000000)' must be at least 0.000000."
        );
    }

    #[test]
    fn bad_syntax_reports_type_description() {
        assert_eq!(
            conv().convert("foo").unwrap_err().get_message(),
            "Parameter 'foo' does not follow correct syntax. This flag takes an integer, or a \
             keyword (\"HOST_CPUS\", \"HOST_RAM\"), optionally followed by an operation \
             ([-|*]<float>) eg. \"HOST_CPUS\", \"HOST_CPUS*.5\"."
        );
        assert!(conv().convert("HOST_CPUS/2").is_err());
    }

    #[test]
    fn expression_grammar() {
        for ok in ["-", "*1", "*.5", "*1.5", "-1x5", "*x55", "*12"] {
            assert!(is_expression(ok), "{}", ok);
        }
        for bad in ["", "+1", "*1.", "*.5.5"] {
            assert!(!is_expression(bad), "{}", bad);
        }
    }
}
