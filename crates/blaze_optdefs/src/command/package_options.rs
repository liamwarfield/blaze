//! Transcription of `com/google/devtools/build/lib/pkgcache/PackageOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:39 PackageOptions
options! {
    /// Options for configuring Packages -- loading and default behaviors.
    pub struct PackageOptions {
        #[option(
            name = "package_path",
            default_value = "%workspace%",
            converter = ColonSeparatedOptionListConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "A colon-separated list of where to look for packages. ",
                "Elements beginning with '%workspace%' are relative to the enclosing ",
                "workspace. If omitted or empty, the default is the output of ",
                "'bazel info default-package-path'.",
            ),
        )]
        pub package_path: Vec<String>,

        #[option(
            name = "show_loading_progress",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "If enabled, causes Bazel to print \"Loading package:\" messages.",
        )]
        pub show_loading_progress: bool,

        #[option(
            name = "deleted_packages",
            default_value = "null",
            converter = CommaSeparatedPackageNameListConverter,
            allow_multiple = true,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "A comma-separated list of names of packages which the ",
                "build system will consider non-existent, even if they are ",
                "visible somewhere on the package path.\n",
                "Use this option when deleting a subpackage 'x/y' of an ",
                "existing package 'x'.  For example, after deleting x/y/BUILD ",
                "in your client, the build system may complain if it ",
                "encounters a label '//x:y/z' if that is still provided by another ",
                "package_path entry.  Specifying --deleted_packages x/y avoids this ",
                "problem.",
            ),
        )]
        pub deleted_packages: Vec<Foreign<PackageIdentifier>>,

        #[option(
            name = "default_visibility",
            default_value = "private",
            converter = DefaultVisibilityConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "Default visibility for packages that don't set it explicitly ('public' or 'private').",
        )]
        pub default_visibility: RuleVisibility,

        // TODO(b/179944632): set this and --incompatible_config_setting_private_default_visibility
        // to true, then make these no-ops, then remove.
        #[option(
            name = "incompatible_enforce_config_setting_visibility",
            default_value = "true",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, enforce config_setting visibility restrictions. If false, every ",
                "config_setting is visible to every target. See ",
                "https://github.com/bazelbuild/bazel/issues/12932.",
            ),
        )]
        pub enforce_config_setting_visibility: bool,

        #[option(
            name = "incompatible_config_setting_private_default_visibility",
            default_value = "false",
            documentation_category = STARLARK_SEMANTICS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If incompatible_enforce_config_setting_visibility=false, this is a noop. Else, if this",
                " flag is false, any config_setting without an explicit visibility attribute is",
                " //visibility:public. If this flag is true, config_setting follows the same",
                " visibility logic as all other rules. See",
                " https://github.com/bazelbuild/bazel/issues/12933.",
            ),
        )]
        pub config_setting_private_default_visibility: bool,

        #[option(
            name = "legacy_globbing_threads",
            default_value = "100",
            converter = ParallelismConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Number of threads to use for glob evaluation. Takes ",
                "an integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), optionally followed by an operation ([-|*]<float>) eg. \"auto\", \"HOST_CPUS*.5\"",
                ". \"auto\" means to use a reasonable value derived from the machine's hardware",
                " profile (e.g. the number of processors).",
            ),
        )]
        pub globbing_threads: i32,

        #[option(
            name = "experimental_max_directories_to_eagerly_visit_in_globbing",
            default_value = "-1",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If non-negative, the first time a glob is evaluated in a package, the subdirectories of ",
                "the package will be traversed in order to warm filesystem caches and compensate for ",
                "lack of parallelism in globbing. At most this many directories will be visited.",
            ),
        )]
        pub max_directories_to_eagerly_visit_in_globbing: i32,

        #[option(
            name = "fetch",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Allows the command to fetch external dependencies. If set to false, the command will",
                " utilize any cached version of the dependency, and if none exists, the command",
                " will result in failure.",
            ),
        )]
        pub fetch: bool,

        #[option(
            name = "experimental_check_output_files",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Check for modifications made to the output files of a build. Consider setting ",
                "this flag to false if you don't expect these files to change outside of bazel ",
                "since it will speed up subsequent runs as they won't have to check a ",
                "previous run's cache.",
            ),
        )]
        pub check_output_files: bool,

        #[option(
            name = "experimental_check_external_other_files",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Check for modifications made to the non-output, non-repo external files, e.g. host",
                " files.",
            ),
        )]
        pub check_external_other_files: bool,

        #[option(
            name = "experimental_lazy_macro_expansion_packages",
            default_value = "",
            converter = LazyMacroExpansionPackagesOptionConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = "List of packages in which symbolic macro are expanded only if necessary.",
        )]
        pub lazy_macro_expansion_packages: LazyMacroExpansionPackages,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const PACKAGE_OPTIONS_OPTION_NAMES: &[&str] = &[
        "package_path",
        "show_loading_progress",
        "deleted_packages",
        "default_visibility",
        "incompatible_enforce_config_setting_visibility",
        "incompatible_config_setting_private_default_visibility",
        "legacy_globbing_threads",
        "experimental_max_directories_to_eagerly_visit_in_globbing",
        "fetch",
        "experimental_check_output_files",
        "experimental_check_external_other_files",
        "experimental_lazy_macro_expansion_packages",
    ];

    #[test]
    fn package_options_option_names() {
        super::super::test_util::check_option_names(
            PackageOptions::class(),
            PACKAGE_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn package_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<PackageOptions>(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

use super::resource_converter::{host_cpus_supplier, resource_integer_converter};

// Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:42 PackageOptions.DefaultVisibilityConverter
/// Converter for the `--default_visibility` option.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultVisibilityConverter;

impl Converter for DefaultVisibilityConverter {
    // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:44 PackageOptions.DefaultVisibilityConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        Ok(Some(Value::new(match input {
            "public" => RuleVisibility::PUBLIC,
            "private" => RuleVisibility::PRIVATE,
            _ => {
                return Err(OptionsParsingException::new(format!(
                    "Not a valid default visibility: '{}' (should be 'public' or 'private'",
                    input
                )));
            }
        })))
    }
    fn get_type_description(&self) -> String {
        "default visibility".into()
    }
}

