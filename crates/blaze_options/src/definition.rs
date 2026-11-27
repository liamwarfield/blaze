//! `OptionDefinition` / `FieldOptionDefinition` / `OptionsBase`.

use std::any::{Any, TypeId};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;

use crate::converter::{ConversionContext, Converter};
use crate::exception::ConstructionException;
use crate::tags::{OptionDocumentationCategory, OptionEffectTag, OptionMetadataTag};
use crate::value::{FieldKind, FieldType, FieldTypeMismatch, Value};

// Java: com/google/devtools/common/options/FieldOptionDefinition.java:41 FieldOptionDefinition.SPECIAL_NULL_DEFAULT_VALUE
pub const SPECIAL_NULL_DEFAULT_VALUE: &str = "null";

// Java: com/google/devtools/common/options/Option.java:35 Option
/// The contents of an `@Option` annotation. Built by [`crate::options!`]; field names are the
/// snake_case of the Java annotation members.
#[derive(Clone)]
pub struct OptionAnnotation {
    pub name: &'static str,
    /// `'\0'` when absent.
    pub abbrev: char,
    pub help: &'static str,
    pub value_help: &'static str,
    /// Required in Java; `None` only while the annotation is being built.
    pub default_value: Option<&'static str>,
    /// Deprecated free-form category (default `"misc"`).
    pub category: &'static str,
    /// Required in Java.
    pub documentation_category: Option<OptionDocumentationCategory>,
    /// Required in Java.
    pub effect_tags: Option<&'static [OptionEffectTag]>,
    pub metadata_tags: &'static [OptionMetadataTag],
    /// `None` means "use the default converter for the field type".
    pub converter: Option<Arc<dyn Converter>>,
    pub allow_multiple: bool,
    pub expansion: &'static [&'static str],
    pub implicit_requirements: &'static [&'static str],
    pub deprecation_warning: &'static str,
    pub old_name: &'static str,
    pub old_name_warning: bool,
    /// The Java field carries `@Deprecated`.
    pub deprecated: bool,
}

impl OptionAnnotation {
    pub const DEFAULT: OptionAnnotation = OptionAnnotation {
        name: "",
        abbrev: '\0',
        help: "",
        value_help: "",
        default_value: None,
        category: "misc",
        documentation_category: None,
        effect_tags: None,
        metadata_tags: &[],
        converter: None,
        allow_multiple: false,
        expansion: &[],
        implicit_requirements: &[],
        deprecation_warning: "",
        old_name: "",
        old_name_warning: true,
        deprecated: false,
    };
}

/// Static facts about the Rust type of an option field (`Field.getType()` in Java).
#[derive(Clone, Copy)]
pub struct FieldInfo {
    pub kind: FieldKind,
    pub is_primitive: bool,
    pub nullable: bool,
    pub is_list: bool,
    pub is_core_type: bool,
    pub java_type_name: &'static str,
    pub rust_type_name: &'static str,
    pub default_converter: fn() -> Option<Arc<dyn Converter>>,
    /// `Class.isEnum()` / `getEnumConstants()` ([`FieldType::enum_constants`]).
    pub enum_constants: fn() -> Option<Vec<String>>,
}

impl FieldInfo {
    pub fn of<T: FieldType>() -> FieldInfo {
        FieldInfo {
            kind: T::KIND,
            is_primitive: T::IS_PRIMITIVE,
            nullable: T::NULLABLE,
            is_list: T::IS_LIST,
            is_core_type: T::IS_CORE_TYPE,
            java_type_name: T::JAVA_TYPE_NAME,
            rust_type_name: std::any::type_name::<T>(),
            default_converter: T::default_converter,
            enum_constants: T::enum_constants,
        }
    }
}

pub type Getter = fn(&dyn Any) -> Option<Value>;
pub type Setter = fn(&mut dyn Any, Option<Value>) -> Result<(), FieldTypeMismatch>;

