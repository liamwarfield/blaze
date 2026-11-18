//! Port of `net.starlark.java.spelling.SpellChecker`.

/// Spell checking: detect typos and make suggestions.
// Java: net/starlark/java/spelling/SpellChecker.java:24 SpellChecker
pub struct SpellChecker;

impl SpellChecker {
    // Java: net/starlark/java/spelling/SpellChecker.java:35 SpellChecker.editDistance
    /// Levenshtein distance between `s1` and `s2`, or -1 if it exceeds `max_edit_distance`.
    /// Works on UTF-16 code units like Java's `charAt`.
    pub fn edit_distance(s1: &str, s2: &str, max_edit_distance: i32) -> i32 {
        if s1 == s2 {
            return 0;
        }
        let s1: Vec<u16> = s1.encode_utf16().collect();
        let s2: Vec<u16> = s2.encode_utf16().collect();
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
                row[j] = (previous + if s1[i - 1] == s2[j - 1] { 0 } else { 1 })
                    .min(1 + row[j - 1].min(row[j]));
                previous = old;
                best_in_the_row = best_in_the_row.min(row[j]);
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

    // Java: net/starlark/java/spelling/SpellChecker.java:78 SpellChecker.suggest
    /// The word most similar to `input` (ignoring case), or `None`; ties go to the first.
    pub fn suggest<I: IntoIterator<Item = S>, S: AsRef<str>>(
        input: &str,
        words: I,
    ) -> Option<String> {
        let mut best: Option<String> = None;
        let len = input.encode_utf16().count() as i32;
        let mut best_distance = 5.min((len + 1) / 2);
        let input = input.to_lowercase();
        for candidate in words {
            let candidate = candidate.as_ref();
            let d = Self::edit_distance(&input, &candidate.to_lowercase(), best_distance);
            if d >= 0 && d < best_distance {
                best_distance = d;
                best = Some(candidate.to_string());
            }
        }
        best
    }

    // Java: net/starlark/java/spelling/SpellChecker.java:97 SpellChecker.didYouMean
    /// Either "" or a suggestion such as ` (did you mean 'x'?)`.
    pub fn did_you_mean<I: IntoIterator<Item = S>, S: AsRef<str>>(input: &str, words: I) -> String {
        match Self::suggest(input, words) {
            None => String::new(),
            Some(s) => format!(" (did you mean '{}'?)", s),
        }
    }
}
