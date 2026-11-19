//! `rules/java/JavaTargetAttributes.java`: an object that captures the temporary state we need
//! to pass around while the initialization hook for a java rule is running.

use blaze_actions::Artifact;
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_optdefs::fragment::converters::CoreOptionConvertersStrictDepsMode as StrictDepsMode;
use blaze_util::{check_argument, check_state};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use super::boot_class_path_info::BootClassPathInfo;
use super::java_plugin_info::{self, JavaPluginInfo};

/// `JavaSemantics.JAR.matches(filename)`.
// Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:34 JavaSemantics.JAR
fn is_jar(filename: &str) -> bool {
    filename.ends_with(".jar")
}

/// `JavaSemantics.JAVA_SOURCE.matches(filename)`.
// Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:33 JavaSemantics.JAVA_SOURCE
fn is_java_source(filename: &str) -> bool {
    filename.ends_with(".java")
}

// Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:52 JavaTargetAttributes.checkJar
fn check_jar(class_path_entry: &Artifact) {
    if !is_jar(class_path_entry.get_filename()) {
        panic!(
            "IllegalArgumentException: not a jar file: {}",
            class_path_entry.pretty_print()
        );
    }
}

/// A builder class for JavaTargetAttributes.
// Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:59 JavaTargetAttributes.Builder
pub struct JavaTargetAttributesBuilder<'v> {
    // The order of source files is important, and there must not be duplicates.
    source_files: Vec<Artifact>,
    runtime_class_path: NestedSetBuilder<Artifact>,
    compile_time_class_path_builder: NestedSetBuilder<Artifact>,
    boot_class_path: BootClassPathInfo<'v>,
    source_path: Vec<Artifact>,
    native_libraries: Vec<Artifact>,
    plugins: JavaPluginInfo,
    resources: IndexMap<PathFragment, Artifact>,
    resource_jars: NestedSetBuilder<Artifact>,
    messages: Vec<Artifact>,
    source_jars: Vec<Artifact>,
    class_path_resources: Vec<Artifact>,
    additional_outputs: IndexSet<Artifact>,
    strict_java_deps: StrictDepsMode,
    direct_jars_builder: NestedSetBuilder<Artifact>,
    header_compilation_direct_jars_builder: NestedSetBuilder<Artifact>,
    compile_time_dependency_artifacts: NestedSetBuilder<Artifact>,
    target_label: Option<Label>,
    injecting_rule_kind: Option<String>,
    excluded_artifacts: NestedSetBuilder<Artifact>,
    prepend_direct_jars: bool,
    built: bool,
}

