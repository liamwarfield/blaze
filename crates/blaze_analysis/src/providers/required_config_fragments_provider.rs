//! Port of `analysis/RequiredConfigFragmentsProvider.java`.

use std::fmt;

use blaze_cmdline::Label;
use indexmap::IndexSet;

use crate::provider_map::TransitiveInfoProvider;

/// Provides a user-friendly list of the configuration fragments and options a rule requires.
///
/// PORT: Java keys fragments and options by `Class`; Blaze names them by Java simple class name
/// (how `ConfigurationFragmentPolicy` and `FragmentOptions` are named in `blaze_build_config`).
// Java: com/google/devtools/build/lib/analysis/RequiredConfigFragmentsProvider.java:52 RequiredConfigFragmentsProvider
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RequiredConfigFragmentsProvider {
    options_classes: IndexSet<String>,
    fragment_classes: IndexSet<String>,
    defines: IndexSet<String>,
    starlark_options: IndexSet<Label>,
}

impl RequiredConfigFragmentsProvider {
    /// Java `RequiredConfigFragmentsProvider.EMPTY`.
    pub fn empty() -> RequiredConfigFragmentsProvider {
        RequiredConfigFragmentsProvider::default()
    }

    pub fn options_classes(&self) -> &IndexSet<String> {
        &self.options_classes
    }

    pub fn fragment_classes(&self) -> &IndexSet<String> {
        &self.fragment_classes
    }

    pub fn defines(&self) -> &IndexSet<String> {
        &self.defines
    }

    pub fn starlark_options(&self) -> &IndexSet<Label> {
        &self.starlark_options
    }

