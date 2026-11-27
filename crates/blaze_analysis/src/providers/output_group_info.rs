//! Port of `analysis/OutputGroupInfo.java`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use allocative::Allocative;
use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::starlark_defs::builtin_providers::{BuiltinProviderId, OUTPUT_GROUP_INFO};
use blaze_starlark::structs::DeeplyImmutable;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_simple_value;
use starlark::values::list_or_tuple::UnpackListOrTuple;
use starlark::values::{
    Demand, Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, UnpackValue, Value, ValueLike,
    starlark_value,
};

use crate::info::{InfoProviderKey, freeze_native_info};
use crate::rule::depsets::{artifacts_to_depset, depset_to_artifacts, depset_value};

fn errorf(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:63 OutputGroupInfo.STARLARK_NAME
pub const STARLARK_NAME: &str = "output_groups";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:71 OutputGroupInfo.HIDDEN_OUTPUT_GROUP_PREFIX
pub const HIDDEN_OUTPUT_GROUP_PREFIX: &str = "_";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:74 OutputGroupInfo.INTERNAL_SUFFIX
pub const INTERNAL_SUFFIX: &str = "_INTERNAL_";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:81 OutputGroupInfo.FILES_TO_COMPILE
pub const FILES_TO_COMPILE: &str = "compilation_outputs";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:91 OutputGroupInfo.COMPILATION_PREREQUISITES
pub const COMPILATION_PREREQUISITES: &str = "compilation_prerequisites_INTERNAL_";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:103 OutputGroupInfo.HIDDEN_TOP_LEVEL
pub const HIDDEN_TOP_LEVEL: &str = "_hidden_top_level_INTERNAL_";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:111 OutputGroupInfo.VALIDATION
pub const VALIDATION: &str = "_validation";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:122 OutputGroupInfo.VALIDATION_TOP_LEVEL
pub const VALIDATION_TOP_LEVEL: &str = "_validation_top_level_INTERNAL_";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:130 OutputGroupInfo.VALIDATION_TRANSITIVE
pub const VALIDATION_TRANSITIVE: &str = "_validation_transitive";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:137 OutputGroupInfo.TEMP_FILES
pub const TEMP_FILES: &str = "temp_files_INTERNAL_";
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:148 OutputGroupInfo.DEFAULT
pub const DEFAULT: &str = "default";

/// Java `OutputGroupInfo.DEFAULT_GROUPS` (an `ImmutableSortedSet`).
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:153 OutputGroupInfo.DEFAULT_GROUPS
pub fn default_groups() -> BTreeSet<String> {
    [DEFAULT, TEMP_FILES, HIDDEN_TOP_LEVEL]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Java `OutputGroupInfo.ValidationMode`.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:160 OutputGroupInfo.ValidationMode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationMode {
    Off,
    OutputGroup,
    Aspect,
}

/// `ConfiguredTarget`s implementing this interface can provide artifacts that can be built when
/// the target is mentioned on the command line (as opposed to being always built, like
/// `FileProvider`).
///
/// PORT: Java's specialized subclasses (`EmptyFiles`, `SingleGroup`s,
/// `HiddenTopLevelAndOneOther`, `ArbitraryGroups`) are memory optimizations with identical
/// behaviour; one struct holds the groups in the subclass's iteration order (empty groups keep
/// their entry, as `EmptyFiles` keeps its group names).
// PORT: skipped, memory optimization (one struct): OutputGroupInfo.EmptyFiles.of, OutputGroupInfo.EmptyFiles.<init>
// PORT: skipped, memory optimization (one struct): OutputGroupInfo.SingleGroup.<init>, OutputGroupInfo.SingleGroup.groupName
// PORT: skipped, memory optimization (one struct): OutputGroupInfo.HiddenTopLevelOnly.<init>, OutputGroupInfo.HiddenTopLevelOnly.groupName
// PORT: skipped, memory optimization (one struct): OutputGroupInfo.ValidationOnly.<init>, OutputGroupInfo.ValidationOnly.groupName
// PORT: skipped, memory optimization (one struct): OutputGroupInfo.DefaultOnly.<init>, OutputGroupInfo.DefaultOnly.groupName
// PORT: skipped, memory optimization (one struct): OutputGroupInfo.OtherGroupOnly.<init>, OutputGroupInfo.OtherGroupOnly.groupName
// PORT: skipped, memory optimization (one struct): OutputGroupInfo.HiddenTopLevelAndOneOther.<init>, OutputGroupInfo.ArbitraryGroups.<init>
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:61 OutputGroupInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct OutputGroupInfo {
    #[allocative(skip)]
    groups: IndexMap<String, NestedSet<Artifact>>,
}

starlark_simple_value!(OutputGroupInfo);

/// Allocates an `OutputGroupInfo` as a frozen info.
pub fn freeze(info: OutputGroupInfo) -> OwnedFrozenValue {
    freeze_native_info(info)
}

fn empty_files() -> NestedSet<Artifact> {
    NestedSetBuilder::empty_set(Order::STABLE_ORDER)
}

impl OutputGroupInfo {
    /// Merges output groups from a list of output providers. The output groups must not
    /// overlap... PORT: merging is by name with NestedSet union, as Java.
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:173 OutputGroupInfo.merge
    pub fn merge(providers: &[OutputGroupInfo]) -> Option<OutputGroupInfo> {
        if providers.is_empty() {
            return None;
        }
        if providers.len() == 1 {
            return Some(providers[0].clone());
        }
        let mut output_group_builders: BTreeMap<String, NestedSetBuilder<Artifact>> =
            BTreeMap::new();
        for provider in providers {
            for group in provider.groups.keys() {
                output_group_builders
                    .entry(group.clone())
                    .or_insert_with(NestedSetBuilder::stable_order)
                    .add_transitive(&provider.get_output_group(group));
            }
        }
        let output_groups: IndexMap<String, NestedSet<Artifact>> = output_group_builders
            .into_iter()
            .map(|(k, b)| (k, b.build()))
            .collect();
        Some(Self::create_internal(output_groups))
    }

    /// Determines the output groups to build based on the flags.
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:211 OutputGroupInfo.determineOutputGroups
    pub fn determine_output_groups(
        default_output_groups: &BTreeSet<String>,
        output_groups: &[String],
        validation_mode: ValidationMode,
        should_run_tests: bool,
    ) -> BTreeSet<String> {
        let mut current: BTreeSet<String> = BTreeSet::new();
        // If all of the requested output groups start with "+" or "-", then these are added or
        // subtracted to the set of default output groups. If any of them don't start with "+" or
        // "-", then the list of requested output groups is used as is.
        let add_default_output_groups = output_groups
            .iter()
            .all(|g| g.starts_with('+') || g.starts_with('-'));
        if add_default_output_groups {
            current.extend(default_output_groups.iter().cloned());
        }
        for output_group in output_groups {
            if let Some(g) = output_group.strip_prefix('+') {
                current.insert(g.to_owned());
            } else if let Some(g) = output_group.strip_prefix('-') {
                current.remove(g);
            } else {
                current.insert(output_group.clone());
            }
        }
        // Add the validation output group regardless of the additions and subtractions above.
        match validation_mode {
            ValidationMode::OutputGroup => {
                current.insert(VALIDATION.to_owned());
            }
            ValidationMode::Aspect => {
                current.insert(VALIDATION_TOP_LEVEL.to_owned());
            }
            ValidationMode::Off => {}
        }
        // The `test` command ultimately requests artifacts from the `default` output group in
        // order to execute tests, so we need to make sure these artifacts are available.
        if should_run_tests {
            current.insert(DEFAULT.to_owned());
        }
        current
    }

    /// Returns an `OutputGroupInfo` with a single group.
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:256 OutputGroupInfo.singleGroup
    pub fn single_group(group: &str, files: NestedSet<Artifact>) -> OutputGroupInfo {
        let mut groups = IndexMap::new();
        groups.insert(group.to_owned(), files);
        OutputGroupInfo { groups }
    }

    /// Creates an instance from builders sorted by group name.
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:268 OutputGroupInfo.fromBuilders
    pub fn from_builders(
        builders: &BTreeMap<String, NestedSetBuilder<Artifact>>,
    ) -> OutputGroupInfo {
        let output_groups: IndexMap<String, NestedSet<Artifact>> = builders
            .iter()
            .map(|(group, files)| (group.clone(), files.build()))
            .collect();
        Self::create_internal(output_groups)
    }

    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:275 OutputGroupInfo.createInternal
    fn create_internal(output_groups: IndexMap<String, NestedSet<Artifact>>) -> OutputGroupInfo {
        // PORT: all of Java's representations keep the input order of the group names (the
        // two-group case only specializes when HIDDEN_TOP_LEVEL comes first, which is the same
        // order).
        OutputGroupInfo {
            groups: output_groups,
        }
    }

    /// Returns the artifacts in a particular output group, or the empty set if the group is
    /// absent.
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:318 OutputGroupInfo.getOutputGroup
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:374 OutputGroupInfo.EmptyFiles.getOutputGroup
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:418 OutputGroupInfo.SingleGroup.getOutputGroup
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:510 OutputGroupInfo.HiddenTopLevelAndOneOther.getOutputGroup
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:545 OutputGroupInfo.ArbitraryGroups.getOutputGroup
    pub fn get_output_group(&self, name: &str) -> NestedSet<Artifact> {
        self.groups.get(name).cloned().unwrap_or_else(empty_files)
    }

    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:338 OutputGroupInfo.containsKey
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:379 OutputGroupInfo.EmptyFiles.containsKey
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:423 OutputGroupInfo.SingleGroup.containsKey
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:521 OutputGroupInfo.HiddenTopLevelAndOneOther.containsKey
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:550 OutputGroupInfo.ArbitraryGroups.containsKey
    pub fn contains_key(&self, name: &str) -> bool {
        self.groups.contains_key(name)
    }

    /// Whether Java would represent this as `EmptyFiles` (every group empty).
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:286 OutputGroupInfo.createInternal
    fn is_empty_files(&self) -> bool {
        self.groups.values().all(|files| files.is_empty())
    }

    /// The group names, in iteration order (Java `iterator()` / `getFieldNames()`).
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:384 OutputGroupInfo.EmptyFiles.iterator
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:389 OutputGroupInfo.EmptyFiles.getFieldNames
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:428 OutputGroupInfo.SingleGroup.iterator
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:433 OutputGroupInfo.SingleGroup.getFieldNames
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:526 OutputGroupInfo.HiddenTopLevelAndOneOther.iterator
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:531 OutputGroupInfo.HiddenTopLevelAndOneOther.getFieldNames
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:555 OutputGroupInfo.ArbitraryGroups.iterator
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:560 OutputGroupInfo.ArbitraryGroups.getFieldNames
    pub fn groups(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }

    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:342 OutputGroupInfo.getValue
    // PORT: `heap` is the heap of the value handed out (None: read at once by Rust code, see
    // `depsets::depset_value_in_command`).
    fn get_value<'v>(&self, name: &str, heap: Option<Heap<'v>>) -> Option<Value<'v>> {
        let result = self.get_output_group(name);
        if result.is_empty() && !self.contains_key(name) {
            return None;
        }
        let depset = artifacts_to_depset(&result);
        Some(match heap {
            Some(heap) => depset_value(heap, &depset),
            None => crate::rule::depsets::depset_value_in_command(&depset),
        })
    }
}

