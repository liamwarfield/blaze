//! Port of `bazel/bzlmod/modcommand/GraphvizOutputFormatter.java`: graph-based results of
//! `ModExecutor` in the Graphviz *dot* format, which can be further pipelined to create an image
//! graph visualization.

use std::collections::{HashSet, VecDeque};

use blaze_bzlmod_model::{ModuleExtensionId, ModuleKey, Version};
use blaze_optdefs::misc::mod_options::ExtensionShow;

use super::mod_executor::IsIndirect;
use super::output_formatters::{FormatterContext, Printer};

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/GraphvizOutputFormatter.java:45 GraphvizOutputFormatter.output
pub(crate) fn output(ctx: &FormatterContext<'_>, printer: &mut Printer<'_>) {
    let mut str = String::new();
    str.push_str("digraph mygraph {\n");
    str.push_str("  ");
    str.push_str("node [ shape=box ]\n");
    str.push_str("  ");
    str.push_str("edge [ fontsize=8 ]\n");
    let mut seen: HashSet<ModuleKey> = HashSet::new();
    let mut seen_extensions: HashSet<ModuleExtensionId> = HashSet::new();
    let mut to_visit: VecDeque<ModuleKey> = VecDeque::new();
    seen.insert(ModuleKey::root());
    to_visit.push_back(ModuleKey::root());

    while let Some(key) = to_visit.pop_front() {
        let module = ctx.module(&key);
        let node = ctx.node(&key);
        let source_id = to_id(&key);

        if key == ModuleKey::root() {
            let root_label = format!("<root> ({}@{})", module.name, module.version);
            str.push_str(&format!("  \"<root>\" [ label=\"{root_label}\" ]\n"));
        } else if node.is_target() || !module.is_used() {
            let shape_string = if node.is_target() { "diamond" } else { "box" };
            let style_string = if module.is_used() { "solid" } else { "dotted" };
            str.push_str(&format!(
                "  {} [ shape={} style={} ]\n",
                to_id(&key),
                shape_string,
                style_string
            ));
        }

        if ctx.options.extension_info != ExtensionShow::HIDDEN {
            let extensions_used = ctx.extensions_used(&key);
            for extension_id in &extensions_used {
                if ctx.options.extension_info == ExtensionShow::USAGES {
                    str.push_str(&format!(
                        "  {} -> \"{}\"\n",
                        to_id(&key),
                        to_extension_id(extension_id)
                    ));
                    continue;
                }
                if seen_extensions.insert(extension_id.clone()) {
                    print_extension(ctx, &mut str, extension_id);
                }
                let repo_imports = ctx.sorted_repo_imports(extension_id, &key);
                for repo in &repo_imports {
                    str.push_str(&format!(
                        "  {} -> {}\n",
                        to_id(&key),
                        to_extension_repo_id(extension_id, repo)
                    ));
                }
            }
        }
        for (child_key, meta) in node.get_children_sorted_by_key() {
            let child_indirect = meta.is_indirect;
            let child_id = to_id(&child_key);
            if child_indirect == IsIndirect::FALSE {
                let reason_label = get_reason_label(ctx, &child_key, &key);
                str.push_str(&format!("  {source_id} -> {child_id} [ {reason_label} ]\n"));
            } else {
                str.push_str(&format!("  {source_id} -> {child_id} [ style=dashed ]\n"));
            }
            if seen.insert(child_key.clone()) {
                to_visit.push_back(child_key);
            }
        }
    }
    str.push('}');
    printer.println(&str);
    printer.flush();
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/GraphvizOutputFormatter.java:114 GraphvizOutputFormatter.toId
fn to_id(key: &ModuleKey) -> String {
    if *key == ModuleKey::root() {
        return "\"<root>\"".to_string();
    }
    let version = if *key.version() == Version::empty() {
        "_".to_string()
    } else {
        key.version().to_string()
    };
    format!("\"{}@{}\"", key.name(), version)
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/GraphvizOutputFormatter.java:122 GraphvizOutputFormatter.toId
fn to_extension_id(id: &ModuleExtensionId) -> String {
    id.to_string()
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/GraphvizOutputFormatter.java:126 GraphvizOutputFormatter.toId
fn to_extension_repo_id(id: &ModuleExtensionId, repo: &str) -> String {
    format!("\"{}%{}\"", to_extension_id(id), repo)
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/GraphvizOutputFormatter.java:130 GraphvizOutputFormatter.printExtension
fn print_extension(ctx: &FormatterContext<'_>, str: &mut String, id: &ModuleExtensionId) {
    str.push_str(&format!(
        "  subgraph \"cluster_{}\" {{\n",
        to_extension_id(id)
    ));
    str.push_str(&format!("    label=\"{}\"\n", to_extension_id(id)));
    if ctx.options.extension_info == ExtensionShow::USAGES {
        return;
    }
    let used_repos = ctx.sorted_used_repos(id);
    for repo in &used_repos {
        str.push_str(&format!(
            "    {} [ label=\"{}\" ]\n",
            to_extension_repo_id(id, repo),
            repo
        ));
    }
    if ctx.options.extension_info == ExtensionShow::REPOS {
        return;
    }
    let unused_repos = ctx.sorted_unused_repos(id, &|r: &str| used_repos.iter().any(|u| u == r));
    for repo in &unused_repos {
        str.push_str(&format!(
            "    {} [ label=\"{}\" style=dotted ]\n",
            to_extension_repo_id(id, repo),
            repo
        ));
    }
    str.push_str("  }\n");
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/GraphvizOutputFormatter.java:152 GraphvizOutputFormatter.getReasonLabel
fn get_reason_label(ctx: &FormatterContext<'_>, key: &ModuleKey, parent: &ModuleKey) -> String {
    if !ctx.options.verbose {
        return String::new();
    }
    let Some(explanation) = ctx.get_extra_resolution_explanation(key, Some(parent)) else {
        return String::new();
    };
    let label = explanation.resolution_reason().get_label();
    if !label.is_empty() {
        return format!("label={label}");
    }
    String::new()
}
