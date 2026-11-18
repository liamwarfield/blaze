//! Ports of `analysis/LocationExpander.java` (with `LocationFunction`) and
//! `analysis/LocationTemplateContext.java`.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

use blaze_actions::Artifact;
use blaze_cmdline::label::LabelPackageContext;
use blaze_cmdline::label_constants::EXTERNAL_RUNFILES_PATH_PREFIX;
use blaze_cmdline::{Label, RepositoryMapping};
use blaze_packages::target::RuleOrMacroInstance;
use blaze_packages::types::Type;
use blaze_shell::util::shell_escaper::ShellEscaper;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::configured_target::ConfiguredTarget;
use crate::providers::alias_provider::AliasProvider;
use crate::providers::file_provider::{FileProvider, FilesToRunProvider};
use crate::rule::rule_context::RuleContext;
use crate::rule::rule_error_consumer::RuleErrorConsumer;
use crate::rule::stringtemplate::{ExpansionException, TemplateContext};

const EXACTLY_ONE: bool = false;
const ALLOW_MULTIPLE: bool = true;

/// The location map: label to the artifacts it expands to (Java
/// `Map<Label, Collection<Artifact>>`; the collections are sets whose order is irrelevant
/// because paths are sorted).
pub type LocationMap = HashMap<Label, Vec<Artifact>>;

/// Lazily-built location map (Java `Suppliers.memoize(...)`).
pub type LocationMapSupplier = Arc<dyn Fn() -> Arc<LocationMap> + Send + Sync>;

/// The kind of path a location function produces.
// Java: com/google/devtools/build/lib/analysis/LocationExpander.java:209 LocationExpander.LocationFunction.PathType
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathType {
    Location,
    Exec,
    Rlocation,
}

/// A `$(location ...)`-style function.
// Java: com/google/devtools/build/lib/analysis/LocationExpander.java:208 LocationExpander.LocationFunction
#[derive(Clone)]
pub struct LocationFunction {
    root: Label,
    location_map_supplier: LocationMapSupplier,
    path_type: PathType,
    multiple: bool,
}

// Java: com/google/devtools/build/lib/analysis/LocationExpander.java:215 LocationFunction.MAX_PATHS_SHOWN
const MAX_PATHS_SHOWN: usize = 5;

impl LocationFunction {
    pub fn new(
        root: Label,
        location_map_supplier: LocationMapSupplier,
        path_type: PathType,
        multiple: bool,
    ) -> LocationFunction {
        LocationFunction {
            root,
            location_map_supplier,
            path_type,
            multiple,
        }
    }

