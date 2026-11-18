//! Port of `packages/AdvertisedProviderSet.java`.

use std::fmt;

use indexmap::IndexSet;

use crate::rule_class::StarlarkProviderIdentifier;

/// The providers a rule advertises (Java `AdvertisedProviderSet`). Builtin providers are named
/// by their Java class simple name.
// Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:38 AdvertisedProviderSet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvertisedProviderSet {
    can_have_any_provider: bool,
    builtin_providers: IndexSet<String>,
    starlark_providers: IndexSet<StarlarkProviderIdentifier>,
}

impl AdvertisedProviderSet {
    /// Java `AdvertisedProviderSet.ANY`.
    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:53 AdvertisedProviderSet.ANY
    pub fn any() -> Self {
        AdvertisedProviderSet {
            can_have_any_provider: true,
            builtin_providers: IndexSet::new(),
            starlark_providers: IndexSet::new(),
        }
    }

    /// Java `AdvertisedProviderSet.EMPTY`.
    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:57 AdvertisedProviderSet.EMPTY
    pub fn empty() -> Self {
        AdvertisedProviderSet {
            can_have_any_provider: false,
            builtin_providers: IndexSet::new(),
            starlark_providers: IndexSet::new(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:60 AdvertisedProviderSet.create
    pub fn create(
        builtin_providers: IndexSet<String>,
        starlark_providers: IndexSet<StarlarkProviderIdentifier>,
    ) -> Self {
        if builtin_providers.is_empty() && starlark_providers.is_empty() {
            return AdvertisedProviderSet::empty();
        }
        AdvertisedProviderSet {
            can_have_any_provider: false,
            builtin_providers,
            starlark_providers,
        }
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:104 AdvertisedProviderSet.canHaveAnyProvider
    pub fn can_have_any_provider(&self) -> bool {
        self.can_have_any_provider
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:109 AdvertisedProviderSet.getBuiltinProviders
    pub fn get_builtin_providers(&self) -> &IndexSet<String> {
        &self.builtin_providers
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:114 AdvertisedProviderSet.getStarlarkProviders
    pub fn get_starlark_providers(&self) -> &IndexSet<StarlarkProviderIdentifier> {
        &self.starlark_providers
    }

    /// Builtin providers are fingerprinted by their Java canonical class name; here they are
    /// named by simple name, so builtin entries differ from Java's fingerprint.
    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:134 AdvertisedProviderSet.fingerprint
    pub fn fingerprint(&self, fp: &mut blaze_util::Fingerprint) {
        fp.add_boolean(self.can_have_any_provider);
        // #builtinProviders and #starlarkProviders are ordered according to the calls to the
        // builder methods, and that order is assumed to be deterministic.
        for clazz in &self.builtin_providers {
            fp.add_string(clazz);
        }
        for starlark_provider in &self.starlark_providers {
            starlark_provider.fingerprint(fp);
        }
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:142 AdvertisedProviderSet.builder
    pub fn builder() -> AdvertisedProviderSetBuilder {
        AdvertisedProviderSetBuilder::new()
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:150 AdvertisedProviderSet.advertises
    pub fn advertises(&self, starlark_provider: &StarlarkProviderIdentifier) -> bool {
        if self.can_have_any_provider() {
            return true;
        }
        self.starlark_providers.contains(starlark_provider)
    }
}

impl fmt::Display for AdvertisedProviderSet {
    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:91 AdvertisedProviderSet.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.can_have_any_provider() {
            return f.write_str("Any Provider");
        }
        // PORT: Java prints the builtin provider Class objects ("[class a.b.C, ...]"); they are
        // named by simple name here.
        write!(
            f,
            "allowed built-in providers=[{}], allowed Starlark providers=[{}]",
            self.builtin_providers
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", "),
            self.starlark_providers
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Java `AdvertisedProviderSet.Builder`.
// Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:158 AdvertisedProviderSet.Builder
#[derive(Debug, Clone)]
pub struct AdvertisedProviderSetBuilder {
    can_have_any_provider: bool,
    builtin_providers: IndexSet<String>,
    starlark_providers: IndexSet<StarlarkProviderIdentifier>,
}

impl AdvertisedProviderSetBuilder {
    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:163 AdvertisedProviderSet.Builder.<init>
    fn new() -> Self {
        AdvertisedProviderSetBuilder {
            can_have_any_provider: false,
            builtin_providers: IndexSet::new(),
            starlark_providers: IndexSet::new(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:170 AdvertisedProviderSet.Builder.addParent
    pub fn add_parent(&mut self, parent_set: &AdvertisedProviderSet) -> &mut Self {
        blaze_util::check_state!(
            !self.can_have_any_provider,
            "Alias rules inherit from no other rules"
        );
        blaze_util::check_state!(
            !parent_set.can_have_any_provider(),
            "Cannot inherit from alias rules"
        );
        self.builtin_providers
            .extend(parent_set.get_builtin_providers().iter().cloned());
        self.starlark_providers
            .extend(parent_set.get_starlark_providers().iter().cloned());
        self
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:180 AdvertisedProviderSet.Builder.addBuiltin
    pub fn add_builtin(&mut self, builtin_provider: &str) -> &mut Self {
        self.builtin_providers.insert(builtin_provider.to_owned());
        self
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:185 AdvertisedProviderSet.Builder.canHaveAnyProvider
    pub fn can_have_any_provider(&mut self) {
        blaze_util::check_state!(
            self.builtin_providers.is_empty() && self.starlark_providers.is_empty()
        );
        self.can_have_any_provider = true;
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:190 AdvertisedProviderSet.Builder.build
    pub fn build(&self) -> AdvertisedProviderSet {
        if self.can_have_any_provider {
            blaze_util::check_state!(
                self.builtin_providers.is_empty() && self.starlark_providers.is_empty()
            );
            return AdvertisedProviderSet::any();
        }
        AdvertisedProviderSet::create(
            self.builtin_providers.clone(),
            self.starlark_providers.clone(),
        )
    }

    // Java: com/google/devtools/build/lib/packages/AdvertisedProviderSet.java:200 AdvertisedProviderSet.Builder.addStarlark
    pub fn add_starlark(&mut self, id: StarlarkProviderIdentifier) -> &mut Self {
        self.starlark_providers.insert(id);
        self
    }
}
