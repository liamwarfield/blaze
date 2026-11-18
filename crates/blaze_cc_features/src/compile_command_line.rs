//! Port of `com.google.devtools.build.lib.rules.cpp.CompileCommandLine`.

use std::sync::Arc;

use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::actions::{CommandLineExpansionException, NOOP, PathMapper};
use crate::cc_common::CoptsFilter;
use crate::cc_toolchain_features::FeatureConfiguration;
use crate::cc_toolchain_variables::{CcToolchainVariables, CcToolchainVariablesRef};
use crate::compile_build_variables::CompileBuildVariables;
use crate::cpp_rule_classes::UNFILTERED_COMPILE_FLAGS_FEATURE_NAME;

/// The compile command line for the C++ compile action.
// Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:35 CompileCommandLine
#[derive(Clone, Debug)]
pub struct CompileCommandLine {
    copts_filter: CoptsFilter,
    feature_configuration: Arc<FeatureConfiguration>,
    variables: CcToolchainVariablesRef,
    action_name: String,
}

impl CompileCommandLine {
    /// Returns the environment variables that should be set for C++ compile actions.
    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:53 CompileCommandLine.getEnvironment
    pub fn get_environment(
        &self,
        path_mapper: &dyn PathMapper,
    ) -> Result<IndexMap<String, String>, CommandLineExpansionException> {
        self.feature_configuration
            .get_environment_variables(&self.action_name, &self.variables, path_mapper)
            .map_err(|e| CommandLineExpansionException { message: e.message })
    }

    /// Returns the tool path for the compilation based on the current feature configuration.
    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:64 CompileCommandLine.getToolPath
    pub fn get_tool_path(&self) -> String {
        blaze_util::check_argument!(
            self.feature_configuration
                .action_is_configured(&self.action_name),
            "Expected action_config for '{}' to be configured",
            self.action_name
        );
        self.feature_configuration
            .get_tool_path_for_action(&self.action_name)
    }

