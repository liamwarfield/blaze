//! Port of `com.google.devtools.build.lib.rules.cpp.LinkCommandLine`.

use std::sync::Arc;

use crate::actions::{
    CommandLineExpansionException, InputMetadataProvider, NOOP, ParamFileInfo, ParameterFileType,
    PathMapper,
};
use crate::cc_toolchain_features::FeatureConfiguration;
use crate::cc_toolchain_variables::{CcToolchainVariables, CcToolchainVariablesRef};
use crate::errors::EvalException;

const LINKER_PARAM_FILE: &str = "linker_param_file";

/// Represents the command line of a linker invocation. It supports executables and dynamic
/// libraries as well as static libraries.
// Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:43 LinkCommandLine
#[derive(Clone, Debug)]
pub struct LinkCommandLine {
    action_name: Option<String>,
    forced_tool_path: Option<String>,
    variables: CcToolchainVariablesRef,
    // The feature config can be null for tests.
    feature_configuration: Option<Arc<FeatureConfiguration>>,
    split_command_line: bool,
    parameter_file_type: ParameterFileType,
}

impl LinkCommandLine {
    fn feature_configuration(&self) -> &FeatureConfiguration {
        self.feature_configuration
            .as_deref()
            .unwrap_or_else(|| panic!("NullPointerException"))
    }

