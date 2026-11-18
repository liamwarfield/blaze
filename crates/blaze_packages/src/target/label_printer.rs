//! Port of `packages/LabelPrinter.java`: printing labels consistently in various formats.

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;

/// Knows how to print labels (Java `LabelPrinter` and its three implementations).
// Java: com/google/devtools/build/lib/packages/LabelPrinter.java:24 LabelPrinter
#[derive(Debug, Clone)]
pub enum LabelPrinter {
    /// Java `LabelPrinter.starlark(semantics)`: like Starlark `str(label)`.
    // PORT: the only semantics input is --incompatible_unambiguous_label_stringification.
    Starlark {
        incompatible_unambiguous_label_stringification: bool,
    },
    /// Java `LabelPrinter.displayForm(mainRepoMapping)`.
    DisplayForm(RepositoryMapping),
    /// Java `LabelPrinter.legacy()` (`Label.toString()`).
    Legacy,
}

impl LabelPrinter {
    // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:33 LabelPrinter.starlark
    pub fn starlark(semantics: &blaze_starlark::StarlarkSemantics) -> LabelPrinter {
        LabelPrinter::Starlark {
            incompatible_unambiguous_label_stringification: semantics
                .incompatible_unambiguous_label_stringification,
        }
    }

    // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:62 LabelPrinter.displayForm
    pub fn display_form(main_repo_mapping: RepositoryMapping) -> LabelPrinter {
        LabelPrinter::DisplayForm(main_repo_mapping)
    }

    // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:92 LabelPrinter.legacy
    pub fn legacy() -> LabelPrinter {
        LabelPrinter::Legacy
    }

    /// A string representation of `label` (Java `toString(Label)`).
    // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:97 LabelPrinter.toString
    pub fn to_string(&self, label: &Label) -> String {
        match self {
            // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:36 LabelPrinter.starlark.toString
            LabelPrinter::Starlark {
                incompatible_unambiguous_label_stringification,
            } => label.str(*incompatible_unambiguous_label_stringification),
            // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:65 LabelPrinter.displayForm.toString
            LabelPrinter::DisplayForm(main_repo_mapping) => {
                label.get_display_form(Some(main_repo_mapping))
            }
            // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:79 LabelPrinter.LEGACY.toString
            LabelPrinter::Legacy => label.to_string(),
        }
    }

    /// A string representation of a package identifier (Java `toString(PackageIdentifier)`).
    // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:100 LabelPrinter.toString
    pub fn to_string_package(&self, package_identifier: &PackageIdentifier) -> String {
        match self {
            // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:43 LabelPrinter.starlark.toString
            LabelPrinter::Starlark { .. } => {
                // PackageIdentifier is not a StarlarkValue and thus doesn't have a str method.
                // Since it is only used in the context of --output=package, we reuse Label#str by
                // stripping a placeholder name.
                let label = self.to_string(&Label::create_unvalidated(
                    package_identifier.clone(),
                    "unused",
                ));
                label[..label.len() - ":unused".len()].to_owned()
            }
            // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:70 LabelPrinter.displayForm.toString
            LabelPrinter::DisplayForm(main_repo_mapping) => {
                package_identifier.get_display_form(Some(main_repo_mapping))
            }
            // Java: com/google/devtools/build/lib/packages/LabelPrinter.java:84 LabelPrinter.LEGACY.toString
            LabelPrinter::Legacy => package_identifier.to_string(),
        }
    }
}
