//! A configured target as a Starlark `Target` value (Java: every `ConfiguredTarget` is a
//! `StarlarkValue`; `AbstractConfiguredTarget` implements `getIndex`, `containsKey`, `getValue`
//! and `getFieldNames`, `AliasConfiguredTarget` forwards to its actual target).
//!
//! The value wraps the shared Rust `ConfiguredTarget` and holds no Starlark values, so it is a
//! simple (frozen-safe) value. Provider instances it returns live on other frozen heaps.
//!
//! PORT: keep-alive. Provider values are returned with `OwnedFrozenValue::unchecked_frozen_value`:
//! `StarlarkValue::at`/`get_attr` have no frozen heap to register the providers' heaps with.
//! Every configured target (and so every provider heap) is held by Skyframe for the life of the
//! process (single build per process), which keeps them valid.

use std::fmt;
use std::hash::Hash;

use allocative::Allocative;
use blaze_packages::starlark_defs::builtin_providers::{OUTPUT_GROUP_INFO, builtin_provider_key};
use blaze_packages::starlark_defs::provider::provider_view;
use blaze_starlark::ThreadContext;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::{DeeplyImmutable, ProviderInfo};
use starlark::any::ProvidesStaticType;
use starlark::collections::Hashed;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::{
    Demand, Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, Value, starlark_value,
};
use starlark_map::StarlarkHasher;

use crate::configured_target::ConfiguredTarget;
use crate::configuredtargets::{get_default_info, get_info, get_legacy, get_rule_class_string};
use crate::providers::default_info::DefaultInfo;
use crate::rule::depsets::{artifacts_to_depset, depset_value};

