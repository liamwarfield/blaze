//! `rules/java/JavaSemantics.java` (pluggable Java compilation semantics) and its Bazel
//! implementation `bazel/rules/java/BazelJavaSemantics.java`; with the file-system helpers of
//! `vfs/FileSystemUtils.java` the Java rules use.

use blaze_analysis::platform::constraint_constants::get_os_from_constraints_or_host;
use blaze_analysis::platform::platform_info::PlatformInfo;
use blaze_cmdline::Label;
use blaze_shell::util::os::OS;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use super::java_util;

/// Transformed by Copybara on export.
// Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:32 JavaSemantics.RULES_JAVA_PROVIDER_LABELS_PREFIX
pub const RULES_JAVA_PROVIDER_LABELS_PREFIX: &str = "@@rules_java+//";

/// Name of the output group used for transitive source jars.
// Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:39 JavaSemantics.SOURCE_JARS_OUTPUT_GROUP
pub const SOURCE_JARS_OUTPUT_GROUP: &str = "_hidden_top_level_INTERNAL_source_jars";

/// Name of the output group used for direct source jars.
// Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:42 JavaSemantics.DIRECT_SOURCE_JARS_OUTPUT_GROUP
pub const DIRECT_SOURCE_JARS_OUTPUT_GROUP: &str = "_hidden_top_level_INTERNAL_direct_source_jars";

/// Pluggable Java compilation semantics.
// Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:29 JavaSemantics
pub trait JavaSemantics: Send + Sync {
    // Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:45 JavaSemantics.getJavaToolchainType
    fn get_java_toolchain_type(&self) -> String;

    // Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:47 JavaSemantics.getJavaRuntimeToolchainType
    fn get_java_runtime_toolchain_type(&self) -> Label;

    /// Takes the path of a Java resource and tries to determine the Java root relative path of
    /// the resource.
    ///
    /// This is only used if the Java rule doesn't have a `resource_strip_prefix` attribute.
    // Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:65 JavaSemantics.getDefaultJavaResourcePath
    fn get_default_java_resource_path(&self, path: &PathFragment) -> PathFragment;

    /// Environment variable that sets the UTF-8 charset for the given execution platform.
    // Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:68 JavaSemantics.utf8Environment
    fn utf8_environment(
        &self,
        execution_platform: Option<&PlatformInfo>,
    ) -> IndexMap<String, String>;
}

/// Semantics for Bazel Java rules.
// Java: com/google/devtools/build/lib/bazel/rules/java/BazelJavaSemantics.java:30 BazelJavaSemantics
pub struct BazelJavaSemantics;

// Java: com/google/devtools/build/lib/bazel/rules/java/BazelJavaSemantics.java:44 BazelJavaSemantics.INSTANCE
pub static BAZEL_JAVA_SEMANTICS: BazelJavaSemantics = BazelJavaSemantics;

/// `Label.parseCanonicalUnchecked("@bazel_tools//tools/jdk:toolchain_type").toString()`.
// Java: com/google/devtools/build/lib/bazel/rules/java/BazelJavaSemantics.java:48 BazelJavaSemantics.JAVA_TOOLCHAIN_TYPE
fn java_toolchain_type() -> String {
    Label::parse_canonical_unchecked("@bazel_tools//tools/jdk:toolchain_type").to_string()
}

impl JavaSemantics for BazelJavaSemantics {
    // Java: com/google/devtools/build/lib/bazel/rules/java/BazelJavaSemantics.java:54 BazelJavaSemantics.getJavaToolchainType
    fn get_java_toolchain_type(&self) -> String {
        java_toolchain_type()
    }

    // Java: com/google/devtools/build/lib/bazel/rules/java/BazelJavaSemantics.java:59 BazelJavaSemantics.getJavaRuntimeToolchainType
    fn get_java_runtime_toolchain_type(&self) -> Label {
        // Java: BazelJavaSemantics.JAVA_RUNITME_TOOLCHAIN_TYPE
        Label::parse_canonical_unchecked("@bazel_tools//tools/jdk:runtime_toolchain_type")
    }

