//! Java's `String.format("%.<n>f", d)` for the summary lines the UI and the metrics print.

/// Java `String.format(Locale.US, "%.<precision>f", d)`: rounded half-up from the shortest
/// decimal representation (Java's `FormattedFloatingDecimal`).
///
/// PORT: the same algorithm as `blaze_buildtool`'s `test_summary_printer::java_format_fixed`,
/// shared here for the crates below `blaze_buildtool` (critical path, UI).
// Java: java/util/Formatter.java FormatSpecifier.print(double)
pub fn java_format_fixed(d: f64, precision: usize) -> String {
    if d.is_nan() {
        return "NaN".into();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    // Rust's `{}` prints the shortest round-trip digits without an exponent.
    let s = format!("{}", d);
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r.to_string()),
        None => (false, s),
    };
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (s.clone(), String::new()),
    };
    let mut digits: Vec<u8> = int_part.bytes().chain(frac_part.bytes()).collect();
    let int_len = int_part.len();
    let mut frac_len = frac_part.len();
    if frac_len > precision {
        let round_up = digits[int_len + precision] >= b'5';
        digits.truncate(int_len + precision);
        frac_len = precision;
        if round_up {
            let mut k = digits.len();
            loop {
                if k == 0 {
                    digits.insert(0, b'1');
                    break;
                }
                k -= 1;
                if digits[k] == b'9' {
                    digits[k] = b'0';
                } else {
                    digits[k] += 1;
                    break;
                }
            }
        }
    }
    let total_int = digits.len() - frac_len;
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    out.push_str(std::str::from_utf8(&digits[..total_int]).unwrap());
    if precision > 0 {
        out.push('.');
        out.push_str(std::str::from_utf8(&digits[total_int..]).unwrap());
        for _ in frac_len..precision {
            out.push('0');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::java_format_fixed;

    #[test]
    fn rounds_half_up_from_the_shortest_representation() {
        assert_eq!(java_format_fixed(0.125, 2), "0.13");
        assert_eq!(java_format_fixed(1.005, 2), "1.01");
        assert_eq!(java_format_fixed(0.0, 2), "0.00");
        assert_eq!(java_format_fixed(2.5, 3), "2.500");
        assert_eq!(java_format_fixed(9.9996, 3), "10.000");
        assert_eq!(java_format_fixed(12.0, 0), "12");
    }
}
