//! The methods of Java's `QueryOptions` / `CommonQueryOptions` that query uses.
//!
//! PORT: move to blaze_optdefs (`QueryOptions` is transcribed there without its methods).

use std::collections::HashSet;

use blaze_cmdline::RepositoryMapping;
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_packages::target::LabelPrinter;
use blaze_query_engine::Setting;
use blaze_starlark::StarlarkSemantics;

// Java: com/google/devtools/build/lib/query2/common/CommonQueryOptions.java:65 CommonQueryOptions.getLineTerminator
pub fn get_line_terminator(options: &QueryOptions) -> &'static str {
    if options.line_terminator_null {
        return "\0";
    }
    "\n"
}

/// Return the current options as a set of QueryEnvironment settings.
// Java: com/google/devtools/build/lib/query2/query/output/QueryOptions.java:184 QueryOptions.toSettings
pub fn to_settings(options: &QueryOptions) -> HashSet<Setting> {
    // Java: com/google/devtools/build/lib/query2/common/CommonQueryOptions.java:165 CommonQueryOptions.toSettings
    let mut settings = HashSet::new();
    if !options.include_tool_deps {
        settings.insert(Setting::OnlyTargetDeps);
    }
    if !options.include_implicit_deps {
        settings.insert(Setting::NoImplicitDeps);
    }
    if !options.include_no_dep_deps {
        settings.insert(Setting::NoNodepDeps);
    }
    if options.use_aspects {
        settings.insert(Setting::IncludeAspects);
    }
    if options.explicit_aspects {
        settings.insert(Setting::ExplicitAspects);
    }
    if options.strict_test_suite {
        settings.insert(Setting::TestsExpressionStrict);
    }
    settings
}

// Java: com/google/devtools/build/lib/query2/common/CommonQueryOptions.java:199 CommonQueryOptions.getLabelPrinter
pub fn get_label_printer(
    options: &QueryOptions,
    starlark_semantics: &StarlarkSemantics,
    main_repo_mapping: RepositoryMapping,
) -> LabelPrinter {
    if options.emit_consistent_labels {
        LabelPrinter::starlark(starlark_semantics)
    } else {
        LabelPrinter::display_form(main_repo_mapping)
    }
}

// Java: com/google/devtools/build/lib/query2/common/CommonQueryOptions.java:206 CommonQueryOptions.getLabelPrinterLegacy
pub fn get_label_printer_legacy(
    options: &QueryOptions,
    starlark_semantics: &StarlarkSemantics,
) -> LabelPrinter {
    if options.emit_consistent_labels {
        LabelPrinter::starlark(starlark_semantics)
    } else {
        LabelPrinter::legacy()
    }
}
