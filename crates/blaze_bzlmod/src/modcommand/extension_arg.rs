//! The resolution half of `bazel/bzlmod/modcommand/ExtensionArg.java`
//! (`resolveToExtensionId`). Parsing (`ExtensionArgConverter`) is transcribed in
//! `blaze_optdefs::misc::extension_arg`.

use std::fmt;

use blaze_bzlmod_model::{ModuleExtensionId, ModuleKey};
use blaze_cmdline::{Label, LabelRepoContextImpl, RepositoryMapping, RepositoryName};
use blaze_optdefs::misc::extension_arg as opt;
use blaze_options::OptionsParsingException;
use indexmap::{IndexMap, IndexSet};

use super::invalid_argument_exception::{InvalidArgumentException, ModCommandCode};
use super::module_arg::{ModuleArg, module_key_set_to_string};
use crate::inspector::AugmentedModule;

/// Represents a reference to a module extension, parsed from a command-line argument in the form
/// of `<module><bzl_file_label>%<extension_name>`. The `<module>` part is parsed as a
/// [`ModuleArg`]. Valid examples include `@rules_java//java:extensions.bzl%toolchains`,
/// `rules_java@6.1.1//java:extensions.bzl%toolchains`, etc.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:44 ExtensionArg
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionArg {
    pub module_arg: ModuleArg,
    pub repo_relative_bzl_label: String,
    pub extension_name: String,
}

impl ExtensionArg {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:51 ExtensionArg.create
    pub fn create(
        module_arg: ModuleArg,
        repo_relative_bzl_label: impl Into<String>,
        extension_name: impl Into<String>,
    ) -> ExtensionArg {
        ExtensionArg {
            module_arg,
            repo_relative_bzl_label: repo_relative_bzl_label.into(),
            extension_name: extension_name.into(),
        }
    }

    /// Converts a parsed option value (`blaze_optdefs`) into this type.
    pub fn from_option(arg: &opt::ExtensionArg) -> ExtensionArg {
        ExtensionArg::create(
            ModuleArg::from_option(&arg.module_arg),
            arg.repo_relative_bzl_label.clone(),
            arg.extension_name.clone(),
        )
    }

    /// `ExtensionArgConverter.INSTANCE.convert(input)`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:111 ExtensionArg.ExtensionArgConverter.convert
    pub fn convert(input: &str) -> Result<ExtensionArg, OptionsParsingException> {
        Ok(ExtensionArg::from_option(
            &opt::ExtensionArgConverter.convert_arg(input)?,
        ))
    }

    /// Resolves this [`ExtensionArg`] to a [`ModuleExtensionId`].
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:57 ExtensionArg.resolveToExtensionId
    pub fn resolve_to_extension_id(
        &self,
        modules_index: &IndexMap<String, IndexSet<ModuleKey>>,
        dep_graph: &IndexMap<ModuleKey, AugmentedModule>,
        module_key_to_canonical_names: &IndexMap<ModuleKey, RepositoryName>,
        base_module_deps: &IndexMap<String, ModuleKey>,
        base_module_unused_deps: &IndexMap<String, ModuleKey>,
    ) -> Result<ModuleExtensionId, InvalidArgumentException> {
        let ref_modules = self.module_arg.resolve_to_module_keys(
            modules_index,
            dep_graph,
            module_key_to_canonical_names,
            base_module_deps,
            base_module_unused_deps,
            /* include_unused= */ false,
            /* warn_unused= */ false,
        )?;
        if ref_modules.len() != 1 {
            return Err(InvalidArgumentException::with_code(
                format!(
                    "Module {}, as part of the extension specifier, should represent exactly one \
                     module version. Choose one of: {}.",
                    self.module_arg,
                    module_key_set_to_string(&ref_modules)
                ),
                ModCommandCode::INVALID_ARGUMENTS,
            ));
        }
        let key = ref_modules.first().unwrap();
        let Some(canonical_name) = module_key_to_canonical_names.get(key) else {
            panic!("java.lang.NullPointerException");
        };
        let repo_context = LabelRepoContextImpl::of(
            canonical_name.clone(),
            // Intentionally allow no repo mapping here: it's a repo-relative label!
            RepositoryMapping::create(
                Vec::<(String, RepositoryName)>::new(),
                canonical_name.clone(),
            ),
        );
        match Label::parse_with_repo_context(&self.repo_relative_bzl_label, &repo_context) {
            // TODO(wyv): support isolated extension usages?
            Ok(label) => Ok(ModuleExtensionId::create(
                label,
                self.extension_name.clone(),
                None,
            )),
            Err(e) => Err(InvalidArgumentException::with_cause(
                format!(
                    "bad label format in {}: {}",
                    self.repo_relative_bzl_label,
                    e.get_message()
                ),
                ModCommandCode::INVALID_ARGUMENTS,
                e.get_message(),
            )),
        }
    }
}

impl fmt::Display for ExtensionArg {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/ExtensionArg.java:102 ExtensionArg.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}%{}",
            self.module_arg, self.repo_relative_bzl_label, self.extension_name
        )
    }
}

/// Java `List.toString()` of extension args (used in `--extension_filter %s` messages).
pub fn extension_arg_list_to_string(args: &[ExtensionArg]) -> String {
    let parts: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    format!("[{}]", parts.join(", "))
}
