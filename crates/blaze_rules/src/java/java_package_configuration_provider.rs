//! `rules/java/JavaPackageConfigurationProvider.java`: a provider for Java per-package
//! configuration (a wrapped `JavaPackageConfigurationInfo` of rules_java).

use blaze_actions::Artifact;
use blaze_analysis::providers::package_specification_provider::PackageSpecificationProvider;
use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_packages::rule_class::ProviderKey;
use starlark::values::{Heap, Value, ValueLike};

use super::java_helper::tokenize_java_options_nested_set;
use super::provider_util::{is_struct_impl, rules_java_provider_key, starlark_type};
use super::starlark_info_wrapper::StarlarkInfoWrapper;
use crate::cpp::util::errorf;

/// `JavaPackageConfigurationProvider.PROVIDER.getKey()`.
// Java: com/google/devtools/build/lib/rules/java/JavaPackageConfigurationProvider.java:107 JavaPackageConfigurationProvider.Provider.<init>
// Java: com/google/devtools/build/lib/rules/java/JavaPackageConfigurationProvider.java:115 JavaPackageConfigurationProvider.Provider.<init>
pub fn provider_key() -> ProviderKey {
    rules_java_provider_key(
        "java/common/rules:java_package_configuration.bzl",
        "JavaPackageConfigurationInfo",
    )
}

/// A provider for Java per-package configuration.
// Java: com/google/devtools/build/lib/rules/java/JavaPackageConfigurationProvider.java:38 JavaPackageConfigurationProvider
#[derive(Clone, Copy)]
pub struct JavaPackageConfigurationProvider<'v> {
    underlying: StarlarkInfoWrapper<'v>,
}

impl<'v> JavaPackageConfigurationProvider<'v> {
    /// `target.get(PROVIDER)`: the wrapped provider of `target`, `None` if it has none.
    // Java: com/google/devtools/build/lib/rules/java/JavaPackageConfigurationProvider.java:53 JavaPackageConfigurationProvider.get
    pub fn get(
        target: &blaze_analysis::configured_target::ConfiguredTarget,
        eval: &starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Option<Self>> {
        match blaze_analysis::configuredtargets::get_info(target, &provider_key()) {
            None => Ok(None),
            Some(info) => {
                Self::wrap(eval.heap().access_owned_frozen_value(&info), eval.heap()).map(Some)
            }
        }
    }

    /// `PROVIDER.wrap(value)`.
    // Java: com/google/devtools/build/lib/rules/java/JavaPackageConfigurationProvider.java:124 JavaPackageConfigurationProvider.Provider.wrap
    pub fn wrap(value: Value<'v>, heap: Heap<'v>) -> starlark::Result<Self> {
        if is_struct_impl(value) {
            return Ok(JavaPackageConfigurationProvider {
                underlying: StarlarkInfoWrapper::new(value, heap),
            });
        }
        Err(errorf(format!(
            "expected an instance of JavaPackageConfigurationProvider, got: {}",
            starlark_type(value)
        )))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPackageConfigurationProvider.java:131 JavaPackageConfigurationProvider.wrapSequence
    pub fn wrap_sequence(sequence: &[Value<'v>], heap: Heap<'v>) -> starlark::Result<Vec<Self>> {
        let mut builder = Vec::with_capacity(sequence.len());
        for r#struct in sequence {
            // this result isn't propagated back to Starlark so we just need any type
            builder.push(Self::wrap(*r#struct, heap)?);
        }
        Ok(builder)
    }

    /// Package specifications for which the configuration should be applied.
    // Java: com/google/devtools/build/lib/rules/java/JavaPackageConfigurationProvider.java:54 JavaPackageConfigurationProvider.packageSpecifications
    fn package_specifications(&self) -> starlark::Result<Vec<&'v PackageSpecificationProvider>> {
        let elems = self.underlying.get_underlying_sequence("package_specs")?;
        let mut out = Vec::with_capacity(elems.len());
        for (i, e) in elems.into_iter().enumerate() {
            match e.downcast_ref::<PackageSpecificationProvider>() {
                Some(p) => out.push(p),
                None => {
                    return Err(errorf(format!(
                        "at index {i} of package_specs, got element of type {}, want PackageSpecificationInfo",
                        e.get_type()
                    )));
                }
            }
        }
        Ok(out)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPackageConfigurationProvider.java:67 JavaPackageConfigurationProvider.javacoptsAsList
    pub fn javacopts_as_list(&self) -> starlark::Result<Vec<String>> {
        Ok(tokenize_java_options_nested_set(
            &self
                .underlying
                .get_underlying_nested_set_strings("javac_opts")?,
        ))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPackageConfigurationProvider.java:76 JavaPackageConfigurationProvider.data
    pub fn data(&self) -> starlark::Result<NestedSet<Artifact>> {
        self.underlying.get_underlying_nested_set_artifacts("data")
    }

    /// Returns true if this configuration matches the current label: that is, if the label's
    /// package is contained by any of the package specifications.
    // Java: com/google/devtools/build/lib/rules/java/JavaPackageConfigurationProvider.java:88 JavaPackageConfigurationProvider.matches
    pub fn matches(&self, label: &Label) -> starlark::Result<bool> {
        // Do not use streams here as they create excessive garbage.
        for provider in self.package_specifications()? {
            for specifications in provider.get_package_specifications().to_list().iter() {
                if specifications
                    .0
                    .contains_package(label.get_package_identifier())
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}