// Java: com/google/devtools/common/options/OptionDefinition.java:27 OptionDefinition
// Java: com/google/devtools/common/options/FieldOptionDefinition.java:35 FieldOptionDefinition
/// Everything the parser needs to know about how an option is defined. Identity (Java: the
/// `Field`) is the declaring class plus the field name.
pub struct OptionDefinition {
    annotation: OptionAnnotation,
    field_name: &'static str,
    class_name: &'static str,
    class_id: TypeId,
    field: FieldInfo,
    getter: Getter,
    setter: Setter,
    converter: OnceLock<Arc<dyn Converter>>,
    default_value: OnceLock<Option<Value>>,
}

impl OptionDefinition {
    pub fn new<C: 'static, T: FieldType>(
        class_name: &'static str,
        field_name: &'static str,
        annotation: OptionAnnotation,
        getter: Getter,
        setter: Setter,
    ) -> OptionDefinition {
        if annotation.name.is_empty() {
            panic!("@Option on {}.{} is missing `name`", class_name, field_name);
        }
        if annotation.default_value.is_none() {
            panic!(
                "@Option '{}' is missing the required `default_value`",
                annotation.name
            );
        }
        if annotation.documentation_category.is_none() {
            panic!(
                "@Option '{}' is missing the required `documentation_category`",
                annotation.name
            );
        }
        if annotation.effect_tags.is_none() {
            panic!(
                "@Option '{}' is missing the required `effect_tags`",
                annotation.name
            );
        }
        OptionDefinition {
            annotation,
            field_name,
            class_name,
            class_id: TypeId::of::<C>(),
            field: FieldInfo::of::<T>(),
            getter,
            setter,
            converter: OnceLock::new(),
            default_value: OnceLock::new(),
        }
    }

    pub fn annotation(&self) -> &OptionAnnotation {
        &self.annotation
    }

    pub fn field_info(&self) -> &FieldInfo {
        &self.field
    }

    /// The Rust field name (Java: `Field.getName()`, which is camelCase there).
    pub fn field_name(&self) -> &'static str {
        self.field_name
    }

    /// Name of the declaring options class.
    pub fn declaring_class_name(&self) -> &'static str {
        self.class_name
    }

    pub fn declaring_class_id(&self) -> TypeId {
        self.class_id
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:132 FieldOptionDefinition.getRawValue
    pub fn get_raw_value(&self, options: &dyn OptionsBase) -> Option<Value> {
        (self.getter)(options.as_any())
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:146 FieldOptionDefinition.getValue
    pub fn get_value(&self, options: &dyn OptionsBase) -> Option<Value> {
        // A Rust field always holds a value, so the Java fallback to the unparsed default string
        // for raw `null` fields of non-null-default options cannot occur.
        self.get_raw_value(options)
    }

    // Java: com/google/devtools/common/options/OptionDefinition.java:46 OptionDefinition.getBooleanValue
    pub fn get_boolean_value(&self, options: &dyn OptionsBase) -> bool {
        if self.field.kind != FieldKind::Boolean && self.field.java_type_name != "boolean" {
            panic!(
                "IllegalStateException: Option {} is not a boolean, has type {}",
                self.get_option_name(),
                self.field.java_type_name
            );
        }
        self.get_value(options)
            .map(|v| v.downcast_ref::<bool>() == Some(&true))
            .unwrap_or(false)
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:155 FieldOptionDefinition.setValue
    pub fn set_value(
        &self,
        options: &mut dyn OptionsBase,
        value: Option<Value>,
    ) -> Result<(), FieldTypeMismatch> {
        (self.setter)(options.as_any_mut(), value)
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:164 FieldOptionDefinition.isDeprecated
    pub fn is_deprecated(&self) -> bool {
        self.annotation.deprecated
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:174 FieldOptionDefinition.getOptionName
    pub fn get_option_name(&self) -> &'static str {
        self.annotation.name
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:180 FieldOptionDefinition.getAbbreviation
    pub fn get_abbreviation(&self) -> char {
        self.annotation.abbrev
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:186 FieldOptionDefinition.getHelpText
    pub fn get_help_text(&self) -> &'static str {
        self.annotation.help
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:192 FieldOptionDefinition.getValueTypeHelpText
    pub fn get_value_type_help_text(&self) -> &'static str {
        self.annotation.value_help
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:198 FieldOptionDefinition.getUnparsedDefaultValue
    pub fn get_unparsed_default_value(&self) -> &'static str {
        self.annotation.default_value.unwrap_or_default()
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:204 FieldOptionDefinition.getOptionCategory
    pub fn get_option_category(&self) -> &'static str {
        self.annotation.category
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:210 FieldOptionDefinition.getDocumentationCategory
    pub fn get_documentation_category(&self) -> OptionDocumentationCategory {
        self.annotation
            .documentation_category
            .expect("documentation_category checked in new()")
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:216 FieldOptionDefinition.getOptionEffectTags
    pub fn get_option_effect_tags(&self) -> &'static [OptionEffectTag] {
        self.annotation.effect_tags.unwrap_or_default()
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:222 FieldOptionDefinition.getOptionMetadataTags
    pub fn get_option_metadata_tags(&self) -> &'static [OptionMetadataTag] {
        self.annotation.metadata_tags
    }

    /// `getProvidedConverter() != Converter.class`.
    pub fn has_provided_converter(&self) -> bool {
        self.annotation.converter.is_some()
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:235 FieldOptionDefinition.allowsMultiple
    pub fn allows_multiple(&self) -> bool {
        self.annotation.allow_multiple
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:241 FieldOptionDefinition.getOptionExpansion
    pub fn get_option_expansion(&self) -> &'static [&'static str] {
        self.annotation.expansion
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:247 FieldOptionDefinition.getImplicitRequirements
    pub fn get_implicit_requirements(&self) -> &'static [&'static str] {
        self.annotation.implicit_requirements
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:253 FieldOptionDefinition.getDeprecationWarning
    pub fn get_deprecation_warning(&self) -> &'static str {
        self.annotation.deprecation_warning
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:259 FieldOptionDefinition.getOldOptionName
    pub fn get_old_option_name(&self) -> &'static str {
        self.annotation.old_name
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:265 FieldOptionDefinition.getOldNameWarning
    pub fn get_old_name_warning(&self) -> bool {
        self.annotation.old_name_warning
    }

    /// `getType().equals(Void.class)`.
    // Java: com/google/devtools/common/options/OptionDefinition.java:133 OptionDefinition.isVoidField
    pub fn is_void_field(&self) -> bool {
        self.field.kind == FieldKind::Void
    }

    /// `getType().equals(boolean.class)`.
    pub fn is_boolean_field(&self) -> bool {
        self.field.kind == FieldKind::Boolean && self.field.is_primitive
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:275 FieldOptionDefinition.isSpecialNullDefault
    pub fn is_special_null_default(&self) -> bool {
        SPECIAL_NULL_DEFAULT_VALUE == self.get_unparsed_default_value() && !self.field.is_primitive
    }

    // Java: com/google/devtools/common/options/OptionDefinition.java:141 OptionDefinition.isExpansionOption
    pub fn is_expansion_option(&self) -> bool {
        !self.get_option_expansion().is_empty()
    }

    // Java: com/google/devtools/common/options/OptionDefinition.java:146 OptionDefinition.hasImplicitRequirements
    pub fn has_implicit_requirements(&self) -> bool {
        !self.get_implicit_requirements().is_empty()
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:301 FieldOptionDefinition.getConverter
    pub fn get_converter(&self) -> &Arc<dyn Converter> {
        self.converter
            .get_or_init(|| match &self.annotation.converter {
                Some(c) => c.clone(),
                None => match (self.field.default_converter)() {
                    Some(c) => c,
                    None => ConstructionException::new(format!(
                        "Error in the provided converter for option {}",
                        self.field_name
                    ))
                    .throw(),
                },
            })
    }

    // Java: com/google/devtools/common/options/OptionDefinition.java:166 OptionDefinition.usesBooleanValueSyntax
    pub fn uses_boolean_value_syntax(&self) -> bool {
        self.is_boolean_field()
            || self.field.kind == FieldKind::TriState
            || self.get_converter().is_bool_or_enum()
    }

    // Java: com/google/devtools/common/options/OptionDefinition.java:176 OptionDefinition.requiresValue
    pub fn requires_value(&self) -> bool {
        !self.is_void_field() && !self.uses_boolean_value_syntax()
    }

    /// Like [`OptionDefinition::get_default_value`] but returns the `ConstructionException`
    /// instead of throwing it.
    pub fn try_get_default_value(
        &self,
        conversion_context: Option<&ConversionContext>,
    ) -> Result<Option<Value>, ConstructionException> {
        if let Some(v) = self.default_value.get() {
            return Ok(v.clone());
        }
        if self.is_special_null_default() {
            return Ok(if self.allows_multiple() {
                Some(Value::list(vec![]))
            } else {
                None
            });
        }
        let converter = self.get_converter();
        let default_value_as_string = self.get_unparsed_default_value();
        match converter.convert(Some(default_value_as_string), conversion_context) {
            Ok(converted) => {
                let value = if self.allows_multiple() {
                    Some(maybe_wrap_multiple_default_value(converted))
                } else {
                    converted
                };
                // Java memoizes only non-null results.
                if value.is_some() {
                    let _ = self.default_value.set(value.clone());
                }
                Ok(value)
            }
            Err(e) => Err(ConstructionException::with_cause(
                format!(
                    "OptionsParsingException while retrieving the default value for {}: {}",
                    self.field_name,
                    e.get_message()
                ),
                e.get_message(),
            )),
        }
    }

    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:334 FieldOptionDefinition.getDefaultValue
    /// The evaluated default value. A conversion failure is a `ConstructionException` (panic).
    pub fn get_default_value(
        &self,
        conversion_context: Option<&ConversionContext>,
    ) -> Option<Value> {
        match self.try_get_default_value(conversion_context) {
            Ok(v) => v,
            Err(e) => e.throw(),
        }
    }
}

// Java: com/google/devtools/common/options/FieldOptionDefinition.java:84 FieldOptionDefinition.maybeWrapMultipleDefaultValue
fn maybe_wrap_multiple_default_value(converted: Option<Value>) -> Value {
    match converted {
        Some(v) if v.as_list().is_some() => v,
        Some(v) => Value::list(vec![v]),
        None => panic!("NullPointerException: null default value for an allowMultiple option"),
    }
}

impl PartialEq for OptionDefinition {
    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:367 FieldOptionDefinition.equals
    fn eq(&self, other: &Self) -> bool {
        self.class_id == other.class_id && self.field_name == other.field_name
    }
}
impl Eq for OptionDefinition {}

impl Hash for OptionDefinition {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.class_id.hash(state);
        self.field_name.hash(state);
    }
}

impl fmt::Display for OptionDefinition {
    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:386 FieldOptionDefinition.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "option '--{}'", self.get_option_name())
    }
}

impl fmt::Debug for OptionDefinition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

/// A reference to a registered option definition.
pub type OptionDefinitionRef = &'static OptionDefinition;

// ---------------------------------------------------------------------------------------------
// Options classes.

/// Static description of an options class (Java: the `Class<? extends OptionsBase>`).
pub struct OptionsClassInfo {
    name: &'static str,
    type_id: TypeId,
    /// All option definitions, ordered by option name.
    definitions: Vec<OptionDefinition>,
    new_raw: fn() -> Box<dyn OptionsBase>,
    uses_only_core_types: bool,
}

impl OptionsClassInfo {
    pub fn new<C: 'static>(
        name: &'static str,
        uses_only_core_types: bool,
        mut definitions: Vec<OptionDefinition>,
        new_raw: fn() -> Box<dyn OptionsBase>,
    ) -> OptionsClassInfo {
        // Java: com/google/devtools/common/options/IsolatedOptionsData.java:58 IsolatedOptionsData.getAllOptionDefinitionsForClass
        definitions.sort_by(|a, b| a.get_option_name().cmp(b.get_option_name()));
        OptionsClassInfo {
            name,
            type_id: TypeId::of::<C>(),
            definitions,
            new_raw,
            uses_only_core_types,
        }
    }

    /// The class name (the Rust type name; Java would print the fully qualified class name).
    pub fn name(&'static self) -> &'static str {
        self.name
    }

    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// All option definitions of this class, in alphabetic order of option name.
    ///
    /// This is the only order Java exposes: `IsolatedOptionsData.getAllOptionDefinitionsForClass`
    /// sorts `Class.getFields()` by option name, and `getFields()` itself has no specified order.
    /// The other readers of declared field order (`OptionsTester.getAllFields`, serialization
    /// codecs, memory instrumentation) do not reach output, so declaration order is not kept.
    pub fn definitions(&'static self) -> impl Iterator<Item = OptionDefinitionRef> {
        self.definitions.iter()
    }

    /// A fresh instance with every field at its raw (unparsed) value.
    pub fn new_raw_instance(&self) -> Box<dyn OptionsBase> {
        (self.new_raw)()
    }

    /// Whether the class is annotated `@UsesOnlyCoreTypes`.
    pub fn uses_only_core_types(&self) -> bool {
        self.uses_only_core_types
    }
}

impl PartialEq for OptionsClassInfo {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id
    }
}
impl Eq for OptionsClassInfo {}
impl Hash for OptionsClassInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.type_id.hash(state)
    }
}
impl fmt::Debug for OptionsClassInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "class {}", self.name)
    }
}

