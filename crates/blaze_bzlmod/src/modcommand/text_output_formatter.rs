//! Port of `bazel/bzlmod/modcommand/TextOutputFormatter.java`: graph-based results of
//! `ModExecutor` in a human-readable text format.

use std::collections::HashSet;

use blaze_bzlmod_model::{ModuleExtensionId, ModuleKey, Version};
use blaze_optdefs::misc::mod_options::{Charset, ExtensionShow};

use super::mod_executor::{IsCycle, IsExpanded, IsIndirect};
use super::output_formatters::{FormatterContext, Printer};

/// The tree drawing characters.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/TextOutputFormatter.java:234 TextOutputFormatter.DrawCharset
#[derive(Debug, Clone, Copy)]
struct DrawCharset {
    empty_indent: &'static str,
    prev_child_indent: &'static str,
    child_indent: &'static str,
    indirect_child_indent: &'static str,
    last_child_indent: &'static str,
    last_indirect_child_indent: &'static str,
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/TextOutputFormatter.java:235 TextOutputFormatter.DrawCharset.ASCII
const ASCII: DrawCharset = DrawCharset {
    empty_indent: "    ",
    prev_child_indent: "|   ",
    child_indent: "|___",
    indirect_child_indent: "|...",
    last_child_indent: "|___",
    last_indirect_child_indent: "|...",
};

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/TextOutputFormatter.java:236 TextOutputFormatter.DrawCharset.UTF8
const UTF8: DrawCharset = DrawCharset {
    empty_indent: "    ",
    prev_child_indent: "\u{2502}   ",
    child_indent: "\u{251c}\u{2500}\u{2500}\u{2500}",
    indirect_child_indent: "\u{251c}\u{254c}\u{254c}\u{254c}",
    last_child_indent: "\u{2514}\u{2500}\u{2500}\u{2500}",
    last_indirect_child_indent: "\u{2514}\u{254c}\u{254c}\u{254c}",
};

/// The per-output state of `TextOutputFormatter`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/TextOutputFormatter.java:43 TextOutputFormatter
struct TextOutputFormatter<'c, 'r> {
    ctx: &'c FormatterContext<'r>,
    /// `Deque<Boolean>` used as a stack: the last element is the deque's first.
    is_last_child_stack: Vec<bool>,
    draw_charset: DrawCharset,
    seen_extensions: HashSet<ModuleExtensionId>,
    str: String,
    visted: HashSet<ModuleKey>,
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/TextOutputFormatter.java:52 TextOutputFormatter.output
pub(crate) fn output(ctx: &FormatterContext<'_>, printer: &mut Printer<'_>) {
    let draw_charset = if ctx.options.charset == Charset::ASCII {
        ASCII
    } else {
        UTF8
    };
    let mut f = TextOutputFormatter {
        ctx,
        is_last_child_stack: Vec::new(),
        draw_charset,
        seen_extensions: HashSet::new(),
        str: String::new(),
        visted: HashSet::new(),
    };
    f.print_module(
        &ModuleKey::root(),
        None,
        IsExpanded::TRUE,
        IsIndirect::FALSE,
        IsCycle::FALSE,
        0,
    );
    printer.println(&f.str);
}

impl TextOutputFormatter<'_, '_> {
    /// Prints the indents and the tree drawing characters.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/TextOutputFormatter.java:67 TextOutputFormatter.printTreeDrawing
    fn print_tree_drawing(&mut self, indirect: IsIndirect, depth: i32) {
        if depth > 0 {
            let indents = self.is_last_child_stack.len() - 1;
            // descendingIterator(): from the bottom of the stack (oldest) upwards.
            for i in 0..indents {
                let is_last_child = self.is_last_child_stack[i];
                if is_last_child {
                    self.str.push_str(self.draw_charset.empty_indent);
                } else {
                    self.str.push_str(self.draw_charset.prev_child_indent);
                }
            }
            let first = *self
                .is_last_child_stack
                .last()
                .expect("java.util.NoSuchElementException");
            if indirect == IsIndirect::TRUE {
                if first {
                    self.str
                        .push_str(self.draw_charset.last_indirect_child_indent);
                } else {
                    self.str.push_str(self.draw_charset.indirect_child_indent);
                }
            } else if first {
                self.str.push_str(self.draw_charset.last_child_indent);
            } else {
                self.str.push_str(self.draw_charset.child_indent);
            }
        }
    }

