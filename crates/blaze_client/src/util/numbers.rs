//! Port of `src/main/cpp/util/numbers.cc`.

// Java: src/main/cpp/util/strings.h ascii_isspace
fn ascii_isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

// Java: src/main/cpp/util/numbers.cc:42 kAsciiToInt
fn ascii_to_int(c: u8) -> i32 {
    match c {
        b'0'..=b'9' => (c - b'0') as i32,
        b'A'..=b'Z' => (c - b'A') as i32 + 10,
        b'a'..=b'z' => (c - b'a') as i32 + 10,
        _ => 36,
    }
}

// Java: src/main/cpp/util/numbers.cc:63 safe_parse_sign
/// Returns the trimmed digits and whether the number is negative, or `None` on failure.
fn safe_parse_sign(text: &[u8]) -> Option<(&[u8], bool)> {
    // C strings end at the first NUL.
    let text = match text.iter().position(|&b| b == 0) {
        Some(i) => &text[..i],
        None => text,
    };
    let mut start = 0;
    let mut end = text.len();
    // Consume whitespace.
    while start < end && ascii_isspace(text[start]) {
        start += 1;
    }
    while start < end && ascii_isspace(text[end - 1]) {
        end -= 1;
    }
    if start >= end {
        return None;
    }
    // Consume sign.
    let negative = text[start] == b'-';
    if negative || text[start] == b'+' {
        start += 1;
        if start >= end {
            return None;
        }
    }
    Some((&text[start..end], negative))
}

// Java: src/main/cpp/util/numbers.cc:111 safe_parse_positive_int
fn safe_parse_positive_int(text: &[u8]) -> (bool, i32) {
    let mut value: i32 = 0;
    let vmax = i32::MAX;
    let vmax_over_base = vmax / 10;
    for &c in text {
        let digit = ascii_to_int(c);
        if digit >= 10 {
            return (false, value);
        }
        if value > vmax_over_base {
            return (false, vmax);
        }
        value *= 10;
        if value > vmax - digit {
            return (false, vmax);
        }
        value += digit;
    }
    (true, value)
}

// Java: src/main/cpp/util/numbers.cc:140 safe_parse_negative_int
fn safe_parse_negative_int(text: &[u8]) -> (bool, i32) {
    let mut value: i32 = 0;
    let vmin = i32::MIN;
    let mut vmin_over_base = vmin / 10;
    if vmin % 10 > 0 {
        vmin_over_base += 1;
    }
    for &c in text {
        let digit = ascii_to_int(c);
        if digit >= 10 {
            return (false, value);
        }
        if value < vmin_over_base {
            return (false, vmin);
        }
        value *= 10;
        if value < vmin + digit {
            return (false, vmin);
        }
        value -= digit;
    }
    (true, value)
}

// Java: src/main/cpp/util/numbers.cc:174 safe_strto32
/// Returns `(ok, value)`. Like the C++, `value` is assigned even when parsing fails (0 when the
/// sign could not be parsed, else the partial or clamped value).
pub fn safe_strto32(text: &str) -> (bool, i32) {
    let Some((rest, negative)) = safe_parse_sign(text.as_bytes()) else {
        return (false, 0);
    };
    if !negative {
        safe_parse_positive_int(rest)
    } else {
        safe_parse_negative_int(rest)
    }
}

// Java: src/main/cpp/util/numbers.cc:188 strto32
/// `strto32(str, &endptr, base)`: returns the value and whether `*endptr == '\0'`.
pub fn strto32(s: &str, base: i32) -> (i32, bool) {
    let Ok(c) = std::ffi::CString::new(s) else {
        return (0, false);
    };
    let mut endptr: *mut libc::c_char = std::ptr::null_mut();
    // SAFETY: c is a valid NUL-terminated string; endptr receives a pointer into it.
    let result = unsafe { libc::strtol(c.as_ptr(), &mut endptr, base) };
    // SAFETY: strtol sets endptr to a position within c (or its start).
    let at_end = unsafe { *endptr == 0 };
    // On LP64 sizeof(int32_t) != sizeof(long): clamp as the C++ does.
    let value = if result < i32::MIN as libc::c_long {
        i32::MIN
    } else if result > i32::MAX as libc::c_long {
        i32::MAX
    } else {
        result as i32
    };
    (value, at_end)
}
