//! Ports of `analysis/config/OptionInfo.java` and `analysis/config/OptionsDiff.java`.

use blaze_cmdline::Label;
use blaze_options::OptionMetadataTag;
use blaze_options::Value;
use blaze_options::definition::OptionDefinitionRef;
use indexmap::{IndexMap, IndexSet};

use crate::build_options::{BuildOptions, StarlarkOptionValue};
use crate::fragment_options_class::FragmentOptionsClass;

// Java: com/google/devtools/build/lib/analysis/config/OptionInfo.java:28 OptionInfo
/// Stores information about a native option (its fragment class and definition).
#[derive(Clone, Copy, Debug)]
pub struct OptionInfo {
    option_class: FragmentOptionsClass,
    definition: OptionDefinitionRef,
}

impl OptionInfo {
    pub fn get_option_class(&self) -> FragmentOptionsClass {
        self.option_class
    }

    pub fn get_definition(&self) -> OptionDefinitionRef {
        self.definition
    }

    // Java: com/google/devtools/build/lib/analysis/config/OptionInfo.java:45 OptionInfo.hasOptionMetadataTag
    pub fn has_option_metadata_tag(&self, tag: OptionMetadataTag) -> bool {
        self.definition.get_option_metadata_tags().contains(&tag)
    }

    // Java: com/google/devtools/build/lib/analysis/config/OptionInfo.java:50 OptionInfo.buildMapFrom
    /// For all the options in the BuildOptions, build a map from option name to its information.
    pub fn build_map_from(build_options: &BuildOptions) -> IndexMap<String, OptionInfo> {
        let mut builder: IndexMap<String, OptionInfo> = IndexMap::new();
        for (option_class, _) in build_options.get_native_options() {
            for def in option_class.get_option_definitions() {
                let option_name = def.get_option_name().to_string();
                // ImmutableMap.Builder.build() rejects duplicate keys.
                if builder.contains_key(&option_name) {
                    panic!(
                        "IllegalArgumentException: Multiple entries with same key: {}",
                        option_name
                    );
                }
                builder.insert(
                    option_name,
                    OptionInfo {
                        option_class,
                        definition: def,
                    },
                );
            }
        }
        builder
    }
}

// Java: com/google/devtools/build/lib/analysis/config/OptionsDiff.java:44 OptionsDiff
/// A diff class for `BuildOptions`. Fields are meant to be populated and returned by
/// `OptionsDiff.diff`.
#[derive(Default, Debug)]
pub struct OptionsDiff {
    differing_options: Vec<(FragmentOptionsClass, OptionDefinitionRef)>,
    // The keyset for the `first` and `second` maps are identical and indicate which specific
    // options differ between the first and second built options.
    first: IndexMap<OptionDefinitionRef, Option<Value>>,
    // Since this class can be used to track the result of transitions, `second` is a multimap to
    // be able to handle `SplitTransition`s.
    second: IndexMap<OptionDefinitionRef, Vec<Option<Value>>>,
    // List of "extra" fragments for each BuildOption aka fragments that were trimmed off one
    // BuildOption but not the other.
    extra_first_fragments: Vec<FragmentOptionsClass>,
    extra_second_fragments: Vec<FragmentOptionsClass>,
    starlark_first: IndexMap<Label, StarlarkOptionValue>,
    starlark_second: IndexMap<Label, StarlarkOptionValue>,
    extra_starlark_options_first: Vec<Label>,
    extra_starlark_options_second: IndexMap<Label, StarlarkOptionValue>,
    has_starlark_options: bool,
}

impl OptionsDiff {
    // Java: com/google/devtools/build/lib/analysis/config/OptionsDiff.java:46 OptionsDiff.diff
    /// Returns the difference between two BuildOptions in a new `OptionsDiff`.
    pub fn diff(first: &BuildOptions, second: &BuildOptions) -> OptionsDiff {
        let mut diff = OptionsDiff::default();
        Self::diff_into(&mut diff, first, second);
        diff
    }

