//! Port of `analysis/LabelExpander.java`.

use blaze_actions::Artifact;
use blaze_cmdline::Label;
use blaze_cmdline::label::LabelPackageContext;
use indexmap::IndexMap;

/// An exception that is thrown when a label is expanded to zero or multiple files during
/// expansion.
// Java: com/google/devtools/build/lib/analysis/LabelExpander.java:39 LabelExpander.NotUniqueExpansionException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct NotUniqueExpansionException {
    message: String,
}

impl NotUniqueExpansionException {
    // Java: com/google/devtools/build/lib/analysis/LabelExpander.java:40 NotUniqueExpansionException.<init>
    pub fn new(size_of_result_set: usize, label_text: &str) -> NotUniqueExpansionException {
        NotUniqueExpansionException {
            message: format!(
                "heuristic label expansion found '{}', which expands to {} files{}",
                label_text,
                size_of_result_set,
                if size_of_result_set > 1 {
                    format!(", please use $(locations {label_text}) instead")
                } else {
                    String::new()
                }
            ),
        }
    }

    /// Java `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

/// CharMatcher to determine if a given character is valid for labels.
///
/// The Build Concept Reference additionally allows '=' and ',' to appear in labels, but for the
/// purposes of the heuristic, this function does not, as it would cause "--foo=:rule1,:rule2" to
/// scan as a single possible label, instead of three ("--foo", ":rule1", ":rule2").
// Java: com/google/devtools/build/lib/analysis/LabelExpander.java:61 LabelExpander.LABEL_CHAR_MATCHER
fn label_char_matcher(c: char) -> bool {
    c.is_ascii_lowercase()
        || c.is_ascii_uppercase()
        || c.is_ascii_digit()
        // Java: CharMatcher.anyOf(":/_.-+" + PathFragment.SEPARATOR_CHAR)
        || matches!(c, ':' | '/' | '_' | '.' | '-' | '+')
}

/// Expands all references to labels embedded within a string using the provided expansion mapping
/// from labels to artifacts.
///
/// Since this pass is heuristic, references to non-existent labels (such as arbitrary words) or
/// invalid labels are simply ignored and are unchanged in the output. However, if the heuristic
/// discovers a label, which identifies an existing target producing zero or multiple files, an
/// error is reported.
///
/// `expression`: the expression to expand. `label_map`: the mapping from labels to artifacts,
/// whose relative path is to be used as the expansion. `label_resolver`: the `Label` that can
/// resolve label strings to `Label` objects. The resolved label is either relative to
/// `label_resolver` or is a global label (i.e. starts with "//").
///
/// Returns the expansion of the string; fails with `NotUniqueExpansionException` if a label that
/// is present in the mapping expands to zero or multiple files.
// Java: com/google/devtools/build/lib/analysis/LabelExpander.java:87 LabelExpander.expand
pub fn expand(
    expression: Option<&str>,
    label_map: &IndexMap<Label, Vec<Artifact>>,
    label_resolver: &Label,
) -> Result<String, NotUniqueExpansionException> {
    let Some(expression) = expression.filter(|e| !e.is_empty()) else {
        return Ok(String::new());
    };

    // PORT: Java scans UTF-16 code units. Every label character is ASCII and any other unit is
    // copied through unchanged, so scanning Unicode scalar values gives the same string.
    let chars: Vec<char> = expression.chars().collect();
    let mut offset = 0usize;
    let mut result = String::new();
    while offset < chars.len() {
        match scan_label(&chars, offset) {
            Some(label_text) => {
                offset += label_text.chars().count();
                result.push_str(&try_resolving_label_text_to_artifact_path(
                    &label_text,
                    label_map,
                    label_resolver,
                )?);
            }
            None => {
                result.push(chars[offset]);
                offset += 1;
            }
        }
    }
    Ok(result)
}

/// Tries resolving a label text to a full label for the associated `Artifact`, using the
/// provided mapping.
///
/// The method succeeds if the label text can be resolved to a `Label` object, which is present
/// in the `label_map` and maps to exactly one `Artifact`.
///
/// Returns an absolute label to an `Artifact` if the resolving was successful or the original
/// label text.
// Java: com/google/devtools/build/lib/analysis/LabelExpander.java:127 LabelExpander.tryResolvingLabelTextToArtifactPath
fn try_resolving_label_text_to_artifact_path(
    label_text: &str,
    label_map: &IndexMap<Label, Vec<Artifact>>,
    label_resolver: &Label,
) -> Result<String, NotUniqueExpansionException> {
    let Some(resolved_label) = resolve_label_text(label_text, label_resolver) else {
        return Ok(label_text.to_owned());
    };
    let Some(artifacts) = label_map.get(&resolved_label) else {
        return Ok(label_text.to_owned());
    };
    // resolvedLabel identifies an existing target
    let mut locations: Vec<String> = Vec::new();
    for artifact in artifacts {
        if !artifact.is_runfiles_tree() {
            locations.push(artifact.get_exec_path_string().to_owned());
        }
    }
    let result_set_size = locations.len();
    if result_set_size == 1 {
        Ok(locations.remove(0)) // success!
    } else {
        Err(NotUniqueExpansionException::new(
            result_set_size,
            label_text,
        ))
    }
}

/// Resolves a string to a label text. Uses `label_resolver` to do so. The result is either
/// relative to `label_resolver` or is an absolute label. In case of an invalid label text, the
/// return value is `None`.
// Java: com/google/devtools/build/lib/analysis/LabelExpander.java:158 LabelExpander.resolveLabelText
fn resolve_label_text(label_text: &str, label_resolver: &Label) -> Option<Label> {
    // It's a heuristic, so quietly ignore "errors".
    Label::parse_with_package_context(
        label_text,
        &LabelPackageContext::of(
            label_resolver.get_package_identifier(),
            blaze_cmdline::repository_mapping::EMPTY.clone(),
        ),
    )
    .ok()
}

/// Scans the argument string from a given start position until the name of a potential label has
/// been consumed, then returns the label text. If the expression contains no possible label
/// starting at the start position, the return value is `None`.
// Java: com/google/devtools/build/lib/analysis/LabelExpander.java:175 LabelExpander.scanLabel
fn scan_label(expression: &[char], start: usize) -> Option<String> {
    let mut offset = start;
    while offset < expression.len() && label_char_matcher(expression[offset]) {
        offset += 1;
    }
    if offset > start {
        Some(expression[start..offset].iter().collect())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    // Port of src/test/java/com/google/devtools/build/lib/analysis/LabelExpanderTest.java.
    //
    // The Java test analyses `filegroup(name = 'foo', srcs = [...])` in package "foo" and uses
    // its files (source artifacts `foo/<name>`); the artifacts are built directly here.
    use super::*;
    use blaze_actions::{ArtifactOwner, ArtifactRoot};
    use blaze_vfs::{DigestHashFunction, Path, PathFragment, Root, UnixFileSystem};

    /// All characters that the heuristic considers to be part of a target. This is a subset of
    /// the allowed label characters. The ones left out may have a special meaning during
    /// expression expansion: comma (",") and equals sign ("=") may separate labels; colon (":")
    /// can only appear in labels, not in target names.
    const ALLOWED_CHARS: &str =
        "_/.-+/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

    /// The label of `dummyTarget` (`//foo:foo`).
    fn dummy_target_label() -> Label {
        Label::parse_canonical("//foo:foo").unwrap()
    }

    /// Gets an artifact for a target in package "foo" (Java `artifactFor`).
    fn artifact_for(target_name: &str) -> Artifact {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = ArtifactRoot::as_source_root(Root::from_path(Path::create(
            PathFragment::create("/workspace"),
            &fs,
        )));
        Artifact::new_source_artifact(
            root,
            PathFragment::create(&format!("foo/{target_name}")),
            ArtifactOwner::NullOwner,
        )
    }

    /// Creates fake label in package "foo".
    fn label_for(target_name: &str) -> Label {
        Label::create("@//foo", target_name).unwrap()
    }

    fn mapping(entries: &[(&str, Vec<&str>)]) -> IndexMap<Label, Vec<Artifact>> {
        entries
            .iter()
            .map(|(label, artifacts)| {
                (
                    label_for(label),
                    artifacts.iter().map(|a| artifact_for(a)).collect(),
                )
            })
            .collect()
    }

    /// Asserts that an expansion with a given mapping produces the expected results.
    fn assert_expansion(
        expected_result: &str,
        expression_to_expand: Option<&str>,
        mapping: &IndexMap<Label, Vec<Artifact>>,
    ) {
        assert_eq!(
            expand(expression_to_expand, mapping, &dummy_target_label()).unwrap(),
            expected_result
        );
    }

    /// Tests that if no mapping is specified, then strings expand to themselves.
    #[test]
    fn test_string_expands_to_itself_when_no_mapping_specified() {
        let empty = IndexMap::new();
        assert_expansion("", None, &empty);
        assert_expansion("cmd", Some("cmd"), &empty);
        assert_expansion("//x:y,:z,w", Some("//x:y,:z,w"), &empty);
        assert_expansion(ALLOWED_CHARS, Some(ALLOWED_CHARS), &empty);
    }

    /// Tests that in case of a one-to-one label-to-artifact mapping the expansion produces the
    /// expected results.
    #[test]
    fn test_expansion() {
        let m = mapping(&[("x1", vec!["x1"])]);
        assert_expansion("foo/x1", Some("x1"), &m);
        assert_expansion("foo/x1", Some(":x1"), &m);
        assert_expansion("foo/x1", Some("//foo:x1"), &m);
    }

    /// Tests that label extraction works as expected - disallowed label characters are resolved
    /// to themselves.
    #[test]
    fn test_label_extraction() {
        assert_expansion(
            &format!("(foo/{ALLOWED_CHARS})"),
            Some(&format!("(//foo:{ALLOWED_CHARS})")),
            &mapping(&[(ALLOWED_CHARS, vec![ALLOWED_CHARS])]),
        );

        assert_expansion(
            "foo/x1,foo/x2=foo/bar/x3",
            Some("x1,x2=bar/x3"),
            &mapping(&[
                ("x1", vec!["x1"]),
                ("x2", vec!["x2"]),
                ("bar/x3", vec!["bar/x3"]),
            ]),
        );
    }

    /// Tests that an exception is thrown when the mapping is not one-to-one.
    #[test]
    fn test_throws_when_mapping_is_not_one_to_one() {
        assert!(
            expand(
                Some("x1"),
                &mapping(&[("x1", vec![])]),
                &dummy_target_label()
            )
            .is_err()
        );
        assert!(
            expand(
                Some("x1"),
                &mapping(&[("x1", vec!["x1", "x2"])]),
                &dummy_target_label()
            )
            .is_err()
        );
    }

    /// Tests expanding labels that result in a SyntaxException.
    #[test]
    fn test_ill_formed_labels() {
        let three = mapping(&[
            ("x1", vec!["x1"]),
            ("x2", vec!["x2"]),
            ("bar/x3", vec!["bar/x3"]),
        ]);
        let two = mapping(&[("x1", vec!["x1"]), ("x2", vec!["x2"])]);
        let one = mapping(&[("x1", vec!["x1"])]);
        assert_expansion("x1:x2:x3", Some("x1:x2:x3"), &three);
        assert_expansion("foo://x1 x1/../x2", Some("foo://x1 x1/../x2"), &three);
        assert_expansion("//foo:/x1", Some("//foo:/x1"), &one);
        assert_expansion("//foo:../x1", Some("//foo:../x1"), &one);
        assert_expansion("//foo:x1/../x2", Some("//foo:x1/../x2"), &two);
        assert_expansion("//foo:x1/./x2", Some("//foo:x1/./x2"), &two);
        assert_expansion("//foo:x1//x2", Some("//foo:x1//x2"), &two);
        assert_expansion("//foo:x1/..", Some("//foo:x1/.."), &one);
        assert_expansion("//foo:x1/", Some("//foo:x1/"), &one);
        assert_expansion(":", Some(":"), &IndexMap::new());
    }

    /// Tests that label parsing is greedy (always extracting the longest possible label). This
    /// means that if a label is a substring of another label, it should not be expanded but be
    /// treated as part of the longer one.
    #[test]
    fn test_label_is_substring_of_valid_label() {
        assert_expansion(
            "x3=foo/bar/x3",
            Some("x3=bar/x3"),
            &mapping(&[("bar/x3", vec!["bar/x3"])]),
        );

        assert_expansion(
            "foo/x1,foo/x11,foo/xx1,foo/xx11",
            Some("x1,x11,xx1,xx11"),
            &mapping(&[
                ("x1", vec!["x1"]),
                ("x11", vec!["x11"]),
                ("xx1", vec!["xx1"]),
                ("xx11", vec!["xx11"]),
            ]),
        );

        assert_expansion("//x1", Some("//x1"), &mapping(&[("x1", vec!["x1"])]));
    }

    /// The message of `NotUniqueExpansionException` (LabelExpander.java:40), as
    /// `GenRuleCommandSubstitutionTest.heuristicLabelExpansion_*` expect it.
    #[test]
    fn not_unique_expansion_exception_message() {
        assert_eq!(
            NotUniqueExpansionException::new(0, "foo").get_message(),
            "heuristic label expansion found 'foo', which expands to 0 files"
        );
        assert_eq!(
            NotUniqueExpansionException::new(2, "foo").get_message(),
            "heuristic label expansion found 'foo', which expands to 2 files, please use $(locations foo) instead"
        );
    }
}
