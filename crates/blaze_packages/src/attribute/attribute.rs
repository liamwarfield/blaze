//! Port of `packages/Attribute.java`: [`Attribute`], [`AttributeBuilder`] (Java
//! `Attribute.Builder`), [`ImmutableAttributeFactory`], [`AllowedValueSet`] and the
//! `PropertyFlag` set. The default-computing classes (`ComputedDefault`,
//! `StarlarkComputedDefaultTemplate`, `LateBoundDefault`, ...) are in
//! [`crate::attribute::computed_default`].

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::EvalException;
use starlark::values::Heap;
use starlark::values::Value;

use crate::attribute::attribute_value_source::AttributeValueSource;
use crate::attribute::computed_default::ComputedDefault;
use crate::attribute::computed_default::LateBoundDefault;
use crate::attribute::computed_default::MaterializingDefault;
use crate::attribute::computed_default::StarlarkComputedDefaultTemplate;
use crate::attribute::string_util::join_english_list_single_quoted;
use crate::attribute::transition_factory::TransitionFactory;
use crate::rule_class::AspectsList;
use crate::rule_class::AspectsListBuilder;
use crate::rule_class::RequiredProviders;
use crate::rule_class::RequiredProvidersBuilder;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassNamePredicate;
use crate::rule_class::StarlarkCallable;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::ConversionException;
use crate::types::FileTypeSet;
use crate::types::LabelClass;
use crate::types::LabelConverter;
use crate::types::Type;
use crate::types::select::SelectorList;
use crate::types::select::SelectorValue;
use crate::types::type_::alloc_immutable_dict;
use crate::types::type_::alloc_immutable_list;
use blaze_starlark::label::LabelValue;