    // Java: com/google/devtools/build/lib/bazel/rules/java/BazelJavaSemantics.java:64 BazelJavaSemantics.getDefaultJavaResourcePath
    fn get_default_java_resource_path(&self, path: &PathFragment) -> PathFragment {
        // Look for src/.../resources to match Maven repository structure.
        let segments = path.split_to_list_of_segments();
        let mut i = 0usize;
        while i + 2 < segments.len() {
            if segments[i] == "src" && segments[i + 2] == "resources" {
                return path.sub_fragment_from(i + 3);
            }
            i += 1;
        }
        let java_path = java_util::get_java_path(path);
        java_path.unwrap_or_else(|| path.clone())
    }

    // Java: com/google/devtools/build/lib/bazel/rules/java/BazelJavaSemantics.java:77 BazelJavaSemantics.utf8Environment
    fn utf8_environment(
        &self,
        execution_platform: Option<&PlatformInfo>,
    ) -> IndexMap<String, String> {
        let mut env = IndexMap::new();
        if get_os_from_constraints_or_host(execution_platform) == OS::Darwin {
            // macOS doesn't have C.UTF-8, so we use en_US.UTF-8 instead.
            // Java: BazelJavaSemantics.MACOS_UTF8_ENVIRONMENT
            env.insert("LC_CTYPE".to_owned(), "en_US.UTF-8".to_owned());
        } else {
            // C.UTF-8 is now the universally accepted standard UTF-8 locale.
            // Java: BazelJavaSemantics.DEFAULT_UTF8_ENVIRONMENT
            env.insert("LC_CTYPE".to_owned(), "C.UTF-8".to_owned());
        }
        env
    }
}

/// `FileSystemUtils.removeExtension(String)`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:128 FileSystemUtils.removeExtension
fn remove_extension(filename: &str) -> &str {
    let Some(last_dot_index) = filename.rfind('.') else {
        return filename;
    };
    if let Some(last_slash_index) = filename.rfind('/')
        && last_slash_index > last_dot_index
    {
        return filename;
    }
    &filename[..last_dot_index]
}

/// `FileSystemUtils.replaceExtension(PathFragment, String)`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:173 FileSystemUtils.replaceExtension
pub fn replace_extension(path: &PathFragment, new_extension: &str) -> PathFragment {
    let new_name = format!(
        "{}{}",
        remove_extension(path.get_base_name()),
        new_extension
    );
    path.replace_name(&new_name)
        .unwrap_or_else(|| panic!("NullPointerException: replaceName of {path}"))
}

/// `FileSystemUtils.appendWithoutExtension(PathFragment, String)`: `toAppend` goes before the
/// last `.` of the base name (`libfoo.jar`, `-src` gives `libfoo-src.jar`).
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:216 FileSystemUtils.appendWithoutExtension
pub fn append_without_extension(path: &PathFragment, to_append: &str) -> PathFragment {
    // Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:230 FileSystemUtils.appendWithoutExtension
    let name = path.get_base_name();
    let new_name = match name.rfind('.') {
        Some(dot_index) if dot_index > 0 => {
            format!("{}{}{}", &name[..dot_index], to_append, &name[dot_index..])
        }
        _ => format!("{name}{to_append}"),
    };
    path.replace_name(&new_name)
        .unwrap_or_else(|| panic!("NullPointerException: replaceName of {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The examples of the Java doc comment of `FileSystemUtils.appendWithoutExtension`.
    #[test]
    fn append_without_extension_doc_examples() {
        assert_eq!(
            append_without_extension(&PathFragment::create("libfoo.jar"), "-src").get_path_string(),
            "libfoo-src.jar"
        );
        assert_eq!(
            append_without_extension(&PathFragment::create("libfoo"), "-src").get_path_string(),
            "libfoo-src"
        );
    }
}
