//! The parsing half of `bazel/bzlmod/modcommand/ExtensionArg.java` (what `ModOptions` needs).
//! `ExtensionArg.resolveToExtensionId` needs the bzlmod dependency graph and is not ported here.

use blaze_options::{
    CommaSeparatedNonEmptyOptionListConverter, ConversionContext, Converter, OptionScalar,
    OptionsParsingException, Value, converter::non_null, option_field_type,
};

use super::module_arg::{ModuleArg, ModuleArgConverter};

type ConvResult = Result<Option<Value>, OptionsParsingException>;

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:44 ExtensionArg
/// Represents a reference to a module extension, parsed from a command-line argument in the form
/// of `<module><bzl_file_label>%<extension_name>`. The `<module>` part is parsed as a
/// [`ModuleArg`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionArg {
    pub module_arg: ModuleArg,
    pub repo_relative_bzl_label: String,
    pub extension_name: String,
}

impl ExtensionArg {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:51 ExtensionArg.create
    pub fn create(
        module_arg: ModuleArg,
        repo_relative_bzl_label: String,
        extension_name: String,
    ) -> ExtensionArg {
        ExtensionArg {
            module_arg,
            repo_relative_bzl_label,
            extension_name,
        }
    }
}

impl OptionScalar for ExtensionArg {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:102 ExtensionArg.toString
    fn java_to_string(&self) -> String {
        format!(
            "{}{}%{}",
            self.module_arg.java_to_string(),
            self.repo_relative_bzl_label,
            self.extension_name
        )
    }
}

option_field_type!(
    ExtensionArg,
    "com.google.devtools.build.lib.bazel.bzlmod.modcommand.ExtensionArg",
    raw = ExtensionArg::create(
        ModuleArg::AllVersionsOfModule(String::new()),
        String::new(),
        String::new()
    )
);

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:107 ExtensionArg.ExtensionArgConverter
/// Converter for `ExtensionArg`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExtensionArgConverter;

impl ExtensionArgConverter {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:111 ExtensionArg.ExtensionArgConverter.convert
    pub fn convert_arg(&self, input: &str) -> Result<ExtensionArg, OptionsParsingException> {
        let Some(slash_idx) = input.find('/') else {
            return Err(OptionsParsingException::new(format!(
                "Invalid argument {}: missing .bzl label",
                input
            )));
        };
        // Java: indexOf returns -1 when absent, which is < slashIdx.
        let percent_idx = match input.find('%') {
            Some(p) if p >= slash_idx => p,
            _ => {
                return Err(OptionsParsingException::new(format!(
                    "Invalid argument {}: missing extension name",
                    input
                )));
            }
        };
        let module_arg = ModuleArgConverter.convert_arg(&input[..slash_idx])?;
        Ok(ExtensionArg::create(
            module_arg,
            input[slash_idx..percent_idx].to_string(),
            input[percent_idx + 1..].to_string(),
        ))
    }
}

impl Converter for ExtensionArgConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_arg(non_null(input))?)))
    }
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:126 ExtensionArg.ExtensionArgConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "an <extension> identifier in the format of <module><bzl_label>%<extension_name>".into()
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:132 ExtensionArg.CommaSeparatedExtensionArgListConverter
/// Converter for a comma-separated list of `ExtensionArg`s.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommaSeparatedExtensionArgListConverter;

impl Converter for CommaSeparatedExtensionArgListConverter {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:136 ExtensionArg.CommaSeparatedExtensionArgListConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let args = CommaSeparatedNonEmptyOptionListConverter.convert_list(non_null(input))?;
        let mut extension_args = Vec::new();
        for arg in args {
            extension_args.push(ExtensionArgConverter.convert_arg(&arg)?);
        }
        Ok(Some(Value::list_of(extension_args)))
    }
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:146 ExtensionArg.CommaSeparatedExtensionArgListConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a comma-separated list of <extension>s".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_extension_arg() {
        let e = ExtensionArgConverter
            .convert_arg("@rules_java//java:extensions.bzl%toolchains")
            .unwrap();
        assert_eq!(
            e.module_arg,
            ModuleArg::ApparentRepoName("rules_java".into())
        );
        assert_eq!(e.repo_relative_bzl_label, "//java:extensions.bzl");
        assert_eq!(e.extension_name, "toolchains");
        assert_eq!(
            e.java_to_string(),
            "@rules_java//java:extensions.bzl%toolchains"
        );
    }

    #[test]
    fn rejects_missing_parts() {
        assert_eq!(
            ExtensionArgConverter
                .convert_arg("foo")
                .unwrap_err()
                .get_message(),
            "Invalid argument foo: missing .bzl label"
        );
        assert_eq!(
            ExtensionArgConverter
                .convert_arg("foo//bar.bzl")
                .unwrap_err()
                .get_message(),
            "Invalid argument foo//bar.bzl: missing extension name"
        );
    }

    // Java tests: src/test/java/com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArgTest.java

    use super::super::module_arg::{ModuleKey, Version};
    use blaze_cmdline::RepositoryName;

    fn create_module_key(name: &str, version: &str) -> ModuleKey {
        ModuleKey {
            name: name.into(),
            version: Version::parse(version).unwrap(),
        }
    }

    fn conv(s: &str) -> Result<ExtensionArg, OptionsParsingException> {
        ExtensionArgConverter.convert_arg(s)
    }

    #[test]
    fn converter() {
        assert_eq!(
            conv("<root>//abc:haha.bzl%ext").unwrap(),
            ExtensionArg::create(
                ModuleArg::SpecificVersionOfModule(ModuleKey::root()),
                "//abc:haha.bzl".into(),
                "ext".into()
            )
        );
        assert_eq!(
            conv("@@abc//:def.bzl%ghi").unwrap(),
            ExtensionArg::create(
                ModuleArg::CanonicalRepoName(RepositoryName::create_unvalidated("abc")),
                "//:def.bzl".into(),
                "ghi".into()
            )
        );
        assert_eq!(
            conv("@abc//:def.bzl%ghi").unwrap(),
            ExtensionArg::create(
                ModuleArg::ApparentRepoName("abc".into()),
                "//:def.bzl".into(),
                "ghi".into()
            )
        );
        assert_eq!(
            conv("abc//:def.bzl%ghi").unwrap(),
            ExtensionArg::create(
                ModuleArg::AllVersionsOfModule("abc".into()),
                "//:def.bzl".into(),
                "ghi".into()
            )
        );
        assert_eq!(
            conv("a@b//:def.bzl%ghi").unwrap(),
            ExtensionArg::create(
                ModuleArg::SpecificVersionOfModule(create_module_key("a", "b")),
                "//:def.bzl".into(),
                "ghi".into()
            )
        );

        assert!(conv("abc@//:def.bzl%ghi").is_err());
        assert!(conv("@_abc//:def.bzl%ghi").is_err());
        assert!(conv("abc").is_err());
        assert!(conv("abc%def").is_err());
        assert!(conv("@_abc%ghi//def").is_err());
    }

    #[test]
    #[ignore = "needs ExtensionArg.resolveToExtensionId and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_good() {}

    #[test]
    #[ignore = "needs ExtensionArg.resolveToExtensionId and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_bad_label() {}

    #[test]
    #[ignore = "needs ExtensionArg.resolveToExtensionId and the bzlmod dep graph (AugmentedModule)"]
    fn resolve_none_ortoo_many_modules() {}
}