fn errorf(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:63 AbstractConfiguredTarget.DATA_RUNFILES_FIELD
const DATA_RUNFILES_FIELD: &str = "data_runfiles";
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:64 AbstractConfiguredTarget.DEFAULT_RUNFILES_FIELD
const DEFAULT_RUNFILES_FIELD: &str = "default_runfiles";
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:67 AbstractConfiguredTarget.ACTIONS_FIELD_NAME
const ACTIONS_FIELD_NAME: &str = "actions";
// Java: com/google/devtools/build/lib/starlarkbuildapi/core/TransitiveInfoCollectionApi.java LABEL_FIELD / FILES_FIELD
const LABEL_FIELD: &str = "label";
const FILES_FIELD: &str = "files";
// Java: com/google/devtools/build/lib/analysis/FilesToRunProvider.java:35 FilesToRunProvider.STARLARK_NAME
const FILES_TO_RUN_FIELD: &str = "files_to_run";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:63 OutputGroupInfo.STARLARK_NAME
const OUTPUT_GROUPS_FIELD: &str = "output_groups";

// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:69 AbstractConfiguredTarget.SPECIAL_FIELD_NAMES
const SPECIAL_FIELD_NAMES: [&str; 7] = [
    LABEL_FIELD,
    FILES_FIELD,
    DEFAULT_RUNFILES_FIELD,
    DATA_RUNFILES_FIELD,
    FILES_TO_RUN_FIELD,
    OUTPUT_GROUPS_FIELD,
    ACTIONS_FIELD_NAME,
];

// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:79 AbstractConfiguredTarget.DEFAULT_PROVIDER_FIELDS
const DEFAULT_PROVIDER_FIELDS: [&str; 5] = [
    DEFAULT_RUNFILES_FIELD,
    DATA_RUNFILES_FIELD,
    FILES_FIELD,
    FILES_TO_RUN_FIELD,
    OUTPUT_GROUPS_FIELD,
];

/// A configured target as Starlark sees it (type `Target`).
// Java: com/google/devtools/build/lib/analysis/ConfiguredTarget.java:37 ConfiguredTarget
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:55 AbstractConfiguredTarget
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkTarget {
    #[allocative(skip)]
    target: ConfiguredTarget,
    /// `--incompatible_disable_target_default_provider_fields` of the evaluation that created
    /// the value (Java reads `thread.getSemantics()` in `getValue`).
    disable_target_default_provider_fields: bool,
}

starlark_simple_value!(StarlarkTarget);

impl StarlarkTarget {
    pub fn new(
        target: ConfiguredTarget,
        disable_target_default_provider_fields: bool,
    ) -> StarlarkTarget {
        StarlarkTarget {
            target,
            disable_target_default_provider_fields,
        }
    }

    /// The wrapped configured target.
    pub fn target(&self) -> &ConfiguredTarget {
        &self.target
    }

    /// Returns the configured target of a Starlark `Target` value.
    pub fn configured_target_of<'v>(v: Value<'v>) -> Option<&'v ConfiguredTarget> {
        v.downcast_ref::<StarlarkTarget>().map(|t| &t.target)
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:240 AbstractConfiguredTarget.getRuleClassStringForError
    fn get_rule_class_string_for_error(&self) -> String {
        let s = get_rule_class_string(&self.target);
        if s.is_empty() {
            String::new()
        } else {
            format!(" (rule '{s}')")
        }
    }

    /// Returns the key and printable name of an exported provider, or Java's error.
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:244 AbstractConfiguredTarget.selectExportedProvider
    fn select_exported_provider(
        &self,
        key: Value,
        operation: &str,
    ) -> starlark::Result<(blaze_packages::rule_class::ProviderKey, String)> {
        let Some(view) = provider_view(key) else {
            return Err(errorf(format!(
                "Type Target only supports {operation}ing by object constructors, got {} instead",
                key.get_type()
            )));
        };
        if !view.exported {
            return Err(errorf(format!(
                "{}{} only supports {operation}ing by exported providers. Assign the provider a name in a top-level assignment statement.",
                self,
                self.get_rule_class_string_for_error()
            )));
        }
        let provider_key = view
            .key
            .or_else(|| builtin_provider_key(key))
            .expect("exported provider has a key");
        let printable_name = key
            .request_value::<ProviderInfo>()
            .map(|p| p.printable_name)
            .unwrap_or_else(|| provider_key.to_string());
        Ok((provider_key, printable_name))
    }

    fn owned_to_value<'v>(v: &OwnedFrozenValue) -> Value<'v> {
        // SAFETY: see the module comment (keep-alive).
        unsafe { v.unchecked_frozen_value() }.to_value()
    }

    /// Java `getValue(String name)` for the special fields.
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:146 AbstractConfiguredTarget.getValue
    fn get_value<'v>(&self, name: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        if name == LABEL_FIELD {
            // Java: ConfiguredTarget.getLabel (an alias reports its actual target's label).
            return Some(heap.alloc(LabelValue(crate::configuredtargets::get_label(
                &self.target,
            ))));
        }
        if name == ACTIONS_FIELD_NAME {
            // PORT: the Starlark `Action` objects (ActionApi) are not ported.
            blaze_util::unported!("RuleConfiguredTarget.rawGetStarlarkProvider(actions)");
        }
        self.get_by_string(name, heap)
    }

    /// Java `get(String providerKey)`.
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:217 AbstractConfiguredTarget.get
    fn get_by_string<'v>(&self, name: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let default_info = || get_default_info(&self.target);
        match name {
            FILES_FIELD => {
                // Java: AliasConfiguredTarget.getValue(FILES_FIELD) uses its own FileProvider
                // (the actual's); AbstractConfiguredTarget the DefaultInfo's files.
                let files = crate::configuredtargets::get_provider::<
                    crate::providers::file_provider::FileProvider,
                >(&self.target)
                .map(|p| p.get_files_to_build().clone())?;
                Some(depset_value(&artifacts_to_depset(&files)))
            }
            DEFAULT_RUNFILES_FIELD => {
                let d = default_info();
                let info = DefaultInfo::from_value(Self::owned_to_value(&d))?;
                Some(info.default_runfiles_value(heap))
            }
            DATA_RUNFILES_FIELD => {
                let d = default_info();
                let info = DefaultInfo::from_value(Self::owned_to_value(&d))?;
                Some(info.data_runfiles_value(heap))
            }
            FILES_TO_RUN_FIELD => {
                let d = default_info();
                let info = DefaultInfo::from_value(Self::owned_to_value(&d))?;
                Some(info.files_to_run_value(heap))
            }
            OUTPUT_GROUPS_FIELD => {
                get_info(&self.target, &OUTPUT_GROUP_INFO.key()).map(|v| Self::owned_to_value(&v))
            }
            _ => get_legacy(&self.target, name).map(|v| Self::owned_to_value(&v)),
        }
    }

    /// Java `getValue(semantics, name)`: the errors for a field that is not special and for a
    /// default-provider field under `--incompatible_disable_target_default_provider_fields`.
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:127 AbstractConfiguredTarget.getValue
    fn get_value_with_semantics<'v>(
        &self,
        disable_target_default_provider_fields: bool,
        name: &str,
        heap: Heap<'v>,
    ) -> starlark::Result<Option<Value<'v>>> {
        if !SPECIAL_FIELD_NAMES.contains(&name) {
            return Err(errorf(
                "Accessing providers via the field syntax on structs is deprecated and removed.",
            ));
        } else if disable_target_default_provider_fields && DEFAULT_PROVIDER_FIELDS.contains(&name)
        {
            return Err(errorf(
                "Accessing the default provider in this manner is deprecated and will be removed \
                 soon. It may be temporarily re-enabled by setting \
                 --incompatible_disable_target_default_provider_fields=false. See \
                 https://github.com/bazelbuild/bazel/issues/20183 for details.",
            ));
        }
        Ok(self.get_value(name, heap))
    }

    /// Java `getFieldNames()`.
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:189 AbstractConfiguredTarget.getFieldNames
    fn get_field_names(&self) -> Vec<String> {
        let target = match &self.target {
            // Java: AliasConfiguredTarget.getFieldNames -> actual.getFieldNames().
            ConfiguredTarget::Alias(a) => a.actual.clone(),
            t => t.clone(),
        };
        let mut result: Vec<String> = vec![
            DATA_RUNFILES_FIELD.to_owned(),
            DEFAULT_RUNFILES_FIELD.to_owned(),
            LABEL_FIELD.to_owned(),
            FILES_FIELD.to_owned(),
            FILES_TO_RUN_FIELD.to_owned(),
        ];
        if get_info(&target, &OUTPUT_GROUP_INFO.key()).is_some() {
            result.push(OUTPUT_GROUPS_FIELD.to_owned());
        }
        // Java: addExtraStarlarkKeys (RuleConfiguredTarget: legacy keys, then "actions").
        if let ConfiguredTarget::Rule(r) = &target {
            for (k, _) in r.providers().entries() {
                if let crate::provider_map::ProviderMapKey::Legacy(s) = k {
                    result.push(s.clone());
                }
            }
            result.push(ACTIONS_FIELD_NAME.to_owned());
        }
        result
    }
}