impl fmt::Display for OutputGroupInfo {
    // Java: StructImpl.repr: `OutputGroupInfo(name = depset([...]), ...)` in field order.
    // Java: com/google/devtools/build/lib/packages/StructImpl.java:269 StructImpl.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OutputGroupInfo(")?;
        let mut names: Vec<&String> = self.groups.keys().collect();
        names.sort();
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            let v = self.get_value(name, None).expect("group present");
            write!(f, "{} = {}", name, v.to_repr())?;
        }
        write!(f, ")")
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/OutputGroupInfoApi.java:28 OutputGroupInfoApi
#[starlark_value(type = "OutputGroupInfo")]
impl<'v> StarlarkValue<'v> for OutputGroupInfo {
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:323 OutputGroupInfo.getIndex
    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let Some(key) = index.unpack_str() else {
            return Err(errorf(format!(
                "Output group names must be strings, got {} instead",
                index.get_type()
            )));
        };
        match self.get_value(key, Some(heap)) {
            Some(v) => Ok(v),
            None => Err(errorf(format!("Output group {key} not present"))),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:334 OutputGroupInfo.containsKey
    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other.unpack_str().is_some_and(|k| self.contains_key(k)))
    }

    // Java: OutputGroupInfo implements StarlarkIterable<String>.
    fn iterate_collect(&self, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self.groups.keys().map(|k| heap.alloc(k.as_str())).collect())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        self.get_value(attribute, Some(heap))
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        self.contains_key(attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        self.groups()
    }

    /// Java: `Object.equals` (identity) except for `EmptyFiles` (every group empty), whose
    /// instances are equal when their group-name sets are (`ImmutableSet.equals`).
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:394 OutputGroupInfo.EmptyFiles.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        // Identical values are equal before this is called (starlark-rust compares pointers).
        let Some(other) = other.downcast_ref::<OutputGroupInfo>() else {
            return Ok(false);
        };
        Ok(self.is_empty_files()
            && other.is_empty_files()
            && self.groups.len() == other.groups.len()
            && self.groups.keys().all(|k| other.groups.contains_key(k)))
    }

    /// `isImmutable()` is true, so an `OutputGroupInfo` is hashable: `EmptyFiles.hashCode` is the
    /// group-name set's hash (order-independent); the other representations hash by identity,
    /// which equality then decides (a constant hash is valid for those).
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:316 OutputGroupInfo.isImmutable
    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:405 OutputGroupInfo.EmptyFiles.hashCode
    fn write_hash(&self, hasher: &mut starlark_map::StarlarkHasher) -> starlark::Result<()> {
        use std::hash::{Hash, Hasher};
        if self.is_empty_files() {
            // Set.hashCode: the sum of the elements' hashes.
            let mut sum: u64 = 0;
            for k in self.groups.keys() {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                k.hash(&mut h);
                sum = sum.wrapping_add(h.finish());
            }
            sum.hash(hasher);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:311 OutputGroupInfo.getProvider
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(OUTPUT_GROUP_INFO.key()));
        demand.provide_value(DeeplyImmutable);
    }
}