    fn action_name_str(&self) -> &str {
        self.action_name.as_deref().unwrap_or_default()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:70 LinkCommandLine.getActionName
    pub fn get_action_name(&self) -> Option<&str> {
        self.action_name.as_deref()
    }

    /// Returns the path to the linker.
    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:75 LinkCommandLine.getLinkerPathString
    pub fn get_linker_path_string(&self) -> Result<String, EvalException> {
        if let Some(forced_tool_path) = &self.forced_tool_path {
            Ok(forced_tool_path.clone())
        } else {
            if !self
                .feature_configuration()
                .action_is_configured(self.action_name_str())
            {
                return Err(EvalException::new(format!(
                    "Expected action_config for '{}' to be configured",
                    self.action_name_str()
                )));
            }
            Ok(self
                .feature_configuration()
                .get_tool_path_for_action(self.action_name_str()))
        }
    }

    /// Returns the build variables used to template the crosstool for this linker invocation.
    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:88 LinkCommandLine.getBuildVariables
    pub fn get_build_variables(&self) -> &CcToolchainVariablesRef {
        &self.variables
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:92 LinkCommandLine.getParamCommandLine
    pub fn get_param_command_line(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineExpansionException> {
        let wrap = |e: crate::errors::ExpansionException| CommandLineExpansionException {
            message: e.message,
        };
        let mut argv = Vec::new();
        if self.variables.is_available(LINKER_PARAM_FILE) {
            // Filter out linker_param_file
            let linker_param_file = self
                .variables
                .get_variable(LINKER_PARAM_FILE, path_mapper)
                .and_then(|v| v.get_string_value(LINKER_PARAM_FILE, path_mapper))
                .map_err(wrap)?;
            argv.extend(
                self.feature_configuration()
                    .get_command_line_with(
                        self.action_name_str(),
                        &self.variables,
                        input_metadata_provider,
                        path_mapper,
                    )
                    .map_err(wrap)?
                    .into_iter()
                    .filter(|s| !s.contains(&linker_param_file)),
            );
        } else {
            argv.extend(
                self.feature_configuration()
                    .get_command_line_with(
                        self.action_name_str(),
                        &self.variables,
                        input_metadata_provider,
                        path_mapper,
                    )
                    .map_err(wrap)?,
            );
        }
        Ok(argv)
    }

    /// Java: `getCommandLines()`: the linker path followed by this command line with
    /// [`LinkCommandLine::get_param_file_info`].
    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:120 LinkCommandLine.getCommandLines
    pub fn get_command_lines(&self) -> Result<(String, Option<ParamFileInfo>), EvalException> {
        Ok((self.get_linker_path_string()?, self.get_param_file_info()?))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:128 LinkCommandLine.getParamFileInfo
    pub fn get_param_file_info(&self) -> Result<Option<ParamFileInfo>, EvalException> {
        let mut param_file_info = None;
        if self.split_command_line {
            let command_line = self
                .feature_configuration()
                .get_command_line_with(self.action_name_str(), &self.variables, None, &NOOP)
                .map_err(EvalException::from)?;
            // PORT: Java's findAny on a sequential stream returns the first match.
            let format_string = command_line
                .iter()
                .find(|s| s.contains("LINKER_PARAM_FILE_PLACEHOLDER"));
            if let Some(format_string) = format_string {
                param_file_info = Some(ParamFileInfo {
                    file_type: self.parameter_file_type,
                    flag_format_string: format_string
                        .replace('%', "%%")
                        .replace("LINKER_PARAM_FILE_PLACEHOLDER", "%s"),
                    use_always: true,
                });
            }
        }
        Ok(param_file_info)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:157 LinkCommandLine.arguments
    pub fn arguments(&self) -> Result<Vec<String>, CommandLineExpansionException> {
        self.arguments_with(None, &NOOP)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:162 LinkCommandLine.arguments
    pub fn arguments_with(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineExpansionException> {
        self.get_param_command_line(input_metadata_provider, path_mapper)
    }
}

/// A builder for a [`LinkCommandLine`].
// Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:168 LinkCommandLine.Builder
pub struct LinkCommandLineBuilder {
    forced_tool_path: Option<String>,
    split_command_line: bool,
    parameter_file_type: ParameterFileType,
    variables: Option<CcToolchainVariablesRef>,
    feature_configuration: Option<Arc<FeatureConfiguration>>,
    action_name: Option<String>,
}

impl Default for LinkCommandLineBuilder {
    fn default() -> Self {
        LinkCommandLineBuilder {
            forced_tool_path: None,
            split_command_line: false,
            parameter_file_type: ParameterFileType::UNQUOTED,
            variables: None,
            feature_configuration: None,
            action_name: None,
        }
    }
}

impl LinkCommandLineBuilder {
    pub fn new() -> LinkCommandLineBuilder {
        LinkCommandLineBuilder::default()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:177 LinkCommandLine.Builder.build
    pub fn build(&mut self) -> LinkCommandLine {
        if self.variables.is_none() {
            self.variables = Some(CcToolchainVariables::empty());
        }

        LinkCommandLine {
            action_name: self.action_name.clone(),
            forced_tool_path: self.forced_tool_path.clone(),
            split_command_line: self.split_command_line,
            parameter_file_type: self.parameter_file_type,
            variables: self.variables.clone().unwrap(),
            feature_configuration: self.feature_configuration.clone(),
        }
    }

    /// Use given tool path instead of the one from feature configuration.
    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:193 LinkCommandLine.Builder.forceToolPath
    pub fn force_tool_path(&mut self, forced_tool_path: &str) -> &mut Self {
        self.forced_tool_path = Some(forced_tool_path.to_string());
        self
    }

    /// Sets the feature configuration for this link action.
    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:200 LinkCommandLine.Builder.setFeatureConfiguration
    pub fn set_feature_configuration(
        &mut self,
        feature_configuration: Arc<FeatureConfiguration>,
    ) -> &mut Self {
        self.feature_configuration = Some(feature_configuration);
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:206 LinkCommandLine.Builder.setSplitCommandLine
    pub fn set_split_command_line(&mut self, split_command_line: bool) -> &mut Self {
        self.split_command_line = split_command_line;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:212 LinkCommandLine.Builder.setParameterFileType
    pub fn set_parameter_file_type(&mut self, parameter_file_type: ParameterFileType) -> &mut Self {
        self.parameter_file_type = parameter_file_type;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:218 LinkCommandLine.Builder.setBuildVariables
    pub fn set_build_variables(&mut self, variables: CcToolchainVariablesRef) -> &mut Self {
        self.variables = Some(variables);
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:224 LinkCommandLine.Builder.setActionName
    pub fn set_action_name(&mut self, action_name: &str) -> &mut Self {
        self.action_name = Some(action_name.to_string());
        self
    }
}
