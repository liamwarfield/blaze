//! Port of `bazel/bzlmod/modcommand/JsonOutputFormatter.java`: graph-based results of
//! `ModExecutor` in JSON format (Gson `JsonObject`s printed with
//! `new GsonBuilder().setPrettyPrinting().disableHtmlEscaping().create().toJson(root)`).

use std::collections::HashSet;

use blaze_bzlmod_model::gson::{JsonValue, JsonWriter};
use blaze_bzlmod_model::{ModuleExtensionId, ModuleKey};
use blaze_optdefs::misc::mod_options::ExtensionShow;

use super::mod_executor::{IsCycle, IsExpanded, IsIndirect};
use super::output_formatters::{FormatterContext, Printer};
use crate::inspector::bimap_inverse_get;

/// Gson `JsonObject.add`/`addProperty`: a repeated name replaces the value in place
/// (`LinkedTreeMap.put`).
fn add(object: &mut Vec<(String, JsonValue)>, name: &str, value: JsonValue) {
    if let Some(entry) = object.iter_mut().find(|(k, _)| k == name) {
        entry.1 = value;
    } else {
        object.push((name.to_string(), value));
    }
}

fn string(s: impl Into<String>) -> JsonValue {
    JsonValue::String(s.into())
}

/// The per-output state of `JsonOutputFormatter`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/JsonOutputFormatter.java:40 JsonOutputFormatter
struct JsonOutputFormatter<'c, 'r> {
    ctx: &'c FormatterContext<'r>,
    seen_extensions: HashSet<ModuleExtensionId>,
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/JsonOutputFormatter.java:44 JsonOutputFormatter.output
pub(crate) fn output(ctx: &FormatterContext<'_>, printer: &mut Printer<'_>) {
    let mut f = JsonOutputFormatter {
        ctx,
        seen_extensions: HashSet::new(),
    };
    let mut root = f.print_module(
        &ModuleKey::root(),
        None,
        IsExpanded::TRUE,
        IsIndirect::FALSE,
    );
    add(&mut root, "root", JsonValue::Bool(true));
    let mut writer = JsonWriter::new(/* pretty= */ true);
    writer.json_value_with_gson_defaults(&JsonValue::Object(root));
    printer.println(&writer.into_string());
}

/// `printKey`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/JsonOutputFormatter.java:52 JsonOutputFormatter.printKey
pub fn print_key(key: &ModuleKey) -> String {
    if *key == ModuleKey::root() {
        return "<root>".to_string();
    }
    key.to_string()
}

impl JsonOutputFormatter<'_, '_> {
    /// Helper to print module extensions similarly to printModule.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/JsonOutputFormatter.java:60 JsonOutputFormatter.printExtension
    fn print_extension(
        &self,
        key: &ModuleKey,
        extension_id: &ModuleExtensionId,
        unexpanded: bool,
    ) -> JsonValue {
        let ctx = self.ctx;
        let mut json: Vec<(String, JsonValue)> = Vec::new();
        add(&mut json, "key", string(extension_id.to_string()));
        add(&mut json, "unexpanded", JsonValue::Bool(unexpanded));
        if ctx.options.extension_info == ExtensionShow::USAGES {
            return JsonValue::Object(json);
        }
        let repo_imports = ctx.sorted_repo_imports(extension_id, key);
        let used_repos: Vec<JsonValue> = repo_imports.into_iter().map(string).collect();
        add(&mut json, "used_repos", JsonValue::Array(used_repos));

        if unexpanded || ctx.options.extension_info == ExtensionShow::REPOS {
            return JsonValue::Object(json);
        }
        let unused_repos = ctx.sorted_unused_repos(extension_id, &|r: &str| {
            ctx.is_imported_repo(extension_id, r)
        });
        let unused_repos_json: Vec<JsonValue> = unused_repos.into_iter().map(string).collect();
        add(
            &mut json,
            "unused_repos",
            JsonValue::Array(unused_repos_json),
        );
        JsonValue::Object(json)
    }

