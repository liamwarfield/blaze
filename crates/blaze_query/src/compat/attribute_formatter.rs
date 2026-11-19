//! Port of `packages/AttributeFormatter.java`: builds `Build.Attribute` protos from attribute
//! values.
//!
//! PORT: move to blaze_packages (needs blaze_protos' `build.proto` types). Java's `Object` value
//! is an [`AttrValue`]; Java's `null` is `None`. Every string field is set from
//! `StringEncoding.internalToUnicode` of an internal string, as in Java.

use blaze_packages::target::LabelPrinter;
use blaze_packages::types::{AttrValue, BuildTypeSelectorList, TriState, Type};
use blaze_protos::blaze_query::attribute::{Selector, SelectorEntry, SelectorList, Tristate};
use blaze_protos::blaze_query::{
    Attribute as AttributePb, LabelDictUnaryEntry, LabelKeyedStringDictEntry, License as LicensePb,
    StringDictEntry, StringListDictEntry,
};

use crate::compat::proto_utils::get_discriminator_from_type;
use blaze_protos::blaze_query::attribute::Discriminator;
use blaze_util::string_encoding::internal_to_unicode;

/// Java `StringEncoding.internalToUnicode(s)`, owned for a proto field.
fn u(s: &str) -> String {
    internal_to_unicode(s).into_owned()
}

/// Java `AttributeFormatter.depTypes`.
// Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:73 AttributeFormatter.depTypes
fn is_dep_type(type_: Type) -> bool {
    matches!(
        type_,
        Type::String
            | Type::StringNoIntern
            | Type::Label
            | Type::Output
            | Type::StringList
            | Type::LabelList
            | Type::LabelDictUnary
            | Type::LabelKeyedStringDict
            | Type::OutputList
    )
}

/// Java `AttributeFormatter.noDepTypes`.
// Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:86 AttributeFormatter.noDepTypes
fn is_no_dep_type(type_: Type) -> bool {
    matches!(type_, Type::NodepLabelList | Type::NodepLabel)
}

/// Convert attribute value to proto representation.
///
/// If `value` is `None`, only the `name`, `explicitly_specified`, `nodep` (if applicable), and
/// `type` fields will be included in the proto message.
///
/// If `encode_boolean_and_tri_state_as_integer_and_string` is true then boolean and tristate
/// values are also encoded as integers and strings.
///
/// `source_aspect` is the name of the aspect class (`Aspect.getAspectClass().getName()`) the
/// attribute comes from, if any.
// Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:128 AttributeFormatter.getAttributeProto
pub fn get_attribute_proto(
    name: &str,
    type_: Type,
    value: Option<&AttrValue>,
    explicitly_specified: bool,
    encode_boolean_and_tri_state_as_integer_and_string: bool,
    source_aspect: Option<&str>,
    include_attribute_source_aspects: bool,
    label_printer: &LabelPrinter,
) -> AttributePb {
    let mut attr_pb = AttributePb {
        name: u(name),
        ..Default::default()
    };
    attr_pb.explicitly_specified = Some(explicitly_specified);
    maybe_set_no_dep(type_, &mut attr_pb);

    if let Some(AttrValue::SelectorList(selector_list)) = value {
        attr_pb.set_type(Discriminator::SelectorList);
        write_selector_list_to_builder(&mut attr_pb, type_, selector_list, label_printer);
    } else {
        attr_pb.set_type(get_discriminator_from_type(type_));
        if let Some(value) = value {
            let mut adapter = AttributeBuilderAdapter {
                attribute_builder: &mut attr_pb,
                encode_boolean_and_tri_state_as_integer_and_string,
            };
            write_attribute_value_to_builder(&mut adapter, type_, value, label_printer);
        }
    }

    if include_attribute_source_aspects {
        attr_pb.source_aspect_name = Some(source_aspect.map(u).unwrap_or_default());
    }

    attr_pb
}

// Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:157 AttributeFormatter.maybeSetNoDep
fn maybe_set_no_dep(type_: Type, attr_pb: &mut AttributePb) {
    if is_dep_type(type_) {
        attr_pb.nodep = Some(false);
    } else if is_no_dep_type(type_) {
        attr_pb.nodep = Some(true);
    }
}

// Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:165 AttributeFormatter.writeSelectorListToBuilder
fn write_selector_list_to_builder(
    attr_pb: &mut AttributePb,
    type_: Type,
    selector_list: &BuildTypeSelectorList,
    label_printer: &LabelPrinter,
) {
    let mut selector_list_builder = SelectorList::default();
    selector_list_builder.set_type(get_discriminator_from_type(type_));
    for selector in selector_list.get_selectors() {
        let mut selector_builder = Selector {
            no_match_error: Some(u(selector.get_no_match_error())),
            has_default_value: Some(selector.has_default()),
            ..Default::default()
        };
        selector.for_each(|condition, condition_value| {
            let mut selector_entry_builder = SelectorEntry {
                label: Some(u(&label_printer.to_string(condition))),
                is_default_value: Some(!selector.is_value_set(condition)),
                ..Default::default()
            };
            if let Some(condition_value) = condition_value {
                write_attribute_value_to_builder(
                    &mut SelectorEntryBuilderAdapter {
                        selector_entry_builder: &mut selector_entry_builder,
                    },
                    type_,
                    condition_value,
                    label_printer,
                );
            }
            selector_builder.entries.push(selector_entry_builder);
        });
        selector_list_builder.elements.push(selector_builder);
    }
    attr_pb.selector_list = Some(selector_list_builder);
}

fn unexpected(type_: Type, value: &AttrValue) -> ! {
    panic!("AssertionError: value {value:?} does not match type {type_:?}")
}

/// Set the appropriate type and value. Since string and string list store values for multiple
/// types, use the toString() method on the objects instead of casting them.
// Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:204 AttributeFormatter.writeAttributeValueToBuilder
fn write_attribute_value_to_builder(
    builder: &mut dyn AttributeValueBuilderAdapter,
    type_: Type,
    value: &AttrValue,
    label_printer: &LabelPrinter,
) {
    match type_ {
        Type::Integer => match value {
            AttrValue::Int(i) => builder.set_int_value(*i),
            _ => unexpected(type_, value),
        },
        Type::String | Type::StringNoIntern => match value {
            AttrValue::String(s) => builder.set_string_value(u(s)),
            _ => unexpected(type_, value),
        },
        Type::Label
        | Type::NodepLabel
        | Type::Output
        | Type::GenqueryScopeType
        | Type::DormantLabel => match value {
            AttrValue::Label(l) => builder.set_string_value(u(&label_printer.to_string(l))),
            _ => unexpected(type_, value),
        },
        Type::StringList | Type::StringSet => match value {
            AttrValue::StringList(list) => {
                for entry in list {
                    builder.add_string_list_value(u(entry));
                }
            }
            AttrValue::StringSet(set) => {
                for entry in set {
                    builder.add_string_list_value(u(entry));
                }
            }
            _ => unexpected(type_, value),
        },
        Type::LabelList
        | Type::NodepLabelList
        | Type::OutputList
        | Type::GenqueryScopeTypeList
        | Type::DormantLabelList => match value {
            AttrValue::LabelList(list) => {
                for entry in list {
                    builder.add_string_list_value(u(&label_printer.to_string(entry)));
                }
            }
            _ => unexpected(type_, value),
        },
        Type::IntegerList => match value {
            AttrValue::IntList(list) => {
                for elem in list {
                    builder.add_int_list_value(*elem);
                }
            }
            _ => unexpected(type_, value),
        },
        Type::Boolean => match value {
            AttrValue::Bool(b) => builder.set_boolean_value(*b),
            _ => unexpected(type_, value),
        },
        Type::Tristate => match value {
            AttrValue::TriState(t) => builder.set_tristate_value(tri_state_to_proto(*t)),
            _ => unexpected(type_, value),
        },
        Type::License => match value {
            AttrValue::License(license) => {
                let mut license_pb = LicensePb::default();
                for license_type in license.get_license_types() {
                    license_pb.license_type.push(u(license_type.name()));
                }
                for exception in license.get_exceptions() {
                    license_pb.exception.push(u(&exception.to_string()));
                }
                builder.set_license(license_pb);
            }
            _ => unexpected(type_, value),
        },
        Type::StringDict => match value {
            AttrValue::StringDict(dict) => {
                for (key, value) in dict {
                    builder.add_string_dict_value(StringDictEntry {
                        key: u(key),
                        value: u(value),
                    });
                }
            }
            _ => unexpected(type_, value),
        },
        Type::StringListDict => match value {
            AttrValue::StringListDict(dict) => {
                for (key, values) in dict {
                    builder.add_string_list_dict_value(StringListDictEntry {
                        key: u(key),
                        value: values.iter().map(|v| u(v)).collect(),
                    });
                }
            }
            _ => unexpected(type_, value),
        },
        Type::LabelListDict => match value {
            AttrValue::LabelListDict(dict) => {
                for (key, values) in dict {
                    // Java: dictEntryValue.toString() (Label.toString, the canonical form).
                    builder.add_string_list_dict_value(StringListDictEntry {
                        key: u(key),
                        value: values.iter().map(|l| u(&l.to_string())).collect(),
                    });
                }
            }
            _ => unexpected(type_, value),
        },
        Type::LabelDictUnary => match value {
            AttrValue::LabelDictUnary(dict) => {
                for (key, label) in dict {
                    builder.add_label_dict_unary_value(LabelDictUnaryEntry {
                        key: u(key),
                        value: u(&label_printer.to_string(label)),
                    });
                }
            }
            _ => unexpected(type_, value),
        },
        Type::LabelKeyedStringDict => match value {
            AttrValue::LabelKeyedStringDict(dict) => {
                for (label, value) in dict {
                    builder.add_label_keyed_string_dict_value(LabelKeyedStringDictEntry {
                        key: u(&label_printer.to_string(label)),
                        value: u(value),
                    });
                }
            }
            _ => unexpected(type_, value),
        },
    }
}

// Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:302 AttributeFormatter.triStateToProto
fn tri_state_to_proto(value: TriState) -> Tristate {
    match value {
        TriState::Auto => Tristate::Auto,
        TriState::No => Tristate::No,
        TriState::Yes => Tristate::Yes,
    }
}

/// An adapter used by `write_attribute_value_to_builder` in order to reuse the same code for
/// writing to both `Build.Attribute` and `SelectorEntry` messages.
// Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:315 AttributeFormatter.AttributeValueBuilderAdapter
trait AttributeValueBuilderAdapter {
    fn add_string_list_value(&mut self, s: String);
    fn add_label_dict_unary_value(&mut self, entry: LabelDictUnaryEntry);
    fn add_label_keyed_string_dict_value(&mut self, entry: LabelKeyedStringDictEntry);
    fn add_int_list_value(&mut self, i: i32);
    fn add_string_dict_value(&mut self, entry: StringDictEntry);
    fn add_string_list_dict_value(&mut self, entry: StringListDictEntry);
    fn set_boolean_value(&mut self, b: bool);
    fn set_int_value(&mut self, i: i32);
    fn set_license(&mut self, license: LicensePb);
    fn set_string_value(&mut self, s: String);
    fn set_tristate_value(&mut self, tristate: Tristate);
}

// Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:339 AttributeFormatter.AttributeBuilderAdapter
struct AttributeBuilderAdapter<'a> {
    attribute_builder: &'a mut AttributePb,
    encode_boolean_and_tri_state_as_integer_and_string: bool,
}