    // Java: com/google/devtools/build/lib/analysis/config/OptionsDiff.java:46 OptionsDiff.diff
    /// Returns the difference between two BuildOptions in a pre-existing `OptionsDiff`.
    pub fn diff_into(diff: &mut OptionsDiff, first: &BuildOptions, second: &BuildOptions) {
        blaze_util::check_argument!(
            !diff.has_starlark_options,
            "OptionsDiff cannot handle multiple 'second' BuildOptions with Starlark options and is trying to diff against {:?}",
            diff
        );
        if first == second {
            return;
        }

        // Check and report if either class has been trimmed of an options class that exists in
        // the other.
        let first_option_classes: Vec<FragmentOptionsClass> = first.get_fragment_classes();
        let second_option_classes: Vec<FragmentOptionsClass> = second.get_fragment_classes();
        for c in &first_option_classes {
            if !second_option_classes.contains(c) {
                diff.add_extra_first_fragment(c);
            }
        }
        for c in &second_option_classes {
            if !first_option_classes.contains(c) {
                diff.add_extra_second_fragment(c);
            }
        }

        // For fragments in common, report differences.
        for clazz in first_option_classes
            .iter()
            .filter(|c| second_option_classes.contains(c))
        {
            let first_options = first.get_dyn(clazz).unwrap();
            let second_options = second.get_dyn(clazz).unwrap();
            for definition in clazz.get_option_definitions() {
                let first_value = first_options.get_value_from_definition(definition);
                let second_value = second_options.get_value_from_definition(definition);
                if first_value != second_value {
                    diff.add_diff(clazz, definition, first_value, second_value);
                }
            }
        }

        // Compare Starlark options for the two classes.
        let starlark_first = first.get_starlark_options();
        let starlark_second = second.get_starlark_options();
        // Sets.union: elements of the first set, then those of the second not in the first.
        let mut union: IndexSet<&Label> = starlark_first.keys().collect();
        union.extend(starlark_second.keys());
        for build_setting in union {
            match (
                starlark_first.get(build_setting),
                starlark_second.get(build_setting),
            ) {
                (None, Some(s)) => diff.add_extra_second_starlark_option(build_setting, s),
                (Some(_), None) => diff.add_extra_first_starlark_option(build_setting),
                // Java: !starlarkFirst.get(buildSetting).equals(starlarkSecond.get(buildSetting))
                (Some(f), Some(s)) if !f.java_equals(s) => {
                    diff.put_starlark_diff(build_setting, f, s)
                }
                _ => {}
            }
        }
    }

    pub fn get_first(&self) -> &IndexMap<OptionDefinitionRef, Option<Value>> {
        &self.first
    }

    pub fn get_second(&self) -> &IndexMap<OptionDefinitionRef, Vec<Option<Value>>> {
        &self.second
    }

    pub fn get_extra_first_fragment_classes(&self) -> &[FragmentOptionsClass] {
        &self.extra_first_fragments
    }

    pub fn get_extra_second_fragments(&self) -> &[FragmentOptionsClass] {
        &self.extra_second_fragments
    }

    fn add_diff(
        &mut self,
        fragment_options_class: FragmentOptionsClass,
        option: OptionDefinitionRef,
        first_value: Option<Value>,
        second_value: Option<Value>,
    ) {
        self.differing_options
            .push((fragment_options_class, option));
        self.first.insert(option, first_value);
        // OrderedSetMultimap: a value is added once per key.
        let values = self.second.entry(option).or_default();
        if !values.contains(&second_value) {
            values.push(second_value);
        }
    }

    fn add_extra_first_fragment(&mut self, options: FragmentOptionsClass) {
        if !self.extra_first_fragments.contains(&options) {
            self.extra_first_fragments.push(options);
        }
    }

    fn add_extra_second_fragment(&mut self, options: FragmentOptionsClass) {
        if !self.extra_second_fragments.contains(&options) {
            self.extra_second_fragments.push(options);
        }
    }

