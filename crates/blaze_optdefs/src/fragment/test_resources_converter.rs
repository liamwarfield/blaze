//! Port of `analysis/test/TestResourcesConverter.java`.

use std::collections::BTreeMap;

use blaze_options::{
    AssignmentConverter, ConversionContext, Converter, OptionScalar, OptionsParsingException,
    Value, converter::non_null, option_field_type, value::java_double_to_string,
};

use super::converters::ConvResult;
use std::sync::Arc;

use super::resource_converter::{
    HOST_CPUS_KEYWORD, HOST_RAM_KEYWORD, ResourceConverterDoubleConverter, host_cpus_supplier,
    host_ram_supplier,
};
use super::test_size::TestSize;

/// `Pair<String, Map<TestSize, Double>>` (the `Map` is an `EnumMap`), the value of
/// [`TestResourcesConverter`].
#[derive(Clone, Debug, PartialEq)]
pub struct TestResourcesPair {
    pub first: String,
    pub second: BTreeMap<TestSize, f64>,
}

impl OptionScalar for TestResourcesPair {
    // Java: com/google/devtools/build/lib/util/Pair.java:60 Pair.toString
    fn java_to_string(&self) -> String {
        let entries: Vec<String> = self
            .second
            .iter()
            .map(|(k, v)| format!("{}={}", k, java_double_to_string(*v)))
            .collect();
        format!("({}, {{{}}})", self.first, entries.join(", "))
    }
}

option_field_type!(
    TestResourcesPair,
    "com.google.devtools.build.lib.util.Pair",
    raw = TestResourcesPair {
        first: String::new(),
        second: BTreeMap::new(),
    }
);

// Java: com/google/devtools/build/lib/analysis/test/TestResourcesConverter.java:30 TestResourcesConverter
/// Converter for `--default_test_resources`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestResourcesConverter;

fn resource_converter() -> ResourceConverterDoubleConverter {
    ResourceConverterDoubleConverter::with_keywords(
        vec![
            (HOST_CPUS_KEYWORD, Arc::new(|| host_cpus_supplier() as f64)),
            (HOST_RAM_KEYWORD, Arc::new(|| host_ram_supplier() as f64)),
        ],
        /* minValue= */ 0.0,
        /* maxValue= */ f64::MAX,
    )
}

impl TestResourcesConverter {
    // Java: com/google/devtools/build/lib/analysis/test/TestResourcesConverter.java:50 TestResourcesConverter.convert
    pub fn convert_pair(&self, input: &str) -> Result<TestResourcesPair, OptionsParsingException> {
        let assignment = AssignmentConverter.convert_entry(input)?;
        let resource_converter = resource_converter();
        let mut values = Vec::with_capacity(TestSize::VALUES.len());
        // Splitter.on(",").splitToList keeps empty strings.
        for s in assignment.1.split(',') {
            values.push(resource_converter.convert_double(s)?);
        }
        if values.len() != 1 && values.len() != TestSize::VALUES.len() {
            return Err(OptionsParsingException::new(format!(
                "Invalid number of comma-separated entries in {}",
                input
            )));
        }
        let mut amounts = BTreeMap::new();
        for size in TestSize::VALUES {
            amounts.insert(size, values[(values.len() - 1).min(size.ordinal())]);
        }
        Ok(TestResourcesPair {
            first: assignment.0,
            second: amounts,
        })
    }
}

impl Converter for TestResourcesConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_pair(non_null(input))?)))
    }
    // Java: com/google/devtools/build/lib/analysis/test/TestResourcesConverter.java:45 TestResourcesConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a resource name followed by equal and 1 float or 4 float, e.g memory=10,30,60,100".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_one_or_four_values() {
        let c = TestResourcesConverter;
        assert_eq!(
            c.convert_pair("memory=10,30,60,100")
                .unwrap()
                .java_to_string(),
            "(memory, {small=10.0, medium=30.0, large=60.0, enormous=100.0})"
        );
        assert_eq!(
            c.convert_pair("cpu=2").unwrap().java_to_string(),
            "(cpu, {small=2.0, medium=2.0, large=2.0, enormous=2.0})"
        );
        assert_eq!(
            c.convert_pair("cpu=1,2").unwrap_err().get_message(),
            "Invalid number of comma-separated entries in cpu=1,2"
        );
    }
}