    /// Helper to print module extensions similarly to printModule.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/TextOutputFormatter.java:97 TextOutputFormatter.printExtension
    fn print_extension(
        &mut self,
        key: &ModuleKey,
        extension_id: &ModuleExtensionId,
        unexpanded: bool,
        depth: i32,
    ) {
        self.print_tree_drawing(IsIndirect::FALSE, depth);
        self.str.push('$');
        self.str.push_str(&extension_id.to_string());
        self.str.push(' ');
        if unexpanded && self.ctx.options.extension_info == ExtensionShow::ALL {
            self.str.push_str("... ");
        }
        self.str.push('\n');
        if self.ctx.options.extension_info == ExtensionShow::USAGES {
            return;
        }
        let repo_imports = self.ctx.sorted_repo_imports(extension_id, key);
        let mut unused_repos: Vec<String> = Vec::new();
        if !unexpanded && self.ctx.options.extension_info == ExtensionShow::ALL {
            let ctx = self.ctx;
            unused_repos = ctx.sorted_unused_repos(extension_id, &|r: &str| {
                ctx.is_imported_repo(extension_id, r)
            });
        }
        let total_children_num = repo_imports.len() + unused_repos.len();
        let mut curr_child = 1;
        for used_repo in &repo_imports {
            self.is_last_child_stack
                .push(curr_child == total_children_num);
            curr_child += 1;
            self.print_extension_repo(used_repo, IsIndirect::FALSE, depth + 1);
            self.is_last_child_stack.pop();
        }
        if unexpanded || self.ctx.options.extension_info == ExtensionShow::REPOS {
            return;
        }
        for unused_package in &unused_repos {
            self.is_last_child_stack
                .push(curr_child == total_children_num);
            curr_child += 1;
            self.print_extension_repo(unused_package, IsIndirect::TRUE, depth + 1);
            self.is_last_child_stack.pop();
        }
    }

    /// Prints an extension repo line.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/TextOutputFormatter.java:137 TextOutputFormatter.printExtensionRepo
    fn print_extension_repo(&mut self, repo_name: &str, indirect_link: IsIndirect, depth: i32) {
        self.print_tree_drawing(indirect_link, depth);
        self.str.push_str(repo_name);
        self.str.push('\n');
    }

    /// Depth-first traversal to print the actual output.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/TextOutputFormatter.java:143 TextOutputFormatter.printModule
    fn print_module(
        &mut self,
        key: &ModuleKey,
        parent: Option<&ModuleKey>,
        expanded: IsExpanded,
        indirect: IsIndirect,
        cycle: IsCycle,
        depth: i32,
    ) {
        self.print_tree_drawing(indirect, depth);

        let added = self.visted.insert(key.clone());
        // try { ... } finally { if (added) visted.remove(key); }
        self.print_module_body(key, parent, expanded, indirect, cycle, depth, added);
        if added {
            self.visted.remove(key);
        }
    }

    fn print_module_body(
        &mut self,
        key: &ModuleKey,
        parent: Option<&ModuleKey>,
        expanded: IsExpanded,
        indirect: IsIndirect,
        cycle: IsCycle,
        depth: i32,
        added: bool,
    ) {
        let ctx = self.ctx;
        let node = ctx.node(key);
        if *key == ModuleKey::root() {
            let Some(root_module) = ctx.dep_graph.get(&ModuleKey::root()) else {
                panic!("java.lang.NullPointerException");
            };
            let version = if root_module.version == Version::empty() {
                "_".to_string()
            } else {
                root_module.version.to_string()
            };
            self.str
                .push_str(&format!("<root> ({}@{})", root_module.name, version));
        } else {
            self.str.push_str(&key.to_string());
            self.str.push(' ');
        }

        let mut total_children_num = node.children_size();

        let extensions_used = ctx.extensions_used(key);
        if ctx.options.extension_info != ExtensionShow::HIDDEN {
            total_children_num += extensions_used.len();
        }

        // If we've already seen this node in the current traversal path, treat it as a cycle
        // even if the graph structure says otherwise (which can happen due to merged paths).
        let is_cycle = cycle == IsCycle::TRUE || !added;

        if is_cycle {
            self.str.push_str("(cycle) ");
        } else if expanded == IsExpanded::FALSE {
            self.str.push_str("(*) ");
        } else if node.is_target() {
            self.str.push_str("# ");
        }
        let module = ctx.module(key);
        if !ctx.options.verbose && !module.is_used() {
            self.str.push_str("(unused) ");
        }
        // If the edge is indirect, the parent is not only unknown, but the node could have come
        // from multiple paths merged in the process, so we skip the resolution explanation.
        if indirect == IsIndirect::FALSE
            && ctx.options.verbose
            && let Some(parent) = parent
        {
            let explanation = ctx.get_extra_resolution_explanation(key, Some(parent));
            if let Some(explanation) = explanation {
                self.str
                    .push_str(&explanation.to_explanation_string(!module.is_used()));
            }
        }

        self.str.push('\n');

        if expanded == IsExpanded::FALSE || is_cycle {
            return;
        }

        let mut curr_child = 1;
        if ctx.options.extension_info != ExtensionShow::HIDDEN {
            for extension_id in &extensions_used {
                let unexpanded_extension = !self.seen_extensions.insert(extension_id.clone());
                self.is_last_child_stack
                    .push(curr_child == total_children_num);
                curr_child += 1;
                self.print_extension(key, extension_id, unexpanded_extension, depth + 1);
                self.is_last_child_stack.pop();
            }
        }
        for (child_key, meta) in node.get_children_sorted_by_edge_type() {
            self.is_last_child_stack
                .push(curr_child == total_children_num);
            curr_child += 1;
            self.print_module(
                &child_key,
                Some(key),
                meta.is_expanded,
                meta.is_indirect,
                meta.is_cycle,
                depth + 1,
            );
            self.is_last_child_stack.pop();
        }
    }
}