/// Converts a Starlark output group value (a sequence of Files or a depset of Files) to a
/// NestedSet.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleConfiguredTargetUtil.java:640 StarlarkRuleConfiguredTargetUtil.convertToOutputGroupValue
// PORT: shared with `crate::starlark::starlark_rule_configured_target_util` (agent S) for the
// rule-result path; the `OutputGroupInfo` constructor needs it too.
pub fn convert_to_output_group_value(
    output_group: &str,
    objects: Value,
) -> starlark::Result<NestedSet<Artifact>> {
    // regrettable preemptive allocation of error message
    let what = format!("output group '{output_group}'");
    if let Some(seq) = UnpackListOrTuple::<Value>::unpack_value(objects)? {
        let mut b = NestedSetBuilder::stable_order();
        for (i, elem) in seq.items.iter().enumerate() {
            match artifact_from_value(*elem) {
                Some(a) => {
                    b.add(a);
                }
                None => {
                    return Err(errorf(format!(
                        "at index {i} of {what}, got element of type {}, want File",
                        elem.get_type()
                    )));
                }
            }
        }
        return Ok(b.build());
    }
    depset_to_artifacts(objects, &what)
}

/// The `OutputGroupInfo(**kwargs)` constructor.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:575 OutputGroupInfo.OutputGroupInfoProvider.constructor
pub fn output_group_info_constructor<'v>(
    _id: &BuiltinProviderId,
    _me: Value<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let positional: Vec<Value<'v>> = args.positions(eval.heap())?.collect();
    if !positional.is_empty() {
        return Err(errorf(format!(
            "OutputGroupInfo() accepts no more than 0 positional arguments but got {}",
            positional.len()
        )));
    }
    let mut kwargs: Vec<(String, Value<'v>)> = args
        .names_map()?
        .into_iter()
        .map(|(k, v)| (k.as_str().to_owned(), v))
        .collect();
    // Java: ImmutableList.sortedCopyOf(Map.Entry.comparingByKey(), kwargs.entrySet()).
    kwargs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut output_groups: IndexMap<String, NestedSet<Artifact>> = IndexMap::new();
    for (k, v) in kwargs {
        let set = convert_to_output_group_value(&k, v)?;
        output_groups.insert(k, set);
    }
    Ok(eval
        .heap()
        .alloc(OutputGroupInfo::create_internal(output_groups)))
}

