//! Port of `bazel/bzlmod/modcommand/OutputFormatters.java` (the formatter registry, the shared
//! `OutputFormatter` state and `Explanation`), plus the `PrintWriter` the formatters write to.

use std::cmp::Ordering;
use std::io::Write;

use blaze_bzlmod_model::{ModuleExtensionId, ModuleKey, Version};
use blaze_optdefs::misc::mod_options::{ModOptions, OutputFormat};
use blaze_options::OptionScalar;
use indexmap::{IndexMap, IndexSet};

use super::mod_executor::ResultNode;
use crate::inspector::{AugmentedModule, ResolutionReason};

/// `ImmutableMap<ModuleExtensionId, ImmutableSetMultimap<String, ModuleKey>>`: for each
/// extension, the repos imported from it and the modules that import each one.
pub type ExtensionRepoImports = IndexMap<ModuleExtensionId, IndexMap<String, IndexSet<ModuleKey>>>;

/// Java `String.compareTo` (UTF-16 code unit order), the natural order of `ImmutableSortedSet`.
pub fn java_string_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// The `PrintWriter(new OutputStreamWriter(outputStream, UTF_8 or US_ASCII))` of `ModExecutor`.
///
/// Text is buffered until [`Printer::flush`], like the writer's encoder buffer; text that is
/// never flushed never reaches the stream. With the ASCII charset, every character the encoder
/// cannot map (any non-ASCII code point) becomes `?`. Write errors are ignored, like
/// `PrintWriter`.
///
/// The text is internal strings (module names, tag values, repo names) mixed with Java literals.
/// With UTF-8, `as_bytes()` is the writer's encoder: an internal string's bytes >= 0x80 come out
/// double encoded, as Bazel prints them (`mod show_extension` prints `é` as C3 83 C2 A9).
pub struct Printer<'a> {
    out: &'a mut dyn Write,
    ascii: bool,
    buffer: Vec<u8>,
}

impl<'a> Printer<'a> {
    pub fn new(out: &'a mut dyn Write, ascii: bool) -> Self {
        Printer {
            out,
            ascii,
            buffer: Vec::new(),
        }
    }

    /// `PrintWriter.print(String)`.
    pub fn print(&mut self, s: &str) {
        if self.ascii {
            for c in s.chars() {
                if c.is_ascii() {
                    self.buffer.push(c as u8);
                } else {
                    self.buffer.push(b'?');
                }
            }
        } else {
            self.buffer.extend_from_slice(s.as_bytes());
        }
    }

    /// `PrintWriter.println(String)` (line separator `\n`).
    pub fn println(&mut self, s: &str) {
        self.print(s);
        self.print("\n");
    }

    /// `PrintWriter.flush()`.
    pub fn flush(&mut self) {
        let _ = self.out.write_all(&self.buffer);
        let _ = self.out.flush();
        self.buffer.clear();
    }
}

/// What the formatters read (the `protected` fields of Java's `OutputFormatter`).
pub struct FormatterContext<'r> {
    pub result: &'r IndexMap<ModuleKey, ResultNode>,
    pub dep_graph: &'r IndexMap<ModuleKey, AugmentedModule>,
    /// `ImmutableSetMultimap<ModuleExtensionId, String>`.
    pub extension_repos: &'r IndexMap<ModuleExtensionId, IndexSet<String>>,
    pub extension_repo_imports: &'r ExtensionRepoImports,
    pub options: &'r ModOptions,
}