impl<'v> JavaTargetAttributesBuilder<'v> {
    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:94 JavaTargetAttributes.Builder.<init>
    pub fn new() -> Self {
        JavaTargetAttributesBuilder {
            source_files: Vec::new(),
            runtime_class_path: NestedSetBuilder::naive_link_order(),
            compile_time_class_path_builder: NestedSetBuilder::naive_link_order(),
            boot_class_path: BootClassPathInfo::empty(),
            source_path: Vec::new(),
            native_libraries: Vec::new(),
            plugins: JavaPluginInfo::empty(java_plugin_info::provider_key()),
            resources: IndexMap::new(),
            resource_jars: NestedSetBuilder::stable_order(),
            messages: Vec::new(),
            source_jars: Vec::new(),
            class_path_resources: Vec::new(),
            additional_outputs: IndexSet::new(),
            strict_java_deps: StrictDepsMode::ERROR,
            direct_jars_builder: NestedSetBuilder::naive_link_order(),
            header_compilation_direct_jars_builder: NestedSetBuilder::naive_link_order(),
            compile_time_dependency_artifacts: NestedSetBuilder::stable_order(),
            target_label: None,
            injecting_rule_kind: None,
            excluded_artifacts: NestedSetBuilder::naive_link_order(),
            prepend_direct_jars: true,
            built: false,
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:97 JavaTargetAttributes.Builder.addSourceFiles
    pub fn add_source_files<'a>(
        &mut self,
        source_files: impl IntoIterator<Item = &'a Artifact>,
    ) -> &mut Self {
        check_argument!(!self.built);
        for artifact in source_files {
            if is_java_source(artifact.get_filename()) {
                self.source_files.push(artifact.clone());
            }
        }
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:108 JavaTargetAttributes.Builder.addSourceJars
    pub fn add_source_jars(&mut self, source_jars: &[Artifact]) -> &mut Self {
        check_argument!(!self.built);
        self.source_jars.extend(source_jars.iter().cloned());
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:115 JavaTargetAttributes.Builder.addRuntimeClassPathEntry
    pub fn add_runtime_class_path_entry(&mut self, class_path_entry: Artifact) -> &mut Self {
        check_argument!(!self.built);
        check_jar(&class_path_entry);
        self.runtime_class_path.add(class_path_entry);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:123 JavaTargetAttributes.Builder.addRuntimeClassPathEntries
    pub fn add_runtime_class_path_entries(
        &mut self,
        class_path_entries: &NestedSet<Artifact>,
    ) -> &mut Self {
        check_argument!(!self.built);
        self.runtime_class_path.add_transitive(class_path_entries);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:130 JavaTargetAttributes.Builder.addCompileTimeClassPathEntries
    pub fn add_compile_time_class_path_entries(
        &mut self,
        entries: &NestedSet<Artifact>,
    ) -> &mut Self {
        check_argument!(!self.built);
        self.compile_time_class_path_builder.add_transitive(entries);
        self
    }

    /// Avoids prepending the direct jars to the compile-time classpath when building the
    /// attributes, assuming that they have already been prepended. This avoids creating a new
    /// NestedSet instance.
    ///
    /// After this method is called, `add_direct_jars` will throw an exception.
    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:144 JavaTargetAttributes.Builder.setCompileTimeClassPathEntriesWithPrependedDirectJars
    pub fn set_compile_time_class_path_entries_with_prepended_direct_jars(
        &mut self,
        entries: &NestedSet<Artifact>,
    ) -> &mut Self {
        check_argument!(!self.built);
        check_argument!(self.compile_time_class_path_builder.is_empty());
        self.prepend_direct_jars = false;
        self.compile_time_class_path_builder.add_transitive(entries);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:154 JavaTargetAttributes.Builder.setTargetLabel
    pub fn set_target_label(&mut self, target_label: Label) -> &mut Self {
        check_argument!(!self.built);
        self.target_label = Some(target_label);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:161 JavaTargetAttributes.Builder.setInjectingRuleKind
    pub fn set_injecting_rule_kind(&mut self, injecting_rule_kind: Option<String>) -> &mut Self {
        check_argument!(!self.built);
        self.injecting_rule_kind = injecting_rule_kind;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:168 JavaTargetAttributes.Builder.setBootClassPath
    pub fn set_boot_class_path(
        &mut self,
        boot_class_path: BootClassPathInfo<'v>,
    ) -> starlark::Result<&mut Self> {
        check_argument!(!self.built);
        check_argument!(!boot_class_path.is_empty()?);
        check_state!(self.boot_class_path.is_empty()?);
        self.boot_class_path = boot_class_path;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:177 JavaTargetAttributes.Builder.setSourcePath
    pub fn set_source_path(&mut self, artifacts: Vec<Artifact>) -> &mut Self {
        check_argument!(!self.built);
        check_argument!(self.source_path.is_empty());
        self.source_path = artifacts;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:185 JavaTargetAttributes.Builder.setStrictJavaDeps
    pub fn set_strict_java_deps(&mut self, strict_deps: StrictDepsMode) -> &mut Self {
        check_argument!(!self.built);
        self.strict_java_deps = strict_deps;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:192 JavaTargetAttributes.Builder.addDirectJars
    pub fn add_direct_jars(&mut self, direct_jars: &NestedSet<Artifact>) -> &mut Self {
        check_argument!(!self.built);
        check_argument!(self.prepend_direct_jars);
        self.direct_jars_builder.add_transitive(direct_jars);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:200 JavaTargetAttributes.Builder.addHeaderCompilationDirectJars
    pub fn add_header_compilation_direct_jars(
        &mut self,
        header_compilation_direct_jars: &NestedSet<Artifact>,
    ) -> &mut Self {
        check_argument!(!self.built);
        self.header_compilation_direct_jars_builder
            .add_transitive(header_compilation_direct_jars);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:207 JavaTargetAttributes.Builder.addCompileTimeDependencyArtifacts
    pub fn add_compile_time_dependency_artifacts(
        &mut self,
        dependency_artifacts: &NestedSet<Artifact>,
    ) -> &mut Self {
        check_argument!(!self.built);
        self.compile_time_dependency_artifacts
            .add_transitive(dependency_artifacts);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:214 JavaTargetAttributes.Builder.addMessages
    pub fn add_messages(&mut self, messages: &[Artifact]) -> &mut Self {
        check_argument!(!self.built);
        self.messages.extend(messages.iter().cloned());
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:221 JavaTargetAttributes.Builder.addResource
    pub fn add_resource(&mut self, exec_path: PathFragment, resource: Artifact) -> &mut Self {
        check_argument!(!self.built);
        self.resources.insert(exec_path, resource);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:228 JavaTargetAttributes.Builder.addResourceJars
    pub fn add_resource_jars(&mut self, resource_jars: &NestedSet<Artifact>) -> &mut Self {
        check_argument!(!self.built);
        self.resource_jars.add_transitive(resource_jars);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:235 JavaTargetAttributes.Builder.addPlugin
    pub fn add_plugin(&mut self, plugins: &JavaPluginInfo) -> &mut Self {
        check_argument!(!self.built);
        self.plugins = JavaPluginInfo::merge_without_java_outputs_pair(&self.plugins, plugins);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:242 JavaTargetAttributes.Builder.addClassPathResources
    pub fn add_class_path_resources(&mut self, class_path_resources: &[Artifact]) -> &mut Self {
        check_argument!(!self.built);
        self.class_path_resources
            .extend(class_path_resources.iter().cloned());
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:249 JavaTargetAttributes.Builder.addAdditionalOutputs
    pub fn add_additional_outputs<'a>(
        &mut self,
        outputs: impl IntoIterator<Item = &'a Artifact>,
    ) -> &mut Self {
        check_argument!(!self.built);
        self.additional_outputs.extend(outputs.into_iter().cloned());
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:255 JavaTargetAttributes.Builder.build
    pub fn build(&mut self) -> JavaTargetAttributes<'v> {
        self.built = true;
        let direct_jars = self.direct_jars_builder.build();
        let header_compilation_direct_jars = self.header_compilation_direct_jars_builder.build();
        let compile_time_class_path = if self.prepend_direct_jars {
            let mut b = NestedSetBuilder::naive_link_order();
            b.add_transitive(&direct_jars);
            b.add_transitive(&self.compile_time_class_path_builder.build());
            b.build()
        } else {
            self.compile_time_class_path_builder.build()
        };
        JavaTargetAttributes {
            source_files: self.source_files.iter().cloned().collect(),
            runtime_class_path: self.runtime_class_path.build(),
            compile_time_class_path,
            boot_class_path: self.boot_class_path,
            source_path: self.source_path.clone(),
            native_libraries: self.native_libraries.clone(),
            plugins: self.plugins.clone(),
            resources: self.resources.clone(),
            resource_jars: self.resource_jars.build(),
            messages: self.messages.clone(),
            source_jars: self.source_jars.clone(),
            class_path_resources: self.class_path_resources.clone(),
            additional_outputs: self.additional_outputs.clone(),
            direct_jars,
            header_compilation_direct_jars,
            compile_time_dependency_artifacts: self.compile_time_dependency_artifacts.build(),
            target_label: self.target_label.clone(),
            injecting_rule_kind: self.injecting_rule_kind.clone(),
            excluded_artifacts: self.excluded_artifacts.build(),
            strict_java_deps: self.strict_java_deps,
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:288 JavaTargetAttributes.Builder.hasSourceFiles
    pub fn has_source_files(&self) -> bool {
        !self.source_files.is_empty()
    }
}

/// An object that captures the temporary state we need to pass around while the initialization
/// hook for a java rule is running.
// Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:47 JavaTargetAttributes
#[derive(Clone)]
pub struct JavaTargetAttributes<'v> {
    source_files: IndexSet<Artifact>,
    runtime_class_path: NestedSet<Artifact>,
    compile_time_class_path: NestedSet<Artifact>,
    boot_class_path: BootClassPathInfo<'v>,
    source_path: Vec<Artifact>,
    native_libraries: Vec<Artifact>,
    plugins: JavaPluginInfo,
    resources: IndexMap<PathFragment, Artifact>,
    resource_jars: NestedSet<Artifact>,
    messages: Vec<Artifact>,
    source_jars: Vec<Artifact>,
    class_path_resources: Vec<Artifact>,
    additional_outputs: IndexSet<Artifact>,
    direct_jars: NestedSet<Artifact>,
    header_compilation_direct_jars: NestedSet<Artifact>,
    compile_time_dependency_artifacts: NestedSet<Artifact>,
    target_label: Option<Label>,
    injecting_rule_kind: Option<String>,
    excluded_artifacts: NestedSet<Artifact>,
    strict_java_deps: StrictDepsMode,
}

impl<'v> JavaTargetAttributes<'v> {
    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:383 JavaTargetAttributes.appendAdditionalTransitiveClassPathEntries
    pub fn append_additional_transitive_class_path_entries(
        &self,
        additional_class_path_entries: &NestedSet<Artifact>,
    ) -> JavaTargetAttributes<'v> {
        let mut b = NestedSetBuilder::from_nested_set(&self.compile_time_class_path);
        b.add_transitive(additional_class_path_entries);
        let mut result = self.clone();
        result.compile_time_class_path = b.build();
        result
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:409 JavaTargetAttributes.getDirectJars
    pub fn get_direct_jars(&self) -> &NestedSet<Artifact> {
        &self.direct_jars
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:413 JavaTargetAttributes.getHeaderCompilationDirectJars
    pub fn get_header_compilation_direct_jars(&self) -> &NestedSet<Artifact> {
        &self.header_compilation_direct_jars
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:417 JavaTargetAttributes.getCompileTimeDependencyArtifacts
    pub fn get_compile_time_dependency_artifacts(&self) -> &NestedSet<Artifact> {
        &self.compile_time_dependency_artifacts
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:421 JavaTargetAttributes.getSourceJars
    pub fn get_source_jars(&self) -> &[Artifact] {
        &self.source_jars
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:425 JavaTargetAttributes.getResources
    pub fn get_resources(&self) -> &IndexMap<PathFragment, Artifact> {
        &self.resources
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:429 JavaTargetAttributes.getResourceJars
    pub fn get_resource_jars(&self) -> &NestedSet<Artifact> {
        &self.resource_jars
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:433 JavaTargetAttributes.getMessages
    pub fn get_messages(&self) -> &[Artifact] {
        &self.messages
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:437 JavaTargetAttributes.getClassPathResources
    pub fn get_class_path_resources(&self) -> &[Artifact] {
        &self.class_path_resources
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:441 JavaTargetAttributes.getAdditionalOutputs
    pub fn get_additional_outputs(&self) -> &IndexSet<Artifact> {
        &self.additional_outputs
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:445 JavaTargetAttributes.getCompileTimeClassPath
    pub fn get_compile_time_class_path(&self) -> &NestedSet<Artifact> {
        &self.compile_time_class_path
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:449 JavaTargetAttributes.getBootClassPath
    pub fn get_boot_class_path(&self) -> BootClassPathInfo<'v> {
        self.boot_class_path
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:453 JavaTargetAttributes.getSourcePath
    pub fn get_source_path(&self) -> &[Artifact] {
        &self.source_path
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:457 JavaTargetAttributes.plugins
    pub fn plugins(&self) -> &JavaPluginInfo {
        &self.plugins
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:461 JavaTargetAttributes.getSourceFiles
    pub fn get_source_files(&self) -> &IndexSet<Artifact> {
        &self.source_files
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:465 JavaTargetAttributes.hasResources
    pub fn has_resources(&self) -> bool {
        !self.resources.is_empty()
            || !self.messages.is_empty()
            || !self.class_path_resources.is_empty()
            || !self.resource_jars.is_empty()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:472 JavaTargetAttributes.getTargetLabel
    pub fn get_target_label(&self) -> Option<&Label> {
        self.target_label.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:477 JavaTargetAttributes.getInjectingRuleKind
    pub fn get_injecting_rule_kind(&self) -> Option<&str> {
        self.injecting_rule_kind.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaTargetAttributes.java:481 JavaTargetAttributes.getStrictJavaDeps
    pub fn get_strict_java_deps(&self) -> StrictDepsMode {
        self.strict_java_deps
    }

    pub fn get_runtime_class_path(&self) -> &NestedSet<Artifact> {
        &self.runtime_class_path
    }

    pub fn get_native_libraries(&self) -> &[Artifact] {
        &self.native_libraries
    }

    pub fn get_excluded_artifacts(&self) -> &NestedSet<Artifact> {
        &self.excluded_artifacts
    }
}
