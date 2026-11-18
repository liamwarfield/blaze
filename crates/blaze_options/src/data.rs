//! `IsolatedOptionsData` + `OptionsData`: the per-parser index of option definitions.

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use indexmap::IndexMap;

use crate::definition::{OptionDefinitionRef, OptionsClassRef};
use crate::exception::{ConstructionException, DUPLICATE_OPTION_DECLARATION_EXCEPTION};
use crate::parser_impl::equivalent_for_parsing;

// Java: com/google/devtools/common/options/OptionsData.java:31 OptionsData
// Java: com/google/devtools/common/options/IsolatedOptionsData.java:51 IsolatedOptionsData
/// The option definitions for a set of options classes, with expansions precomputed.
pub struct OptionsData {
    options_classes: Vec<OptionsClassRef>,
    name_to_field: IndexMap<String, OptionDefinitionRef>,
    old_name_to_field: IndexMap<String, OptionDefinitionRef>,
    abbrev_to_field: HashMap<char, OptionDefinitionRef>,
    uses_only_core_types: HashMap<TypeId, bool>,
    evaluated_expansions: HashMap<OptionDefinitionRef, Vec<String>>,
    allow_duplicates_parsing_equivalently: bool,
}

type CacheKey = (Vec<TypeId>, bool);

static OPTIONS_DATA_CACHE: LazyLock<Mutex<HashMap<CacheKey, Arc<OptionsData>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// Java: com/google/devtools/common/options/IsolatedOptionsData.java:158 IsolatedOptionsData.checkForCollisions
fn check_for_collisions<A: std::hash::Hash + Eq + std::fmt::Display>(
    a_field_map: &impl Fn(&A) -> Option<OptionDefinitionRef>,
    option_name: &A,
    definition: OptionDefinitionRef,
    description: &str,
    allow_duplicates_parsing_equivalently: bool,
) -> Result<(), String> {
    if let Some(other_definition) = a_field_map(option_name) {
        if allow_duplicates_parsing_equivalently
            && equivalent_for_parsing(other_definition, definition)
        {
            return Ok(());
        }
        return Err(format!(
            "Duplicate option name, due to {}: --{}",
            description, option_name
        ));
    }
    Ok(())
}

// Java: com/google/devtools/common/options/IsolatedOptionsData.java:179 IsolatedOptionsData.checkForBooleanAliasCollisions
fn check_for_boolean_alias_collisions(
    boolean_alias_map: &HashMap<String, String>,
    option_name: &str,
    description: &str,
) -> Result<(), String> {
    if let Some(original) = boolean_alias_map.get(option_name) {
        return Err(format!(
            "Duplicate option name, due to {} --{}, it conflicts with a negating alias for boolean flag --{}",
            description, option_name, original
        ));
    }
    Ok(())
}

// Java: com/google/devtools/common/options/IsolatedOptionsData.java:198 IsolatedOptionsData.checkAndUpdateBooleanAliases
fn check_and_update_boolean_aliases(
    name_to_field_map: &IndexMap<String, OptionDefinitionRef>,
    old_name_to_field_map: &IndexMap<String, OptionDefinitionRef>,
    boolean_alias_map: &mut HashMap<String, String>,
    option_name: &str,
    option_definition: OptionDefinitionRef,
    allow_duplicates_parsing_equivalently: bool,
) -> Result<(), String> {
    let no_name = format!("no{}", option_name);
    check_for_collisions(
        &|k: &String| name_to_field_map.get(k).copied(),
        &no_name,
        option_definition,
        "boolean option alias",
        allow_duplicates_parsing_equivalently,
    )?;
    check_for_collisions(
        &|k: &String| old_name_to_field_map.get(k).copied(),
        &no_name,
        option_definition,
        "boolean option alias",
        allow_duplicates_parsing_equivalently,
    )?;
    boolean_alias_map.insert(no_name, option_name.to_string());
    Ok(())
}