    /// Depth-first traversal to display modules (while explicitly detecting cycles).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/JsonOutputFormatter.java:92 JsonOutputFormatter.printModule
    fn print_module(
        &mut self,
        key: &ModuleKey,
        parent: Option<&ModuleKey>,
        expanded: IsExpanded,
        indirect: IsIndirect,
    ) -> Vec<(String, JsonValue)> {
        let ctx = self.ctx;
        let node = ctx.result.get(key);
        let module = ctx.module(key);
        let mut json: Vec<(String, JsonValue)> = Vec::new();
        add(&mut json, "key", string(print_key(key)));
        add(&mut json, "name", string(module.name.clone()));
        add(&mut json, "version", string(module.version.to_string()));
        let apparent_name: Option<String> = match parent {
            // The apparent repository name under which parent refers to key.
            Some(parent) => bimap_inverse_get(&ctx.module(parent).deps, key).cloned(),
            // The apparent repository name under which key refers to itself.
            None => Some(module.repo_name.clone()),
        };
        // addProperty(name, null) adds JsonNull, which Gson's default toJson drops.
        add(
            &mut json,
            "apparentName",
            apparent_name.map(string).unwrap_or(JsonValue::Null),
        );

        if indirect == IsIndirect::FALSE
            && ctx.options.verbose
            && let Some(parent) = parent
        {
            let explanation = ctx.get_extra_resolution_explanation(key, Some(parent));
            if let Some(explanation) = explanation {
                if !module.is_used() {
                    add(&mut json, "unused", JsonValue::Bool(true));
                    add(
                        &mut json,
                        "resolvedVersion",
                        string(explanation.changed_version().to_string()),
                    );
                } else {
                    add(
                        &mut json,
                        "originalVersion",
                        string(explanation.changed_version().to_string()),
                    );
                }
                // Java writes the changed version here too.
                add(
                    &mut json,
                    "resolutionReason",
                    string(explanation.changed_version().to_string()),
                );
                if let Some(requested_by_modules) = explanation.requested_by_modules() {
                    let requested_by: Vec<JsonValue> = requested_by_modules
                        .iter()
                        .map(|k| string(print_key(k)))
                        .collect();
                    add(
                        &mut json,
                        "resolvedRequestedBy",
                        JsonValue::Array(requested_by),
                    );
                }
            }
        }

        if expanded == IsExpanded::FALSE {
            add(&mut json, "unexpanded", JsonValue::Bool(true));
            return json;
        }

        let Some(node) = node else {
            panic!("java.lang.NullPointerException: no result node for {key}");
        };
        let mut deps: Vec<JsonValue> = Vec::new();
        let mut indirect_deps: Vec<JsonValue> = Vec::new();
        let mut cycles: Vec<JsonValue> = Vec::new();
        for (child_key, meta) in node.get_children_sorted_by_edge_type() {
            let child_expanded = meta.is_expanded;
            let child_indirect = meta.is_indirect;
            let child_cycles = meta.is_cycle;
            if child_cycles == IsCycle::TRUE {
                cycles.push(JsonValue::Object(self.print_module(
                    &child_key,
                    Some(key),
                    IsExpanded::FALSE,
                    IsIndirect::FALSE,
                )));
            } else if child_indirect == IsIndirect::TRUE {
                indirect_deps.push(JsonValue::Object(self.print_module(
                    &child_key,
                    Some(key),
                    child_expanded,
                    IsIndirect::TRUE,
                )));
            } else {
                deps.push(JsonValue::Object(self.print_module(
                    &child_key,
                    Some(key),
                    child_expanded,
                    IsIndirect::FALSE,
                )));
            }
        }
        add(&mut json, "dependencies", JsonValue::Array(deps));
        add(
            &mut json,
            "indirectDependencies",
            JsonValue::Array(indirect_deps),
        );
        add(&mut json, "cycles", JsonValue::Array(cycles));

        if ctx.options.extension_info == ExtensionShow::HIDDEN {
            return json;
        }
        let extensions_used = ctx.extensions_used(key);
        let mut extension_usages: Vec<JsonValue> = Vec::new();
        for extension_id in &extensions_used {
            let unexpanded_extension = !self.seen_extensions.insert(extension_id.clone());
            extension_usages.push(self.print_extension(key, extension_id, unexpanded_extension));
        }
        add(
            &mut json,
            "extensionUsages",
            JsonValue::Array(extension_usages),
        );

        json
    }
}
