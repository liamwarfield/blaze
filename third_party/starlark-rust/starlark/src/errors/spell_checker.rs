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

//! BLAZE PATCH 0049: Bazel's spelling suggestions.
//!
//! Java: net/starlark/java/spelling/SpellChecker.java (editDistance, suggest, didYouMean), and
//! the iteration order of a `java.util.HashSet<String>`, which decides between equally good
//! candidates where Java passes a hash set (Resolver.getAllSymbols).

/// Java `String.toLowerCase()` for one char (`Character.toLowerCase`). For U+0000..U+00FF (every
/// char of an internal string) this is Java's table: `A`-`Z` and U+00C0..U+00DE except U+00D7.
fn java_to_lower_case(c: char) -> char {
    let u = c as u32;
    if c.is_ascii_uppercase() || ((0xC0..=0xDE).contains(&u) && u != 0xD7) {
        return char::from_u32(u + 0x20).unwrap_or(c);
    }
    if u <= 0xFF {
        return c;
    }
    // PORT: above U+00FF (not reachable from Blaze's internal strings) Rust's single-char
    // mapping stands in for Character.toLowerCase.
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

fn java_lower(s: &str) -> Vec<char> {
    s.chars().map(java_to_lower_case).collect()
}

/// Java `SpellChecker.editDistance`: the Levenshtein distance, or -1 when it would exceed
/// `max_edit_distance`. Strings are compared char by char (Java `charAt`).
// Java: net/starlark/java/spelling/SpellChecker.java:33 SpellChecker.editDistance
pub fn edit_distance(s1: &[char], s2: &[char], max_edit_distance: i32) -> i32 {
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
    if result <= max_edit_distance { result } else { -1 }
}

/// Java `SpellChecker.suggest`: the word most similar to `input` (edit distance, ignoring
/// case), or `None` if none is similar enough. In case of equality, the first one wins.
// Java: net/starlark/java/spelling/SpellChecker.java:89 SpellChecker.suggest
pub fn suggest<'a>(input: &str, words: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best = None;
    // Heuristic: the expected number of typos depends on the length of the word.
    let mut best_distance = std::cmp::min(5, (input.chars().count() as i32 + 1) / 2);
    let input = java_lower(input);
    for candidate in words {
        let d = edit_distance(&input, &java_lower(candidate), best_distance);
        if d >= 0 && d < best_distance {
            best_distance = d;
            best = Some(candidate);
        }
    }
    best
}

/// Java `SpellChecker.didYouMean`: `""` or ` (did you mean 'x'?)`.
// Java: net/starlark/java/spelling/SpellChecker.java:107 SpellChecker.didYouMean
pub fn did_you_mean<'a>(input: &str, words: impl IntoIterator<Item = &'a str>) -> String {
    match suggest(input, words) {
        None => String::new(),
        Some(s) => format!(" (did you mean '{s}'?)"),
    }
}

/// Java `String.hashCode` (over UTF-16 code units, wrapping).
pub fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    let mut buf = [0u16; 2];
    for c in s.chars() {
        for u in c.encode_utf16(&mut buf) {
            h = h.wrapping_mul(31).wrapping_add(*u as i32);
        }
    }
    h
}

/// The iteration order of a `java.util.HashSet<String>` created empty and filled by `add` in
/// the order of `words` (duplicates dropped): buckets in index order, each bucket in insertion
/// order. Java: `HashMap.hash` spreads `hashCode ^ (hashCode >>> 16)`; the table starts at 16
/// buckets and doubles when the size exceeds 3/4 of it; a resize keeps the relative order of a
/// bucket's entries.
///
/// PORT: a bucket of 8 or more entries in a table of 64 or more becomes a tree in Java, whose
/// order differs; that needs 8 names with colliding hashes and is not modelled.
// Java: java/util/HashMap.java hash, resize, putVal; java/util/HashSet.java iterator
pub fn java_hash_set_order(words: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<String> = words
        .into_iter()
        .filter(|w| seen.insert(w.clone()))
        .collect();
    let mut cap: usize = 16;
    while unique.len() > cap * 3 / 4 {
        cap *= 2;
    }
    let mut keyed: Vec<(usize, usize, String)> = unique
        .into_iter()
        .enumerate()
        .map(|(i, w)| {
            let h = java_string_hash_code(&w) as u32;
            let spread = h ^ (h >> 16);
            ((spread as usize) & (cap - 1), i, w)
        })
        .collect();
    keyed.sort_by_key(|(b, i, _)| (*b, *i));
    keyed.into_iter().map(|(_, _, w)| w).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_java() {
        let d = |a: &str, b: &str, m| {
            edit_distance(
                &a.chars().collect::<Vec<_>>(),
                &b.chars().collect::<Vec<_>>(),
                m,
            )
        };
        // Java: src/test/java/net/starlark/java/spelling/SpellCheckerTest.java
        assert_eq!(d("abc", "abc", 2), 0);
        assert_eq!(d("abc", "abd", 2), 1);
        assert_eq!(d("abc", "", 2), -1);
        assert_eq!(d("abc", "", 3), 3);
        assert_eq!(d("kitten", "sitting", 3), 3);
        assert_eq!(d("kitten", "sitting", 2), -1);
    }

    #[test]
    fn suggest_java() {
        assert_eq!(suggest("lenn", ["len", "list"]), Some("len"));
        assert_eq!(suggest("x", ["y"]), None);
        assert_eq!(suggest("ab", ["abc"]), None);
        assert_eq!(suggest("FOO", ["foo"]), Some("foo"));
        assert_eq!(did_you_mean("lenn", ["len"]), " (did you mean 'len'?)");
    }

    #[test]
    fn java_hash_codes() {
        assert_eq!(java_string_hash_code(""), 0);
        assert_eq!(java_string_hash_code("a"), 97);
        assert_eq!(java_string_hash_code("hello"), 99162322);
    }
}
