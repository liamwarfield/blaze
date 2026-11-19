//! Port of `bazel/repository/RepositoryOptions.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::Foreign;
use blaze_vfs::PathFragment;
use std::time::Duration;

use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::is_valid_module_name;
use blaze_options::{
    ConversionContext, Converter, DurationConverter, OptionScalar, OptionsParsingException,
    StringToStringListConverter, Value, bool_or_enum_converter, converter::non_null,
    enum_converter, option_enum, option_field_type, options,
};

use super::options_utils::PathFragmentConverter;

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:38 RepositoryOptions
options! {
    /// Command-line options for repositories.
    pub struct RepositoryOptions {
        #[option(
            name = "repository_cache",
            old_name = "experimental_repository_cache",
            default_value = "null",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            converter = PathFragmentConverter,
            help = concat!(
                "Specifies the cache location of the downloaded values obtained\n",
                "during the fetching of external repositories. An empty string\n",
                "as argument requests the cache to be disabled,\n",
                "otherwise the default of `{--output_user_root}/cache/repos/v1` is used.\n",
            ),
        )]
        pub repository_cache: Option<Foreign<PathFragment>>,

        #[option(
            name = "repo_contents_cache",
            old_name = "repository_contents_cache",
            old_name_warning = false,
            default_value = "null",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            converter = PathFragmentConverter,
            help = concat!(
                "Specifies the location of the repo contents cache, which contains fetched repo\n",
                "directories shareable across workspaces. An empty string as argument requests the repo\n",
                "contents cache to be disabled, otherwise the default of `{--repository_cache}/contents`\n",
                "is used. Note that this means setting `--repository_cache=` would by default disable the\n",
                "repo contents cache as well, unless `--repo_contents_cache={some_path}` is also set.\n",
            ),
        )]
        pub repo_contents_cache: Option<Foreign<PathFragment>>,

        #[option(
            name = "repo_contents_cache_gc_max_age",
            default_value = "14d",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            converter = DurationConverter,
            help = concat!(
                "Specifies the amount of time an entry in the repo contents cache can stay unused before\n",
                "it's garbage collected. If set to zero, only duplicate entries will be garbage collected.\n",
            ),
        )]
        pub repo_contents_cache_gc_max_age: Duration,

        #[option(
            name = "repo_contents_cache_gc_idle_delay",
            default_value = "5m",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            converter = DurationConverter,
            help = concat!(
                "Specifies the amount of time the server must remain idle before garbage collection happens\n",
                "to the repo contents cache.\n",
            ),
        )]
        pub repo_contents_cache_gc_idle_delay: Duration,

        #[option(
            name = "registry",
            default_value = "null",
            allow_multiple = true,
            documentation_category = BZLMOD,
            effect_tags = [CHANGES_INPUTS],
            help = concat!(
                "Specifies the registries to use to locate Bazel module dependencies. The order is",
                " important: modules will be looked up in earlier registries first, and only fall",
                " back to later registries when they're missing from the earlier ones.",
            ),
        )]
        pub registries: Vec<String>,

        #[option(
            name = "module_mirrors",
            default_value = "null",
            converter = StringToStringListConverter,
            allow_multiple = true,
            documentation_category = BZLMOD,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Specifies URLs under which the source URLs of Bazel modules can be found, in addition\n",
                "to and taking precedence over any registry-provided mirror URLs. This flag can be\n",
                "specified per-registry using the syntax\n",
                "`--module_mirrors=<registry>=<mirror1>[,<mirror2>,...]` (e.g.,\n",
                "`--module_mirrors=https://bcr.bazel.build=https://mirror.example.com`). It can also\n",
                "be specified as a comma-separated list of mirror URLs that applies to all registries that\n",
                "don't have an explicit list (e.g., `--module_mirrors=https://mirror1,https://mirror2`).\n",
                "Set this to an empty value to disable the use of any mirrors not specified by the\n",
                "registries. Later uses of this flag override earlier ones with the same (or no) registry.\n",
                "The default set of mirrors may change over time, but all downloads from mirrors are\n",
                "verified by hashes stored in the registry (and thus pinned by the lockfile).\n",
            ),
        )]
        pub module_mirrors: Vec<(String, Vec<String>)>,

        #[option(
            name = "allow_yanked_versions",
            default_value = "null",
            allow_multiple = true,
            documentation_category = BZLMOD,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Specified the module versions in the form of\n",
                "`{module1}@{version1},module2@{version2}` that will be allowed in the resolved\n",
                "dependency graph even if they are declared yanked in the registry where they come\n",
                "from (if they are not coming from a [`NonRegistryOverride`]).\n",
                "Otherwise, yanked versions will cause the resolution to fail. You can also define allowed\n",
                "yanked versions with the `BZLMOD_ALLOW_YANKED_VERSIONS` environment variable. You can\n",
                "disable this check by using the keyword `all` (not recommended).\n",
                "\n",
                "[`NonRegistryOverride`]: https://github.com/bazelbuild/bazel/blob/master/src/main/java/com/google/devtools/build/lib/bazel/bzlmod/NonRegistryOverride.java\n",
            ),
        )]
        pub allowed_yanked_versions: Vec<String>,

        #[option(
            name = "experimental_repository_cache_hardlinks",
            default_value = "false",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "If set, the repository cache will hardlink the file in case of a",
                " cache hit, rather than copying. This is intended to save disk space.",
            ),
        )]
        pub use_hardlinks: bool,

        #[option(
            name = "repository_disable_download",
            old_name = "experimental_repository_disable_download",
            default_value = "false",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "If set, downloading using `ctx.download{,_and_extract}` is not allowed during repository\n",
                "fetching. Note that network access is not completely disabled; ctx.execute could\n",
                "still run an arbitrary executable that accesses the Internet.\n",
            ),
        )]
        pub disable_download: bool,

        #[option(
            name = "experimental_repository_downloader_retries",
            default_value = "5",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [UNKNOWN],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "The maximum number of attempts to retry a download error. If set to 0, retries are",
                " disabled.",
            ),
        )]
        pub repository_downloader_retries: i32,

        #[option(
            name = "distdir",
            old_name = "experimental_distdir",
            default_value = "null",
            allow_multiple = true,
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            converter = PathFragmentConverter,
            help = concat!(
                "Additional places to search for archives before accessing the network ",
                "to download them.",
            ),
        )]
        pub experimental_distdir: Vec<Foreign<PathFragment>>,

        #[option(
            name = "override_repository",
            default_value = "null",
            allow_multiple = true,
            converter = RepositoryOverrideConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Override a repository with a local path in the form of `{repository name}={path}`,\n",
                "where the repository name can be either a canonical name or an apparent name from the\n",
                "point of view of the main repository.\n",
                "\n",
                "Note that if this is flag is used to override a module's repository, changes to the\n",
                "MODULE.bazel file will not be effective if the module is obtained from a registry. Use\n",
                "`--override_module` instead to for that purpose.\n",
                "\n",
                "If the given path is an absolute path, it will be used as it is. If the given path is a\n",
                "relative path, it is relative to the current working directory. If the given path starts\n",
                "with `%workspace%`, it is relative to the workspace root, which is the output of `bazel\n",
                "info workspace`. If the given path is empty, then remove any previous overrides.\n",
            ),
        )]
        pub repository_overrides: Vec<RepositoryOverride>,

        #[option(
            name = "inject_repository",
            default_value = "null",
            allow_multiple = true,
            converter = RepositoryInjectionConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Adds a new repository with a local path in the form of `{repository name}={path}`. This\n",
                "only takes effect with `--enable_bzlmod` and is equivalent to adding a\n",
                "corresponding `local_repository` to the root module's `MODULE.bazel` file via\n",
                "`use_repo_rule`. If the given path is an absolute path, it will be used as it is.\n",
                "If the given path is a relative path, it is relative to the current working\n",
                "directory. If the given path starts with `%workspace%`, it is relative to the\n",
                "workspace root, which is the output of `bazel info workspace`. If the given path\n",
                "is empty, then remove any previous injections.\n",
            ),
        )]
        pub repository_injections: Vec<RepositoryInjection>,

        #[option(
            name = "override_module",
            default_value = "null",
            allow_multiple = true,
            converter = ModuleOverrideConverter,
            documentation_category = BZLMOD,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Override a module with a local path in the form of `{module name}={path}`. If the given\n",
                "path is an absolute path, it will be used as it is. If the given path is a\n",
                "relative path, it is relative to the current working directory. If the given path\n",
                "starts with `%workspace%`, it is relative to the workspace root, which is the\n",
                "output of `bazel info workspace`. If the given path is empty, then remove any\n",
                "previous overrides.\n",
            ),
        )]
        pub module_overrides: Vec<ModuleOverride>,

        #[option(
            name = "experimental_scale_timeouts",
            default_value = "1.0",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Scale all timeouts in Starlark repository rules by this factor.",
                " In this way, external repositories can be made working on machines",
                " that are slower than the rule author expected, without changing the",
                " source code",
            ),
        )]
        pub experimental_scale_timeouts: f64,

        #[option(
            name = "downloader_config",
            old_name = "experimental_downloader_config",
            default_value = "null",
            documentation_category = REMOTE,
            effect_tags = [UNKNOWN],
            converter = PathFragmentConverter,
            help = concat!(
                "Specify a file to configure the remote downloader with. This file consists of lines, ",
                "each of which starts with a directive (`allow`, `block` or `rewrite`) followed ",
                "by either a host name (for `allow` and `block`) or two patterns, one to match ",
                "against, and one to use as a substitute URL, with back-references starting from ",
                "`$1`. It is possible for multiple `rewrite` directives for the same URL to be ",
                "given, and in this case multiple URLs will be returned.",
            ),
        )]
        pub downloader_config: Option<Foreign<PathFragment>>,

        #[option(
            name = "ignore_dev_dependency",
            default_value = "false",
            documentation_category = BZLMOD,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "If true, Bazel ignores `bazel_dep`, `use_extension`, and `include` declared as\n",
                "`dev_dependency` in the `MODULE.bazel` of the root module. Note that these dev\n",
                "dependencies are always ignored in the `MODULE.bazel` if it is not the root module,\n",
                "regardless of the value of this flag.\n",
            ),
        )]
        pub ignore_dev_dependency: bool,

        #[option(
            name = "check_direct_dependencies",
            default_value = "warning",
            converter = CheckDirectDepsModeConverter,
            documentation_category = BZLMOD,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Check if the direct `bazel_dep` dependencies declared in the root module are the same",
                " versions you get in the resolved dependency graph. Valid values are `off` to",
                " disable the check, `warning` to print a warning when mismatch detected or `error`",
                " to escalate it to a resolution failure.",
            ),
        )]
        pub check_direct_dependencies: CheckDirectDepsMode,

        #[option(
            name = "experimental_check_external_repository_files",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Check for modifications to files in external repositories. Consider setting ",
                "this flag to false if you don't expect these files to change outside of bazel ",
                "since it will speed up subsequent runs as they won't have to check a ",
                "previous run's cache.",
            ),
        )]
        pub check_external_repository_files: bool,

        #[option(
            name = "check_bazel_compatibility",
            default_value = "error",
            converter = BazelCompatibilityModeConverter,
            documentation_category = BZLMOD,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Check bazel version compatibility of Bazel modules. Valid values are `error` to escalate",
                " it to a resolution failure, `off` to disable the check, or `warning` to print a",
                " warning when mismatch detected.",
            ),
        )]
        pub bazel_compatibility_mode: BazelCompatibilityMode,

        #[option(
            name = "incompatible_require_repo_extension_metadata",
            default_value = "false",
            converter = RequireRepoExtensionMetadataModeConverter,
            documentation_category = BZLMOD,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "Require repository rules and module extensions to return explicit metadata. Valid values",
                " are `false` to disable the check, `all` to require metadata everywhere, or `root`",
                " to require metadata only for repo rules and extensions defined in the root",
                " module.",
            ),
        )]
        pub require_repo_extension_metadata: RequireRepoExtensionMetadataMode,

        #[option(
            name = "lockfile_mode",
            converter = LockfileModeConverter,
            default_value = "update",
            documentation_category = BZLMOD,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Specifies how and whether or not to use the lockfile. Valid values are `update` to",
                " use the lockfile and update it if there are changes, `refresh` to additionally",
                " refresh mutable information (yanked versions and previously missing modules)",
                " from remote registries from time to time, `error` to use the lockfile but throw",
                " an error if it's not up-to-date, or `off` to neither read from or write to the",
                " lockfile.",
            ),
        )]
        pub lockfile_mode: LockfileMode,

        #[option(
            name = "vendor_dir",
            default_value = "null",
            converter = PathFragmentConverter,
            documentation_category = BZLMOD,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Specifies the directory that should hold the external repositories in vendor mode, ",
                "whether for the purpose of fetching them into it or using them while building. ",
                "The path can be specified as either an absolute path or a path relative to the ",
                "workspace directory.",
            ),
        )]
        pub vendor_directory: Option<Foreign<PathFragment>>,
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:388 RepositoryOptions.CheckDirectDepsMode
option_enum! {
    /// An enum for specifying different modes for checking direct dependency accuracy.
    pub enum CheckDirectDepsMode {
        OFF, // Don't check direct dependency accuracy.
        WARNING, // Print warning when mismatch.
        ERROR, // Throw an error when mismatch.
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:394 RepositoryOptions.CheckDirectDepsMode.Converter
enum_converter!(
    pub CheckDirectDepsModeConverter,
    CheckDirectDepsMode,
    "direct deps check mode"
);

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:402 RepositoryOptions.BazelCompatibilityMode
option_enum! {
    /// An enum for specifying different modes for bazel compatibility check.
    pub enum BazelCompatibilityMode {
        ERROR, // Check and throw an error when mismatched.
        WARNING, // Print warning when mismatched.
        OFF, // Don't check bazel version compatibility.
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:408 RepositoryOptions.BazelCompatibilityMode.Converter
enum_converter!(
    pub BazelCompatibilityModeConverter,
    BazelCompatibilityMode,
    "Bazel compatibility check mode"
);

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:416 RepositoryOptions.RequireRepoExtensionMetadataMode
option_enum! {
    /// An enum for specifying when repo and extension metadata is required.
    pub enum RequireRepoExtensionMetadataMode {
        FALSE = "false", // Don't require metadata.
        ALL = "all", // Require metadata everywhere.
        ROOT = "root", // Require metadata for repo rules and extensions defined in the root module.
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:431 RepositoryOptions.RequireRepoExtensionMetadataMode.Converter
bool_or_enum_converter!(
    pub RequireRepoExtensionMetadataModeConverter,
    RequireRepoExtensionMetadataMode,
    "repo extension metadata mode",
    RequireRepoExtensionMetadataMode::ALL,
    RequireRepoExtensionMetadataMode::FALSE
);

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:443 RepositoryOptions.LockfileMode
option_enum! {
    /// An enum for specifying how to use the lockfile.
    pub enum LockfileMode {
        OFF, // Don't use the lockfile at all.
        UPDATE, // Update the lockfile wh
        REFRESH,
        ERROR, // Throw an error when it mismatc
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:450 RepositoryOptions.LockfileMode.Converter
enum_converter!(pub LockfileModeConverter, LockfileMode, "Lockfile mode");

/// Java `input.split("=", 2)`: at most two pieces; a missing '=' gives one piece.
fn split_once_eq(input: &str) -> Option<(&str, &str)> {
    input.split_once('=')
}

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:460 RepositoryOptions.RepositoryOverrideConverter
/// Converts from an equals-separated pair of strings into RepositoryName->PathFragment mapping.
#[derive(Clone, Copy, Debug, Default)]
pub struct RepositoryOverrideConverter;

impl Converter for RepositoryOverrideConverter {
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:464 RepositoryOptions.RepositoryOverrideConverter.convert
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let input = non_null(input);
        let Some((name, path)) = split_once_eq(input) else {
            return Err(OptionsParsingException::with_argument(
                "Repository overrides must be of the form 'repository-name=path'",
                input,
            ));
        };
        let path_string = PathFragmentConverter
            .convert_path(path)
            .get_path_string()
            .to_string();
        let valid = if RepositoryName::is_apparent(name) {
            RepositoryName::validate_user_provided_repo_name(name).is_ok()
        } else {
            RepositoryName::create(name).is_ok()
        };
        if !valid {
            // PORT: Java also attaches the LabelSyntaxException/EvalException as the cause;
            // OptionsParsingException has no (message, argument, cause) constructor yet.
            return Err(OptionsParsingException::with_argument(
                "Invalid repository name given to override",
                input,
            ));
        }
        Ok(Some(Value::new(RepositoryOverride {
            repository_name: name.to_string(),
            path: path_string,
        })))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:485 RepositoryOptions.RepositoryOverrideConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "an equals-separated mapping of repository name to path".into()
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:493 RepositoryOptions.RepositoryInjectionConverter
/// Converts from an equals-separated pair of strings into RepositoryName->PathFragment mapping.
#[derive(Clone, Copy, Debug, Default)]
pub struct RepositoryInjectionConverter;

impl Converter for RepositoryInjectionConverter {
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:497 RepositoryOptions.RepositoryInjectionConverter.convert
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let input = non_null(input);
        let Some((name, path)) = split_once_eq(input) else {
            return Err(OptionsParsingException::with_argument(
                "Repository injections must be of the form 'repository-name=path'",
                input,
            ));
        };
        let path_string = PathFragmentConverter
            .convert_path(path)
            .get_path_string()
            .to_string();
        if RepositoryName::validate_user_provided_repo_name(name).is_err() {
            // PORT: Java also attaches the EvalException as the cause (see above).
            return Err(OptionsParsingException::with_argument(
                "Invalid repository name given to inject",
                input,
            ));
        }
        Ok(Some(Value::new(RepositoryInjection {
            apparent_name: name.to_string(),
            path: path_string,
        })))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:514 RepositoryOptions.RepositoryInjectionConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "an equals-separated mapping of repository name to path".into()
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:520 RepositoryOptions.ModuleOverrideConverter
/// Converts from an equals-separated pair of strings into ModuleName->PathFragment mapping.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModuleOverrideConverter;

impl Converter for ModuleOverrideConverter {
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:523 RepositoryOptions.ModuleOverrideConverter.convert
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let input = non_null(input);
        let Some((name, path)) = split_once_eq(input) else {
            return Err(OptionsParsingException::with_argument(
                "Module overrides must be of the form 'module-name=path'",
                input,
            ));
        };

        if !is_valid_module_name(name) {
            return Err(OptionsParsingException::new(format!(
                "invalid module name '{}': valid names must 1) only contain lowercase letters \
                 (a-z), digits (0-9), dots (.), hyphens (-), and underscores (_); 2) begin with a \
                 lowercase letter; 3) end with a lowercase letter or digit.",
                name
            )));
        }

        let path_string = PathFragmentConverter
            .convert_path(path)
            .get_path_string()
            .to_string();
        Ok(Some(Value::new(ModuleOverride {
            module_name: name.to_string(),
            path: path_string,
        })))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:545 RepositoryOptions.ModuleOverrideConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "an equals-separated mapping of module name to path".into()
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:551 RepositoryOptions.RepositoryOverride
/// A repository override, represented by a name and an absolute path to a repository.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RepositoryOverride {
    pub repository_name: String,
    pub path: String,
}

impl OptionScalar for RepositoryOverride {
    fn java_to_string(&self) -> String {
        format!(
            "RepositoryOverride[repositoryName={}, path={}]",
            self.repository_name, self.path
        )
    }
}

option_field_type!(
    RepositoryOverride,
    "com.google.devtools.build.lib.bazel.repository.RepositoryOptions$RepositoryOverride",
    raw = RepositoryOverride {
        repository_name: String::new(),
        path: String::new()
    }
);

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:557 RepositoryOptions.RepositoryInjection
/// A repository injected into the scope of the root module, represented by a name and an
/// absolute path to a repository.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RepositoryInjection {
    pub apparent_name: String,
    pub path: String,
}

impl OptionScalar for RepositoryInjection {
    fn java_to_string(&self) -> String {
        format!(
            "RepositoryInjection[apparentName={}, path={}]",
            self.apparent_name, self.path
        )
    }
}

option_field_type!(
    RepositoryInjection,
    "com.google.devtools.build.lib.bazel.repository.RepositoryOptions$RepositoryInjection",
    raw = RepositoryInjection {
        apparent_name: String::new(),
        path: String::new()
    }
);

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:560 RepositoryOptions.ModuleOverride
/// A module override, represented by a name and an absolute path to a module.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModuleOverride {
    pub module_name: String,
    pub path: String,
}

impl OptionScalar for ModuleOverride {
    fn java_to_string(&self) -> String {
        format!(
            "ModuleOverride[moduleName={}, path={}]",
            self.module_name, self.path
        )
    }
}

option_field_type!(
    ModuleOverride,
    "com.google.devtools.build.lib.bazel.repository.RepositoryOptions$ModuleOverride",
    raw = ModuleOverride {
        module_name: String::new(),
        path: String::new()
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<RepositoryOptions>(&[
            "repository_cache",
            "repo_contents_cache",
            "repo_contents_cache_gc_max_age",
            "repo_contents_cache_gc_idle_delay",
            "registry",
            "module_mirrors",
            "allow_yanked_versions",
            "experimental_repository_cache_hardlinks",
            "repository_disable_download",
            "experimental_repository_downloader_retries",
            "distdir",
            "override_repository",
            "inject_repository",
            "override_module",
            "experimental_scale_timeouts",
            "downloader_config",
            "ignore_dev_dependency",
            "check_direct_dependencies",
            "experimental_check_external_repository_files",
            "check_bazel_compatibility",
            "incompatible_require_repo_extension_metadata",
            "lockfile_mode",
            "vendor_dir",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<RepositoryOptions>(&[
            "--override_repository=foo=/a/b/../c",
            "--inject_repository=bar=/x",
            "--override_module=my_mod=/m",
            "--registry=https://bcr.bazel.build",
            "--registry=https://other",
            "--lockfile_mode=off",
            "--incompatible_require_repo_extension_metadata",
            "--check_direct_dependencies=error",
            "--repository_cache=/tmp/rc",
            "--module_mirrors=https://a,https://b",
        ])
        .unwrap();
        assert_eq!(
            o.repository_overrides,
            vec![RepositoryOverride {
                repository_name: "foo".into(),
                path: "/a/c".into()
            }]
        );
        assert_eq!(o.repository_injections[0].apparent_name, "bar");
        assert_eq!(o.module_overrides[0].module_name, "my_mod");
        assert_eq!(
            o.registries,
            vec!["https://bcr.bazel.build", "https://other"]
        );
        assert_eq!(o.lockfile_mode, LockfileMode::OFF);
        assert_eq!(
            o.require_repo_extension_metadata,
            RequireRepoExtensionMetadataMode::ALL
        );
        assert_eq!(o.check_direct_dependencies, CheckDirectDepsMode::ERROR);
        assert_eq!(o.repository_cache.unwrap().get_path_string(), "/tmp/rc");
        assert_eq!(
            o.module_mirrors,
            vec![(
                String::new(),
                vec!["https://a".to_string(), "https://b".to_string()]
            )]
        );
        let d = RepositoryOptions::default();
        assert_eq!(d.lockfile_mode, LockfileMode::UPDATE);
        assert_eq!(
            d.repo_contents_cache_gc_max_age,
            Duration::from_secs(14 * 86400)
        );
    }

    #[test]
    fn rejects_bad_overrides() {
        let err = |a: &str| {
            let e = parse::<RepositoryOptions>(&[a]).unwrap_err();
            (
                e.get_message().to_string(),
                e.get_invalid_argument().map(str::to_string),
            )
        };
        assert_eq!(
            err("--override_repository=foo").0,
            "While parsing option --override_repository=foo: Repository overrides must be of the \
             form 'repository-name=path'"
        );
        let e = RepositoryOverrideConverter
            .convert(Some("foo"), None)
            .unwrap_err();
        assert_eq!(e.get_invalid_argument(), Some("foo"));
        assert_eq!(
            err("--override_repository=f$o=/x").0,
            "While parsing option --override_repository=f$o=/x: Invalid repository name given to \
             override"
        );
        assert_eq!(
            err("--override_module=Foo=/x").0,
            "While parsing option --override_module=Foo=/x: invalid module name 'Foo': valid \
             names must 1) only contain lowercase letters (a-z), digits (0-9), dots (.), hyphens \
             (-), and underscores (_); 2) begin with a lowercase letter; 3) end with a lowercase \
             letter or digit."
        );
        assert_eq!(
            err("--inject_repository=+x=/y").0,
            "While parsing option --inject_repository=+x=/y: Invalid repository name given to \
             inject"
        );
    }

    // Java tests: src/test/java/com/google/devtools/build/lib/bazel/repository/RepositoryOptionsTest.java

    fn override_converter(input: &str) -> Result<RepositoryOverride, OptionsParsingException> {
        Ok(RepositoryOverrideConverter
            .convert(Some(input), None)?
            .unwrap()
            .get::<RepositoryOverride>())
    }

    fn module_override(input: &str) -> Result<ModuleOverride, OptionsParsingException> {
        Ok(ModuleOverrideConverter
            .convert(Some(input), None)?
            .unwrap()
            .get::<ModuleOverride>())
    }

    #[test]
    fn test_override_converter() {
        let actual = override_converter("foo=/bar").unwrap();
        assert_eq!(actual.repository_name, "foo");
        assert_eq!(
            blaze_vfs::PathFragment::create(&actual.path),
            blaze_vfs::PathFragment::create("/bar")
        );
    }

    #[test]
    fn test_override_path_with_equals_sign() {
        let actual = override_converter("foo=/bar=/baz").unwrap();
        assert_eq!(actual.repository_name, "foo");
        assert_eq!(
            blaze_vfs::PathFragment::create(&actual.path),
            blaze_vfs::PathFragment::create("/bar=/baz")
        );
    }

    // Java: RepositoryOptionsTest.testOverridePathWithTilde (USER_HOME.value() is the JVM's
    // user.home, as here)
    #[test]
    fn test_override_path_with_tilde() {
        let actual = override_converter("foo=~/bar").unwrap();
        assert_eq!(actual.repository_name, "foo");
        assert_eq!(
            blaze_vfs::PathFragment::create(&actual.path),
            blaze_vfs::PathFragment::create(&format!(
                "{}/bar",
                crate::common::options_utils::user_home()
            ))
        );
    }

    // Java: RepositoryOptionsTest.testModuleOverridePathWithTilde
    #[test]
    fn test_module_override_path_with_tilde() {
        let actual = module_override("foo=~/bar").unwrap();
        assert_eq!(
            blaze_vfs::PathFragment::create(&actual.path),
            blaze_vfs::PathFragment::create(&format!(
                "{}/bar",
                crate::common::options_utils::user_home()
            ))
        );
    }

    #[test]
    fn test_module_override_relative_path() {
        let actual = module_override("foo=%workspace%/bar").unwrap();
        assert_eq!(actual.path, "%workspace%/bar");
        let actual = module_override("foo=../../bar").unwrap();
        assert_eq!(actual.path, "../../bar");
    }

    #[test]
    fn test_invalid_override() {
        let e = override_converter("foo").unwrap_err();
        assert!(
            e.get_message()
                .contains("Repository overrides must be of the form 'repository-name=path'")
        );
    }

    #[test]
    fn test_invalid_repo_override() {
        let e = override_converter("foo/bar=/baz").unwrap_err();
        assert!(
            e.get_message()
                .contains("Invalid repository name given to override")
        );
    }
}
