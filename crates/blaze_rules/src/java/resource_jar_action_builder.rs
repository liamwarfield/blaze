//! `rules/java/ResourceJarActionBuilder.java`: builds the action to package the resources for a
//! Java rule into a jar.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::param_file_info::ParamFileInfo;
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_actions::analysis::actions::custom_command_line::{
    CustomCommandLine, CustomCommandLineBuilder,
};
use blaze_actions::analysis::actions::spawn_action::SpawnActionBuilder;
use blaze_analysis::rule::RuleContext;
use blaze_analysis::rule::config_util::ConfigurationSpawnActionConfiguration;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::target::target_utils;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use super::java_compilation_helper::register_action;
use super::java_semantics::JavaSemantics;
use super::java_toolchain_provider::JavaToolchainProvider;

// Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:39 ResourceJarActionBuilder.MNEMONIC
pub const MNEMONIC: &str = "JavaResourceJar";

/// `ParamFileInfo.builder(SHELL_QUOTED).build()`.
// Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:40 ResourceJarActionBuilder.PARAM_FILE_INFO
fn param_file_info() -> ParamFileInfo {
    ParamFileInfo::builder(ParameterFileType::SHELL_QUOTED).build()
}

/// `ImmutableMap.of(ExecutionRequirements.SUPPORTS_PATH_MAPPING, "1")`.
// Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:42 ResourceJarActionBuilder.EXECUTION_INFO
fn execution_info() -> IndexMap<String, String> {
    let mut m = IndexMap::new();
    m.insert("supports-path-mapping".to_owned(), "1".to_owned());
    m
}

/// Builds the action to package the resources for a Java rule into a jar.
// Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:38 ResourceJarActionBuilder
pub struct ResourceJarActionBuilder<'v> {
    output_jar: Option<Artifact>,
    resources: IndexMap<PathFragment, Artifact>,
    resource_jars: NestedSet<Artifact>,
    classpath_resources: Vec<Artifact>,
    messages: Vec<Artifact>,
    java_toolchain: Option<JavaToolchainProvider<'v>>,
    additional_inputs: NestedSet<Artifact>,
}