    /// `overwritten_variables`: Variables that will overwrite original build variables. When
    /// `None`, unmodified original variables are used.
    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:76 CompileCommandLine.getArguments
    pub fn get_arguments(
        &self,
        parameter_file_path: Option<&PathFragment>,
        overwritten_variables: Option<&CcToolchainVariables>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineExpansionException> {
        let mut command_line = Vec::new();

        // first: The command name.
        if path_mapper.is_noop() {
            command_line.push(self.get_tool_path());
        } else {
            // getToolPath() ultimately returns a PathFragment's getSafePathString(), so its safe
            // to reparse it here with no risk of e.g. altering a user-specified absolute path.
            command_line.push(
                path_mapper
                    .map(&PathFragment::create(&self.get_tool_path()))
                    .get_safe_path_string()
                    .to_string(),
            );
        }

        // second: The compiler options.
        if let Some(parameter_file_path) = parameter_file_path {
            command_line.push(format!("@{}", parameter_file_path.get_safe_path_string()));
        } else {
            command_line.extend(self.get_compiler_options(overwritten_variables, path_mapper)?);
        }
        Ok(command_line)
    }

    /// The command line `CppCompileAction` reports to `getFilteredFeatureConfigurationCommandLine`
    /// consumers: the tool path and the compiler options with path mapping disabled.
    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:107 CompileCommandLine.getFilteredFeatureConfigurationCommandLine
    pub fn get_filtered_feature_configuration_command_line(
        &self,
        overwritten_variables: Option<&CcToolchainVariables>,
    ) -> Result<Vec<String>, CommandLineExpansionException> {
        let compiler_options = self.get_compiler_options(overwritten_variables, &NOOP)?;
        let mut result = vec![self.get_tool_path()];
        result.extend(compiler_options);
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:119 CompileCommandLine.getCompilerOptions
    pub fn get_compiler_options(
        &self,
        overwritten_variables: Option<&CcToolchainVariables>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineExpansionException> {
        let mut options = Vec::new();

        let mut updated_variables = self.variables.clone();
        if let Some(overwritten_variables) = overwritten_variables {
            let mut variables_builder =
                CcToolchainVariables::builder_with_parent(Some(self.variables.clone()));
            variables_builder.add_all_non_transitive(overwritten_variables);
            updated_variables = variables_builder.build();
        }
        let expansions = self
            .feature_configuration
            .get_per_feature_expansions(&self.action_name, &updated_variables, path_mapper)
            .map_err(|e| CommandLineExpansionException { message: e.message })?;
        self.add_filtered_options(&mut options, expansions);

        Ok(options)
    }

    // For each option in 'in', add it to 'out' unless it is matched by the 'coptsFilter' regexp.
    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:142 CompileCommandLine.addFilteredOptions
    fn add_filtered_options(
        &self,
        out: &mut Vec<String>,
        expanded_features: Vec<(String, Vec<String>)>,
    ) {
        for (first, second) in expanded_features {
            if first == UNFILTERED_COMPILE_FLAGS_FEATURE_NAME {
                out.extend(second);
                continue;
            }
            for flag in second {
                if self.copts_filter.passes_filter(&flag) {
                    out.push(flag);
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:159 CompileCommandLine.getVariables
    pub fn get_variables(&self) -> &CcToolchainVariablesRef {
        &self.variables
    }

    /// Returns all user provided copts flags.
    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:170 CompileCommandLine.getCopts
    pub fn get_copts(&self, path_mapper: &dyn PathMapper) -> Vec<String> {
        let name = CompileBuildVariables::USER_COMPILE_FLAGS.get_variable_name();
        if self.variables.is_available(name) {
            match CcToolchainVariables::to_string_list(&self.variables, name, path_mapper) {
                Ok(l) => l,
                Err(_) => panic!(
                    "IllegalStateException: Should not happen - 'user_compile_flags' should be a \
                     string list, but wasn't."
                ),
            }
        } else {
            Vec::new()
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:184 CompileCommandLine.builder
    pub fn builder(copts_filter: CoptsFilter, action_name: &str) -> CompileCommandLineBuilder {
        CompileCommandLineBuilder {
            copts_filter,
            feature_configuration: None,
            variables: CcToolchainVariables::empty(),
            action_name: action_name.to_string(),
        }
    }
}

/// A builder for a [`CompileCommandLine`].
// Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:189 CompileCommandLine.Builder
pub struct CompileCommandLineBuilder {
    copts_filter: CoptsFilter,
    feature_configuration: Option<Arc<FeatureConfiguration>>,
    variables: CcToolchainVariablesRef,
    action_name: String,
}

impl CompileCommandLineBuilder {
    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:195 CompileCommandLine.Builder.build
    pub fn build(&self) -> CompileCommandLine {
        CompileCommandLine {
            copts_filter: self.copts_filter.clone(),
            feature_configuration: self
                .feature_configuration
                .clone()
                .unwrap_or_else(|| panic!("NullPointerException")),
            variables: self.variables.clone(),
            action_name: self.action_name.clone(),
        }
    }

    /// Sets the feature configuration for this compile action.
    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:210 CompileCommandLine.Builder.setFeatureConfiguration
    pub fn set_feature_configuration(
        &mut self,
        feature_configuration: Arc<FeatureConfiguration>,
    ) -> &mut Self {
        self.feature_configuration = Some(feature_configuration);
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:216 CompileCommandLine.Builder.setVariables
    pub fn set_variables(&mut self, variables: CcToolchainVariablesRef) -> &mut Self {
        self.variables = variables;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CompileCommandLine.java:223 CompileCommandLine.Builder.setCoptsFilter
    pub fn set_copts_filter(&mut self, filter: CoptsFilter) -> &mut Self {
        self.copts_filter = filter;
        self
    }
}