/// Java `Object.toString()` of an attribute value, for error messages.
// PORT: Java prints the value's Java form (String as is, List as "[a, b]", Map as "{k=v}").
pub(crate) fn java_to_string(v: &AttrValue) -> String {
    fn list<T>(items: impl Iterator<Item = T>, f: impl Fn(T) -> String) -> String {
        format!("[{}]", items.map(f).collect::<Vec<_>>().join(", "))
    }
    match v {
        AttrValue::Int(i) => i.to_string(),
        AttrValue::String(s) => s.clone(),
        AttrValue::Bool(b) => b.to_string(),
        AttrValue::Label(l) => l.to_string(),
        AttrValue::StringList(l) => list(l.iter(), |s| s.clone()),
        AttrValue::StringSet(l) => list(l.iter(), |s| s.clone()),
        AttrValue::IntList(l) => list(l.iter(), |i| i.to_string()),
        AttrValue::LabelList(l) => list(l.iter(), |x| x.to_string()),
        AttrValue::StringDict(m) => format!(
            "{{{}}}",
            m.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        AttrValue::TriState(t) => format!("{t:?}").to_uppercase(),
        other => format!("{other:?}"),
    }
}

/// Java `Attribute.ANY_RULE`.
// Java: com/google/devtools/build/lib/packages/Attribute.java:77 Attribute.ANY_RULE
pub fn any_rule() -> RuleClassNamePredicate {
    RuleClassNamePredicate::unspecified()
}

// Java: com/google/devtools/build/lib/packages/Attribute.java:79 Attribute.NO_RULE
fn no_rule() -> RuleClassNamePredicate {
    RuleClassNamePredicate::only(&[])
}

/// Attribute property flags (Java `Attribute.PropertyFlag`, private there). `Ord` is Java's
/// declaration order (EnumSet iteration order).
// Java: com/google/devtools/build/lib/packages/Attribute.java:83 Attribute.PropertyFlag
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PropertyFlag {
    Mandatory,
    Executable,
    Undocumented,
    Taggable,
    OrderIndependent,
    StrictLabelChecking,
    DirectCompileTimeInput,
    NonEmpty,
    SingleArtifact,
    SilentRuleclassFilter,
    SkipAnalysisTimeFiletypeCheck,
    CheckAllowedValues,
    Nonconfigurable,
    ConfigurableAttrWasUserSet,
    SkipPrereqValidatorChecks,
    CheckConstraintsOverride,
    SkipConstraintsOverride,
    OutputLicenses,
    HasStarlarkDefinedTransition,
    HasAnalysisTestTransition,
    IsToolDependency,
    StarlarkDefined,
    SkipValidations,
    ForDependencyResolution,
    ForDependencyResolutionExplicitlySet,
}

impl PropertyFlag {
    /// Java `PropertyFlag.valueOf(name)` (the Java constant names).
    pub fn value_of(name: &str) -> Option<PropertyFlag> {
        use PropertyFlag::*;
        Some(match name {
            "MANDATORY" => Mandatory,
            "EXECUTABLE" => Executable,
            "UNDOCUMENTED" => Undocumented,
            "TAGGABLE" => Taggable,
            "ORDER_INDEPENDENT" => OrderIndependent,
            "STRICT_LABEL_CHECKING" => StrictLabelChecking,
            "DIRECT_COMPILE_TIME_INPUT" => DirectCompileTimeInput,
            "NON_EMPTY" => NonEmpty,
            "SINGLE_ARTIFACT" => SingleArtifact,
            "SILENT_RULECLASS_FILTER" => SilentRuleclassFilter,
            "SKIP_ANALYSIS_TIME_FILETYPE_CHECK" => SkipAnalysisTimeFiletypeCheck,
            "CHECK_ALLOWED_VALUES" => CheckAllowedValues,
            "NONCONFIGURABLE" => Nonconfigurable,
            "CONFIGURABLE_ATTR_WAS_USER_SET" => ConfigurableAttrWasUserSet,
            "SKIP_PREREQ_VALIDATOR_CHECKS" => SkipPrereqValidatorChecks,
            "CHECK_CONSTRAINTS_OVERRIDE" => CheckConstraintsOverride,
            "SKIP_CONSTRAINTS_OVERRIDE" => SkipConstraintsOverride,
            "OUTPUT_LICENSES" => OutputLicenses,
            "HAS_STARLARK_DEFINED_TRANSITION" => HasStarlarkDefinedTransition,
            "HAS_ANALYSIS_TEST_TRANSITION" => HasAnalysisTestTransition,
            "IS_TOOL_DEPENDENCY" => IsToolDependency,
            "STARLARK_DEFINED" => StarlarkDefined,
            "SKIP_VALIDATIONS" => SkipValidations,
            "FOR_DEPENDENCY_RESOLUTION" => ForDependencyResolution,
            "FOR_DEPENDENCY_RESOLUTION_EXPLICITLY_SET" => ForDependencyResolutionExplicitlySet,
            _ => return None,
        })
    }
}

/// Java `PredicateWithMessage<Object>` (lib/util): a predicate on attribute values with an
/// error reason.
pub trait PredicateWithMessage: fmt::Debug + Send + Sync {
    fn apply(&self, input: &AttrValue) -> bool;

    fn get_error_reason(&self, value: &AttrValue) -> String;

    /// `this instanceof Attribute.AllowedValueSet`.
    fn allowed_value_set(&self) -> Option<&AllowedValueSet> {
        None
    }
}

/// A shared [`PredicateWithMessage`]; equality is identity (Java does not override equals).
#[derive(Debug, Clone)]
pub struct AllowedValues(pub Arc<dyn PredicateWithMessage>);

impl PartialEq for AllowedValues {
    fn eq(&self, other: &AllowedValues) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A set of allowed values for an attribute (Java `Attribute.AllowedValueSet`).
// Java: com/google/devtools/build/lib/packages/Attribute.java:210 Attribute.AllowedValueSet
#[derive(Debug, Clone, PartialEq)]
pub struct AllowedValueSet {
    // PORT: Java ImmutableSet<Object> of Starlark values (strings, ints); stored converted.
    allowed_values: Vec<AttrValue>,
}

impl AllowedValueSet {
    // Java: com/google/devtools/build/lib/packages/Attribute.java:219 Attribute.AllowedValueSet.<init>
    pub fn new(values: Vec<AttrValue>) -> Self {
        blaze_util::check_argument!(!values.is_empty());
        let mut allowed_values: Vec<AttrValue> = Vec::with_capacity(values.len());
        for v in values {
            if !allowed_values.contains(&v) {
                allowed_values.push(v);
            }
        }
        AllowedValueSet { allowed_values }
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:247 Attribute.AllowedValueSet.getAllowedValues
    pub fn get_allowed_values(&self) -> &[AttrValue] {
        &self.allowed_values
    }
}

impl PredicateWithMessage for AllowedValueSet {
    fn allowed_value_set(&self) -> Option<&AllowedValueSet> {
        Some(self)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:231 Attribute.AllowedValueSet.apply
    fn apply(&self, input: &AttrValue) -> bool {
        self.allowed_values.contains(input)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:236 Attribute.AllowedValueSet.getErrorReason
    fn get_error_reason(&self, value: &AttrValue) -> String {
        let allowed: Vec<String> = self.allowed_values.iter().map(java_to_string).collect();
        format!(
            "has to be one of {} instead of '{}'",
            join_english_list_single_quoted(&allowed),
            java_to_string(value)
        )
    }
}

/// Creates a new attribute builder (Java `Attribute.attr(name, type)`).
// Java: com/google/devtools/build/lib/packages/Attribute.java:254 Attribute.attr
pub fn attr(name: &str, type_: Type) -> AttributeBuilder {
    AttributeBuilder::new(name, type_)
}

/// An attribute definition without a name, from which attributes are built (Java
/// `Attribute.ImmutableAttributeFactory`).
// Java: com/google/devtools/build/lib/packages/Attribute.java:258 Attribute.ImmutableAttributeFactory
// PORT: skipped, derived PartialEq over the same fields (never a hash key): Attribute.ImmutableAttributeFactory.equals
#[derive(Debug, Clone, PartialEq)]
pub struct ImmutableAttributeFactory {
    type_: Type,
    doc: Option<String>,
    transition_factory: TransitionFactory,
    allowed_rule_classes_for_labels: RuleClassNamePredicate,
    allowed_rule_classes_for_labels_warning: RuleClassNamePredicate,
    allowed_file_types_for_labels: Option<FileTypeSet>,
    value: Option<AttrValue>,
    value_source: AttributeValueSource,
    value_set: bool,
    property_flags: BTreeSet<PropertyFlag>,
    allowed_values: Option<AllowedValues>,
    required_providers: RequiredProviders,
    aspects: AspectsList,
}

impl ImmutableAttributeFactory {
    // Java: com/google/devtools/build/lib/packages/Attribute.java:321 Attribute.ImmutableAttributeFactory.getValueSource
    pub fn get_value_source(&self) -> AttributeValueSource {
        self.value_source
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:325 Attribute.ImmutableAttributeFactory.isValueSet
    pub fn is_value_set(&self) -> bool {
        self.value_set
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:329 Attribute.ImmutableAttributeFactory.getType
    pub fn get_type(&self) -> Type {
        self.type_
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:333 Attribute.ImmutableAttributeFactory.build
    pub fn build(&self, name: &str) -> Attribute {
        blaze_util::check_state!(!name.is_empty(), "name has not been set");
        if self.value_source == AttributeValueSource::LateBound {
            blaze_util::check_state!(Attribute::is_analysis_dependent(name));
            blaze_util::check_state!(!self.transition_factory.is_split());
        }
        if self.value_source == AttributeValueSource::Materializer {
            blaze_util::check_state!(Attribute::is_analysis_dependent(name));
        }
        // TODO(bazel-team): Set the default to be no file type, then remove this check, and also
        // remove all allowedFileTypes() calls without parameters.

        // do not modify this.allowedFileTypesForLabels, instead create a copy.
        let mut allowed_file_types_for_labels = self.allowed_file_types_for_labels.clone();
        if self.type_.get_label_class() == LabelClass::Dependency {
            if Attribute::is_private_attribute(name) && allowed_file_types_for_labels.is_none() {
                allowed_file_types_for_labels = Some(FileTypeSet::AnyFile);
            }
            if allowed_file_types_for_labels.is_none() {
                panic!("NullPointerException: allowedFileTypesForLabels not set for {name}");
            }
        } else if self.type_.get_label_class() == LabelClass::Output {
            // TODO(bazel-team): Set the default to no file type and make explicit calls instead.
            if allowed_file_types_for_labels.is_none() {
                allowed_file_types_for_labels = Some(FileTypeSet::AnyFile);
            }
        }
        Attribute::new(
            name.to_owned(),
            self.doc.clone(),
            self.type_,
            self.property_flags.clone(),
            self.value.clone(),
            self.transition_factory.clone(),
            self.allowed_rule_classes_for_labels.clone(),
            self.allowed_rule_classes_for_labels_warning.clone(),
            allowed_file_types_for_labels,
            self.allowed_values.clone(),
            self.required_providers.clone(),
            self.aspects.clone(),
        )
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:410 Attribute.ImmutableAttributeFactory.getTransitionFactory
    pub fn get_transition_factory(&self) -> &TransitionFactory {
        &self.transition_factory
    }
}

/// Builder for [`Attribute`] (Java `Attribute.Builder<TYPE>`; the Rust builder is untyped and
/// checks values against its [`Type`]).
// Java: com/google/devtools/build/lib/packages/Attribute.java:417 Attribute.Builder
#[derive(Debug, Clone)]
pub struct AttributeBuilder {
    name: String,
    type_: Type,
    transition_factory: TransitionFactory,
    allowed_rule_classes_for_labels: RuleClassNamePredicate,
    allowed_rule_classes_for_labels_warning: RuleClassNamePredicate,
    allowed_file_types_for_labels: Option<FileTypeSet>,
    value: Option<AttrValue>,
    doc: Option<String>,
    value_source: AttributeValueSource,
    value_set: bool,
    property_flags: BTreeSet<PropertyFlag>,
    allowed_values: Option<AllowedValues>,
    required_providers_builder: RequiredProvidersBuilder,
    aspects_list_builder: AspectsListBuilder,
}

impl AttributeBuilder {
    // Java: com/google/devtools/build/lib/packages/Attribute.java:444 Attribute.Builder.<init>
    pub fn new(name: &str, type_: Type) -> Self {
        let mut b = AttributeBuilder {
            name: name.to_owned(),
            type_,
            transition_factory: TransitionFactory::NoTransition,
            allowed_rule_classes_for_labels: any_rule(),
            allowed_rule_classes_for_labels_warning: no_rule(),
            allowed_file_types_for_labels: None,
            value: None,
            doc: None,
            value_source: AttributeValueSource::Direct,
            value_set: false,
            property_flags: BTreeSet::new(),
            allowed_values: None,
            required_providers_builder: RequiredProviders::accept_any_builder(),
            aspects_list_builder: AspectsListBuilder::new(),
        };
        if Attribute::is_implicit_name(name) || Attribute::is_analysis_dependent(name) {
            b.set_property_flag_internal(PropertyFlag::Undocumented);
        }
        b
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:453 Attribute.Builder.setPropertyFlag
    fn set_property_flag_internal(&mut self, flag: PropertyFlag) -> &mut Self {
        self.property_flags.insert(flag);
        self
    }

    /// Sets a property flag by its Java name.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:466 Attribute.Builder.setPropertyFlag
    pub fn set_property_flag(&mut self, property_name: &str) -> Result<&mut Self, EvalException> {
        let flag = Self::resolve_property_flag_by_name(property_name)?;
        Ok(self.set_property_flag_internal(flag))
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:472 Attribute.Builder.resolvePropertyFlagByName
    fn resolve_property_flag_by_name(property_name: &str) -> Result<PropertyFlag, EvalException> {
        PropertyFlag::value_of(property_name)
            .ok_or_else(|| EvalException::new(format!("unknown attribute flag '{property_name}'")))
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:482 Attribute.Builder.removePropertyFlag
    pub fn remove_property_flag(
        &mut self,
        property_name: &str,
    ) -> Result<&mut Self, EvalException> {
        let flag = Self::resolve_property_flag_by_name(property_name)?;
        self.property_flags.remove(&flag);
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:489 Attribute.Builder.mandatory
    pub fn mandatory(&mut self) -> &mut Self {
        self.set_property_flag_internal(PropertyFlag::Mandatory)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:497 Attribute.Builder.nonEmpty
    pub fn non_empty(&mut self) -> &mut Self {
        if self.type_.get_list_element_type().is_none() {
            panic!(
                "NullPointerException: attribute '{}' must be a list",
                self.name
            );
        }
        self.set_property_flag_internal(PropertyFlag::NonEmpty)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:503 Attribute.Builder.singleArtifact
    pub fn single_artifact(&mut self) -> &mut Self {
        blaze_util::check_state!(
            self.type_.get_label_class() == LabelClass::Dependency,
            "attribute '{}' must be a label-valued type",
            self.name
        );
        self.set_property_flag_internal(PropertyFlag::SingleArtifact)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:515 Attribute.Builder.silentRuleClassFilter
    pub fn silent_rule_class_filter(&mut self) -> &mut Self {
        blaze_util::check_state!(
            self.type_.get_label_class() == LabelClass::Dependency,
            "must be a label-valued type"
        );
        self.set_property_flag_internal(PropertyFlag::SilentRuleclassFilter)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:522 Attribute.Builder.skipAnalysisTimeFileTypeCheck
    pub fn skip_analysis_time_file_type_check(&mut self) -> &mut Self {
        blaze_util::check_state!(
            self.type_.get_label_class() == LabelClass::Dependency,
            "must be a label-valued type"
        );
        self.set_property_flag_internal(PropertyFlag::SkipAnalysisTimeFiletypeCheck)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:530 Attribute.Builder.orderIndependent
    pub fn order_independent(&mut self) -> &mut Self {
        if self.type_.get_list_element_type().is_none() {
            panic!(
                "NullPointerException: attribute '{}' must be a list",
                self.name
            );
        }
        self.set_property_flag_internal(PropertyFlag::OrderIndependent)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:536 Attribute.Builder.useOutputLicenses
    pub fn use_output_licenses(&mut self) -> &mut Self {
        blaze_util::check_state!(self.type_.is_label_type(), "must be a label type");
        self.set_property_flag_internal(PropertyFlag::OutputLicenses)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:546 Attribute.Builder.hasStarlarkDefinedTransition
    pub fn has_starlark_defined_transition(&mut self) -> &mut Self {
        self.set_property_flag_internal(PropertyFlag::HasStarlarkDefinedTransition)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:554 Attribute.Builder.hasAnalysisTestTransition
    pub fn has_analysis_test_transition(&mut self) -> &mut Self {
        self.set_property_flag_internal(PropertyFlag::HasAnalysisTestTransition)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:560 Attribute.Builder.cfg
    pub fn cfg(&mut self, transition_factory: TransitionFactory) -> &mut Self {
        blaze_util::check_state!(
            self.transition_factory.is_no_transition(),
            "the configuration transition is already set"
        );
        self.transition_factory = transition_factory;
        self
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:573 Attribute.Builder.exec
    pub fn exec(&mut self) -> &mut Self {
        self.set_property_flag_internal(PropertyFlag::Executable)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:581 Attribute.Builder.direct_compile_time_input
    pub fn direct_compile_time_input(&mut self) -> &mut Self {
        self.set_property_flag_internal(PropertyFlag::DirectCompileTimeInput)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:591 Attribute.Builder.undocumented
    pub fn undocumented(&mut self, reason: &str) -> &mut Self {
        let _ = reason;
        self.set_property_flag_internal(PropertyFlag::Undocumented)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:601 Attribute.Builder.setDoc
    pub fn set_doc(&mut self, doc: Option<String>) -> &mut Self {
        self.doc = doc;
        self
    }

    /// Java `value(TYPE defaultValue)`: a default of this attribute's type.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:616 Attribute.Builder.value
    pub fn value(&mut self, default_value: Option<AttrValue>) -> &mut Self {
        self.value = default_value;
        self.value_set = true;
        self
    }

    /// Java `value(ComputedDefault)`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:633 Attribute.Builder.value
    pub fn value_computed_default(&mut self, default_value: Arc<ComputedDefault>) -> &mut Self {
        self.value = Some(AttrValue::ComputedDefault(default_value));
        self.value_source = AttributeValueSource::ComputedDefault;
        self.value_set = true;
        self
    }

    /// Java `value(NativeComputedDefaultApi)` (a `_builtins` native computed default).
    // Java: com/google/devtools/build/lib/packages/Attribute.java:642 Attribute.Builder.value
    pub fn value_native_computed_default(&mut self, default_value: StarlarkCallable) -> &mut Self {
        let _ = default_value;
        blaze_util::unported!("Attribute.Builder.value(NativeComputedDefaultApi)")
    }

    /// Java `value(StarlarkComputedDefaultTemplate)`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:664 Attribute.Builder.value
    pub fn value_starlark_computed_default(
        &mut self,
        template: Arc<StarlarkComputedDefaultTemplate>,
    ) -> &mut Self {
        self.value = Some(AttrValue::StarlarkComputedDefaultTemplate(template));
        self.value_source = AttributeValueSource::ComputedDefault;
        self.value_set = true;
        self
    }

    /// Java `value(LateBoundDefault)`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:676 Attribute.Builder.value
    pub fn value_late_bound(&mut self, default_value: Arc<LateBoundDefault>) -> &mut Self {
        self.value = Some(AttrValue::LateBound(default_value));
        self.value_source = AttributeValueSource::LateBound;
        self.value_set = true;
        self
    }

    /// Java `value(MaterializingDefault)`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:684 Attribute.Builder.value
    pub fn value_materializer(&mut self, default_value: Arc<MaterializingDefault>) -> &mut Self {
        self.value = Some(AttrValue::Materializer(default_value));
        self.value_source = AttributeValueSource::Materializer;
        self.value_set = true;
        self
    }

    /// Converts `default_value` (a Starlark value) with this attribute's type and sets it.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:699 Attribute.Builder.defaultValue
    // Java: com/google/devtools/build/lib/packages/Attribute.java:713 Attribute.Builder.defaultValue
    // Java: com/google/devtools/build/lib/packages/Attribute.java:// (Java's one-argument overload passes no converter and no parameter name: `None, None`)
    pub fn default_value<'v>(
        &mut self,
        default_value: Value<'v>,
        label_converter: Option<&LabelConverter>,
        parameter_name: Option<&str>,
    ) -> Result<&mut Self, ConversionException> {
        let what = match parameter_name {
            None => format!("attribute '{}'", self.name),
            Some(p) => format!("parameter '{}' of attribute '{}'", p, self.name),
        };
        self.value = Some(
            self.type_
                .convert(default_value, Some(&what), label_converter)?,
        );
        self.value_set = true;
        Ok(self)
    }

    /// Sets the default to Java null.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:722 Attribute.Builder.defaultValueNone
    pub fn default_value_none(&mut self) -> &mut Self {
        self.value = None;
        self.value_set = true;
        self.value_source = AttributeValueSource::Direct;
        self
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:730 Attribute.Builder.getValueSource
    pub fn get_value_source(&self) -> AttributeValueSource {
        self.value_source
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:735 Attribute.Builder.taggable
    pub fn taggable(&mut self) -> &mut Self {
        self.set_property_flag_internal(PropertyFlag::Taggable)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:743 Attribute.Builder.skipPrereqValidatorCheck
    pub fn skip_prereq_validator_check(&mut self) -> &mut Self {
        self.set_property_flag_internal(PropertyFlag::SkipPrereqValidatorChecks)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:756 Attribute.Builder.checkConstraints
    pub fn check_constraints(&mut self) -> &mut Self {
        if self
            .property_flags
            .contains(&PropertyFlag::SkipConstraintsOverride)
        {
            panic!("VerifyException: constraint checking is already overridden to be skipped");
        }
        self.set_property_flag_internal(PropertyFlag::CheckConstraintsOverride)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:770 Attribute.Builder.dontCheckConstraints
    pub fn dont_check_constraints(&mut self) -> &mut Self {
        if self
            .property_flags
            .contains(&PropertyFlag::CheckConstraintsOverride)
        {
            panic!("VerifyException: constraint checking is already overridden to be checked");
        }
        self.set_property_flag_internal(PropertyFlag::SkipConstraintsOverride)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:792 Attribute.Builder.allowedRuleClasses
    // Java: com/google/devtools/build/lib/packages/Attribute.java:835 Attribute.Builder.allowedRuleClasses
    pub fn allowed_rule_classes(&mut self, allowed_rule_classes: &[&str]) -> &mut Self {
        self.allowed_rule_classes_predicate(RuleClassNamePredicate::only(allowed_rule_classes))
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:812 Attribute.Builder.allowedRuleClasses
    pub fn allowed_rule_classes_predicate(
        &mut self,
        allowed_rule_classes: RuleClassNamePredicate,
    ) -> &mut Self {
        blaze_util::check_state!(
            self.type_.get_label_class() == LabelClass::Dependency,
            "must be a label-valued type"
        );
        self.property_flags
            .insert(PropertyFlag::StrictLabelChecking);
        self.allowed_rule_classes_for_labels = allowed_rule_classes;
        self
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:848 Attribute.Builder.allowedFileTypes
    // Java: com/google/devtools/build/lib/packages/Attribute.java:866 Attribute.Builder.allowedFileTypes
    pub fn allowed_file_types(&mut self, allowed_file_types: FileTypeSet) -> &mut Self {
        blaze_util::check_state!(
            self.type_.get_label_class() == LabelClass::Dependency
                || self.type_.get_label_class() == LabelClass::GenqueryScopeReference,
            "must be a label-valued type"
        );
        self.property_flags
            .insert(PropertyFlag::StrictLabelChecking);
        self.allowed_file_types_for_labels = Some(allowed_file_types);
        self
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:875 Attribute.Builder.legacyAllowAnyFileType
    pub fn legacy_allow_any_file_type(&mut self) -> &mut Self {
        self.allowed_file_types(FileTypeSet::AnyFile)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:894 Attribute.Builder.allowedRuleClassesWithWarning
    // Java: com/google/devtools/build/lib/packages/Attribute.java:937 Attribute.Builder.allowedRuleClassesWithWarning
    pub fn allowed_rule_classes_with_warning(
        &mut self,
        allowed_rule_classes: &[&str],
    ) -> &mut Self {
        self.allowed_rule_classes_with_warning_predicate(RuleClassNamePredicate::only(
            allowed_rule_classes,
        ))
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:913 Attribute.Builder.allowedRuleClassesWithWarning
    pub fn allowed_rule_classes_with_warning_predicate(
        &mut self,
        allowed_rule_classes: RuleClassNamePredicate,
    ) -> &mut Self {
        blaze_util::check_state!(
            self.type_.get_label_class() == LabelClass::Dependency,
            "must be a label-valued type"
        );
        self.property_flags
            .insert(PropertyFlag::StrictLabelChecking);
        self.allowed_rule_classes_for_labels_warning = allowed_rule_classes;
        self
    }

    /// Java `mandatoryBuiltinProvidersList`; builtin providers are named by their Java class
    /// simple name.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:947 Attribute.Builder.mandatoryBuiltinProvidersList
    pub fn mandatory_builtin_providers_list(
        &mut self,
        providers_list: Vec<Vec<String>>,
    ) -> &mut Self {
        blaze_util::check_state!(
            self.type_.get_label_class() == LabelClass::Dependency,
            "must be a label-valued type"
        );
        for providers in providers_list {
            self.required_providers_builder.add_builtin_set(providers);
        }
        self
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:959 Attribute.Builder.mandatoryBuiltinProviders
    pub fn mandatory_builtin_providers(&mut self, providers: Vec<String>) -> &mut Self {
        if !providers.is_empty() {
            self.mandatory_builtin_providers_list(vec![providers]);
        }
        self
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:973 Attribute.Builder.mandatoryProvidersList
    pub fn mandatory_providers_list(
        &mut self,
        providers_list: Vec<Vec<StarlarkProviderIdentifier>>,
    ) -> &mut Self {
        blaze_util::check_state!(
            self.type_.get_label_class() == LabelClass::Dependency,
            "must be a label-valued type"
        );
        for providers in providers_list {
            self.required_providers_builder.add_starlark_set(providers);
        }
        self
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:984 Attribute.Builder.mandatoryProviders
    // Java: com/google/devtools/build/lib/packages/Attribute.java:991 Attribute.Builder.mandatoryProviders
    pub fn mandatory_providers(&mut self, providers: Vec<StarlarkProviderIdentifier>) -> &mut Self {
        if !providers.is_empty() {
            self.mandatory_providers_list(vec![providers]);
        }
        self
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:997 Attribute.Builder.addAspects
    pub fn add_aspects(&mut self, aspects_list: &AspectsList) -> Result<(), EvalException> {
        self.aspects_list_builder.add_aspects(aspects_list)
    }

    /// Java `aspect(StarlarkAspect)`: `aspect` is the frozen Starlark aspect value.
    // PORT: skipped, only `attr.*(aspects = ...)` adds aspects at the pin (StarlarkAttrModule): Attribute.aspect
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1002 Attribute.Builder.aspect
    pub fn aspect(&mut self, aspect: StarlarkCallable) -> Result<&mut Self, EvalException> {
        self.aspects_list_builder.add_starlark_aspect(aspect)?;
        Ok(self)
    }

    /// Java `aspect(StarlarkAspect)` with the aspect's description.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1002 Attribute.Builder.aspect
    pub fn aspect_info(
        &mut self,
        aspect: &crate::rule_class::StarlarkAspectInfo,
    ) -> Result<&mut Self, EvalException> {
        self.aspects_list_builder.add_starlark_aspect_info(aspect)?;
        Ok(self)
    }

    /// Java `aspect(NativeAspectClass, Function<Rule, AspectParameters>)`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1015 Attribute.Builder.aspect
    pub fn native_aspect_with_extractor(
        &mut self,
        aspect: Arc<dyn crate::aspect::NativeAspectClass>,
        evaluator: crate::rule_class::AspectParametersExtractor,
    ) -> &mut Self {
        self.aspects_list_builder
            .add_native_aspect_with_extractor(aspect, evaluator);
        self
    }

    /// Java `aspect(NativeAspectClass)`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1026 Attribute.Builder.aspect
    pub fn native_aspect(
        &mut self,
        aspect: Arc<dyn crate::aspect::NativeAspectClass>,
    ) -> &mut Self {
        self.aspects_list_builder.add_native_aspect(aspect);
        self
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1040 Attribute.Builder.allowedValues
    pub fn allowed_values(&mut self, allowed_values: Arc<dyn PredicateWithMessage>) -> &mut Self {
        self.allowed_values = Some(AllowedValues(allowed_values));
        self.property_flags.insert(PropertyFlag::CheckAllowedValues);
        self
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1059 Attribute.Builder.nonconfigurable
    pub fn nonconfigurable(&mut self, reason: &str) -> &mut Self {
        blaze_util::check_state!(!reason.is_empty());
        self.set_property_flag_internal(PropertyFlag::Nonconfigurable)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1065 Attribute.Builder.configurableAttrWasUserSet
    pub fn configurable_attr_was_user_set(&mut self) -> &mut Self {
        self.set_property_flag_internal(PropertyFlag::ConfigurableAttrWasUserSet)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1069 Attribute.Builder.tool
    pub fn tool(&mut self, reason: &str) -> &mut Self {
        blaze_util::check_state!(
            self.type_.get_label_class() == LabelClass::Dependency,
            "must be a label-valued type"
        );
        blaze_util::check_state!(!reason.is_empty());
        self.set_property_flag_internal(PropertyFlag::IsToolDependency)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1078 Attribute.Builder.starlarkDefined
    pub fn starlark_defined(&mut self) -> &mut Self {
        self.set_property_flag_internal(PropertyFlag::StarlarkDefined)
    }

    /// Builds the attribute definition without a name.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1083 Attribute.Builder.buildPartial
    pub fn build_partial(&self) -> ImmutableAttributeFactory {
        if self
            .allowed_rule_classes_for_labels
            .considered_overlapping(&self.allowed_rule_classes_for_labels_warning)
        {
            panic!(
                "IllegalStateException: allowedRuleClasses {} and allowedRuleClassesWithWarning {} \
                 may not contain the same rule classes",
                self.allowed_rule_classes_for_labels.describe(),
                self.allowed_rule_classes_for_labels_warning.describe()
            );
        }
        ImmutableAttributeFactory {
            type_: self.type_,
            doc: self.doc.clone(),
            transition_factory: self.transition_factory.clone(),
            allowed_rule_classes_for_labels: self.allowed_rule_classes_for_labels.clone(),
            allowed_rule_classes_for_labels_warning: self
                .allowed_rule_classes_for_labels_warning
                .clone(),
            allowed_file_types_for_labels: self.allowed_file_types_for_labels.clone(),
            value: if self.value_set {
                self.value.clone()
            } else {
                self.type_.get_default_value()
            },
            value_source: self.value_source,
            value_set: self.value_set,
            property_flags: self.property_flags.clone(),
            allowed_values: self.allowed_values.clone(),
            required_providers: self.required_providers_builder.build(),
            aspects: self.aspects_list_builder.build(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1111 Attribute.Builder.build
    pub fn build(&self) -> Attribute {
        self.build_named(&self.name)
    }

    /// Java `build(String name)`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1121 Attribute.Builder.build
    pub fn build_named(&self, name: &str) -> Attribute {
        self.build_partial().build(name)
    }
}

/// A rule or macro attribute definition (Java `Attribute`). Immutable.
// Java: com/google/devtools/build/lib/packages/Attribute.java:75 Attribute
// PORT: skipped, derived PartialEq over the same fields (Java's cached hashCode only shortcuts equals; an Attribute is never a hash key here): Attribute.equals, Attribute.hashCode
// PORT: skipped, no caller at the pin (RuleContext reads the String predicate, get_allowed_rule_classes_for_labels_warning): Attribute.getAllowedRuleClassObjectWarningPredicate
// PORT: skipped, no caller at the pin: Attribute.fromRuleAndAttributesOnly
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    name: String,
    doc: Option<String>,
    type_: Type,
    property_flags: BTreeSet<PropertyFlag>,
    /// The default: Java null, a value of the type, or a ComputedDefault /
    /// StarlarkComputedDefaultTemplate / LateBoundDefault / MaterializingDefault.
    default_value: Option<AttrValue>,
    transition_factory: TransitionFactory,
    allowed_rule_classes_for_labels: RuleClassNamePredicate,
    allowed_rule_classes_for_labels_warning: RuleClassNamePredicate,
    allowed_file_types_for_labels: Option<FileTypeSet>,
    allowed_values: Option<AllowedValues>,
    required_providers: RequiredProviders,
    aspects: AspectsList,
}

impl Attribute {
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1876 Attribute.<init>
    pub(crate) fn new(
        name: String,
        doc: Option<String>,
        type_: Type,
        property_flags: BTreeSet<PropertyFlag>,
        default_value: Option<AttrValue>,
        transition_factory: TransitionFactory,
        allowed_rule_classes_for_labels: RuleClassNamePredicate,
        allowed_rule_classes_for_labels_warning: RuleClassNamePredicate,
        allowed_file_types_for_labels: Option<FileTypeSet>,
        allowed_values: Option<AllowedValues>,
        required_providers: RequiredProviders,
        aspects: AspectsList,
    ) -> Attribute {
        blaze_util::check_argument!(
            transition_factory.is_no_transition()
                || type_.get_label_class() == LabelClass::Dependency
                || type_.get_label_class() == LabelClass::NondepReference,
            "Configuration transitions can only be specified for label or label list attributes"
        );
        blaze_util::check_argument!(
            Attribute::is_analysis_dependent(&name)
                == matches!(
                    default_value,
                    Some(AttrValue::LateBound(_)) | Some(AttrValue::Materializer(_))
                ),
            "analysis dependent attributes require a default value that is one (and vice versa): {}",
            name
        );
        Attribute {
            name,
            doc,
            type_,
            property_flags,
            default_value,
            transition_factory,
            allowed_rule_classes_for_labels,
            allowed_rule_classes_for_labels_warning,
            allowed_file_types_for_labels,
            allowed_values,
            required_providers,
            aspects,
        }
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1942 Attribute.getName
    pub fn get_name(&self) -> &str {
        &self.name
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1947 Attribute.getDoc
    pub fn get_doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    /// The Starlark-visible name (`_foo` for `$foo` / `:foo`).
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1956 Attribute.getPublicName
    pub fn get_public_name(&self) -> String {
        Attribute::get_starlark_name(&self.name)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1965 Attribute.getType
    pub fn get_type(&self) -> Type {
        self.type_
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1969 Attribute.getPropertyFlag
    fn get_property_flag(&self, flag: PropertyFlag) -> bool {
        self.property_flags.contains(&flag)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1974 Attribute.isMandatory
    pub fn is_mandatory(&self) -> bool {
        self.get_property_flag(PropertyFlag::Mandatory)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1979 Attribute.isNonEmpty
    pub fn is_non_empty(&self) -> bool {
        self.get_property_flag(PropertyFlag::NonEmpty)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1984 Attribute.isSingleArtifact
    pub fn is_single_artifact(&self) -> bool {
        self.get_property_flag(PropertyFlag::SingleArtifact)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1989 Attribute.isSilentRuleClassFilter
    pub fn is_silent_rule_class_filter(&self) -> bool {
        self.get_property_flag(PropertyFlag::SilentRuleclassFilter)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2001 Attribute.isForDependencyResolution
    pub fn is_for_dependency_resolution(&self) -> bool {
        self.get_property_flag(PropertyFlag::ForDependencyResolution)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2005 Attribute.forDependencyResolutionExplicitlySet
    pub fn for_dependency_resolution_explicitly_set(&self) -> bool {
        self.get_property_flag(PropertyFlag::ForDependencyResolutionExplicitlySet)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2009 Attribute.skipValidations
    pub fn skip_validations(&self) -> bool {
        self.get_property_flag(PropertyFlag::SkipValidations)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2014 Attribute.isSkipAnalysisTimeFileTypeCheck
    pub fn is_skip_analysis_time_file_type_check(&self) -> bool {
        self.get_property_flag(PropertyFlag::SkipAnalysisTimeFiletypeCheck)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2019 Attribute.isOrderIndependent
    pub fn is_order_independent(&self) -> bool {
        self.get_property_flag(PropertyFlag::OrderIndependent)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2024 Attribute.useOutputLicenses
    pub fn use_output_licenses(&self) -> bool {
        self.get_property_flag(PropertyFlag::OutputLicenses)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2033 Attribute.hasStarlarkDefinedTransition
    pub fn has_starlark_defined_transition(&self) -> bool {
        self.get_property_flag(PropertyFlag::HasStarlarkDefinedTransition)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2041 Attribute.hasAnalysisTestTransition
    pub fn has_analysis_test_transition(&self) -> bool {
        self.get_property_flag(PropertyFlag::HasAnalysisTestTransition)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2049 Attribute.getTransitionFactory
    pub fn get_transition_factory(&self) -> &TransitionFactory {
        &self.transition_factory
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2057 Attribute.isExecutable
    pub fn is_executable(&self) -> bool {
        self.get_property_flag(PropertyFlag::Executable)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2062 Attribute.isDirectCompileTimeInput
    pub fn is_direct_compile_time_input(&self) -> bool {
        self.get_property_flag(PropertyFlag::DirectCompileTimeInput)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2067 Attribute.isDocumented
    pub fn is_documented(&self) -> bool {
        !self.get_property_flag(PropertyFlag::Undocumented)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2075 Attribute.isTaggable
    pub fn is_taggable(&self) -> bool {
        self.get_property_flag(PropertyFlag::Taggable)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2079 Attribute.isStrictLabelCheckingEnabled
    pub fn is_strict_label_checking_enabled(&self) -> bool {
        self.get_property_flag(PropertyFlag::StrictLabelChecking)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2084 Attribute.checkAllowedValues
    pub fn check_allowed_values(&self) -> bool {
        self.get_property_flag(PropertyFlag::CheckAllowedValues)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2088 Attribute.performPrereqValidatorCheck
    pub fn perform_prereq_validator_check(&self) -> bool {
        !self.get_property_flag(PropertyFlag::SkipPrereqValidatorChecks)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2092 Attribute.checkConstraintsOverride
    pub fn check_constraints_override(&self) -> bool {
        self.get_property_flag(PropertyFlag::CheckConstraintsOverride)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2096 Attribute.skipConstraintsOverride
    pub fn skip_constraints_override(&self) -> bool {
        self.get_property_flag(PropertyFlag::SkipConstraintsOverride)
    }

    /// Output types are excluded because of Rule#populateExplicitOutputFiles.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2101 Attribute.isConfigurable
    pub fn is_configurable(&self) -> bool {
        self.type_.get_label_class() != LabelClass::Output
            && !self.get_property_flag(PropertyFlag::Nonconfigurable)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2108 Attribute.configurableAttrWasUserSet
    pub fn configurable_attr_was_user_set(&self) -> bool {
        self.get_property_flag(PropertyFlag::ConfigurableAttrWasUserSet)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2119 Attribute.isToolDependency
    pub fn is_tool_dependency(&self) -> bool {
        if self.type_.get_label_class() != LabelClass::Dependency {
            return false;
        }
        if self.get_property_flag(PropertyFlag::IsToolDependency) {
            return true;
        }
        self.transition_factory.is_tool()
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2137 Attribute.starlarkDefined
    pub fn starlark_defined(&self) -> bool {
        self.get_property_flag(PropertyFlag::StarlarkDefined)
    }

    /// Java `getAllowedRuleClassObjectPredicate().apply(ruleClass)`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2150 Attribute.getAllowedRuleClassObjectPredicate
    pub fn get_allowed_rule_class_object_predicate(&self, rule_class: &RuleClass) -> bool {
        self.allowed_rule_classes_for_labels
            .as_predicate_of_rule_class_object(rule_class)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2154 Attribute.getAllowedRuleClassPredicate
    pub fn get_allowed_rule_classes_for_labels(&self) -> &RuleClassNamePredicate {
        &self.allowed_rule_classes_for_labels
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2167 Attribute.getAllowedRuleClassWarningPredicate
    pub fn get_allowed_rule_classes_for_labels_warning(&self) -> &RuleClassNamePredicate {
        &self.allowed_rule_classes_for_labels_warning
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2171 Attribute.getRequiredProviders
    pub fn get_required_providers(&self) -> &RequiredProviders {
        &self.required_providers
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2175 Attribute.getAllowedFileTypesPredicate
    pub fn get_allowed_file_types_predicate(&self) -> Option<&FileTypeSet> {
        self.allowed_file_types_for_labels.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2179 Attribute.getAllowedValues
    pub fn get_allowed_values(&self) -> Option<&Arc<dyn PredicateWithMessage>> {
        self.allowed_values.as_ref().map(|a| &a.0)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2183 Attribute.hasAspects
    pub fn has_aspects(&self) -> bool {
        self.aspects.has_aspects()
    }

    /// Returns the list of aspects required for dependencies through this attribute.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2188 Attribute.getAspects
    pub fn get_aspects(&self, rule: &crate::target::Rule) -> Vec<crate::aspect::Aspect> {
        self.aspects.get_aspects(rule)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2192 Attribute.getAspectsList
    pub fn get_aspects_list(&self) -> &AspectsList {
        &self.aspects
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2196 Attribute.getAspectClasses
    pub fn get_aspect_classes(&self) -> Vec<crate::aspect::AspectClass> {
        self.aspects.get_aspect_classes()
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2200 Attribute.validateRulePropagatedAspectsParameters
    pub fn validate_rule_propagated_aspects_parameters(
        &self,
        rule_class: &RuleClass,
    ) -> Result<(), EvalException> {
        self.aspects
            .validate_rule_propagated_aspects_parameters(rule_class)
    }

    /// The default value; for a late-bound default, its value for `rule`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2221 Attribute.getDefaultValue
    pub fn get_default_value(&self, rule: Option<&dyn RuleOrMacroInstance>) -> Option<AttrValue> {
        match &self.default_value {
            Some(AttrValue::LateBound(l)) => l.get_default(rule),
            other => other.clone(),
        }
    }

    /// The default value, even if it is a computed or late-bound default.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2234 Attribute.getDefaultValueUnchecked
    pub fn get_default_value_unchecked(&self) -> Option<&AttrValue> {
        self.default_value.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2238 Attribute.getLateBoundDefault
    pub fn get_late_bound_default(&self) -> Option<&Arc<LateBoundDefault>> {
        match &self.default_value {
            Some(AttrValue::LateBound(l)) => Some(l),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2242 Attribute.getMaterializer
    pub fn get_materializer(&self) -> &Arc<MaterializingDefault> {
        blaze_util::check_state!(self.is_materializing());
        match &self.default_value {
            Some(AttrValue::Materializer(m)) => m,
            _ => unreachable!(),
        }
    }

    /// True iff this attribute has a computed default.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2252 Attribute.hasComputedDefault
    pub fn has_computed_default(&self) -> bool {
        matches!(
            self.default_value,
            Some(AttrValue::ComputedDefault(_))
                | Some(AttrValue::StarlarkComputedDefaultTemplate(_))
        )
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2257 Attribute.isPublic
    pub fn is_public(&self) -> bool {
        !Attribute::is_private_attribute(&self.name)
    }

    /// True if this attribute is an implicit dependency by the naming policy (`$name`).
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2265 Attribute.isImplicit
    pub fn is_implicit(&self) -> bool {
        Attribute::is_implicit_name(&self.name)
    }

    /// Java static `Attribute.isImplicit(String name)`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2273 Attribute.isImplicit
    pub fn is_implicit_name(name: &str) -> bool {
        name.starts_with('$')
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2281 Attribute.isLateBound
    pub fn is_late_bound(&self) -> bool {
        matches!(self.default_value, Some(AttrValue::LateBound(_)))
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2285 Attribute.isMaterializing
    pub fn is_materializing(&self) -> bool {
        matches!(self.default_value, Some(AttrValue::Materializer(_)))
    }

    /// True if an attribute named `name` is analysis-dependent (`:name`).
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2293 Attribute.isAnalysisDependent
    pub fn is_analysis_dependent(name: &str) -> bool {
        name.starts_with(':')
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:2298 Attribute.isPrivateAttribute
    pub(crate) fn is_private_attribute(native_attr_name: &str) -> bool {
        Attribute::is_analysis_dependent(native_attr_name)
            || Attribute::is_implicit_name(native_attr_name)
    }

    /// The Starlark-usable name: implicit and late-bound attributes start with `_`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2307 Attribute.getStarlarkName
    pub fn get_starlark_name(native_attr_name: &str) -> String {
        if Attribute::is_private_attribute(native_attr_name) {
            return format!("_{}", &native_attr_name[1..]);
        }
        native_attr_name.to_owned()
    }

    /// Fails unless this attribute may override another in a Starlark rule extension.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2328 Attribute.failIfNotAValidOverride
    pub fn fail_if_not_a_valid_override(&self) -> Result<(), EvalException> {
        // Java: com/google/devtools/build/lib/packages/Attribute.java:2312 Attribute.failIf
        fn fail_if(condition: bool, message: String) -> Result<(), EvalException> {
            if condition {
                return Err(EvalException::new(message));
            }
            Ok(())
        }
        let name = &self.name;
        fail_if(
            self.allowed_rule_classes_for_labels != any_rule(),
            format!("attribute `{name}`: can't override allowed rule classes"),
        )?;
        fail_if(
            self.allowed_rule_classes_for_labels_warning != no_rule(),
            format!("attribute `{name}`: can't override allowed rule classes"),
        )?;
        fail_if(
            !self.transition_factory.is_no_transition(),
            format!("attribute `{name}`: can't override configuration transition"),
        )?;
        fail_if(
            self.allowed_file_types_for_labels != Some(FileTypeSet::NoFile),
            format!("attribute `{name}`: can't override allowed files"),
        )?;
        fail_if(
            !self.required_providers.accepts_any(),
            format!("attribute `{name}`: can't override required providers"),
        )?;
        fail_if(
            self.property_flags
                != BTreeSet::from([
                    PropertyFlag::StarlarkDefined,
                    PropertyFlag::StrictLabelChecking,
                ]),
            format!("attribute `{name}`: can't have additional flags"),
        )?;
        Ok(())
    }

    /// A replica builder of this attribute.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2395 Attribute.cloneBuilder
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2413 Attribute.cloneBuilder
    pub fn clone_builder(&self) -> AttributeBuilder {
        AttributeBuilder {
            name: self.name.clone(),
            type_: self.type_,
            transition_factory: self.transition_factory.clone(),
            allowed_rule_classes_for_labels: self.allowed_rule_classes_for_labels.clone(),
            allowed_rule_classes_for_labels_warning: self
                .allowed_rule_classes_for_labels_warning
                .clone(),
            allowed_file_types_for_labels: self.allowed_file_types_for_labels.clone(),
            value: self.default_value.clone(),
            doc: self.doc.clone(),
            value_source: AttributeValueSource::Direct,
            value_set: true,
            property_flags: self.property_flags.clone(),
            allowed_values: self.allowed_values.clone(),
            required_providers_builder: self.required_providers.copy_as_builder(),
            aspects_list_builder: AspectsListBuilder::from_aspects_list(&self.aspects),
        }
    }

    /// Converts an attribute value from internal form to Starlark form. Lists and dicts are
    /// immutable: Java builds them with `Dict.Builder.buildImmutable`,
    /// `StarlarkList.immutableCopyOf` and `Starlark.fromJava(x, /* mutability= */ null)`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2442 Attribute.valueToStarlark
    pub fn value_to_starlark<'v>(x: Option<&AttrValue>, heap: Heap<'v>) -> Value<'v> {
        let Some(x) = x else {
            // Java: Starlark.fromJava(null) is None.
            return Value::new_none();
        };
        // Java `StarlarkList.copyOf(null, list)` / `Dict.copyOf(null, map)`. Elements are
        // allocated before their container.
        let list = |items: Vec<Value<'v>>| alloc_immutable_list(heap, items);
        let dict = |items: Vec<(Value<'v>, Value<'v>)>| alloc_immutable_dict(heap, items);
        match x {
            // Is x a non-empty {label,string}_list_dict? Recursively convert subelements.
            AttrValue::StringListDict(m) => dict(
                m.iter()
                    .map(|(k, v)| {
                        (
                            heap.alloc_str(k).to_value(),
                            list(v.iter().map(|s| heap.alloc_str(s).to_value()).collect()),
                        )
                    })
                    .collect(),
            ),
            AttrValue::LabelListDict(m) => dict(
                m.iter()
                    .map(|(k, v)| {
                        (
                            heap.alloc_str(k).to_value(),
                            list(
                                v.iter()
                                    .map(|l| heap.alloc(LabelValue(l.clone())))
                                    .collect(),
                            ),
                        )
                    })
                    .collect(),
            ),
            // Until Starlark gains a set data type, shallow-convert Java sets to Starlark lists.
            AttrValue::StringSet(s) => {
                list(s.iter().map(|e| heap.alloc_str(e).to_value()).collect())
            }
            // Convert TriState to integer (same as in query output and native.existing_rules())
            AttrValue::TriState(t) => heap.alloc(t.to_int()),
            AttrValue::SelectorList(selector_list) => {
                let mut selectors: Vec<Value<'v>> = Vec::new();
                for selector in selector_list.get_selectors() {
                    let mut m: Vec<(Value<'v>, Value<'v>)> = Vec::new();
                    selector.for_each(|raw_key, raw_value| {
                        let key = heap.alloc(LabelValue(raw_key.clone()));
                        // BuildType.Selector constructor transforms `None` values of selector
                        // branches into the default value of the attribute's type. We need to
                        // reverse this transformation, but leave alone cases where the select value
                        // was actually set to the default value.
                        let map_val = if !selector.is_value_set(raw_key) {
                            Value::new_none()
                        } else {
                            Attribute::value_to_starlark(raw_value, heap)
                        };
                        // buildKeepingLast: a repeated key keeps its first position, last value.
                        if let Some(e) = m.iter_mut().find(|(k, _)| k.equals(key).unwrap_or(false))
                        {
                            e.1 = map_val;
                        } else {
                            m.push((key, map_val));
                        }
                    });
                    if !m.is_empty() {
                        selectors.push(heap.alloc(SelectorValue::new(
                            m,
                            selector.get_no_match_error().to_owned(),
                        )));
                    }
                }
                match SelectorList::of(&selectors) {
                    Ok(l) => heap.alloc(l),
                    // This should never happen because we are converting from a valid Native
                    // select.
                    Err(e) => panic!("IllegalStateException: {e}"),
                }
            }
            // For all other attribute values, shallow conversion is safe.
            AttrValue::Int(i) => heap.alloc(*i),
            AttrValue::String(s) => heap.alloc_str(s).to_value(),
            AttrValue::Bool(b) => Value::new_bool(*b),
            AttrValue::Label(l) => heap.alloc(LabelValue(l.clone())),
            AttrValue::StringList(l) => {
                list(l.iter().map(|s| heap.alloc_str(s).to_value()).collect())
            }
            AttrValue::IntList(l) => list(l.iter().map(|i| heap.alloc(*i)).collect()),
            AttrValue::LabelList(l) => list(
                l.iter()
                    .map(|x| heap.alloc(LabelValue(x.clone())))
                    .collect(),
            ),
            AttrValue::StringDict(m) => dict(
                m.iter()
                    .map(|(k, v)| (heap.alloc_str(k).to_value(), heap.alloc_str(v).to_value()))
                    .collect(),
            ),
            AttrValue::LabelDictUnary(m) => dict(
                m.iter()
                    .map(|(k, v)| {
                        (
                            heap.alloc_str(k).to_value(),
                            heap.alloc(LabelValue(v.clone())),
                        )
                    })
                    .collect(),
            ),
            AttrValue::LabelKeyedStringDict(m) => dict(
                m.iter()
                    .map(|(k, v)| {
                        (
                            heap.alloc(LabelValue(k.clone())),
                            heap.alloc_str(v).to_value(),
                        )
                    })
                    .collect(),
            ),
            // Java: License is a StarlarkValue (LicenseApi), returned as it is.
            AttrValue::License(l) => heap.alloc(l.clone()),
            AttrValue::ComputedDefault(_)
            | AttrValue::StarlarkComputedDefaultTemplate(_)
            | AttrValue::LateBound(_)
            | AttrValue::Materializer(_) => {
                blaze_util::unported!("Attribute.valueToStarlark of a default-computing object")
            }
        }
    }
}

impl fmt::Display for Attribute {
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2355 Attribute.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Attribute({}, {})", self.name, self.type_)
    }
}

impl PartialOrd for Attribute {
    // Java: com/google/devtools/build/lib/packages/Attribute.java:2360 Attribute.compareTo
    fn partial_cmp(&self, other: &Attribute) -> Option<std::cmp::Ordering> {
        Some(self.name.cmp(&other.name))
    }
}