impl AttributeValueBuilderAdapter for AttributeBuilderAdapter<'_> {
    fn add_string_list_value(&mut self, s: String) {
        self.attribute_builder.string_list_value.push(s);
    }

    fn add_label_dict_unary_value(&mut self, entry: LabelDictUnaryEntry) {
        self.attribute_builder.label_dict_unary_value.push(entry);
    }

    fn add_label_keyed_string_dict_value(&mut self, entry: LabelKeyedStringDictEntry) {
        self.attribute_builder
            .label_keyed_string_dict_value
            .push(entry);
    }

    fn add_int_list_value(&mut self, i: i32) {
        self.attribute_builder.int_list_value.push(i);
    }

    fn add_string_dict_value(&mut self, entry: StringDictEntry) {
        self.attribute_builder.string_dict_value.push(entry);
    }

    fn add_string_list_dict_value(&mut self, entry: StringListDictEntry) {
        self.attribute_builder.string_list_dict_value.push(entry);
    }

    // Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:388 AttributeBuilderAdapter.setBooleanValue
    fn set_boolean_value(&mut self, b: bool) {
        if b {
            self.attribute_builder.boolean_value = Some(true);
            if self.encode_boolean_and_tri_state_as_integer_and_string {
                self.attribute_builder.string_value = Some("true".to_owned());
                self.attribute_builder.int_value = Some(1);
            }
        } else {
            self.attribute_builder.boolean_value = Some(false);
            if self.encode_boolean_and_tri_state_as_integer_and_string {
                self.attribute_builder.string_value = Some("false".to_owned());
                self.attribute_builder.int_value = Some(0);
            }
        }
    }

    fn set_int_value(&mut self, i: i32) {
        self.attribute_builder.int_value = Some(i);
    }

    fn set_license(&mut self, license: LicensePb) {
        self.attribute_builder.license = Some(license);
    }

    fn set_string_value(&mut self, s: String) {
        self.attribute_builder.string_value = Some(s);
    }

    // Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:421 AttributeBuilderAdapter.setTristateValue
    fn set_tristate_value(&mut self, tristate: Tristate) {
        self.attribute_builder.set_tristate_value(tristate);
        if self.encode_boolean_and_tri_state_as_integer_and_string {
            match tristate {
                Tristate::Auto => {
                    self.attribute_builder.int_value = Some(-1);
                    self.attribute_builder.string_value = Some("auto".to_owned());
                }
                Tristate::No => {
                    self.attribute_builder.int_value = Some(0);
                    self.attribute_builder.string_value = Some("no".to_owned());
                }
                Tristate::Yes => {
                    self.attribute_builder.int_value = Some(1);
                    self.attribute_builder.string_value = Some("yes".to_owned());
                }
            }
        }
    }
}

// Java: com/google/devtools/build/lib/packages/AttributeFormatter.java:447 AttributeFormatter.SelectorEntryBuilderAdapter
struct SelectorEntryBuilderAdapter<'a> {
    selector_entry_builder: &'a mut SelectorEntry,
}

impl AttributeValueBuilderAdapter for SelectorEntryBuilderAdapter<'_> {
    fn add_string_list_value(&mut self, s: String) {
        self.selector_entry_builder.string_list_value.push(s);
    }

    fn add_label_dict_unary_value(&mut self, entry: LabelDictUnaryEntry) {
        self.selector_entry_builder
            .label_dict_unary_value
            .push(entry);
    }

    fn add_label_keyed_string_dict_value(&mut self, entry: LabelKeyedStringDictEntry) {
        self.selector_entry_builder
            .label_keyed_string_dict_value
            .push(entry);
    }

    fn add_int_list_value(&mut self, i: i32) {
        self.selector_entry_builder.int_list_value.push(i);
    }

    fn add_string_dict_value(&mut self, entry: StringDictEntry) {
        self.selector_entry_builder.string_dict_value.push(entry);
    }

    fn add_string_list_dict_value(&mut self, entry: StringListDictEntry) {
        self.selector_entry_builder
            .string_list_dict_value
            .push(entry);
    }

    fn set_boolean_value(&mut self, b: bool) {
        self.selector_entry_builder.boolean_value = Some(b);
    }

    fn set_int_value(&mut self, i: i32) {
        self.selector_entry_builder.int_value = Some(i);
    }

    fn set_license(&mut self, license: LicensePb) {
        self.selector_entry_builder.license = Some(license);
    }

    fn set_string_value(&mut self, s: String) {
        self.selector_entry_builder.string_value = Some(s);
    }

    fn set_tristate_value(&mut self, tristate: Tristate) {
        self.selector_entry_builder.set_tristate_value(tristate);
    }
}