    /// Merges the values of one or more `RequiredConfigFragmentsProvider` instances.
    // Java: com/google/devtools/build/lib/analysis/RequiredConfigFragmentsProvider.java:93 RequiredConfigFragmentsProvider.merge
    pub fn merge(providers: &[RequiredConfigFragmentsProvider]) -> RequiredConfigFragmentsProvider {
        if providers.is_empty() {
            return RequiredConfigFragmentsProvider::empty();
        }
        let empty = RequiredConfigFragmentsProvider::empty();
        let mut merged: Option<RequiredConfigFragmentsProviderBuilder> = None;
        let mut candidate = empty.clone();
        for provider in providers {
            if *provider == empty {
                continue;
            }
            if let Some(m) = merged.as_mut() {
                m.merge(Some(provider));
            } else if candidate == empty {
                candidate = provider.clone();
            } else {
                let mut b = RequiredConfigFragmentsProvider::builder();
                b.merge(Some(&candidate)).merge(Some(provider));
                merged = Some(b);
            }
        }
        match merged {
            None => candidate,
            Some(m) => m.build(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RequiredConfigFragmentsProvider.java:116 RequiredConfigFragmentsProvider.builder
    pub fn builder() -> RequiredConfigFragmentsProviderBuilder {
        RequiredConfigFragmentsProviderBuilder::default()
    }
}

impl fmt::Debug for RequiredConfigFragmentsProvider {
    // Java: com/google/devtools/build/lib/analysis/RequiredConfigFragmentsProvider.java:76 RequiredConfigFragmentsProvider.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let join = |s: &IndexSet<String>| s.iter().cloned().collect::<Vec<_>>().join(", ");
        write!(
            f,
            "RequiredConfigFragmentsProvider{{optionsClasses=[{}], fragmentClasses=[{}], defines=[{}], starlarkOptions=[{}]}}",
            join(&self.options_classes),
            join(&self.fragment_classes),
            join(&self.defines),
            self.starlark_options
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl TransitiveInfoProvider for RequiredConfigFragmentsProvider {
    fn java_class_name(&self) -> &'static str {
        "RequiredConfigFragmentsProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Builder for required config fragments.
///
/// PORT: Java's builder switches from the `ImmutableSet` it was given to a `HashSet` on the
/// first addition (an allocation optimization; iteration order of the result is then hash
/// order). Blaze keeps insertion order.
// Java: com/google/devtools/build/lib/analysis/RequiredConfigFragmentsProvider.java:121 RequiredConfigFragmentsProvider.Builder
#[derive(Default)]
pub struct RequiredConfigFragmentsProviderBuilder {
    options_classes: IndexSet<String>,
    fragment_classes: IndexSet<String>,
    defines: IndexSet<String>,
    starlark_options: IndexSet<Label>,
}

impl RequiredConfigFragmentsProviderBuilder {
    pub fn add_options_class(&mut self, options_class: &str) -> &mut Self {
        self.options_classes.insert(options_class.to_owned());
        self
    }

    pub fn add_options_classes<'a>(
        &mut self,
        classes: impl IntoIterator<Item = &'a str>,
    ) -> &mut Self {
        self.options_classes
            .extend(classes.into_iter().map(str::to_owned));
        self
    }

    pub fn add_fragment_classes<'a>(
        &mut self,
        classes: impl IntoIterator<Item = &'a str>,
    ) -> &mut Self {
        self.fragment_classes
            .extend(classes.into_iter().map(str::to_owned));
        self
    }

    pub fn add_define(&mut self, define: &str) -> &mut Self {
        self.defines.insert(define.to_owned());
        self
    }

    pub fn add_defines<'a>(&mut self, defines: impl IntoIterator<Item = &'a str>) -> &mut Self {
        self.defines.extend(defines.into_iter().map(str::to_owned));
        self
    }

    pub fn add_starlark_option(&mut self, option: Label) -> &mut Self {
        self.starlark_options.insert(option);
        self
    }

    pub fn add_starlark_options(&mut self, options: impl IntoIterator<Item = Label>) -> &mut Self {
        self.starlark_options.extend(options);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RequiredConfigFragmentsProvider.java:175 RequiredConfigFragmentsProvider.Builder.merge
    pub fn merge(&mut self, provider: Option<&RequiredConfigFragmentsProvider>) -> &mut Self {
        if let Some(p) = provider {
            self.options_classes
                .extend(p.options_classes.iter().cloned());
            self.fragment_classes
                .extend(p.fragment_classes.iter().cloned());
            self.defines.extend(p.defines.iter().cloned());
            self.starlark_options
                .extend(p.starlark_options.iter().cloned());
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RequiredConfigFragmentsProvider.java:227 RequiredConfigFragmentsProvider.Builder.build
    pub fn build(&self) -> RequiredConfigFragmentsProvider {
        RequiredConfigFragmentsProvider {
            options_classes: self.options_classes.clone(),
            fragment_classes: self.fragment_classes.clone(),
            defines: self.defines.clone(),
            starlark_options: self.starlark_options.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    // Ports of src/test/java/com/google/devtools/build/lib/analysis/RequiredConfigFragmentsProviderTest.java.
    use super::*;

    #[test]
    fn merge_empty_list_returns_empty() {
        assert_eq!(
            RequiredConfigFragmentsProvider::merge(&[]),
            RequiredConfigFragmentsProvider::empty()
        );
    }

    #[test]
    fn merge_single_returns_same() {
        let mut b = RequiredConfigFragmentsProvider::builder();
        b.add_define("a");
        let p = b.build();
        assert_eq!(
            RequiredConfigFragmentsProvider::merge(&[
                RequiredConfigFragmentsProvider::empty(),
                p.clone()
            ]),
            p
        );
    }

    #[test]
    fn merge_multiple() {
        let mut b1 = RequiredConfigFragmentsProvider::builder();
        b1.add_define("a")
            .add_fragment_classes(["CppConfiguration"]);
        let mut b2 = RequiredConfigFragmentsProvider::builder();
        b2.add_define("b");
        let merged = RequiredConfigFragmentsProvider::merge(&[b1.build(), b2.build()]);
        assert_eq!(
            merged.defines().iter().cloned().collect::<Vec<_>>(),
            vec!["a".to_owned(), "b".to_owned()]
        );
        assert!(merged.fragment_classes().contains("CppConfiguration"));
    }
}
