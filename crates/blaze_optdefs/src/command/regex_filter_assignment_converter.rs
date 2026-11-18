//! Port of `com/google/devtools/build/lib/exec/RegexFilterAssignmentConverter.java`.

use super::prelude::*;

/// `Map.Entry<RegexFilter, List<String>>`, the converter's result.
#[derive(Clone, Debug, PartialEq)]
pub struct RegexFilterToStringList {
    pub key: RegexFilter,
    pub value: Vec<String>,
}

// Java `Map.Entry.toString()`: key=value.
impl OptionScalar for RegexFilterToStringList {
    fn java_to_string(&self) -> String {
        format!(
            "{}={}",
            self.key,
            blaze_options::value::java_list_to_string(self.value.iter().cloned())
        )
    }
}
option_field_type!(
    RegexFilterToStringList,
    "java.util.Map$Entry",
    raw = RegexFilterToStringList {
        key: RegexFilterConverter.convert_filter("").unwrap(),
        value: vec![],
    }
);

// Java: com/google/devtools/build/lib/exec/RegexFilterAssignmentConverter.java:22 RegexFilterAssignmentConverter
/// A converter for options of the form RegexFilter=String.
#[derive(Clone, Copy, Debug, Default)]
pub struct RegexFilterAssignmentConverter;

impl Converter for RegexFilterAssignmentConverter {
    fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
        let inner = AssignmentToListOfValuesConverter {
            key_converter: RegexFilterConverter,
            value_converter: StringConverter,
            allow_empty_keys: AllowEmptyKeys::NO,
        };
        let (key, values) = inner.convert_parts(non_null(input), c)?;
        Ok(Some(Value::new(RegexFilterToStringList {
            key: key.expect("RegexFilterConverter never returns null").get(),
            value: values
                .into_iter()
                .map(|v| v.expect("StringConverter never returns null").get())
                .collect(),
        })))
    }
    fn get_type_description(&self) -> String {
        "a '<RegexFilter>=value[,value]' assignment".into()
    }
}