impl OptionsData {
    // Java: com/google/devtools/common/options/OptionsParser.java:133 OptionsParser.getOptionsDataInternal
    /// Returns the (cached) options data for the given classes.
    pub fn get(
        classes: &[OptionsClassRef],
        allow_duplicates_parsing_equivalently: bool,
    ) -> Result<Arc<OptionsData>, ConstructionException> {
        let key = (
            classes.iter().map(|c| c.type_id()).collect::<Vec<_>>(),
            allow_duplicates_parsing_equivalently,
        );
        if let Some(d) = OPTIONS_DATA_CACHE.lock().unwrap().get(&key) {
            return Ok(d.clone());
        }
        let data = Arc::new(Self::from(classes, allow_duplicates_parsing_equivalently)?);
        OPTIONS_DATA_CACHE.lock().unwrap().insert(key, data.clone());
        Ok(data)
    }

    // Java: com/google/devtools/common/options/OptionsData.java:79 OptionsData.from
    // Java: com/google/devtools/common/options/IsolatedOptionsData.java:236 IsolatedOptionsData.from
    pub fn from(
        classes: &[OptionsClassRef],
        allow_duplicates_parsing_equivalently: bool,
    ) -> Result<OptionsData, ConstructionException> {
        let mut options_classes: Vec<OptionsClassRef> = Vec::new();
        let mut name_to_field: IndexMap<String, OptionDefinitionRef> = IndexMap::new();
        let mut old_name_to_field: IndexMap<String, OptionDefinitionRef> = IndexMap::new();
        let mut abbrev_to_field: HashMap<char, OptionDefinitionRef> = HashMap::new();
        let mut boolean_alias_map: HashMap<String, String> = HashMap::new();
        let mut uses_only_core_types: HashMap<TypeId, bool> = HashMap::new();
        let dup = |m: String| {
            ConstructionException::from_cause(DUPLICATE_OPTION_DECLARATION_EXCEPTION, m)
        };
        let allow = allow_duplicates_parsing_equivalently;

        for &parsed_options_class in classes {
            if !options_classes.contains(&parsed_options_class) {
                options_classes.push(parsed_options_class);
            }
            for option_definition in parsed_options_class.definitions() {
                let option_name = option_definition.get_option_name().to_string();
                (|| -> Result<(), String> {
                    check_for_collisions(
                        &|k: &String| name_to_field.get(k).copied(),
                        &option_name,
                        option_definition,
                        "option name collision",
                        allow,
                    )?;
                    check_for_collisions(
                        &|k: &String| old_name_to_field.get(k).copied(),
                        &option_name,
                        option_definition,
                        "option name collision with another option's old name",
                        allow,
                    )?;
                    check_for_boolean_alias_collisions(&boolean_alias_map, &option_name, "option")?;
                    if option_definition.uses_boolean_value_syntax() {
                        check_and_update_boolean_aliases(
                            &name_to_field,
                            &old_name_to_field,
                            &mut boolean_alias_map,
                            &option_name,
                            option_definition,
                            allow,
                        )?;
                    }
                    name_to_field.insert(option_name.clone(), option_definition);

                    if !option_definition.get_old_option_name().is_empty() {
                        let old_name = option_definition.get_old_option_name().to_string();
                        check_for_collisions(
                            &|k: &String| name_to_field.get(k).copied(),
                            &old_name,
                            option_definition,
                            "old option name collision with another option's canonical name",
                            allow,
                        )?;
                        check_for_collisions(
                            &|k: &String| old_name_to_field.get(k).copied(),
                            &old_name,
                            option_definition,
                            "old option name collision with another old option name",
                            allow,
                        )?;
                        check_for_boolean_alias_collisions(
                            &boolean_alias_map,
                            &old_name,
                            "old option name",
                        )?;
                        if option_definition.uses_boolean_value_syntax() {
                            check_and_update_boolean_aliases(
                                &name_to_field,
                                &old_name_to_field,
                                &mut boolean_alias_map,
                                &old_name,
                                option_definition,
                                allow,
                            )?;
                        }
                        old_name_to_field.insert(old_name, option_definition);
                    }
                    if option_definition.get_abbreviation() != '\0' {
                        let abbrev = option_definition.get_abbreviation();
                        check_for_collisions(
                            &|k: &char| abbrev_to_field.get(k).copied(),
                            &abbrev,
                            option_definition,
                            "option abbreviation",
                            allow,
                        )?;
                        abbrev_to_field.insert(abbrev, option_definition);
                    }
                    Ok(())
                })()
                .map_err(dup)?;
            }

            let core = parsed_options_class.uses_only_core_types();
            if core {
                for option_definition in parsed_options_class.definitions() {
                    let fi = option_definition.field_info();
                    if !fi.is_core_type || fi.nullable && fi.is_primitive {
                        return Err(ConstructionException::new(format!(
                            "Options class '{}' is marked as @UsesOnlyCoreTypes, but field '{}' has type '{}'",
                            parsed_options_class.name(),
                            option_definition.field_name(),
                            fi.java_type_name
                        )));
                    }
                }
            }
            uses_only_core_types.insert(parsed_options_class.type_id(), core);
        }

        // Validate field types against the Java rules that the Rust type system cannot express
        // (these would be NullPointerExceptions or annotation-processor errors in Java).
        for definition in name_to_field.values() {
            let fi = definition.field_info();
            if definition.is_special_null_default() && !definition.allows_multiple() && !fi.nullable
            {
                return Err(ConstructionException::new(format!(
                    "Option {} has default value \"null\" but its field type {} cannot hold null; \
                     use Option<...>",
                    definition, fi.rust_type_name
                )));
            }
            if definition.allows_multiple() && !fi.is_list {
                return Err(ConstructionException::new(format!(
                    "Option {} allows multiple values but its field type {} is not a Vec",
                    definition, fi.rust_type_name
                )));
            }
        }

        let mut evaluated_expansions = HashMap::new();
        for definition in name_to_field.values() {
            let const_expansion = definition.get_option_expansion();
            if !const_expansion.is_empty() {
                evaluated_expansions.insert(
                    *definition,
                    const_expansion.iter().map(|s| s.to_string()).collect(),
                );
            }
        }

        Ok(OptionsData {
            options_classes,
            name_to_field,
            old_name_to_field,
            abbrev_to_field,
            uses_only_core_types,
            evaluated_expansions,
            allow_duplicates_parsing_equivalently,
        })
    }

