//! The parsing half of `bazel/bzlmod/modcommand/ModuleArg.java` (what `ModOptions` needs), with
//! the parts of `bzlmod/ModuleKey.java` and `bzlmod/Version.java` it uses.
//!
//! PORT: `ModuleKey` and `Version` belong to the bzlmod model crate, which `blaze_optdefs` does
//! not depend on. They are ported here only as far as option parsing needs: `Version.parse`
//! (validation and the normalized form) and `toString`. Resolution against a dependency graph
//! (`ModuleArg.resolveToModuleKeys`, `resolveToRepoNames`) is not ported here.

use blaze_cmdline::RepositoryName;
use blaze_options::{
    CommaSeparatedNonEmptyOptionListConverter, ConversionContext, Converter, OptionScalar,
    OptionsParsingException, Value, converter::non_null, option_field_type,
};

type ConvResult = Result<Option<Value>, OptionsParsingException>;

// Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:65 Version
/// A version in the Bazel module system (`RELEASE[-PRERELEASE][+BUILD]`); only the normalized
/// string is kept, which is what `equals` and `toString` use.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    normalized: String,
}

/// `Version.ParseException`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionParseException {
    pub message: String,
}

impl Version {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:81 Version.EMPTY
    pub fn empty() -> Version {
        Version {
            normalized: String::new(),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:123 Version.getNormalized
    pub fn get_normalized(&self) -> &str {
        &self.normalized
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:131 Version.isEmpty
    pub fn is_empty(&self) -> bool {
        self.normalized.is_empty()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:145 Version.parse
    /// Parses a version string into a `Version` object.
    pub fn parse(version: &str) -> Result<Version, VersionParseException> {
        if version.is_empty() {
            return Ok(Version::empty());
        }
        let Some((release, prerelease)) = match_version_pattern(version) else {
            return Err(VersionParseException {
                message: format!("bad version (does not match regex): {}", version),
            });
        };
        for ident in release.split('.') {
            identifier_from(ident).map_err(|_| VersionParseException {
                message: format!("error parsing version: {}", version),
            })?;
        }
        if let Some(prerelease) = prerelease
            && !prerelease.is_empty()
        {
            for ident in prerelease.split('.') {
                identifier_from(ident).map_err(|_| VersionParseException {
                    message: format!("error parsing version: {}", version),
                })?;
            }
        }
        let normalized = match prerelease {
            Some(p) if !p.is_empty() => format!("{}-{}", release, p),
            _ => release.to_string(),
        };
        Ok(Version { normalized })
    }
}

/// `PATTERN.matcher(version).matches()` with
/// `(?<release>[a-zA-Z0-9.]+)(?:-(?<prerelease>[a-zA-Z0-9.-]+))?(?:\+[a-zA-Z0-9.-]+)?`.
fn match_version_pattern(v: &str) -> Option<(&str, Option<&str>)> {
    let b = v.as_bytes();
    let is_rel = |c: u8| c.is_ascii_alphanumeric() || c == b'.';
    let is_pre = |c: u8| c.is_ascii_alphanumeric() || c == b'.' || c == b'-';
    let mut i = 0;
    while i < b.len() && is_rel(b[i]) {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    let release = &v[..i];
    let mut prerelease = None;
    if i < b.len() && b[i] == b'-' {
        let start = i + 1;
        let mut j = start;
        while j < b.len() && is_pre(b[j]) {
            j += 1;
        }
        if j == start {
            return None;
        }
        prerelease = Some(&v[start..j]);
        i = j;
    }
    if i < b.len() && b[i] == b'+' {
        let start = i + 1;
        let mut j = start;
        while j < b.len() && is_pre(b[j]) {
            j += 1;
        }
        if j == start {
            return None;
        }
        i = j;
    }
    if i != b.len() {
        return None;
    }
    Some((release, prerelease))
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:96 Version.Identifier.from
fn identifier_from(string: &str) -> Result<(), VersionParseException> {
    if string.is_empty() {
        return Err(VersionParseException {
            message: "identifier is empty".into(),
        });
    }
    if string.bytes().all(|c| c.is_ascii_digit()) && string.parse::<u64>().is_err() {
        return Err(VersionParseException {
            message: format!("numeric version segment is too large: {}", string),
        });
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:34 ModuleKey
/// A module name, version pair that identifies a module in the external dependency graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModuleKey {
    pub name: String,
    pub version: Version,
}

impl ModuleKey {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:57 ModuleKey.ROOT
    pub fn root() -> ModuleKey {
        ModuleKey {
            name: String::new(),
            version: Version::empty(),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:63 ModuleKey.toString
    pub fn java_to_string(&self) -> String {
        if *self == ModuleKey::root() {
            return "<root>".into();
        }
        format!(
            "{}@{}",
            self.name,
            if self.version.is_empty() {
                "_"
            } else {
                self.version.get_normalized()
            }
        )
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:44 ModuleArg
/// Represents a reference to one or more modules in the external dependency graph, used for
/// modquery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleArg {
    /// Refers to a specific version of a module. Parsed from `<module>@<version>`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:70 ModuleArg.SpecificVersionOfModule
    SpecificVersionOfModule(ModuleKey),
    /// Refers to all versions of a module. Parsed from `<module>`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:151 ModuleArg.AllVersionsOfModule
    AllVersionsOfModule(String),
    /// Refers to a module with the given apparent repo name. Parsed from `@<name>`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:229 ModuleArg.ApparentRepoName
    ApparentRepoName(String),
    /// Refers to a module with the given canonical repo name. Parsed from `@@<name>`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:291 ModuleArg.CanonicalRepoName
    CanonicalRepoName(RepositoryName),
}

impl OptionScalar for ModuleArg {
    fn java_to_string(&self) -> String {
        match self {
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:145 ModuleArg.SpecificVersionOfModule.toString
            ModuleArg::SpecificVersionOfModule(k) => k.java_to_string(),
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:219 ModuleArg.AllVersionsOfModule.toString
            ModuleArg::AllVersionsOfModule(name) => name.clone(),
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:285 ModuleArg.ApparentRepoName.toString
            ModuleArg::ApparentRepoName(name) => format!("@{}", name),
            // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:346 ModuleArg.CanonicalRepoName.toString
            ModuleArg::CanonicalRepoName(r) => format!("@@{}", r.get_name()),
        }
    }
}

option_field_type!(
    ModuleArg,
    "com.google.devtools.build.lib.bazel.bzlmod.modcommand.ModuleArg",
    raw = ModuleArg::AllVersionsOfModule(String::new())
);

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:352 ModuleArg.ModuleArgConverter
/// Converter for `ModuleArg`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModuleArgConverter;

impl ModuleArgConverter {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:356 ModuleArg.ModuleArgConverter.convert
    pub fn convert_arg(&self, input: &str) -> Result<ModuleArg, OptionsParsingException> {
        if input.eq_ignore_ascii_case("<root>") {
            return Ok(ModuleArg::SpecificVersionOfModule(ModuleKey::root()));
        }
        if let Some(name) = input.strip_prefix("@@") {
            return match RepositoryName::create(name) {
                Ok(r) => Ok(ModuleArg::CanonicalRepoName(r)),
                Err(e) => Err(OptionsParsingException::new(format!(
                    "invalid argument '{}': {}",
                    input,
                    e.get_message()
                ))),
            };
        }
        if let Some(apparent_repo_name) = input.strip_prefix('@') {
            if let Err(e) = RepositoryName::validate_user_provided_repo_name(apparent_repo_name) {
                return Err(OptionsParsingException::new(format!(
                    "invalid argument '{}': {}",
                    input,
                    e.get_message()
                )));
            }
            return Ok(ModuleArg::ApparentRepoName(apparent_repo_name.to_string()));
        }
        if let Some(at_idx) = input.find('@') {
            let module_name = &input[..at_idx];
            let version_str = &input[at_idx + 1..];
            if version_str.is_empty() {
                return Err(OptionsParsingException::new(format!(
                    "invalid argument '{}': use _ for the empty version",
                    input
                )));
            }
            let version = if version_str == "_" {
                Ok(Version::empty())
            } else {
                Version::parse(version_str)
            };
            return match version {
                Ok(version) => Ok(ModuleArg::SpecificVersionOfModule(ModuleKey {
                    name: module_name.to_string(),
                    version,
                })),
                Err(e) => Err(OptionsParsingException::new(format!(
                    "invalid argument '{}': {}",
                    input, e.message
                ))),
            };
        }
        Ok(ModuleArg::AllVersionsOfModule(input.to_string()))
    }
}

impl Converter for ModuleArgConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_arg(non_null(input))?)))
    }
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:394 ModuleArg.ModuleArgConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "\"<root>\" for the root module; <module>@<version> for a specific version of a module; \
         <module> for all versions of a module; @<name> for a repo with the given apparent name; \
         or @@<name> for a repo with the given canonical name"
            .into()
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:403 ModuleArg.CommaSeparatedModuleArgListConverter
/// Converter for a comma-separated list of `ModuleArg`s.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommaSeparatedModuleArgListConverter;

impl Converter for CommaSeparatedModuleArgListConverter {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:407 ModuleArg.CommaSeparatedModuleArgListConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let args = CommaSeparatedNonEmptyOptionListConverter.convert_list(non_null(input))?;
        let mut module_args = Vec::new();
        for arg in args {
            module_args.push(ModuleArgConverter.convert_arg(&arg)?);
        }
        Ok(Some(Value::list_of(module_args)))
    }
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArg.java:417 ModuleArg.CommaSeparatedModuleArgListConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a comma-separated list of <module>s".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(s: &str) -> Result<ModuleArg, OptionsParsingException> {
        ModuleArgConverter.convert_arg(s)
    }

    #[test]
    fn converts_each_form() {
        assert_eq!(
            conv("<ROOT>").unwrap(),
            ModuleArg::SpecificVersionOfModule(ModuleKey::root())
        );
        assert_eq!(conv("<root>").unwrap().java_to_string(), "<root>");
        assert_eq!(conv("foo@1.2.3").unwrap().java_to_string(), "foo@1.2.3");
        assert_eq!(
            conv("foo@1.2-rc1+build5").unwrap().java_to_string(),
            "foo@1.2-rc1"
        );
        assert_eq!(conv("foo@_").unwrap().java_to_string(), "foo@_");
        assert_eq!(
            conv("foo").unwrap(),
            ModuleArg::AllVersionsOfModule("foo".into())
        );
        assert_eq!(conv("@bar").unwrap().java_to_string(), "@bar");
        assert_eq!(conv("@@bar+").unwrap().java_to_string(), "@@bar+");
    }

    #[test]
    fn rejects_bad_forms() {
        assert_eq!(
            conv("foo@").unwrap_err().get_message(),
            "invalid argument 'foo@': use _ for the empty version"
        );
        assert_eq!(
            conv("foo@1..2").unwrap_err().get_message(),
            "invalid argument 'foo@1..2': error parsing version: 1..2"
        );
        assert_eq!(
            conv("foo@1$").unwrap_err().get_message(),
            "invalid argument 'foo@1$': bad version (does not match regex): 1$"
        );
        assert!(
            conv("@+bad")
                .unwrap_err()
                .get_message()
                .starts_with("invalid argument '@+bad': invalid user-provided repo name '+bad'")
        );
    }

    #[test]
    fn comma_separated_list() {
        let v = CommaSeparatedModuleArgListConverter
            .convert(Some("foo,@bar"), None)
            .unwrap()
            .unwrap();
        assert_eq!(v.java_to_string(), "[foo, @bar]");
    }

    // Java tests: src/test/java/com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArgTest.java

    fn create_module_key(name: &str, version: &str) -> ModuleKey {
        ModuleKey {
            name: name.into(),
            version: Version::parse(version).unwrap(),
        }
    }

    #[test]
    fn converter() {
        assert_eq!(
            conv("<root>").unwrap(),
            ModuleArg::SpecificVersionOfModule(ModuleKey::root())
        );
        assert_eq!(
            conv("@@abc").unwrap(),
            ModuleArg::CanonicalRepoName(RepositoryName::create_unvalidated("abc"))
        );
        assert_eq!(
            conv("@abc").unwrap(),
            ModuleArg::ApparentRepoName("abc".into())
        );
        assert_eq!(
            conv("abc").unwrap(),
            ModuleArg::AllVersionsOfModule("abc".into())
        );
        assert_eq!(
            conv("a@b").unwrap(),
            ModuleArg::SpecificVersionOfModule(create_module_key("a", "b"))
        );
        assert_eq!(
            conv("a@3.1.0-pre").unwrap(),
            ModuleArg::SpecificVersionOfModule(create_module_key("a", "3.1.0-pre"))
        );

        assert!(conv("abc@").is_err());
        assert!(conv("@_abc").is_err());
    }

    #[test]
    #[ignore = "needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_specific_version_good() {}

    #[test]
    #[ignore = "needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_specific_version_not_found() {}

    #[test]
    #[ignore = "needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_specific_version_unused() {}

    #[test]
    #[ignore = "needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_all_versions_good() {}

    #[test]
    #[ignore = "needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_all_versions_not_found() {}

    #[test]
    #[ignore = "needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_apparent_repo_name_good() {}

    #[test]
    #[ignore = "needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_apparent_repo_name_not_found() {}

    #[test]
    #[ignore = "needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_canonical_repo_name_good() {}

    #[test]
    #[ignore = "needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_canonical_repo_name_not_found() {}

    #[test]
    #[ignore = "needs ModuleArg.resolveToModuleKeys and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_canonical_repo_name_unused() {}
}
