//! Port of `query2/query/output/SyntheticAttributeHashCalculator.java`.

use std::collections::HashMap;

use base64::Engine;
use blaze_packages::attribute::raw_attribute_mapper::RawAttributeMapper;
use blaze_packages::target::{LabelPrinter, Rule, RuleOrMacroInstance};
use blaze_packages::types::AttrValue;
use blaze_protos::JavaMessage;
use blaze_protos::blaze_query::Attribute as AttributePb;
use blaze_vfs::DigestHashFunction;

use crate::compat::attribute_formatter;

/// `CodedOutputStream.writeUInt32NoTag` (a varint).
fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        if value < 0x80 {
            out.push(value as u8);
            return;
        }
        out.push(((value & 0x7f) | 0x80) as u8);
        value >>= 7;
    }
}

/// Calculator for a synthetic attribute hash of a rule.
// Java: com/google/devtools/build/lib/query2/query/output/SyntheticAttributeHashCalculator.java:37 SyntheticAttributeHashCalculator
pub struct SyntheticAttributeHashCalculator;

impl SyntheticAttributeHashCalculator {
    /// Returns a hash of various properties of a rule which might contribute to the rule's
    /// "affectedness". This includes, but is not limited to, attribute values and error-state.
    ///
    /// - `rule`: The rule instance to calculate the hash for.
    /// - `serialized_attributes`: Any available attribute which have already been serialized
    ///   (by attribute name). This is an optimization to avoid re-serializing attributes
    ///   internally.
    /// - `extra_data_for_attr_hash`: Extra data to add to the hash.
    // Java: com/google/devtools/build/lib/query2/query/output/SyntheticAttributeHashCalculator.java:53 SyntheticAttributeHashCalculator.compute
    pub fn compute(
        rule: &Rule,
        serialized_attributes: &HashMap<String, AttributePb>,
        extra_data_for_attr_hash: &str,
        hash_function: DigestHashFunction,
        include_attribute_source_aspects: bool,
        include_starlark_rule_env: bool,
    ) -> String {
        // HashingOutputStream(hashFunction, nullOutputStream) behind a CodedOutputStream: the
        // bytes written, hashed at the end.
        let mut coded_out: Vec<u8> = Vec::new();

        let rule_class = rule.get_rule_class_object();
        if rule_class.is_starlark() && include_starlark_rule_env {
            let digest = rule_class
                .get_rule_definition_environment_digest()
                .unwrap_or_else(|| panic!("NullPointerException: {rule}"));
            // writeByteArrayNoTag
            write_varint(&mut coded_out, digest.len() as u64);
            coded_out.extend_from_slice(digest);
        }

        let raw_attribute_mapper = RawAttributeMapper::of(rule);
        for attr in rule.get_attributes() {
            let attr_name = attr.get_name();

            if attr_name == "generator_location" {
                // generator_location can be ignored for the purpose of telling if a rule has
                // changed.
                continue;
            }

            let mut value_to_hash = raw_attribute_mapper.get_raw_attribute_value_of(rule, attr);

            if let Some(AttrValue::ComputedDefault(computed_default)) = &value_to_hash {
                // ConfiguredDefaults need special handling to detect changes in evaluated values.
                if !computed_default.dependencies().is_empty() {
                    // TODO(b/29038463): We're skipping computed defaults that depend on other
                    // configurable attributes because there currently isn't a way to evaluate
                    // such a computed default; there isn't *one* value it evaluates to.
                    continue;
                }

                // PORT: Java catches the IllegalArgumentException that AbstractAttributeMapper.get
                // throws on a type mismatch and skips the attribute (logging a warning);
                // blaze_packages' mapper fails loudly there instead.
                value_to_hash = computed_default.get_default(&raw_attribute_mapper);
            }

            let attr_pb = match (&value_to_hash, serialized_attributes.get(attr_name)) {
                (Some(AttrValue::SelectorList(_)), _) | (_, None) => {
                    // We didn't already serialize the attribute or it's a SelectorList. Latter may
                    // have been flattened while we want the full representation, so we start
                    // from scratch.
                    attribute_formatter::get_attribute_proto(
                        attr.get_name(),
                        attr.get_type(),
                        value_to_hash.as_ref(),
                        /* explicitly_specified= */
                        false, // We care about value, not how it was set.
                        /* encode_boolean_and_tri_state_as_integer_and_string= */ false,
                        /* source_aspect= */ None,
                        include_attribute_source_aspects,
                        &LabelPrinter::legacy(),
                    )
                }
                (_, Some(serialized)) => serialized.clone(),
            };

            coded_out.extend_from_slice(&attr_pb.to_byte_array());
        }

        // Rules can be considered changed when the containing package goes in/out of error.
        // writeBoolNoTag
        coded_out.push(u8::from(rule.get_package_declarations().contains_errors()));

        // Include a summary of any package-wide data that applies to this target (e.g. custom
        // make variables aka `vardef`). writeStringNoTag: the UTF-8 bytes of the Java string.
        write_varint(&mut coded_out, extra_data_for_attr_hash.len() as u64);
        coded_out.extend_from_slice(extra_data_for_attr_hash.as_bytes());

        let hash = hash_function
            .hash_reader(&mut &coded_out[..])
            .expect("hashing an in-memory buffer");
        base64::engine::general_purpose::STANDARD.encode(hash)
    }
}

#[cfg(test)]
mod tests {
    // Ports of SyntheticAttributeHashCalculatorTest
    // (src/test/java/com/google/devtools/build/lib/query2/query/output). Each test loads a
    // package from scratch files with `getTarget`, which needs a package-loading test harness
    // Blaze does not have yet.

    #[test]
    #[ignore = "needs a package-loading test harness (scratch files + getTarget)"]
    fn test_compute_attribute_change_changes_hash() {}

    #[test]
    #[ignore = "needs a package-loading test harness (scratch files + getTarget)"]
    fn test_compute_location_doesnt_change_hash() {}

    #[test]
    #[ignore = "needs a package-loading test harness (scratch files + getTarget)"]
    fn test_compute_serialized_attributes_used_over_available() {}

    #[test]
    #[ignore = "needs a package-loading test harness (scratch files + getTarget)"]
    fn test_compute_extra_data_changes_hash() {}

    #[test]
    #[ignore = "needs a package-loading test harness (scratch files + getTarget)"]
    fn test_compute_package_error_status_changes_hash() {}

    #[test]
    #[ignore = "needs a package-loading test harness (scratch files + getTarget)"]
    fn test_compute_include_attribute_source_aspects_changes_hash() {}

    #[test]
    #[ignore = "needs a package-loading test harness (scratch files + getTarget)"]
    fn test_starlark_rule_env_changes() {}

    #[test]
    fn varint_encoding() {
        let mut out = Vec::new();
        super::write_varint(&mut out, 300);
        assert_eq!(out, vec![0xac, 0x02]);
    }
}
