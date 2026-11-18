//! Port of `packages/TargetSuggester.java`: suggested targets when a requested target does not
//! exist.

use crate::attribute::string_util::edit_distance;

// Java: com/google/devtools/build/lib/packages/TargetSuggester.java:31 TargetSuggester.MAX_SUGGESTED_TARGETS_SIZE
const MAX_SUGGESTED_TARGETS_SIZE: usize = 10;

// Java: com/google/devtools/build/lib/packages/TargetSuggester.java:33 TargetSuggester.MAX_SUGGESTION_EDIT_DISTANCE
const MAX_SUGGESTION_EDIT_DISTANCE: i32 = 5;

/// The suggestions for `input` among `words`, pretty-printed (`""` or
/// `" (did you mean a, b, or c?)"`).
// Java: com/google/devtools/build/lib/packages/TargetSuggester.java:52 TargetSuggester.suggestTargets
pub fn suggest_targets(input: &str, words: &[&str]) -> String {
    let suggested_targets = suggested_targets(input, words);
    pretty_print_targets(&suggested_targets)
}

/// The targets closest to `input` by edit distance.
// PORT: Java iterates a HashSet of words; the order of equally distant words is the order of
// `words` here (the package's sorted target names).
// Java: com/google/devtools/build/lib/packages/TargetSuggester.java:66 TargetSuggester.suggestedTargets
pub fn suggested_targets(input: &str, words: &[&str]) -> Vec<String> {
    let lower_case_input = input.to_lowercase();

    // Add words based on edit distance
    let mut edit_distance_to_words: Vec<(i32, String)> = Vec::new();

    // PORT: Java String.length counts UTF-16 units; chars here (same for the BMP).
    let input_length = input.chars().count() as i32;
    let max_edit_distance = std::cmp::min(MAX_SUGGESTION_EDIT_DISTANCE, (input_length + 1) / 2);
    for word in words {
        let lower_case_word = word.to_lowercase();

        let edit_distance = edit_distance(&lower_case_input, &lower_case_word, max_edit_distance);

        if edit_distance >= 0 {
            edit_distance_to_words.push((edit_distance, (*word).to_owned()));
        }
    }
    let get = |d: i32| -> Vec<String> {
        edit_distance_to_words
            .iter()
            .filter(|(e, _)| *e == d)
            .map(|(_, w)| w.clone())
            .collect()
    };

    let zero_edit_distance_words = get(0);
    let one_edit_distance_words = get(1);

    if edit_distance_to_words.is_empty() {
        Vec::new()
    } else if !zero_edit_distance_words.is_empty() {
        let sublist_length = zero_edit_distance_words
            .len()
            .min(MAX_SUGGESTED_TARGETS_SIZE);
        zero_edit_distance_words[..sublist_length].to_vec()
    } else if !one_edit_distance_words.is_empty() {
        let sublist_length = one_edit_distance_words
            .len()
            .min(MAX_SUGGESTED_TARGETS_SIZE);
        one_edit_distance_words[..sublist_length].to_vec()
    } else {
        // Java: com/google/devtools/build/lib/packages/TargetSuggester.java:103 TargetSuggester.getSuggestedTargets
        let mut total = 0;
        let mut suggested_targets = Vec::new();
        let mut edit_distance = 2;
        while edit_distance < max_edit_distance && total < MAX_SUGGESTED_TARGETS_SIZE {
            let values = get(edit_distance);
            let add_amount = values.len().min(MAX_SUGGESTED_TARGETS_SIZE - total);
            suggested_targets.extend_from_slice(&values[..add_amount]);
            total += add_amount;
            edit_distance += 1;
        }
        suggested_targets
    }
}

/// `""`, `" (did you mean a?)"` or `" (did you mean a, b, or c?)"`.
// Java: com/google/devtools/build/lib/packages/TargetSuggester.java:126 TargetSuggester.prettyPrintTargets
pub fn pretty_print_targets(targets: &[String]) -> String {
    let target_string = if targets.is_empty() {
        return String::new();
    } else if targets.len() == 1 {
        targets[0].clone()
    } else {
        let first_part = targets[..targets.len() - 1].join(", ");
        format!("{}, or {}", first_part, targets[targets.len() - 1])
    };
    format!(" (did you mean {target_string}?)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TargetSuggesterTest.java:29 TargetSuggesterTest.testRangeDoesntSuggestTarget
    #[test]
    fn test_range_doesnt_suggest_target() {
        assert!(suggested_targets("range", &["target"]).is_empty());
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TargetSuggesterTest.java:40 TargetSuggesterTest.testMisspelledTargetRetrievesProperSuggestion
    #[test]
    fn test_misspelled_target_retrieves_proper_suggestion() {
        let package_targets = ["AndroidTest", "AndroidTest_deploy", "AndroidTest_java"];
        assert_eq!(
            suggested_targets("AnrdiodTest", &package_targets),
            vec!["AndroidTest".to_owned()]
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TargetSuggesterTest.java:54 TargetSuggesterTest.testRetrieveMultipleTargets
    #[test]
    fn test_retrieve_multiple_targets() {
        let package_targets = ["pixel_5_test", "pixel_6_test", "android_2_test"];
        assert_eq!(
            sorted(suggested_targets("pixel_2_test", &package_targets)),
            vec!["pixel_5_test".to_owned(), "pixel_6_test".to_owned()]
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TargetSuggesterTest.java:68 TargetSuggesterTest.testOnlyClosestTargetIsReturned
    #[test]
    fn test_only_closest_target_is_returned() {
        let package_targets = ["pixel_5_test", "pixel_6_test", "android_2_test"];
        assert_eq!(
            suggested_targets("Pixel_5_test", &package_targets),
            vec!["pixel_5_test".to_owned()]
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TargetSuggesterTest.java:82 TargetSuggesterTest.prettyPrintEmpty
    #[test]
    fn pretty_print_empty() {
        assert_eq!(pretty_print_targets(&[]), "");
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TargetSuggesterTest.java:88 TargetSuggesterTest.prettyPrintSingleTarget_returnsSingleTarget
    #[test]
    fn pretty_print_single_target_returns_single_target() {
        assert_eq!(
            pretty_print_targets(&["pixel_5_test".to_owned()]),
            " (did you mean pixel_5_test?)"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/packages/TargetSuggesterTest.java:95 TargetSuggesterTest.prettyPrintMultipleTargets_returnsMultipleTargets
    #[test]
    fn pretty_print_multiple_targets_returns_multiple_targets() {
        assert_eq!(
            pretty_print_targets(&["pixel_5_test".to_owned(), "pixel_6_test".to_owned()]),
            " (did you mean pixel_5_test, or pixel_6_test?)"
        );
    }
}