    fn put_starlark_diff(
        &mut self,
        build_setting: &Label,
        first_value: &StarlarkOptionValue,
        second_value: &StarlarkOptionValue,
    ) {
        self.starlark_first
            .insert(build_setting.clone(), first_value.clone());
        self.starlark_second
            .insert(build_setting.clone(), second_value.clone());
        self.has_starlark_options = true;
    }

    fn add_extra_first_starlark_option(&mut self, build_setting: &Label) {
        self.extra_starlark_options_first
            .push(build_setting.clone());
        self.has_starlark_options = true;
    }

    fn add_extra_second_starlark_option(
        &mut self,
        build_setting: &Label,
        value: &StarlarkOptionValue,
    ) {
        self.extra_starlark_options_second
            .insert(build_setting.clone(), value.clone());
        self.has_starlark_options = true;
    }

    // Java: com/google/devtools/build/lib/analysis/config/OptionsDiff.java:201 OptionsDiff.getChangedStarlarkOptions
    /// Returns the labels of all starlark options that caused a difference between the first and
    /// second options set.
    pub fn get_changed_starlark_options(&self) -> IndexSet<Label> {
        let mut result: IndexSet<Label> = IndexSet::new();
        result.extend(self.starlark_first.keys().cloned());
        result.extend(self.starlark_second.keys().cloned());
        result.extend(self.extra_starlark_options_first.iter().cloned());
        result.extend(self.extra_starlark_options_second.keys().cloned());
        result
    }

    pub fn get_starlark_first_for_testing(&self) -> &IndexMap<Label, StarlarkOptionValue> {
        &self.starlark_first
    }

    pub fn get_starlark_second_for_testing(&self) -> &IndexMap<Label, StarlarkOptionValue> {
        &self.starlark_second
    }

    pub fn get_extra_starlark_options_first_for_testing(&self) -> &[Label] {
        &self.extra_starlark_options_first
    }

    pub fn get_extra_starlark_options_second_for_testing(
        &self,
    ) -> &IndexMap<Label, StarlarkOptionValue> {
        &self.extra_starlark_options_second
    }

    // Java: com/google/devtools/build/lib/analysis/config/OptionsDiff.java:234 OptionsDiff.areSame
    /// Note: it's not enough for first and second to be empty, with trimming, they must also
    /// contain the same options classes.
    pub fn are_same(&self) -> bool {
        self.first.is_empty()
            && self.second.is_empty()
            && self.extra_second_fragments.is_empty()
            && self.extra_first_fragments.is_empty()
            && self.differing_options.is_empty()
            && self.starlark_first.is_empty()
            && self.starlark_second.is_empty()
            && self.extra_starlark_options_first.is_empty()
            && self.extra_starlark_options_second.is_empty()
    }

    // Java: com/google/devtools/build/lib/analysis/config/OptionsDiff.java:246 OptionsDiff.prettyPrint
    pub fn pretty_print(&self) -> String {
        let mut to_return = String::new();
        for diff in self.get_pretty_print_list() {
            to_return.push_str(&diff);
            to_return.push('\n');
        }
        to_return
    }

    // Java: com/google/devtools/build/lib/analysis/config/OptionsDiff.java:248 OptionsDiff.getPrettyPrintList
    pub fn get_pretty_print_list(&self) -> Vec<String> {
        let mut to_return = Vec::new();
        for (option, value) in &self.first {
            let second: Vec<String> = self.second[option]
                .iter()
                .map(|v| blaze_options::value::java_string_of(v.as_ref()))
                .collect();
            to_return.push(format!(
                "{}:{} -> [{}]",
                option.get_option_name(),
                blaze_options::value::java_string_of(value.as_ref()),
                second.join(", ")
            ));
        }
        for (option, value) in &self.starlark_first {
            to_return.push(format!(
                "{}:{}{}",
                option,
                value.java_to_string(),
                self.starlark_second[option].java_to_string()
            ));
        }
        to_return
    }
}