impl FormatterContext<'_> {
    pub(crate) fn module(&self, key: &ModuleKey) -> &AugmentedModule {
        match self.dep_graph.get(key) {
            Some(m) => m,
            None => panic!("java.lang.NullPointerException: no module {key} in the dep graph"),
        }
    }

    pub(crate) fn node(&self, key: &ModuleKey) -> &ResultNode {
        match self.result.get(key) {
            Some(n) => n,
            None => panic!("java.lang.NullPointerException: no result node for {key}"),
        }
    }

    /// `extensionRepoImports.get(extensionId)`.
    fn imports(&self, extension_id: &ModuleExtensionId) -> &IndexMap<String, IndexSet<ModuleKey>> {
        match self.extension_repo_imports.get(extension_id) {
            Some(i) => i,
            None => panic!("java.lang.NullPointerException"),
        }
    }

    /// `ImmutableSortedSet.copyOf(extensionRepoImports.get(extensionId).inverse().get(key))`.
    pub(crate) fn sorted_repo_imports(
        &self,
        extension_id: &ModuleExtensionId,
        key: &ModuleKey,
    ) -> Vec<String> {
        let mut repos: Vec<String> = self
            .imports(extension_id)
            .iter()
            .filter(|(_, modules)| modules.contains(key))
            .map(|(repo, _)| repo.clone())
            .collect();
        repos.sort_by(|a, b| java_string_cmp(a, b));
        repos.dedup();
        repos
    }

    /// `ImmutableSortedSet.copyOf(extensionRepoImports.get(id).keySet())`.
    pub(crate) fn sorted_used_repos(&self, extension_id: &ModuleExtensionId) -> Vec<String> {
        let mut repos: Vec<String> = self.imports(extension_id).keys().cloned().collect();
        repos.sort_by(|a, b| java_string_cmp(a, b));
        repos
    }

    /// `ImmutableSortedSet.copyOf(Sets.difference(extensionRepos.get(id), used))`.
    pub(crate) fn sorted_unused_repos(
        &self,
        extension_id: &ModuleExtensionId,
        used: &dyn Fn(&str) -> bool,
    ) -> Vec<String> {
        let mut repos: Vec<String> = self
            .extension_repos
            .get(extension_id)
            .map(|s| s.iter().filter(|r| !used(r.as_str())).cloned().collect())
            .unwrap_or_default();
        repos.sort_by(|a, b| java_string_cmp(a, b));
        repos
    }

    /// `extensionRepoImports.get(extensionId).keySet()` membership.
    pub(crate) fn is_imported_repo(&self, extension_id: &ModuleExtensionId, repo: &str) -> bool {
        self.imports(extension_id).contains_key(repo)
    }

    /// The extensions whose imports include `key`, sorted by
    /// `ModuleExtensionId.LEXICOGRAPHIC_COMPARATOR`.
    pub(crate) fn extensions_used(&self, key: &ModuleKey) -> Vec<ModuleExtensionId> {
        let mut used: Vec<ModuleExtensionId> = self
            .extension_repo_imports
            .iter()
            .filter(|(_, imports)| imports.values().any(|modules| modules.contains(key)))
            .map(|(e, _)| e.clone())
            .collect();
        used.sort_by(ModuleExtensionId::lexicographic_cmp);
        used
    }

    /// Returns `None` if the module version has not changed during resolution or if the module
    /// is `<root>`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/OutputFormatters.java:144 OutputFormatters.OutputFormatter.getExtraResolutionExplanation
    pub(crate) fn get_extra_resolution_explanation(
        &self,
        key: &ModuleKey,
        parent: Option<&ModuleKey>,
    ) -> Option<Explanation> {
        get_extra_resolution_explanation(self.dep_graph, self.options, key, parent)
    }
}

/// Compact representation of the data provided by the `--verbose` flag.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/OutputFormatters.java:73 OutputFormatters.OutputFormatter.Explanation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    /// The version from/to which the module was changed after resolution.
    pub changed_version: Version,
    pub resolution_reason: ResolutionReason,
    /// The list of modules who originally requested the selected version in the case of
    /// Minimal-Version-Selection.
    pub requested_by_modules: Option<IndexSet<ModuleKey>>,
}

impl Explanation {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/OutputFormatters.java:82 OutputFormatters.OutputFormatter.Explanation.create
    pub fn create(
        version: Version,
        reason: ResolutionReason,
        requested_by_modules: Option<IndexSet<ModuleKey>>,
    ) -> Explanation {
        Explanation {
            changed_version: version,
            resolution_reason: reason,
            requested_by_modules,
        }
    }

    pub fn changed_version(&self) -> &Version {
        &self.changed_version
    }

    pub fn resolution_reason(&self) -> ResolutionReason {
        self.resolution_reason
    }

    pub fn requested_by_modules(&self) -> Option<&IndexSet<ModuleKey>> {
        self.requested_by_modules.as_ref()
    }