impl fmt::Display for StarlarkTarget {
    // Java: repr of each ConfiguredTarget class.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.target {
            // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:268 RuleConfiguredTarget.repr
            ConfiguredTarget::Rule(_) => write!(f, "<target {}>", self.target.get_label()),
            // Java: com/google/devtools/build/lib/analysis/configuredtargets/InputFileConfiguredTarget.java:90 InputFileConfiguredTarget.repr
            ConfiguredTarget::InputFile(_) => {
                write!(f, "<input file target {}>", self.target.get_label())
            }
            // Java: com/google/devtools/build/lib/analysis/configuredtargets/OutputFileConfiguredTarget.java:146 OutputFileConfiguredTarget.repr
            ConfiguredTarget::OutputFile(_) => {
                write!(f, "<output file target {}>", self.target.get_label())
            }
            // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:186 AliasConfiguredTarget.repr
            ConfiguredTarget::Alias(a) => write!(
                f,
                "<alias target {} of {}>",
                a.get_original_label(),
                crate::configuredtargets::get_label(&a.actual)
            ),
            // Java: com/google/devtools/build/lib/analysis/configuredtargets/MergedConfiguredTarget.java:371 MergedConfiguredTarget.repr
            ConfiguredTarget::Merged(m) => {
                write!(
                    f,
                    "<merged target {}>",
                    crate::configuredtargets::get_label(&m.base_configured_target)
                )
            }
            // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:236 AbstractConfiguredTarget.repr
            _ => write!(f, "<unknown target {}>", self.target.get_label()),
        }
    }
}

#[starlark_value(type = "Target")]
impl<'v> StarlarkValue<'v> for StarlarkTarget {
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:164 AbstractConfiguredTarget.getIndex
    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let (key, printable_name) = self.select_exported_provider(index, "index")?;
        if let Some(declared_provider) = get_info(&self.target, &key) {
            return Ok(Self::owned_to_value(&declared_provider));
        }
        Err(errorf(format!(
            "{}{} doesn't contain declared provider '{}'",
            self,
            self.get_rule_class_string_for_error(),
            printable_name
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:176 AbstractConfiguredTarget.containsKey
    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        let (key, _) = self.select_exported_provider(other, "query")?;
        Ok(get_info(&self.target, &key).is_some())
    }

    /// Rust callers without an evaluator: Java's `getValue(semantics, name)` with the semantics
    /// the value was created with, `None` where Java throws.
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        self.get_value_with_semantics(self.disable_target_default_provider_fields, attribute, heap)
            .ok()
            .flatten()
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:127 AbstractConfiguredTarget.getValue
    fn get_attr_fallible(
        &self,
        attribute: Hashed<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Option<Value<'v>>> {
        let disable_target_default_provider_fields = ThreadContext::get(eval)
            .map_err(starlark::Error::new_other)?
            .semantics
            .incompatible_disable_target_default_provider_fields;
        self.get_value_with_semantics(
            disable_target_default_provider_fields,
            attribute.key(),
            eval.heap(),
        )
    }

    // Java `Starlark.hasattr`: `((Structure) x).getValue(name) != null`, the one-argument
    // `getValue(String)`, which neither rejects non-special names nor reads
    // `--incompatible_disable_target_default_provider_fields` (so `hasattr(t, "files")` is
    // True with the flag although `t.files` fails).
    // Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr
    fn has_attr(&self, attribute: &str, heap: Heap<'v>) -> bool {
        if attribute == ACTIONS_FIELD_NAME {
            return matches!(self.target, ConfiguredTarget::Rule(_));
        }
        self.get_value(attribute, heap).is_some()
    }

    fn dir_attr(&self) -> Vec<String> {
        self.get_field_names()
    }

    // Java: AbstractConfiguredTarget.isImmutable (immutable and Starlark-hashable, by identity).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        let id = self.target.get_lookup_key() as *const _ as usize;
        id.hash(hasher);
        Ok(())
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(match other.downcast_ref::<StarlarkTarget>() {
            Some(o) => std::ptr::eq(self.target.get_lookup_key(), o.target.get_lookup_key()),
            None => false,
        })
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "needs configured targets from analysis (StarlarkRuleContextTest dep[...] tests)"]
    fn index_by_provider() {}
}
