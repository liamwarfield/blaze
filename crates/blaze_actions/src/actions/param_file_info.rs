//! Port of `lib/actions/ParamFileInfo.java`.

use super::parameter_file::ParameterFileType;

/// An object that encapsulates how a params file should be constructed: what is the filetype,
/// what charset to use and what prefix (typically "@") to use.
// Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:30 ParamFileInfo
// PORT: Java interns instances (`BlazeInterners.newWeakInterner`); values are compared by
// `equals`, which the derive reproduces. Java's `hashCode` omits `flagsOnly`; the derived hash
// includes it, which is consistent with `equals`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParamFileInfo {
    file_type: ParameterFileType,
    flag_format_string: String,
    always: bool,
    flags_only: bool,
}

impl ParamFileInfo {
    /// Returns the file type.
    // Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:47 ParamFileInfo.getFileType
    pub fn get_file_type(&self) -> ParameterFileType {
        self.file_type
    }

    /// Returns the format string for the params filename on the command line (typically "@%s").
    // Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:52 ParamFileInfo.getFlagFormatString
    pub fn get_flag_format_string(&self) -> &str {
        &self.flag_format_string
    }

    /// Returns true if a params file should always be used.
    // Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:57 ParamFileInfo.always
    pub fn always(&self) -> bool {
        self.always
    }

    /// If true, only the flags will be spilled to the file, leaving positional args on the
    /// command line.
    // Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:65 ParamFileInfo.flagsOnly
    pub fn flags_only(&self) -> bool {
        self.flags_only
    }

    // Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:88 ParamFileInfo.builder
    pub fn builder(parameter_file_type: ParameterFileType) -> ParamFileInfoBuilder {
        ParamFileInfoBuilder {
            file_type: parameter_file_type,
            flag_format_string: "@%s".to_string(),
            always: false,
            flags_only: false,
        }
    }
}

/// Builder for a ParamFileInfo.
// Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:93 ParamFileInfo.Builder
#[derive(Clone, Debug)]
pub struct ParamFileInfoBuilder {
    file_type: ParameterFileType,
    flag_format_string: String,
    always: bool,
    flags_only: bool,
}

impl ParamFileInfoBuilder {
    /// Sets a format string to use for the flag that is passed to original command.
    ///
    /// The format string must have a single "%s" that will be replaced by the execution path to
    /// the param file.
    // Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:110 ParamFileInfo.Builder.setFlagFormatString
    pub fn set_flag_format_string(&mut self, flag_format_string: impl Into<String>) -> &mut Self {
        self.flag_format_string = flag_format_string.into();
        self
    }

    /// Set whether the parameter file is always used, regardless of parameter file length.
    // Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:117 ParamFileInfo.Builder.setUseAlways
    pub fn set_use_always(&mut self, always: bool) -> &mut Self {
        self.always = always;
        self
    }

    /// If true, only the flags will be spilled to the file, leaving positional args on the
    /// command line. (Default is false.)
    // Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:127 ParamFileInfo.Builder.setFlagsOnly
    pub fn set_flags_only(&mut self, flags_only: bool) -> &mut Self {
        self.flags_only = flags_only;
        self
    }

    // Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:132 ParamFileInfo.Builder.build
    pub fn build(&self) -> ParamFileInfo {
        ParamFileInfo {
            file_type: self.file_type,
            flag_format_string: self.flag_format_string.clone(),
            always: self.always,
            flags_only: self.flags_only,
        }
    }
}