#[cfg(test)]
mod tests {
    // Ports of src/test/java/com/google/devtools/build/lib/analysis/OutputGroupInfoTest.java.
    use super::*;

    fn groups(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn sorted(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn determine_output_groups_override_defaults() {
        let out = OutputGroupInfo::determine_output_groups(
            &sorted(&["a", "b", "c"]),
            &groups(&["foo"]),
            ValidationMode::Off,
            false,
        );
        assert_eq!(out, sorted(&["foo"]));
    }

    #[test]
    fn determine_output_groups_add_to_defaults() {
        let out = OutputGroupInfo::determine_output_groups(
            &sorted(&["a", "b", "c"]),
            &groups(&["+foo"]),
            ValidationMode::Off,
            false,
        );
        assert_eq!(out, sorted(&["a", "b", "c", "foo"]));
    }

    #[test]
    fn determine_output_groups_remove_from_defaults() {
        let out = OutputGroupInfo::determine_output_groups(
            &sorted(&["a", "b", "c"]),
            &groups(&["-b"]),
            ValidationMode::Off,
            false,
        );
        assert_eq!(out, sorted(&["a", "c"]));
    }

    #[test]
    fn determine_output_groups_validation() {
        let out = OutputGroupInfo::determine_output_groups(
            &sorted(&["a"]),
            &groups(&[]),
            ValidationMode::OutputGroup,
            false,
        );
        assert_eq!(out, sorted(&["a", VALIDATION]));
        let out = OutputGroupInfo::determine_output_groups(
            &sorted(&["a"]),
            &groups(&[]),
            ValidationMode::Aspect,
            false,
        );
        assert_eq!(out, sorted(&["a", VALIDATION_TOP_LEVEL]));
    }

    #[test]
    fn determine_output_groups_with_tests() {
        let out = OutputGroupInfo::determine_output_groups(
            &sorted(&["a"]),
            &groups(&["foo"]),
            ValidationMode::Off,
            true,
        );
        assert_eq!(out, sorted(&["foo", DEFAULT]));
    }

    #[test]
    fn single_group_contains_only_its_group() {
        let info = OutputGroupInfo::single_group(VALIDATION, empty_files());
        assert!(info.contains_key(VALIDATION));
        assert!(!info.contains_key(DEFAULT));
        assert!(info.get_output_group(DEFAULT).is_empty());
    }

    /// `EmptyFiles.equals` / `hashCode`: infos whose groups are all empty are equal (and hash
    /// alike) when their group-name sets are, in any order.
    #[test]
    fn empty_files_equal_by_group_names() {
        starlark::environment::Module::with_temp_heap(|module| {
            let heap = module.heap();
            let make = |names: &[&str]| {
                let mut map = IndexMap::new();
                for n in names {
                    map.insert(n.to_string(), empty_files());
                }
                heap.alloc(OutputGroupInfo::create_internal(map))
            };
            let ab = make(&["a", "b"]);
            let ba = make(&["b", "a"]);
            let a = make(&["a"]);
            assert!(ab.equals(ba).unwrap());
            assert!(!ab.equals(a).unwrap());
            assert_eq!(
                ab.get_hashed().unwrap().hash(),
                ba.get_hashed().unwrap().hash()
            );
            assert!(a.get_hashed().is_ok());
        });
    }
}