pub type OptionsClassRef = &'static OptionsClassInfo;

// Java: com/google/devtools/common/options/OptionsBase.java:46 OptionsBase
/// Object-safe base of all options classes.
pub trait OptionsBase: Any + fmt::Debug + Send + Sync {
    fn options_class(&self) -> OptionsClassRef;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn clone_box(&self) -> Box<dyn OptionsBase>;
}

/// Statically typed options class (implemented by [`crate::options!`]).
pub trait OptionsClass: OptionsBase + Clone + Sized {
    fn class() -> OptionsClassRef;
}

impl dyn OptionsBase {
    // Java: com/google/devtools/common/options/OptionsBase.java:73 OptionsBase.asMap
    /// Option name to value, for each option of this object, sorted alphabetically.
    pub fn as_map(&self) -> IndexMap<String, Option<Value>> {
        let mut map = IndexMap::new();
        for definition in self.options_class().definitions() {
            map.insert(
                definition.get_option_name().to_string(),
                self.get_value_from_definition(definition),
            );
        }
        map
    }

    // Java: com/google/devtools/common/options/OptionsBase.java:84 OptionsBase.getValueFromDefinition
    pub fn get_value_from_definition(&self, definition: &OptionDefinition) -> Option<Value> {
        definition.get_value(self)
    }

