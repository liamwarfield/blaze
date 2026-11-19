//! `blaze license`.

use std::ffi::OsStr;
use std::path::PathBuf;

use blaze_events::OutErr;
use blaze_util::string_encoding::{internal_to_platform, platform_to_internal};

use crate::blaze_command_utils::ResourceFileLoader;
use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::options_result::OptionsParsingResult;

/// The package of `LicenseCommand` (its resource `license/LICENSE`).
const LICENSE_COMMAND_PACKAGE: &str = "com/google/devtools/build/lib/runtime/commands";

// Java: com/google/devtools/build/lib/runtime/commands/LicenseCommand.java:46 LicenseCommand.JAVA_LICENSE_FILES
const JAVA_LICENSE_FILES: &[&str] = &[
    "ASSEMBLY_EXCEPTION",
    "DISCLAIMER",
    "LICENSE",
    "THIRD_PARTY_README",
];

// Java: com/google/devtools/build/lib/runtime/commands/LicenseCommand.java:49 LicenseCommand.BAZEL_LICENSE
const BAZEL_LICENSE: &str = "license/LICENSE";

/// A command that prints an embedded license text.
///
/// PORT: the license text is the server jar's resource `license/LICENSE`, embedded at build time
/// from the oracle's `A-server.jar` (`build.rs`); the bundled JDK's license files are read from
/// the install base, which Blaze extracts from Bazel's (`crates/blaze/build.rs`).
// Java: com/google/devtools/build/lib/runtime/commands/LicenseCommand.java:43 LicenseCommand
pub struct LicenseCommand;

impl LicenseCommand {
    // Java: com/google/devtools/build/lib/runtime/commands/LicenseCommand.java:51 LicenseCommand.isSupported
    pub fn is_supported() -> bool {
        ResourceFileLoader::resource_exists(LICENSE_COMMAND_PACKAGE, BAZEL_LICENSE)
    }

    // Java: com/google/devtools/build/lib/runtime/commands/LicenseCommand.java:96 LicenseCommand.printJavaLicenseFiles
    fn print_java_license_files(out_err: &OutErr, bundled_jdk_or_jre: &std::path::Path) {
        if let Err(e) = Self::walk_file_tree(out_err, bundled_jdk_or_jre) {
            panic!(
                "UncheckedIOException: I/O error while trying to print license file of bundled \
                 JDK or JRE: {}",
                e
            );
        }
    }

    /// `Files.walkFileTree` (no symbolic links followed) with the visitor of
    /// `printJavaLicenseFiles`: depth first, each directory's entries in the order the file
    /// system lists them (`DirectoryStream`, i.e. readdir order).
    fn walk_file_tree(out_err: &OutErr, path: &std::path::Path) -> std::io::Result<()> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir() {
            // visitFile
            let file_name = path
                .file_name()
                .map(platform_to_internal)
                .unwrap_or_default();
            if JAVA_LICENSE_FILES.contains(&file_name.as_ref()) {
                let path_string = platform_to_internal(path.as_os_str()).into_owned();
                out_err.print_out_ln(&format!("{}:\n", path_string));
                let bytes = std::fs::read(path)?;
                let _ = out_err.get_output_stream().write(&bytes);
                out_err.print_out_ln("\n");
            }
            return Ok(());
        }
        for entry in std::fs::read_dir(path)? {
            Self::walk_file_tree(out_err, &entry?.path())?;
        }
        Ok(())
    }
}

impl BlazeCommand for LicenseCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::LICENSE_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/LicenseCommand.java:56 LicenseCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        _options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        // PORT: `env.getEventBus().post(new NoBuildEvent())` only reaches the Build Event
        // Protocol, which is ported for build and test; a BEP file requested for this command
        // fails loudly before it runs (crate::unported_options).
        let out_err = env.get_reporter().get_out_err().clone();

        out_err.print_out_ln("Licenses of all components included in this binary:\n");

        match ResourceFileLoader::load_resource(LICENSE_COMMAND_PACKAGE, BAZEL_LICENSE) {
            Ok(text) => out_err.print_out_ln(&text),
            Err(e) => panic!(
                "IllegalStateException: I/O error while trying to print 'LICENSE' resource: {}",
                e.0
            ),
        }

        let embedded_binaries_root = env.get_directories().get_embedded_binaries_root().clone();
        let platform = |relative: &str| -> PathBuf {
            let path = embedded_binaries_root.get_relative_str(relative);
            PathBuf::from(OsStr::new(&*internal_to_platform(path.get_path_string())))
        };

        let bundled_jdk = platform("embedded_tools/jdk");
        if bundled_jdk.exists() {
            out_err.print_out_ln(
                "This binary comes with a bundled JDK, which contains the following license \
                 files:\n",
            );
            Self::print_java_license_files(&out_err, &bundled_jdk);
        }

        let bundled_jre = platform("embedded_tools/jre");
        if bundled_jre.exists() {
            out_err.print_out_ln(
                "This binary comes with a bundled JRE, which contains the following license \
                 files:\n",
            );
            Self::print_java_license_files(&out_err, &bundled_jre);
        }

        BlazeCommandResult::success()
    }
}