    // Java: com/google/devtools/common/options/IsolatedOptionsData.java:120 IsolatedOptionsData.getOptionsClasses
    pub fn get_options_classes(&self) -> &[OptionsClassRef] {
        &self.options_classes
    }

    pub fn contains_class(&self, class: OptionsClassRef) -> bool {
        self.options_classes.contains(&class)
    }

    // Java: com/google/devtools/common/options/IsolatedOptionsData.java:133 IsolatedOptionsData.getOptionDefinitionFromName
    pub fn get_option_definition_from_name(&self, name: &str) -> Option<OptionDefinitionRef> {
        self.name_to_field
            .get(name)
            .or_else(|| self.old_name_to_field.get(name))
            .copied()
    }

    // Java: com/google/devtools/common/options/IsolatedOptionsData.java:142 IsolatedOptionsData.getAllOptionDefinitions
    pub fn get_all_option_definitions(
        &self,
    ) -> impl Iterator<Item = (&str, OptionDefinitionRef)> + '_ {
        self.name_to_field.iter().map(|(k, v)| (k.as_str(), *v))
    }

    // Java: com/google/devtools/common/options/IsolatedOptionsData.java:146 IsolatedOptionsData.getFieldForAbbrev
    pub fn get_field_for_abbrev(&self, abbrev: char) -> Option<OptionDefinitionRef> {
        self.abbrev_to_field.get(&abbrev).copied()
    }

    // Java: com/google/devtools/common/options/IsolatedOptionsData.java:150 IsolatedOptionsData.getUsesOnlyCoreTypes
    pub fn get_uses_only_core_types(&self, class: OptionsClassRef) -> bool {
        self.uses_only_core_types[&class.type_id()]
    }

    // Java: com/google/devtools/common/options/OptionsData.java:58 OptionsData.getEvaluatedExpansion
    pub fn get_evaluated_expansion(&self, option_definition: OptionDefinitionRef) -> Vec<String> {
        self.evaluated_expansions
            .get(option_definition)
            .cloned()
            .unwrap_or_default()
    }

    // Java: com/google/devtools/common/options/OptionsData.java:67 OptionsData.createdWithAllowDuplicatesParsingEquivalently
    pub fn created_with_allow_duplicates_parsing_equivalently(&self) -> bool {
        self.allow_duplicates_parsing_equivalently
    }
}
