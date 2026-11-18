//! Port of `packages/RequiredProviders.java`.

use std::fmt;

use indexmap::IndexSet;

use crate::rule_class::AdvertisedProviderSet;
use crate::rule_class::StarlarkProviderIdentifier;

/// Java `RequiredProviders.Constraint`.
// Java: com/google/devtools/build/lib/packages/RequiredProviders.java:80 RequiredProviders.Constraint
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequiredProvidersConstraint {
    /// Accepts any set of providers.
    Any,
    /// Accepts no set of providers.
    None,
    /// Accepts a set that satisfies one of the required sets.
    Restricted,
}

/// Providers a dependency must advertise: a disjunction of conjunctive sets of builtin
/// (named by Java class simple name) and Starlark providers (Java `RequiredProviders`).
// Java: com/google/devtools/build/lib/packages/RequiredProviders.java:44 RequiredProviders
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredProviders {
    constraint: RequiredProvidersConstraint,
    builtin_providers: Vec<IndexSet<String>>,
    starlark_providers: Vec<IndexSet<StarlarkProviderIdentifier>>,
}

impl RequiredProviders {
    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:57 RequiredProviders.getDescription
    pub fn get_description(&self) -> String {
        match self.constraint {
            RequiredProvidersConstraint::Any => "no providers required".to_owned(),
            RequiredProvidersConstraint::None => "no providers accepted".to_owned(),
            RequiredProvidersConstraint::Restricted => {
                let mut result = String::new();
                describe(&mut result, &self.builtin_providers, |c| c.clone());
                describe(&mut result, &self.starlark_providers, |id| {
                    format!("'{id}'")
                });
                result
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:84 RequiredProviders.Constraint.satisfies
    fn satisfies(
        &self,
        has_builtin_provider: &dyn Fn(&String) -> bool,
        has_starlark_provider: &dyn Fn(&StarlarkProviderIdentifier) -> bool,
        mut missing_providers: Option<&mut RequiredProvidersBuilder>,
    ) -> bool {
        match self.constraint {
            RequiredProvidersConstraint::Any => true,
            RequiredProvidersConstraint::None => false,
            RequiredProvidersConstraint::Restricted => {
                for builtin_provider_set in &self.builtin_providers {
                    if builtin_provider_set.iter().all(has_builtin_provider) {
                        return true;
                    }
                    // Collect missing providers
                    if let Some(m) = missing_providers.as_deref_mut() {
                        m.add_builtin_set_unchecked(
                            builtin_provider_set
                                .iter()
                                .filter(|p| !has_builtin_provider(p))
                                .cloned()
                                .collect(),
                        );
                    }
                }
                for starlark_provider_set in &self.starlark_providers {
                    if starlark_provider_set.iter().all(has_starlark_provider) {
                        return true;
                    }
                    // Collect missing providers
                    if let Some(m) = missing_providers.as_deref_mut() {
                        m.add_starlark_set_unchecked(
                            starlark_provider_set
                                .iter()
                                .filter(|p| !has_starlark_provider(p))
                                .cloned()
                                .collect(),
                        );
                    }
                }
                false
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:113 RequiredProviders.Constraint.RESTRICTED.satisfies
    fn satisfies_set(
        &self,
        advertised_provider_set: &AdvertisedProviderSet,
        missing: Option<&mut RequiredProvidersBuilder>,
    ) -> bool {
        match self.constraint {
            RequiredProvidersConstraint::Any => true,
            RequiredProvidersConstraint::None => false,
            RequiredProvidersConstraint::Restricted => {
                if advertised_provider_set.can_have_any_provider() {
                    return true;
                }
                self.satisfies(
                    &|p| advertised_provider_set.get_builtin_providers().contains(p),
                    &|p| advertised_provider_set.get_starlark_providers().contains(p),
                    missing,
                )
            }
        }
    }

    /// Java `isSatisfiedBy(Predicate, Predicate)`.
    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:248 RequiredProviders.isSatisfiedBy
    pub fn is_satisfied_by_predicates(
        &self,
        has_builtin_provider: &dyn Fn(&String) -> bool,
        has_starlark_provider: &dyn Fn(&StarlarkProviderIdentifier) -> bool,
    ) -> bool {
        self.satisfies(has_builtin_provider, has_starlark_provider, None)
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:74 RequiredProviders.getStarlarkProviders
    pub fn get_starlark_providers(&self) -> &[IndexSet<StarlarkProviderIdentifier>] {
        &self.starlark_providers
    }

    pub fn get_builtin_providers(&self) -> &[IndexSet<String>] {
        &self.builtin_providers
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:240 RequiredProviders.isSatisfiedBy
    pub fn is_satisfied_by(&self, advertised_provider_set: &AdvertisedProviderSet) -> bool {
        self.satisfies_set(advertised_provider_set, None)
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:273 RequiredProviders.getMissing
    pub fn get_missing(&self, set: &AdvertisedProviderSet) -> RequiredProviders {
        let mut builder = RequiredProviders::accept_any_builder();
        if self.satisfies_set(set, Some(&mut builder)) {
            // Ignore all collected missing providers.
            return RequiredProviders::accept_any_builder().build();
        }
        builder.build()
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:283 RequiredProviders.acceptsAny
    pub fn accepts_any(&self) -> bool {
        self.constraint == RequiredProvidersConstraint::Any
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:288 RequiredProviders.acceptsNone
    pub fn accepts_none(&self) -> bool {
        self.constraint == RequiredProvidersConstraint::None
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:346 RequiredProviders.acceptAnyBuilder
    pub fn accept_any_builder() -> RequiredProvidersBuilder {
        RequiredProvidersBuilder::new(false)
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:354 RequiredProviders.acceptNoneBuilder
    pub fn accept_none_builder() -> RequiredProvidersBuilder {
        RequiredProvidersBuilder::new(true)
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:359 RequiredProviders.copyAsBuilder
    pub fn copy_as_builder(&self) -> RequiredProvidersBuilder {
        // Java: Constraint.copyAsBuilder (ANY / NONE: a fresh builder; RESTRICTED: all sets).
        let mut builder =
            RequiredProvidersBuilder::new(self.constraint == RequiredProvidersConstraint::None);
        if self.constraint == RequiredProvidersConstraint::Restricted {
            for set in &self.builtin_providers {
                builder.add_builtin_set(set.iter().cloned().collect());
            }
            for set in &self.starlark_providers {
                builder.add_starlark_set(set.iter().cloned().collect());
            }
        }
        builder
    }
}

impl fmt::Display for RequiredProviders {
    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:62 RequiredProviders.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_description())
    }
}

/// Java `RequiredProviders.Builder`.
// Java: com/google/devtools/build/lib/packages/RequiredProviders.java:364 RequiredProviders.Builder
#[derive(Debug, Clone)]
pub struct RequiredProvidersBuilder {
    builtin_providers: Vec<IndexSet<String>>,
    starlark_providers: Vec<IndexSet<StarlarkProviderIdentifier>>,
    constraint: RequiredProvidersConstraint,
}

impl RequiredProvidersBuilder {
    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:370 RequiredProviders.Builder.<init>
    fn new(accept_none: bool) -> Self {
        RequiredProvidersBuilder {
            builtin_providers: Vec::new(),
            starlark_providers: Vec::new(),
            constraint: if accept_none {
                RequiredProvidersConstraint::None
            } else {
                RequiredProvidersConstraint::Any
            },
        }
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:383 RequiredProviders.Builder.addStarlarkSet
    pub fn add_starlark_set(
        &mut self,
        starlark_provider_set: Vec<StarlarkProviderIdentifier>,
    ) -> &mut Self {
        self.constraint = RequiredProvidersConstraint::Restricted;
        blaze_util::check_state!(!starlark_provider_set.is_empty());
        self.starlark_providers
            .push(starlark_provider_set.into_iter().collect());
        self
    }

    /// Builtin providers are named by their Java class simple name.
    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:397 RequiredProviders.Builder.addBuiltinSet
    pub fn add_builtin_set(&mut self, builtin_provider_set: Vec<String>) -> &mut Self {
        self.constraint = RequiredProvidersConstraint::Restricted;
        blaze_util::check_state!(!builtin_provider_set.is_empty());
        self.builtin_providers
            .push(builtin_provider_set.into_iter().collect());
        self
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:397 RequiredProviders.Builder.addBuiltinSet
    // PORT: Java's addBuiltinSet checks non-emptiness, which getMissing relies on never failing
    // (missing sets are non-empty when the set is unsatisfied).
    fn add_builtin_set_unchecked(&mut self, set: IndexSet<String>) {
        self.constraint = RequiredProvidersConstraint::Restricted;
        blaze_util::check_state!(!set.is_empty());
        self.builtin_providers.push(set);
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:383 RequiredProviders.Builder.addStarlarkSet
    fn add_starlark_set_unchecked(&mut self, set: IndexSet<StarlarkProviderIdentifier>) {
        self.constraint = RequiredProvidersConstraint::Restricted;
        blaze_util::check_state!(!set.is_empty());
        self.starlark_providers.push(set);
    }

    // Java: com/google/devtools/build/lib/packages/RequiredProviders.java:405 RequiredProviders.Builder.build
    pub fn build(&self) -> RequiredProviders {
        RequiredProviders {
            constraint: self.constraint,
            builtin_providers: self.builtin_providers.clone(),
            starlark_providers: self.starlark_providers.clone(),
        }
    }
}

// Java: com/google/devtools/build/lib/packages/RequiredProviders.java:308 RequiredProviders.describe
fn describe<T>(
    result: &mut String,
    list_of_sets: &[IndexSet<T>],
    describe_one: impl Fn(&T) -> String,
) {
    for ids in list_of_sets {
        if !result.is_empty() {
            result.push_str(" or ");
        }
        if ids.len() > 1 {
            result.push('[');
        }
        result.push_str(&ids.iter().map(&describe_one).collect::<Vec<_>>().join(", "));
        if ids.len() > 1 {
            result.push(']');
        }
    }
}