    /// `toString` with `getClass().getName()` given as `class_name` (the options classes here
    /// know their simple name only; the fragment options classes know the binary name, see
    /// `blaze_build_config::fragment_options_class`).
    // Java: com/google/devtools/common/options/OptionsBase.java:89 OptionsBase.toString
    pub fn java_to_string_with_class_name(&self, class_name: &str) -> String {
        let entries: Vec<String> = self
            .as_map()
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, crate::value::java_string_of(v.as_ref())))
            .collect();
        format!("{}{{{}}}", class_name, entries.join(", "))
    }

    /// `toString` with the simple class name in place of `getClass().getName()`.
    pub fn java_to_string(&self) -> String {
        self.java_to_string_with_class_name(self.options_class().name())
    }

    // Java: com/google/devtools/common/options/OptionsBase.java:97 OptionsBase.cacheKey
    pub fn cache_key(&self) -> String {
        format!(
            "{}{{{}}}",
            self.options_class().name(),
            map_to_cache_key(&self.as_map())
        )
    }

    // Java: com/google/devtools/common/options/OptionsBase.java:127 OptionsBase.equals
    pub fn options_equal(&self, other: &dyn OptionsBase) -> bool {
        if self.options_class() != other.options_class() {
            return false;
        }
        self.options_class()
            .definitions()
            .all(|d| self.get_value_from_definition(d) == other.get_value_from_definition(d))
    }
}

// Java: com/google/devtools/common/options/OptionsBase.java:103 OptionsBase.mapToCacheKey
pub fn map_to_cache_key(options_map: &IndexMap<String, Option<Value>>) -> String {
    let mut result = String::new();
    for (key, value) in options_map {
        result.push_str(key);
        result.push('=');
        match value {
            Some(v) if v.as_list().is_some_and(|l| l.is_empty()) => result.push_str("EMPTY"),
            None => result.push_str("NULL"),
            Some(v) => {
                result.push('"');
                result.push_str(
                    &v.java_to_string()
                        .replace('\\', "\\\\")
                        .replace('"', "\\\""),
                );
                result.push('"');
            }
        }
        result.push_str(", ");
    }
    result
}
