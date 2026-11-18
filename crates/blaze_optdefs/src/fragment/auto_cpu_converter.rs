//! Port of `analysis/config/AutoCpuConverter.java`.

use blaze_options::{
    ConversionContext, Converter, OptionsParsingException, Value, converter::non_null,
};

use super::converters::ConvResult;

// Java: com/google/devtools/build/lib/analysis/config/AutoCpuConverter.java:28 AutoCpuConverter
/// Converter to auto-detect the cpu of the machine on which Bazel runs.
///
/// If the compilation happens remotely then the cpu of the remote machine might be different
/// from the auto-detected one and the --cpu and --host_cpu options must be set explicitly.
#[derive(Clone, Copy, Debug, Default)]
pub struct AutoCpuConverter;

impl AutoCpuConverter {
    // Java: com/google/devtools/build/lib/analysis/config/AutoCpuConverter.java:30 AutoCpuConverter.convert
    pub fn convert_cpu(&self, input: &str) -> Result<String, OptionsParsingException> {
        if input.is_empty() {
            // TODO(philwo) - replace these deprecated names with more logical ones (e.g. k8
            // becomes linux-x86_64, darwin includes the CPU architecture, ...).
            // PORT: OS.getCurrent()/CPU.getCurrent() are resolved at compile time; Blaze targets
            // Linux only (macOS/Windows/BSD arms skipped).
            let cpu = if cfg!(target_os = "linux") {
                if cfg!(target_arch = "x86") {
                    "piii"
                } else if cfg!(target_arch = "x86_64") {
                    "k8"
                } else if cfg!(target_arch = "powerpc") || cfg!(target_arch = "powerpc64") {
                    "ppc"
                } else if cfg!(target_arch = "arm") {
                    "arm"
                } else if cfg!(target_arch = "aarch64") {
                    "aarch64"
                } else if cfg!(target_arch = "s390x") {
                    "s390x"
                } else if cfg!(target_arch = "mips64") {
                    "mips64"
                } else if cfg!(target_arch = "riscv64") {
                    "riscv64"
                } else {
                    "unknown"
                }
            } else {
                blaze_util::unported!("AutoCpuConverter.convert (non-Linux host)")
            };
            return Ok(cpu.to_string());
        }
        Ok(input.to_string())
    }
}

impl Converter for AutoCpuConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_cpu(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "a string".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_detects_host_cpu() {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(AutoCpuConverter.convert_cpu("").unwrap(), "k8");
        assert_eq!(AutoCpuConverter.convert_cpu("armeabi").unwrap(), "armeabi");
    }
}