    /// Looks up the label-like string in the location map and returns the resolved path string.
    /// `Err` is Java's `IllegalStateException` (message).
    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:240 LocationFunction.apply
    pub fn apply(
        &self,
        arg: &str,
        repository_mapping: &RepositoryMapping,
        workspace_runfiles_directory: &str,
    ) -> Result<String, String> {
        let label = Label::parse_with_package_context(
            arg,
            &LabelPackageContext::of(
                self.root.get_package_identifier(),
                repository_mapping.clone(),
            ),
        )
        .map_err(|e| {
            format!(
                "invalid label in {} expression: {}",
                self.function_name(),
                e.get_message()
            )
        })?;
        let paths = self.resolve_label(&label, workspace_runfiles_directory)?;
        Ok(Self::join_paths(&paths))
    }

    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:262 LocationFunction.resolveLabel
    fn resolve_label(
        &self,
        unresolved: &Label,
        workspace_runfiles_directory: &str,
    ) -> Result<BTreeSet<String>, String> {
        let map = (self.location_map_supplier)();
        let Some(artifacts) = map.get(unresolved) else {
            return Err(format!(
                "label '{}' in {} expression is not a declared prerequisite of this rule",
                unresolved,
                self.function_name()
            ));
        };
        let paths = self.get_paths(artifacts, workspace_runfiles_directory);
        if paths.is_empty() {
            return Err(format!(
                "label '{}' in {} expression expands to no files",
                unresolved,
                self.function_name()
            ));
        }
        if !self.multiple && paths.len() > 1 {
            let shown: Vec<&String> = paths.iter().take(MAX_PATHS_SHOWN).collect();
            return Err(format!(
                "label '{}' in $(location) expression expands to more than one file, please use $(locations {}) instead.  Files (at most {} shown) are: [{}]",
                unresolved,
                unresolved,
                MAX_PATHS_SHOWN,
                shown
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(paths)
    }

    /// Extracts list of all executables associated with given collection of label artifacts.
    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:299 LocationFunction.getPaths
    fn get_paths(
        &self,
        artifacts: &[Artifact],
        workspace_runfiles_directory: &str,
    ) -> BTreeSet<String> {
        let mut paths = BTreeSet::new();
        for artifact in artifacts {
            if let Some(path) = self.get_path(artifact, workspace_runfiles_directory) {
                paths.insert(path.get_callable_path_string());
            }
        }
        paths
    }

    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:311 LocationFunction.getPath
    fn get_path(
        &self,
        artifact: &Artifact,
        workspace_runfiles_directory: &str,
    ) -> Option<PathFragment> {
        Some(match self.path_type {
            PathType::Location => artifact.get_runfiles_path(),
            PathType::Exec => artifact.get_exec_path().clone(),
            PathType::Rlocation => {
                let runfiles_path = artifact.get_runfiles_path();
                if runfiles_path.starts_with(&EXTERNAL_RUNFILES_PATH_PREFIX) {
                    runfiles_path.relative_to(&EXTERNAL_RUNFILES_PATH_PREFIX)
                } else {
                    PathFragment::create(workspace_runfiles_directory).get_relative(&runfiles_path)
                }
            }
        })
    }

    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:327 LocationFunction.joinPaths
    fn join_paths(paths: &BTreeSet<String>) -> String {
        paths
            .iter()
            .map(|p| ShellEscaper::escape_string(p))
            .collect::<Vec<_>>()
            .join(" ")
    }

    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:331 LocationFunction.functionName
    fn function_name(&self) -> &'static str {
        if self.multiple {
            "$(locations)"
        } else {
            "$(location)"
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/LocationExpander.java:336 LocationExpander.allLocationFunctions
pub fn all_location_functions(
    root: &Label,
    location_map: LocationMapSupplier,
    exec_paths: bool,
) -> IndexMap<String, LocationFunction> {
    let f = |path_type: PathType, multiple: bool| {
        LocationFunction::new(root.clone(), location_map.clone(), path_type, multiple)
    };
    let mut m = IndexMap::new();
    m.insert(
        "location".to_owned(),
        f(
            if exec_paths {
                PathType::Exec
            } else {
                PathType::Location
            },
            EXACTLY_ONE,
        ),
    );
    m.insert(
        "locations".to_owned(),
        f(
            if exec_paths {
                PathType::Exec
            } else {
                PathType::Location
            },
            ALLOW_MULTIPLE,
        ),
    );
    m.insert("rootpath".to_owned(), f(PathType::Location, EXACTLY_ONE));
    m.insert(
        "rootpaths".to_owned(),
        f(PathType::Location, ALLOW_MULTIPLE),
    );
    m.insert("execpath".to_owned(), f(PathType::Exec, EXACTLY_ONE));
    m.insert("execpaths".to_owned(), f(PathType::Exec, ALLOW_MULTIPLE));
    m.insert(
        "rlocationpath".to_owned(),
        f(PathType::Rlocation, EXACTLY_ONE),
    );
    m.insert(
        "rlocationpaths".to_owned(),
        f(PathType::Rlocation, ALLOW_MULTIPLE),
    );
    m
}

/// Extracts all possible target locations from target specification.
// Java: com/google/devtools/build/lib/analysis/LocationExpander.java:370 LocationExpander.buildLocationMap
pub fn build_location_map(
    rule_context: &RuleContext,
    label_map: Option<&IndexMap<Label, Vec<Artifact>>>,
    allow_data_attribute_entries_in_label: bool,
    collect_srcs: bool,
) -> LocationMap {
    let mut location_map: LocationMap = HashMap::new();
    fn map_add(map: &mut LocationMap, key: Label, values: impl IntoIterator<Item = Artifact>) {
        let entry = map.entry(key).or_default();
        for v in values {
            // Java: a HashSet of artifacts.
            if !entry.contains(&v) {
                entry.push(v);
            }
        }
    }
    if let Some(label_map) = label_map {
        for (k, v) in label_map {
            map_add(&mut location_map, k.clone(), v.iter().cloned());
        }
    }

    if !rule_context.has_main_aspect() {
        for out in rule_context.get_rule().get_output_files() {
            let artifact = rule_context.create_output_artifact_for(&out);
            map_add(&mut location_map, out.get_label().clone(), [artifact]);
        }
    }

    let rule = rule_context.get_rule();
    if collect_srcs && rule.is_attr_defined("srcs", Type::LabelList) {
        for src in rule_context
            .get_rule_prerequisites_collection()
            .get_prerequisites_if::<FileProvider>("srcs")
        {
            let files = crate::configuredtargets::get_provider::<FileProvider>(&src)
                .expect("FileProvider")
                .get_files_to_build()
                .to_list();
            for label in AliasProvider::get_dependency_labels(&src) {
                map_add(&mut location_map, label, files.iter().cloned());
            }
        }
    }

    // Add all locations associated with dependencies and tools
    let mut deps_data_and_tools: Vec<ConfiguredTarget> = Vec::new();
    for attr in ["deps", "implementation_deps"] {
        if rule.is_attr_defined(attr, Type::LabelList) {
            deps_data_and_tools.extend(
                rule_context
                    .get_rule_prerequisites_collection()
                    .get_prerequisites_if::<FilesToRunProvider>(attr),
            );
        }
    }
    if allow_data_attribute_entries_in_label && rule.is_attr_defined("data", Type::LabelList) {
        deps_data_and_tools.extend(
            rule_context
                .get_rule_prerequisites_collection()
                .get_prerequisites_if::<FilesToRunProvider>("data"),
        );
    }
    if rule.is_attr_defined("tools", Type::LabelList) {
        deps_data_and_tools.extend(
            rule_context
                .get_rule_prerequisites_collection()
                .get_prerequisites_if::<FilesToRunProvider>("tools"),
        );
    }

    for dep in &deps_data_and_tools {
        let labels = AliasProvider::get_dependency_labels(dep);
        let files_to_run =
            crate::configuredtargets::get_files_to_run_provider(dep).expect("FilesToRunProvider");
        let values: Vec<Artifact> = match files_to_run.get_executable() {
            Some(executable_artifact) => vec![executable_artifact.clone()],
            None => crate::configuredtargets::get_provider::<FileProvider>(dep)
                .expect("FileProvider")
                .get_files_to_build()
                .to_list()
                .to_vec(),
        };
        for label in labels {
            map_add(&mut location_map, label, values.iter().cloned());
        }
    }
    location_map
}

/// Expands `$(location)` and related tags in rule attributes (Java `LocationExpander`).
// Java: com/google/devtools/build/lib/analysis/LocationExpander.java:53 LocationExpander
pub struct LocationExpander<'a> {
    rule_error_consumer: &'a dyn RuleErrorConsumer,
    functions: IndexMap<String, LocationFunction>,
    repository_mapping: RepositoryMapping,
    workspace_runfiles_directory: String,
}

impl<'a> LocationExpander<'a> {
    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:63 LocationExpander.<init>
    pub fn new(
        rule_error_consumer: &'a dyn RuleErrorConsumer,
        functions: IndexMap<String, LocationFunction>,
        repository_mapping: RepositoryMapping,
        workspace_runfiles_directory: String,
    ) -> LocationExpander<'a> {
        LocationExpander {
            rule_error_consumer,
            functions,
            repository_mapping,
            workspace_runfiles_directory,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:91 LocationExpander.<init>
    fn for_rule(
        rule_context: &'a RuleContext,
        label_map: Option<IndexMap<Label, Vec<Artifact>>>,
        exec_paths: bool,
        allow_data: bool,
    ) -> LocationExpander<'a> {
        let supplier = memoized_location_map(rule_context, label_map, allow_data, true);
        LocationExpander::new(
            rule_context,
            all_location_functions(rule_context.get_label(), supplier, exec_paths),
            rule_context
                .get_rule()
                .get_package_metadata()
                .repository_mapping
                .clone(),
            rule_context.get_workspace_name().to_owned(),
        )
    }

    /// Creates an expander that expands `$(location)`/`$(locations)` using runfiles paths.
    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:117 LocationExpander.withRunfilesPaths
    pub fn with_runfiles_paths(
        rule_context: &'a RuleContext,
        label_map: Option<IndexMap<Label, Vec<Artifact>>>,
    ) -> LocationExpander<'a> {
        Self::for_rule(rule_context, label_map, false, false)
    }

    /// Creates an expander that expands `$(location)`/`$(locations)` using execpaths.
    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:132 LocationExpander.withExecPaths
    pub fn with_exec_paths(
        rule_context: &'a RuleContext,
        label_map: IndexMap<Label, Vec<Artifact>>,
    ) -> LocationExpander<'a> {
        Self::for_rule(rule_context, Some(label_map), true, false)
    }

    /// Like `with_exec_paths`, but also allows labels in the `data` attribute.
    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:145 LocationExpander.withExecPathsAndData
    pub fn with_exec_paths_and_data(
        rule_context: &'a RuleContext,
        label_map: IndexMap<Label, Vec<Artifact>>,
    ) -> LocationExpander<'a> {
        Self::for_rule(rule_context, Some(label_map), true, true)
    }

    /// Expands `$(location)` in `input`, reporting errors as rule errors.
    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:150 LocationExpander.expand
    pub fn expand(&self, input: &str) -> String {
        self.expand_with(input, &|e: &str| self.rule_error_consumer.rule_error(e))
    }

    /// Expands attribute's location and locations tags based on the target and location map.
    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:201 LocationExpander.expandAttribute
    pub fn expand_attribute(&self, attr_name: &str, attr_value: &str) -> String {
        self.expand_with(attr_value, &|e: &str| {
            self.rule_error_consumer.attribute_error(attr_name, e)
        })
    }

    // Java: com/google/devtools/build/lib/analysis/LocationExpander.java:154 LocationExpander.expand
    fn expand_with(&self, value: &str, reporter: &dyn Fn(&str)) -> String {
        let mut restart = 0usize;
        let mut result = String::with_capacity(value.len());
        loop {
            // (1) find '$(<fname> '
            let Some(start) = value[restart..].find("$(").map(|i| i + restart) else {
                result.push_str(&value[restart..]);
                break;
            };
            let Some(next_whitespace) = value[start..].find(' ').map(|i| i + start) else {
                result.push_str(&value[restart..start + 2]);
                restart = start + 2;
                continue;
            };
            let fname = &value[start + 2..next_whitespace];
            if !self.functions.contains_key(fname) {
                result.push_str(&value[restart..start + 2]);
                restart = start + 2;
                continue;
            }

            result.push_str(&value[restart..start]);

            let Some(end) = value[next_whitespace..]
                .find(')')
                .map(|i| i + next_whitespace)
            else {
                reporter(&format!(
                    "unterminated $({}) expression",
                    &value[start + 2..next_whitespace]
                ));
                return value.to_owned();
            };

            // (2) Call appropriate function to obtain string replacement.
            let function_value = value[next_whitespace + 1..end].trim_matches(|c: char| c <= ' ');
            match self.functions[fname].apply(
                function_value,
                &self.repository_mapping,
                &self.workspace_runfiles_directory,
            ) {
                Ok(replacement) => result.push_str(&replacement),
                Err(ise) => {
                    reporter(&ise);
                    return value.to_owned();
                }
            }
            restart = end + 1;
        }
        result
    }
}

/// `Suppliers.memoize(() -> buildLocationMap(...))`.
///
/// PORT: the supplier captures a raw pointer to the rule context: Java's supplier captures the
/// `RuleContext`, which outlives every expander created from it (expanders are not kept after
/// analysis of the rule).
pub fn memoized_location_map(
    rule_context: &RuleContext,
    label_map: Option<IndexMap<Label, Vec<Artifact>>>,
    allow_data: bool,
    collect_srcs: bool,
) -> LocationMapSupplier {
    let cell: Arc<OnceLock<Arc<LocationMap>>> = Arc::new(OnceLock::new());
    let ctx_ptr = rule_context as *const RuleContext as usize;
    Arc::new(move || {
        cell.get_or_init(|| {
            // SAFETY: see the PORT note above; the rule context outlives this supplier.
            let rule_context = unsafe { &*(ctx_ptr as *const RuleContext) };
            Arc::new(build_location_map(
                rule_context,
                label_map.as_ref(),
                allow_data,
                collect_srcs,
            ))
        })
        .clone()
    })
}

/// Expands `$(location)` and related functions in addition to Make variables of a delegate
/// template context (Java `LocationTemplateContext`).
// Java: com/google/devtools/build/lib/analysis/LocationTemplateContext.java:45 LocationTemplateContext
pub struct LocationTemplateContext {
    delegate: Arc<dyn TemplateContext + Send + Sync>,
    functions: IndexMap<String, LocationFunction>,
    repository_mapping: RepositoryMapping,
    windows_path: bool,
    workspace_runfiles_directory: String,
}

impl LocationTemplateContext {
    // Java: com/google/devtools/build/lib/analysis/LocationTemplateContext.java:72 LocationTemplateContext.<init>
    pub fn new(
        delegate: Arc<dyn TemplateContext + Send + Sync>,
        rule_context: &RuleContext,
        label_map: Option<IndexMap<Label, Vec<Artifact>>>,
        exec_paths: bool,
        allow_data: bool,
        collect_srcs: bool,
        windows_path: bool,
    ) -> LocationTemplateContext {
        let supplier = memoized_location_map(rule_context, label_map, allow_data, collect_srcs);
        LocationTemplateContext {
            delegate,
            functions: all_location_functions(rule_context.get_label(), supplier, exec_paths),
            repository_mapping: rule_context
                .get_rule()
                .get_package_metadata()
                .repository_mapping
                .clone(),
            windows_path,
            workspace_runfiles_directory: rule_context.get_workspace_name().to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/LocationTemplateContext.java:110 LocationTemplateContext.lookupFunctionImpl
    fn lookup_function_impl(&self, name: &str, param: &str) -> Result<String, ExpansionException> {
        if let Some(f) = self.functions.get(name) {
            return f
                .apply(
                    param,
                    &self.repository_mapping,
                    &self.workspace_runfiles_directory,
                )
                .map_err(ExpansionException);
        }
        self.delegate.lookup_function(name, param)
    }
}

impl TemplateContext for LocationTemplateContext {
    // Java: com/google/devtools/build/lib/analysis/LocationTemplateContext.java:92 LocationTemplateContext.lookupVariable
    fn lookup_variable(&self, name: &str) -> Result<String, ExpansionException> {
        let mut val = self.delegate.lookup_variable(name)?;
        if self.windows_path {
            val = val.replace('/', "\\");
        }
        Ok(val)
    }

    // Java: com/google/devtools/build/lib/analysis/LocationTemplateContext.java:101 LocationTemplateContext.lookupFunction
    fn lookup_function(&self, name: &str, param: &str) -> Result<String, ExpansionException> {
        let mut val = self.lookup_function_impl(name, param)?;
        if self.windows_path {
            val = val.replace('/', "\\");
        }
        Ok(val)
    }
}

#[cfg(test)]
mod tests {
    // Ports of src/test/java/com/google/devtools/build/lib/analysis/LocationExpanderTest.java
    // (the parts that do not need a RuleContext).
    use super::*;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct Capture {
        errors: Mutex<Vec<String>>,
    }

    impl RuleErrorConsumer for Capture {
        fn rule_warning(&self, _m: &str) {}
        fn rule_error(&self, m: &str) {
            self.errors.lock().push(m.to_owned());
        }
        fn attribute_warning(&self, _a: &str, _m: &str) {}
        fn attribute_error(&self, _a: &str, m: &str) {
            self.errors.lock().push(m.to_owned());
        }
        fn has_errors(&self) -> bool {
            !self.errors.lock().is_empty()
        }
    }

    fn make_expander<'a>(capture: &'a Capture, functions: &[(&str, &str)]) -> LocationExpander<'a> {
        // A fake function returning "PREFIX(arg)" through a location map over no artifacts is
        // not expressible; test the scanning with an empty function table and with unknown
        // function names.
        let _ = functions;
        LocationExpander::new(
            capture,
            IndexMap::new(),
            blaze_cmdline::repository_mapping::EMPTY.clone(),
            "workspace".to_owned(),
        )
    }

    #[test]
    fn no_expansion() {
        let c = Capture::default();
        let e = make_expander(&c, &[]);
        assert_eq!(e.expand("foo"), "foo");
        assert!(!c.has_errors());
    }

    #[test]
    fn unknown_function_left_alone() {
        let c = Capture::default();
        let e = make_expander(&c, &[]);
        assert_eq!(e.expand("$(foo bar) $(baz"), "$(foo bar) $(baz");
        assert!(!c.has_errors());
    }

    #[test]
    #[ignore = "needs a RuleContext with prerequisites (LocationExpanderTest.location)"]
    fn location() {}

    #[test]
    #[ignore = "needs a RuleContext with prerequisites (LocationExpanderTest.locations)"]
    fn locations() {}
}