resource_integer_converter!(
    // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:61 PackageOptions.ParallelismConverter
    /// Converter for globbing threads.
    pub ParallelismConverter => ResourceIntegerConverter::new(host_cpus_supplier, 1, i32::MAX)
);

fn parse_package_identifier(
    s: &str,
) -> Result<Foreign<PackageIdentifier>, OptionsParsingException> {
    PackageIdentifier::parse(s)
        .map(Foreign)
        .map_err(|e| OptionsParsingException::new(e.get_message()))
}

// Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:219 PackageOptions.CommaSeparatedPackageNameListConverter
/// A converter from strings containing comma-separated names of packages to lists of strings.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommaSeparatedPackageNameListConverter;

impl Converter for CommaSeparatedPackageNameListConverter {
    // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:225 PackageOptions.CommaSeparatedPackageNameListConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let Some(input) = input.filter(|s| !s.is_empty()) else {
            return Ok(Some(Value::list(vec![])));
        };
        let mut list = Vec::new();
        // Splitter.on(',') keeps empty pieces.
        for s in input.split(',') {
            list.push(parse_package_identifier(s)?);
        }
        Ok(Some(Value::list_of(list)))
    }
    fn get_type_description(&self) -> String {
        "comma-separated list of package names".into()
    }
}

impl PackageOptions {
    // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:246 PackageOptions.getDeletedPackages
    pub fn get_deleted_packages(&self) -> Vec<PackageIdentifier> {
        let mut out: Vec<PackageIdentifier> = Vec::new();
        for p in &self.deleted_packages {
            if !out.contains(&p.0) {
                out.push(p.0.clone());
            }
        }
        out
    }
}

// Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:257 PackageOptions.LazyMacroExpansionPackages
/// The set of packages in which symbolic macros are to be expanded lazily. Used by Skyframe and
/// by PackageLoader.
#[derive(Clone, Debug)]
pub enum LazyMacroExpansionPackages {
    // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:269 PackageOptions.LazyMacroExpansionPackages.ALL
    /// All packages should have symbolic macros expanded lazily.
    All,
    // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:313 PackageOptions.LazyMacroExpansionPackagesSet
    /// The given packages (insertion order, no duplicates). Empty is
    /// `LazyMacroExpansionPackages.NONE`.
    Set(Vec<Foreign<PackageIdentifier>>),
}

impl LazyMacroExpansionPackages {
    // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:262 PackageOptions.LazyMacroExpansionPackages.NONE
    pub const NONE: LazyMacroExpansionPackages = LazyMacroExpansionPackages::Set(Vec::new());

    // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:278 PackageOptions.LazyMacroExpansionPackages.contains
    /// Returns true if symbolic macros in the given package should be expanded lazily.
    pub fn contains(&self, package_id: &PackageIdentifier) -> bool {
        match self {
            LazyMacroExpansionPackages::All => true,
            LazyMacroExpansionPackages::Set(ids) => ids.iter().any(|p| p.0 == *package_id),
        }
    }
}

// Java: ALL is a singleton (identity equality); sets compare as sets.
impl PartialEq for LazyMacroExpansionPackages {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LazyMacroExpansionPackages::All, LazyMacroExpansionPackages::All) => true,
            (LazyMacroExpansionPackages::Set(a), LazyMacroExpansionPackages::Set(b)) => {
                a.len() == b.len() && a.iter().all(|x| b.contains(x))
            }
            _ => false,
        }
    }
}

impl OptionScalar for LazyMacroExpansionPackages {
    fn java_to_string(&self) -> String {
        match self {
            // PORT: Java prints the anonymous class's Object.toString (class name and identity
            // hash), which is not reproducible.
            LazyMacroExpansionPackages::All => {
                "com.google.devtools.build.lib.pkgcache.PackageOptions$LazyMacroExpansionPackages$1"
                    .to_string()
            }
            // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:340 PackageOptions.LazyMacroExpansionPackagesSet.toString
            LazyMacroExpansionPackages::Set(ids) => format!(
                "LazyMacroExpansionPackages[{}]",
                blaze_options::value::java_list_to_string(
                    ids.iter().map(OptionScalar::java_to_string)
                )
            ),
        }
    }
}
option_field_type!(
    LazyMacroExpansionPackages,
    "com.google.devtools.build.lib.pkgcache.PackageOptions$LazyMacroExpansionPackages",
    raw = LazyMacroExpansionPackages::NONE
);

// Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:281 PackageOptions.LazyMacroExpansionPackages.OptionConverter
/// Converter for [`LazyMacroExpansionPackages`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LazyMacroExpansionPackagesOptionConverter;

impl Converter for LazyMacroExpansionPackagesOptionConverter {
    // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:287 PackageOptions.LazyMacroExpansionPackages.OptionConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let strings = CommaSeparatedNonEmptyOptionListConverter.convert_list(non_null(input))?;
        if strings.is_empty() {
            return Ok(Some(Value::new(LazyMacroExpansionPackages::NONE)));
        }
        if strings.iter().any(|s| s == "*") {
            return Ok(Some(Value::new(LazyMacroExpansionPackages::All)));
        }
        let mut package_ids: Vec<Foreign<PackageIdentifier>> = Vec::new();
        for s in &strings {
            let id = parse_package_identifier(s)?;
            if !package_ids.contains(&id) {
                package_ids.push(id);
            }
        }
        Ok(Some(Value::new(LazyMacroExpansionPackages::Set(
            package_ids,
        ))))
    }
    fn get_type_description(&self) -> String {
        "comma-separated list of package names; or '*' to indicate all packages".into()
    }
}

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, parse_err};
    use super::*;

    #[test]
    fn deleted_packages_and_visibility() {
        let o = parse::<PackageOptions>(&[
            "--deleted_packages=a/b,c",
            "--deleted_packages=a/b",
            "--default_visibility=public",
        ]);
        assert_eq!(o.deleted_packages.len(), 3);
        assert_eq!(o.get_deleted_packages().len(), 2);
        assert_eq!(o.default_visibility, RuleVisibility::PUBLIC);
        assert_eq!(
            parse_err::<PackageOptions>(&["--default_visibility=x"]),
            "While parsing option --default_visibility=x: Not a valid default visibility: 'x' (should be 'public' or 'private'"
        );
    }

    #[test]
    fn lazy_macro_expansion() {
        let o = parse::<PackageOptions>(&["--experimental_lazy_macro_expansion_packages=*"]);
        assert_eq!(
            o.lazy_macro_expansion_packages,
            LazyMacroExpansionPackages::All
        );
        let o = parse::<PackageOptions>(&["--experimental_lazy_macro_expansion_packages=a,b"]);
        assert!(
            o.lazy_macro_expansion_packages
                .contains(&PackageIdentifier::create_in_main_repo_str("a"))
        );
        assert_eq!(
            PackageOptions::default().lazy_macro_expansion_packages,
            LazyMacroExpansionPackages::NONE
        );
    }
}