    /// Gets the exact label that is printed next to the module if the `--verbose` flag is
    /// enabled.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/OutputFormatters.java:91 OutputFormatters.OutputFormatter.Explanation.toExplanationString
    pub fn to_explanation_string(&self, unused: bool) -> String {
        let changed_version_label = if self.changed_version == Version::empty() {
            "_".to_string()
        } else {
            self.changed_version.to_string()
        };
        let to_or_was_string = if unused { "to" } else { "was" };
        let reason_string = match &self.requested_by_modules {
            Some(modules) => modules
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            None => self.resolution_reason.name().to_ascii_lowercase(),
        };
        format!("({to_or_was_string} {changed_version_label}, cause {reason_string})")
    }
}

/// Returns `None` if the module version has not changed during resolution or if the module is
/// `<root>`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/OutputFormatters.java:144 OutputFormatters.OutputFormatter.getExtraResolutionExplanation
pub fn get_extra_resolution_explanation(
    dep_graph: &IndexMap<ModuleKey, AugmentedModule>,
    options: &ModOptions,
    key: &ModuleKey,
    parent: Option<&ModuleKey>,
) -> Option<Explanation> {
    if *key == ModuleKey::root() {
        return None;
    }
    fn npe() -> ! {
        panic!("java.lang.NullPointerException")
    }
    let module = dep_graph.get(key).unwrap_or_else(|| npe());
    let parent_module = parent
        .and_then(|p| dep_graph.get(p))
        .unwrap_or_else(|| npe());
    let all_deps = parent_module.get_all_deps(options.include_unused);
    let repo_name: Option<&String> = all_deps.get(key);
    let reason: Option<ResolutionReason> =
        repo_name.and_then(|r| parent_module.dep_reasons.get(r).copied());
    let replacement: Option<&AugmentedModule> = if module.is_used() {
        Some(module)
    } else {
        repo_name
            .and_then(|r| parent_module.deps.get(r))
            .and_then(|k| dep_graph.get(k))
    };
    if reason != Some(ResolutionReason::ORIGINAL) {
        let changed_version = if !module.is_used() {
            replacement.unwrap_or_else(|| npe()).version.clone()
        } else {
            let old = repo_name
                .and_then(|r| parent_module.unused_deps.get(r))
                .and_then(|k| dep_graph.get(k))
                .unwrap_or_else(|| npe());
            old.version.clone()
        };
        let mut changed_by_modules = None;
        if reason == Some(ResolutionReason::MINIMAL_VERSION_SELECTION) {
            changed_by_modules = Some(
                replacement
                    .unwrap_or_else(|| npe())
                    .original_dependants
                    .clone(),
            );
        }
        // Explanation's constructor requires a non-null reason.
        let reason = reason.unwrap_or_else(|| npe());
        return Some(Explanation::create(
            changed_version,
            reason,
            changed_by_modules,
        ));
    }
    None
}

/// The graph-based output formatters of `ModExecutor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormatter {
    Text,
    Json,
    Graph,
}

/// `OutputFormatters.getFormatter`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/OutputFormatters.java:46 OutputFormatters.getFormatter
pub fn get_formatter(format: OutputFormat) -> OutputFormatter {
    match format {
        OutputFormat::TEXT => OutputFormatter::Text,
        OutputFormat::JSON => OutputFormatter::Json,
        OutputFormat::GRAPH => OutputFormatter::Graph,
        other => panic!(
            "IllegalArgumentException: Unsupported output format: {}",
            other.java_to_string()
        ),
    }
}

impl OutputFormatter {
    /// Exposed API of the formatter during which the necessary objects are injected.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/OutputFormatters.java:104 OutputFormatters.OutputFormatter.output
    pub fn output(self, ctx: &FormatterContext<'_>, printer: &mut Printer<'_>) {
        match self {
            OutputFormatter::Text => super::text_output_formatter::output(ctx, printer),
            OutputFormatter::Json => super::json_output_formatter::output(ctx, printer),
            OutputFormatter::Graph => super::graphviz_output_formatter::output(ctx, printer),
        }
        printer.flush();
    }

    /// Exists only for testing, because normally the depGraph and options are injected inside
    /// the public API call.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/OutputFormatters.java:129 OutputFormatters.OutputFormatter.getExtraResolutionExplanation
    pub fn get_extra_resolution_explanation(
        self,
        key: &ModuleKey,
        parent: Option<&ModuleKey>,
        dep_graph: &IndexMap<ModuleKey, AugmentedModule>,
        options: &ModOptions,
    ) -> Option<Explanation> {
        get_extra_resolution_explanation(dep_graph, options, key, parent)
    }
}
