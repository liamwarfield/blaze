//! `rules/java/JavaToolchainTool.java`: an executable tool that is part of `java_toolchain`.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::analysis::actions::custom_command_line::CustomCommandLine;
use blaze_analysis::providers::file_provider::FilesToRunProvider;
use blaze_analysis::starlark::starlark_action_factory::files_to_run_provider_from_value;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use starlark::values::{Heap, Value};

use super::java_toolchain_provider::JavaToolchainProvider;
use super::starlark_info_wrapper::{StarlarkInfoWrapper, struct_get_value};

/// An executable tool that is part of `java_toolchain`.
///
/// PORT: Java interns instances and caches the command line per toolchain (a memory
/// optimisation with no observable effect); the command line is computed on each request.
// Java: com/google/devtools/build/lib/rules/java/JavaToolchainTool.java:36 JavaToolchainTool
#[derive(Clone)]
pub struct JavaToolchainTool {
    /// The executable, possibly a `_deploy.jar`.
    tool: Arc<FilesToRunProvider>,
    /// Additional inputs required by the tool, e.g. a Class Data Sharing archive.
    data: NestedSet<Artifact>,
    /// JVM flags to invoke the tool with. Location expansion is performed on these flags using
    /// the inputs in `data`.
    jvm_opts: NestedSet<String>,
}

impl JavaToolchainTool {
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainTool.java:44 JavaToolchainTool.fromStarlark
    pub fn from_starlark<'v>(
        r#struct: Option<Value<'v>>,
        heap: Heap<'v>,
    ) -> starlark::Result<Option<JavaToolchainTool>> {
        let Some(s) = r#struct else {
            return Ok(None);
        };
        let tool_value = struct_get_value(s, "tool", heap);
        let tool = match tool_value {
            None => panic!("NullPointerException: tool"),
            Some(v) if v.is_none() => panic!("NullPointerException: tool"),
            Some(v) => match files_to_run_provider_from_value(v) {
                Some(t) => t,
                None => {
                    return Err(StarlarkInfoWrapper::cast_error(
                        "tool",
                        v,
                        "FilesToRunProvider",
                    ));
                }
            },
        };
        let data = match struct_get_value(s, "data", heap) {
            None => NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            Some(v) if v.is_none() => NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            Some(v) => blaze_analysis::rule::depsets::depset_to_artifacts(v, "data")?,
        };
        let jvm_opts = match struct_get_value(s, "jvm_opts", heap) {
            None => NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            Some(v) if v.is_none() => NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            Some(v) => blaze_analysis::rule::depsets::depset_to_strings(v, "jvm_opts")?,
        };
        Ok(Some(Self::create(tool, data, jvm_opts)))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainTool.java:85 JavaToolchainTool.create
    fn create(
        tool: Arc<FilesToRunProvider>,
        data: NestedSet<Artifact>,
        jvm_opts: NestedSet<String>,
    ) -> JavaToolchainTool {
        JavaToolchainTool {
            tool,
            data,
            jvm_opts,
        }
    }

    /// The executable, possibly a `_deploy.jar`.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainTool.java:61 JavaToolchainTool.tool
    pub fn tool(&self) -> &Arc<FilesToRunProvider> {
        &self.tool
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainTool.java:64 JavaToolchainTool.data
    pub fn data(&self) -> &NestedSet<Artifact> {
        &self.data
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainTool.java:70 JavaToolchainTool.jvmOpts
    pub fn jvm_opts(&self) -> &NestedSet<String> {
        &self.jvm_opts
    }

    fn executable(&self) -> Artifact {
        self.tool
            .get_executable()
            .cloned()
            .unwrap_or_else(|| panic!("NullPointerException: tool().getExecutable()"))
    }

    /// Returns the executable command line for the tool.
    ///
    /// For a Java command, the executable command line will include `java -jar deploy.jar` as
    /// well as any JVM flags.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainTool.java:98 JavaToolchainTool.getCommandLine
    pub fn get_command_line(
        &self,
        toolchain: &JavaToolchainProvider<'_>,
    ) -> starlark::Result<CustomCommandLine> {
        self.extract_command_line(toolchain)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainTool.java:102 JavaToolchainTool.extractCommandLine
    fn extract_command_line(
        &self,
        toolchain: &JavaToolchainProvider<'_>,
    ) -> starlark::Result<CustomCommandLine> {
        let mut command = CustomCommandLine::builder();
        let executable = self.executable();
        if executable.get_extension() != "jar" {
            command
                .add_exec_path(executable)
                .add_all_nested_set(self.jvm_opts.clone());
        } else {
            command
                .add_path(
                    toolchain
                        .get_java_runtime()?
                        .java_binary_exec_path_fragment()?,
                )
                .add_all_nested_set(toolchain.get_jvm_options()?)
                .add_all_nested_set(self.jvm_opts.clone())
                .add("-jar")
                .add_path(executable.get_exec_path().clone());
        }
        Ok(command.build())
    }

    /// Adds its inputs for the tool to provided input builder.
    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainTool.java:122 JavaToolchainTool.addInputs
    pub fn add_inputs(
        &self,
        toolchain: &JavaToolchainProvider<'_>,
        inputs: &mut NestedSetBuilder<Artifact>,
    ) -> starlark::Result<()> {
        inputs.add_transitive(&self.data);
        let executable = self.executable();
        // The runfiles of the tool are not added. If this is desired, add getFilesToRun() to
        // inputs instead.
        inputs.add(executable.clone());
        if executable.get_extension() == "jar" {
            inputs.add_transitive(&toolchain.get_java_runtime()?.java_base_inputs()?);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaToolchainTool.java:134 JavaToolchainTool.withAdditionalJvmFlags
    pub fn with_additional_jvm_flags(
        &self,
        additional_jvm_flags: &NestedSet<String>,
    ) -> JavaToolchainTool {
        if additional_jvm_flags.is_empty() {
            return self.clone();
        }
        let mut jvm_opts = NestedSetBuilder::stable_order();
        jvm_opts.add_transitive(&self.jvm_opts);
        jvm_opts.add_transitive(additional_jvm_flags);
        Self::create(self.tool.clone(), self.data.clone(), jvm_opts.build())
    }
}
