//! The methods of `query2/common/CommonQueryOptions.java` and `query2/cquery/CqueryOptions.java`
//! that cquery and aquery use (blaze_optdefs transcribes the option fields only).

use std::collections::HashSet;

use blaze_cmdline::RepositoryMapping;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_optdefs::command::cquery_options::CqueryOptions;
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_options::Options;
use blaze_packages::target::LabelPrinter;
use blaze_query_engine::Setting;
use blaze_starlark::StarlarkSemantics;

/// The `CommonQueryOptions` fields both cquery and aquery options carry.
pub trait CommonQueryOptionsView {
    fn line_terminator_null(&self) -> bool;
    fn include_tool_deps(&self) -> bool;
    fn include_implicit_deps(&self) -> bool;
    fn include_no_dep_deps(&self) -> bool;
    fn use_aspects(&self) -> bool;
    fn explicit_aspects(&self) -> bool;
    fn emit_consistent_labels(&self) -> bool;
    fn universe_scope(&self) -> &[String];
    fn query_file(&self) -> &str;

    // Java: com/google/devtools/build/lib/query2/common/CommonQueryOptions.java:65 CommonQueryOptions.getLineTerminator
    fn get_line_terminator(&self) -> &'static str {
        if self.line_terminator_null() {
            return "\0";
        }
        "\n"
    }

    /// Return the current options as a set of QueryEnvironment settings.
    // Java: com/google/devtools/build/lib/query2/common/CommonQueryOptions.java:165 CommonQueryOptions.toSettings
    fn to_settings(&self) -> HashSet<Setting> {
        let mut settings = HashSet::new();
        if !self.include_tool_deps() {
            settings.insert(Setting::OnlyTargetDeps);
        }
        if !self.include_implicit_deps() {
            settings.insert(Setting::NoImplicitDeps);
        }
        if !self.include_no_dep_deps() {
            settings.insert(Setting::NoNodepDeps);
        }
        if self.use_aspects() {
            settings.insert(Setting::IncludeAspects);
        }
        if self.explicit_aspects() {
            settings.insert(Setting::ExplicitAspects);
        }
        settings
    }

    // Java: com/google/devtools/build/lib/query2/common/CommonQueryOptions.java:199 CommonQueryOptions.getLabelPrinter
    fn get_label_printer(
        &self,
        starlark_semantics: &StarlarkSemantics,
        main_repo_mapping: RepositoryMapping,
    ) -> LabelPrinter {
        if self.emit_consistent_labels() {
            LabelPrinter::starlark(starlark_semantics)
        } else {
            LabelPrinter::display_form(main_repo_mapping)
        }
    }
}

macro_rules! common_query_options_view {
    ($t:ty) => {
        impl CommonQueryOptionsView for $t {
            fn line_terminator_null(&self) -> bool {
                self.line_terminator_null
            }
            fn include_tool_deps(&self) -> bool {
                self.include_tool_deps
            }
            fn include_implicit_deps(&self) -> bool {
                self.include_implicit_deps
            }
            fn include_no_dep_deps(&self) -> bool {
                self.include_no_dep_deps
            }
            fn use_aspects(&self) -> bool {
                self.use_aspects
            }
            fn explicit_aspects(&self) -> bool {
                self.explicit_aspects
            }
            fn emit_consistent_labels(&self) -> bool {
                self.emit_consistent_labels
            }
            fn universe_scope(&self) -> &[String] {
                &self.universe_scope
            }
            fn query_file(&self) -> &str {
                &self.query_file
            }
        }
    };
}

common_query_options_view!(CqueryOptions);
common_query_options_view!(AqueryOptions);

/// The cquery options as the `QueryOptions` blaze_query's `ProtoOutputFormatter.setOptions`
/// takes (Java passes the `CommonQueryOptions` superclass).
///
/// PORT: blaze_query's formatter reads a `QueryOptions`; the `CommonQueryOptions` fields are
/// copied into one with `QueryOptions`' own fields at their defaults.
pub fn as_query_options(options: &CqueryOptions) -> QueryOptions {
    let mut q = Options::<QueryOptions>::get_defaults();
    q.universe_scope = options.universe_scope.clone();
    q.line_terminator_null = options.line_terminator_null;
    q.infer_universe_scope = options.infer_universe_scope;
    q.include_tool_deps = options.include_tool_deps;
    q.include_implicit_deps = options.include_implicit_deps;
    q.include_no_dep_deps = options.include_no_dep_deps;
    q.use_aspects = options.use_aspects;
    q.incompatible_package_group_includes_double_slash =
        options.incompatible_package_group_includes_double_slash;
    q.explicit_aspects = options.explicit_aspects;
    q.emit_consistent_labels = options.emit_consistent_labels;
    q.relative_locations = options.relative_locations;
    q.proto_include_locations = options.proto_include_locations;
    q.proto_include_default_values = options.proto_include_default_values;
    q.proto_flatten_selects = options.proto_flatten_selects;
    q.proto_output_rule_attributes = options.proto_output_rule_attributes.clone();
    q.proto_include_rule_inputs_and_outputs = options.proto_include_rule_inputs_and_outputs;
    q.proto_include_synthetic_attribute_hash = options.proto_include_synthetic_attribute_hash;
    q.proto_include_instantiation_stack = options.proto_include_instantiation_stack;
    q.proto_include_definition_stack = options.proto_include_definition_stack;
    q.proto_include_starlark_rule_env = options.proto_include_starlark_rule_env;
    q.proto_include_attribute_source_aspects = options.proto_include_attribute_source_aspects;
    q.proto_rule_classes = options.proto_rule_classes;
    q.aspect_deps = options.aspect_deps;
    q.graph_node_string_limit = options.graph_node_string_limit;
    q.graph_factored = options.graph_factored;
    q.query_file = options.query_file.clone();
    q.output_file = options.output_file.clone();
    q
}
