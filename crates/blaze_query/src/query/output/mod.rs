//! Port of `query2/query/output`: the `--output` formatters of `query`.

pub mod build_output_formatter;
pub mod conditional_edges;
pub mod format_utils;
pub mod graph_output_formatter;
pub mod graph_output_writer;
pub mod label_output_formatter;
pub mod location_output_formatter;
pub mod maxrank_output_formatter;
pub mod minrank_output_formatter;
pub mod output_formatter;
pub mod output_formatters;
pub mod package_output_formatter;
pub mod possible_attribute_values;
pub mod proto_output_formatter;
pub mod query_options;
pub mod query_output_utils;
pub mod query_runtime_helper;
pub mod rank_and_label;
pub mod synthetic_attribute_hash_calculator;
pub mod xml_output_formatter;
