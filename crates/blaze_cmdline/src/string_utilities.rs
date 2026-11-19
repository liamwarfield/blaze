//! Private ports of the `lib/util` and `net.starlark.java.spelling` helpers that `lib/cmdline`
//! needs. They live here until a shared util crate owns them.

/// Escapes control characters: `\r` becomes `\\r`; 0-31 (except 13) and 127 become `<?>`.
// Java: com/google/devtools/build/lib/util/StringUtilities.java:35 StringUtilities.CONTROL_CHAR_ESCAPER
// Java: com/google/devtools/build/lib/util/StringUtilities.java:139 StringUtilities.sanitizeControlChars
pub(crate) fn sanitize_control_chars(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    for c in message.chars() {
        match c {
            '\r' => out.push_str("\\r"),
            '\u{0}'..='\u{1f}' | '\u{7f}' => out.push_str("<?>"),
            _ => out.push(c),
        }
    }
    out
}

// Java String.toLowerCase, as the chars it yields: `Character.toLowerCase` per char. On an
// internal string (every char <= U+00FF) that lowercases ASCII letters and the Latin-1 letters
// U+00C0-U+00DE except U+00D7; other chars are kept (the callers only compare the result).
fn java_to_lower_chars(s: &str) -> Vec<char> {
    s.chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            '\u{c0}'..='\u{de}' if c != '\u{d7}' => {
                char::from_u32(c as u32 + 0x20).expect("Latin-1")
            }
            '\u{100}'.. => c.to_lowercase().next().unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// Levenshtein distance capped at `max_edit_distance`; returns -1 if greater.
// Java: net/starlark/java/spelling/SpellChecker.java:33 SpellChecker.editDistance
// Java's strings are sequences of chars; an internal string's chars are the `char`s here.
pub(crate) fn edit_distance(s1: &[char], s2: &[char], max_edit_distance: i32) -> i32 {
    // This is the Levenshtein distance, as described here:
    // http://en.wikipedia.org/wiki/Levenshtein_distance
    if s1 == s2 {
        return 0;
    }
    // Short-circuit based on string length.
    if (s1.len() as i32 - s2.len() as i32).abs() > max_edit_distance {
        return -1;
    }
    let mut row: Vec<i32> = (0..=s2.len() as i32).collect();
    for i in 1..=s1.len() {
        row[0] = i as i32;
        let mut best_in_the_row = row[0];
        let mut previous = i as i32 - 1;
        for j in 1..=s2.len() {
            let old = row[j];
            row[j] = std::cmp::min(
                previous + if s1[i - 1] == s2[j - 1] { 0 } else { 1 },
                1 + std::cmp::min(row[j - 1], row[j]),
            );
            previous = old;
            best_in_the_row = std::cmp::min(best_in_the_row, row[j]);
        }
        if best_in_the_row > max_edit_distance {
            return -1;
        }
    }
    let result = row[s2.len()];
    if result <= max_edit_distance {
        result
    } else {
        -1
    }
}

/// The most similar word to `input` (edit distance, ignoring case), or `None`.
// Java: net/starlark/java/spelling/SpellChecker.java:83 SpellChecker.suggest
pub(crate) fn suggest<'a>(
    input: &str,
    words: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let mut best = None;
    // Heuristic: the expected number of typos depends on the length of the word.
    // `input.length()`: chars.
    let mut best_distance = std::cmp::min(5, (input.chars().count() as i32 + 1) / 2);
    let input = java_to_lower_chars(input);
    for candidate in words {
        let d = edit_distance(&input, &java_to_lower_chars(candidate), best_distance);
        if d >= 0 && d < best_distance {
            best_distance = d;
            best = Some(candidate);
        }
    }
    best
}

/// Either "" or a suggestion such as ` (did you mean 'x'?)`.
// Java: net/starlark/java/spelling/SpellChecker.java:102 SpellChecker.didYouMean
pub(crate) fn did_you_mean<'a>(input: &str, words: impl IntoIterator<Item = &'a str>) -> String {
    match suggest(input, words) {
        None => String::new(),
        Some(suggestion) => format!(" (did you mean '{}'?)", suggestion),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize() {
        assert_eq!(sanitize_control_chars("a\rb\u{0}c\u{7f}"), "a\\rb<?>c<?>");
    }

    #[test]
    fn spelling() {
        assert_eq!(did_you_mean("boo", ["foo"]), " (did you mean 'foo'?)");
        assert_eq!(did_you_mean("xyzzy", ["foo"]), "");
    }

    #[test]
    fn spelling_counts_and_lowercases_chars_of_internal_strings() {
        // "é" as an internal string is two chars (C3 A9): `input.length()` is 2, so the
        // allowed distance is 1 and a one-char difference is not a suggestion.
        assert_eq!(did_you_mean("\u{c3}\u{a9}", ["\u{c3}\u{a8}"]), "");
        // Character.toLowerCase on each Latin-1 char: U+00C3 lowers to U+00E3.
        assert_eq!(
            did_you_mean("\u{c3}\u{a9}XY", ["\u{e3}\u{a9}xy"]),
            " (did you mean '\u{e3}\u{a9}xy'?)"
        );
        // A byte that is not UTF-8 (FF) is one char.
        assert_eq!(
            did_you_mean("\u{ff}abc", ["\u{ff}abd"]),
            " (did you mean '\u{ff}abd'?)"
        );
    }
}