impl<'v> ResourceJarActionBuilder<'v> {
    pub fn new() -> Self {
        ResourceJarActionBuilder {
            output_jar: None,
            resources: IndexMap::new(),
            resource_jars: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            classpath_resources: Vec::new(),
            messages: Vec::new(),
            java_toolchain: None,
            additional_inputs: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:53 ResourceJarActionBuilder.setOutputJar
    pub fn set_output_jar(&mut self, output_jar: Artifact) -> &mut Self {
        self.output_jar = Some(output_jar);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:59 ResourceJarActionBuilder.setAdditionalInputs
    pub fn set_additional_inputs(&mut self, additional_inputs: NestedSet<Artifact>) -> &mut Self {
        self.additional_inputs = additional_inputs;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:65 ResourceJarActionBuilder.setClasspathResources
    pub fn set_classpath_resources(&mut self, classpath_resources: Vec<Artifact>) -> &mut Self {
        self.classpath_resources = classpath_resources;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:72 ResourceJarActionBuilder.setResources
    pub fn set_resources(&mut self, resources: IndexMap<PathFragment, Artifact>) -> &mut Self {
        self.resources = resources;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:78 ResourceJarActionBuilder.setResourceJars
    pub fn set_resource_jars(&mut self, resource_jars: NestedSet<Artifact>) -> &mut Self {
        self.resource_jars = resource_jars;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:84 ResourceJarActionBuilder.setTranslations
    pub fn set_translations(&mut self, translations: Vec<Artifact>) -> &mut Self {
        self.messages = translations;
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:90 ResourceJarActionBuilder.setJavaToolchain
    pub fn set_java_toolchain(&mut self, java_toolchain: JavaToolchainProvider<'v>) -> &mut Self {
        self.java_toolchain = Some(java_toolchain);
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:95 ResourceJarActionBuilder.build
    pub fn build(
        &self,
        eval: &starlark::eval::Evaluator,
        semantics: &dyn JavaSemantics,
        rule_context: &RuleContext,
        exec_group: &str,
    ) -> starlark::Result<()> {
        let output_jar = self
            .output_jar
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: outputJar must not be null"));
        let java_toolchain = self
            .java_toolchain
            .unwrap_or_else(|| panic!("NullPointerException: javaToolchain must not be null"));
        // Java: checkNotNull(javaToolchain.getJavaRuntime(), "javabase must not be null")
        java_toolchain.get_java_runtime()?;
        let mut builder = SpawnActionBuilder::new();
        let mut command = CustomCommandLine::builder();
        command
            .add("--normalize")
            .add("--dont_change_compression")
            .add("--exclude_build_data")
            .add("--no_strip_module_info") // bazelbuild/rules_java/issues/293
            .add_exec_path_with_arg("--output", Some(output_jar.clone()));
        if !self.resource_jars.is_empty() {
            command.add_exec_paths_nested_set_with_arg("--sources", self.resource_jars.clone());
        }
        self.add_resources(&mut command, semantics);
        if !self.classpath_resources.is_empty() {
            command.add_exec_paths_with_arg(
                "--classpath_resources",
                self.classpath_resources.iter().cloned(),
            );
        }
        let mut execution_info = execution_info();
        if rule_context.is_allow_tags_propagation() {
            // Java: ImmutableMap.Builder.build() (no duplicate keys expected).
            for (k, v) in target_utils::get_execution_info_with_propagation(
                rule_context.get_rule(),
                rule_context.is_allow_tags_propagation(),
            ) {
                if execution_info.contains_key(&k) {
                    panic!("IllegalArgumentException: Multiple entries with same key: {k}");
                }
                execution_info.insert(k, v);
            }
        }
        let single_jar = java_toolchain
            .get_single_jar()?
            .unwrap_or_else(|| panic!("NullPointerException: single_jar"));
        builder
            .set_executable_files_to_run(
                single_jar.get_executable().cloned(),
                single_jar.get_files_to_run(),
            )
            .use_default_shell_environment(IndexMap::new())
            .add_output(output_jar)
            .add_inputs(self.messages.iter().cloned())
            .add_inputs(self.resources.values().cloned())
            .add_transitive_inputs(&self.resource_jars)
            .add_transitive_inputs(&self.additional_inputs)
            .add_inputs(self.classpath_resources.iter().cloned())
            .add_command_line_with_param_file(Arc::new(command.build()), Some(param_file_info()))
            .set_progress_message("Building Java resource jar")
            .set_mnemonic(MNEMONIC)
            .set_execution_info(execution_info)
            .set_exec_group(exec_group);
        let owner = rule_context
            .get_action_owner_for(builder.get_exec_group())
            .unwrap_or_else(|| panic!("NullPointerException: action owner"));
        let action = builder.build(
            owner,
            &ConfigurationSpawnActionConfiguration(rule_context.get_configuration().clone()),
        );
        register_action(eval, Arc::new(action))
    }

    // Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:146 ResourceJarActionBuilder.addResources
    fn add_resources(&self, command: &mut CustomCommandLineBuilder, semantics: &dyn JavaSemantics) {
        if self.resources.is_empty() && self.messages.is_empty() {
            return;
        }
        command.add("--resources");
        let resources_with_default_path: Vec<Artifact>;
        // When all resources use the default path (common case), save memory by throwing away
        // those path fragments. The artifacts can be lazily converted to default-prefixed
        // strings.
        if self
            .resources
            .iter()
            .all(|(k, v)| *k == default_resource_path(v, semantics))
        {
            resources_with_default_path = self
                .resources
                .values()
                .cloned()
                .chain(self.messages.iter().cloned())
                .collect();
        } else {
            command.add_all(
                self.resources
                    .iter()
                    .map(|(k, v)| resource_prefixed_exec_path(k, v)),
            );
            resources_with_default_path = self.messages.clone();
        }
        if !resources_with_default_path.is_empty() {
            command.add_all(resources_with_default_path.iter().map(|artifact| {
                resource_prefixed_exec_path(&default_resource_path(artifact, semantics), artifact)
            }));
        }
    }
}

// Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:177 ResourceJarActionBuilder.defaultResourcePath
fn default_resource_path(artifact: &Artifact, semantics: &dyn JavaSemantics) -> PathFragment {
    semantics.get_default_java_resource_path(&artifact.get_root_relative_path())
}

// Java: com/google/devtools/build/lib/rules/java/ResourceJarActionBuilder.java:181 ResourceJarActionBuilder.resourcePrefixedExecPath
fn resource_prefixed_exec_path(resource_path: &PathFragment, artifact: &Artifact) -> String {
    let exec_path = artifact.get_exec_path();
    if exec_path == resource_path {
        exec_path.get_path_string().to_string()
    } else {
        format!("{}:{}", exec_path, resource_path)
    }
}
