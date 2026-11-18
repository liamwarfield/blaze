//! Ports of the string helpers attribute and rule-class code needs: `lib/util/StringUtil`
//! (`joinEnglishList`) and `net/starlark/java/spelling/SpellChecker` (`didYouMean`).
// PORT: move to blaze_util / blaze_starlark when shared more widely.

/// Java `StringUtil.joinEnglishList(choices, lastSeparator, quote, oxfordComma)`.
// Java: com/google/devtools/build/lib/util/StringUtil.java:54 StringUtil.joinEnglishList
pub fn join_english_list_full<S: AsRef<str>>(
    choices: &[S],
    last_separator: &str,
    quote: &str,
    oxford_comma: bool,
) -> String {
    let mut buf = String::new();
    let mut num_choices_seen = 0;
    let n = choices.len();
    // PORT: Java's own counter, kept apart from the index.
    #[allow(clippy::explicit_counter_loop)]
    for (i, choice) in choices.iter().enumerate() {
        let has_next = i + 1 < n;
        if !buf.is_empty() {
            if has_next || (oxford_comma && num_choices_seen >= 2) {
                buf.push(',');
            }
            if !has_next {
                buf.push(' ');
                buf.push_str(last_separator);
            }
            buf.push(' ');
        }
        buf.push_str(quote);
        buf.push_str(choice.as_ref());
        buf.push_str(quote);
        num_choices_seen += 1;
    }
    if buf.is_empty() {
        "nothing".to_owned()
    } else {
        buf
    }
}

/// Java `StringUtil.joinEnglishList(choices)`: "a, b, or c".
// Java: com/google/devtools/build/lib/util/StringUtil.java:35 StringUtil.joinEnglishList
pub fn join_english_list<S: AsRef<str>>(choices: &[S]) -> String {
    join_english_list_full(choices, "or", "", true)
}

/// Java `StringUtil.joinEnglishListSingleQuoted(choices)`: "'a', 'b', or 'c'".
// Java: com/google/devtools/build/lib/util/StringUtil.java:86 StringUtil.joinEnglishListSingleQuoted
pub fn join_english_list_single_quoted<S: AsRef<str>>(choices: &[S]) -> String {
    join_english_list_full(choices, "or", "'", true)
}

/// Levenshtein distance bounded by `max_edit_distance` (-1 if larger).
// PORT: Java compares UTF-16 chars; this compares Unicode scalar values (same for ASCII).
// Java: net/starlark/java/spelling/SpellChecker.java SpellChecker.editDistance
pub fn edit_distance(s1: &str, s2: &str, max_edit_distance: i32) -> i32 {
    if s1 == s2 {
        return 0;
    }
    let a: Vec<char> = s1.chars().collect();
    let b: Vec<char> = s2.chars().collect();
    // Short-circuit based on string length.
    if (a.len() as i32 - b.len() as i32).abs() > max_edit_distance {
        return -1;
    }
    let mut row: Vec<i32> = (0..=b.len() as i32).collect();
    for i in 1..=a.len() {
        row[0] = i as i32;
        let mut best_in_the_row = row[0];
        let mut previous = i as i32 - 1;
        for j in 1..=b.len() {
            let old = row[j];
            row[j] = std::cmp::min(
                previous + if a[i - 1] == b[j - 1] { 0 } else { 1 },
                1 + std::cmp::min(row[j - 1], row[j]),
            );
            previous = old;
            best_in_the_row = std::cmp::min(best_in_the_row, row[j]);
        }
        if best_in_the_row > max_edit_distance {
            return -1;
        }
    }
    let result = row[b.len()];
    if result <= max_edit_distance {
        result
    } else {
        -1
    }
}

/// The closest word to `input`, if close enough.
// Java: net/starlark/java/spelling/SpellChecker.java SpellChecker.suggest
pub fn suggest<S: AsRef<str>>(input: &str, words: &[S]) -> Option<String> {
    let mut best: Option<String> = None;
    // Heuristic: the expected number of typos depends on the length of the word.
    let mut best_distance = std::cmp::min(5, (input.chars().count() as i32 + 1) / 2);
    let input = input.to_lowercase();
    for candidate in words {
        let candidate = candidate.as_ref();
        let d = edit_distance(&input, &candidate.to_lowercase(), best_distance);
        if d >= 0 && d < best_distance {
            best_distance = d;
            best = Some(candidate.to_owned());
        }
    }
    best
}

/// `" (did you mean 'x'?)"` or `""`.
// Java: net/starlark/java/spelling/SpellChecker.java SpellChecker.didYouMean
pub fn did_you_mean<S: AsRef<str>>(input: &str, words: &[S]) -> String {
    match suggest(input, words) {
        None => String::new(),
        Some(s) => format!(" (did you mean '{s}'?)"),
    }
}
