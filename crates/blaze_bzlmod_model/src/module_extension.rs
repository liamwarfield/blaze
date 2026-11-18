//! Ports of `ModuleExtensionId`, `ModuleExtensionUsage` (with `Proxy` and `RepoOverride`) and
//! `Tag`.

use std::cmp::Ordering;
use std::fmt;

use blaze_cmdline::{Label, LabelSyntaxException};
use blaze_events::Location;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::module_key::ModuleKey;
use crate::repo_spec::AttributeValues;
use crate::version::{VersionParseException, java_string_compare};

/// A unique identifier for a single isolated usage of a fixed module extension.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionId.java:55 ModuleExtensionId.IsolationKey
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IsolationKey {
    /// The module which contains this isolated usage of a module extension.
    module: ModuleKey,
    /// The exported name of the first extension proxy for this usage.
    usage_exported_name: String,
}

impl IsolationKey {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionId.java:67 IsolationKey.create
    pub fn create(module: ModuleKey, usage_exported_name: impl Into<String>) -> IsolationKey {
        IsolationKey {
            module,
            usage_exported_name: usage_exported_name.into(),
        }
    }

    pub fn module(&self) -> &ModuleKey {
        &self.module
    }

    pub fn usage_exported_name(&self) -> &str {
        &self.usage_exported_name
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionId.java:63 IsolationKey.LEXICOGRAPHIC_COMPARATOR
    pub fn lexicographic_cmp(a: &IsolationKey, b: &IsolationKey) -> Ordering {
        ModuleKey::lexicographic_cmp(&a.module, &b.module)
            .then_with(|| java_string_compare(&a.usage_exported_name, &b.usage_exported_name))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionId.java:76 IsolationKey.fromString
    pub fn from_string(s: &str) -> Result<IsolationKey, VersionParseException> {
        let parts: Vec<&str> = s.split('+').collect();
        let module = ModuleKey::from_string(parts[0])?;
        let Some(name) = parts.get(1) else {
            panic!(
                "IndexOutOfBoundsException: Index 1 out of bounds for length {}",
                parts.len()
            );
        };
        Ok(IsolationKey::create(module, *name))
    }
}

impl fmt::Display for IsolationKey {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionId.java:71 IsolationKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}+{}", self.module, self.usage_exported_name)
    }
}

/// A unique identifier for a module extension. Port of `ModuleExtensionId`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionId.java:31 ModuleExtensionId
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleExtensionId {
    bzl_file_label: Label,
    extension_name: String,
    isolation_key: Option<IsolationKey>,
}

impl ModuleExtensionId {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionId.java:83 ModuleExtensionId.create
    pub fn create(
        bzl_file_label: Label,
        extension_name: impl Into<String>,
        isolation_key: Option<IsolationKey>,
    ) -> ModuleExtensionId {
        ModuleExtensionId {
            bzl_file_label,
            extension_name: extension_name.into(),
            isolation_key,
        }
    }

    pub fn bzl_file_label(&self) -> &Label {
        &self.bzl_file_label
    }

    pub fn extension_name(&self) -> &str {
        &self.extension_name
    }

    pub fn isolation_key(&self) -> Option<&IsolationKey> {
        self.isolation_key.as_ref()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionId.java:41 ModuleExtensionId.LEXICOGRAPHIC_COMPARATOR
    pub fn lexicographic_cmp(a: &ModuleExtensionId, b: &ModuleExtensionId) -> Ordering {
        a.bzl_file_label
            .cmp(&b.bzl_file_label)
            .then_with(|| java_string_compare(&a.extension_name, &b.extension_name))
            .then_with(|| match (&a.isolation_key, &b.isolation_key) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Less,
                (Some(_), None) => Ordering::Greater,
                (Some(x), Some(y)) => IsolationKey::lexicographic_cmp(x, y),
            })
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionId.java:88 ModuleExtensionId.isInnate
    pub fn is_innate(&self) -> bool {
        self.extension_name.contains(' ')
    }

    /// Parses the lockfile form (`MODULE_EXTENSION_ID_TYPE_ADAPTER.read`); the error carries the
    /// adapter's `JsonParseException` message.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:166 GsonTypeAdapterUtil.MODULE_EXTENSION_ID_TYPE_ADAPTER
    pub fn parse_lockfile_form(json_string: &str) -> Result<ModuleExtensionId, String> {
        let ext_id_parts: Vec<&str> = json_string.split('%').collect();
        let isolation_key = if ext_id_parts.len() > 2 {
            match IsolationKey::from_string(ext_id_parts[2]) {
                Ok(k) => Some(k),
                Err(_) => {
                    return Err(format!(
                        "Unable to parse ModuleExtensionID isolation key: '{}' from the lockfile",
                        ext_id_parts[2]
                    ));
                }
            }
        } else {
            None
        };
        let label: Result<Label, LabelSyntaxException> = Label::parse_canonical(ext_id_parts[0]);
        let Ok(label) = label else {
            return Err(format!(
                "Unable to parse ModuleExtensionID bzl file label: '{}' from the lockfile",
                ext_id_parts[0]
            ));
        };
        let Some(name) = ext_id_parts.get(1) else {
            panic!(
                "IndexOutOfBoundsException: Index 1 out of bounds for length {}",
                ext_id_parts.len()
            );
        };
        Ok(ModuleExtensionId::create(label, *name, isolation_key))
    }

    /// The lockfile form (`MODULE_EXTENSION_ID_TYPE_ADAPTER.write`): uses `Label.toString`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:158 GsonTypeAdapterUtil.MODULE_EXTENSION_ID_TYPE_ADAPTER
    pub fn to_lockfile_form(&self) -> String {
        let isolation_key_part = self
            .isolation_key
            .as_ref()
            .map(|k| format!("%{k}"))
            .unwrap_or_default();
        format!(
            "{}%{}{}",
            self.bzl_file_label, self.extension_name, isolation_key_part
        )
    }
}

impl fmt::Display for ModuleExtensionId {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionId.java:93 ModuleExtensionId.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let isolation_key_part = self
            .isolation_key
            .as_ref()
            .map(|k| format!("%{k}"))
            .unwrap_or_default();
        write!(
            f,
            "{}%{}{}",
            self.bzl_file_label.get_unambiguous_canonical_form(),
            self.extension_name,
            isolation_key_part
        )
    }
}

/// A module extension tag: data following the attribute schema of a tag class. Tag attributes
/// are opaque here (checked at extension resolution time). Port of `Tag`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Tag.java:30 Tag
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub tag_name: String,
    /// All keyword arguments supplied to the tag instance.
    pub attribute_values: AttributeValues,
    /// Whether this tag was created using a proxy created with dev_dependency = True.
    pub dev_dependency: bool,
    /// The source location in the module file where this tag was created.
    pub location: Location,
}

impl Tag {
    /// Returns a new tag with all information removed that does not influence the evaluation of
    /// the extension defining the tag.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Tag.java:53 Tag.trimForEvaluation
    pub fn trim_for_evaluation(&self) -> Tag {
        Tag {
            location: Location::builtin().clone(),
            ..self.clone()
        }
    }
}

/// Represents one "proxy object" returned from one `use_extension` call.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionUsage.java:50 ModuleExtensionUsage.Proxy
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleExtensionUsageProxy {
    /// The location of the `use_extension` call.
    pub location: Location,
    /// The name the return value of `use_extension` is bound to, or empty.
    pub proxy_name: String,
    /// The path to the MODULE.bazel file (or one of its includes) that contains this proxy,
    /// relative to the workspace root.
    pub containing_module_file_path: PathFragment,
    /// Whether `dev_dependency` is set to true.
    pub dev_dependency: bool,
    /// Repos imported through this proxy: local repo name to the name exported by the
    /// extension (a bimap in Java; callers keep values unique).
    pub imports: IndexMap<String, String>,
}

impl ModuleExtensionUsageProxy {
    pub fn get_imports(&self) -> &IndexMap<String, String> {
        &self.imports
    }

    pub fn is_dev_dependency(&self) -> bool {
        self.dev_dependency
    }
}

/// A repo that overrides another repo within the scope of the extension.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionUsage.java:136 ModuleExtensionUsage.RepoOverride
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoOverride {
    /// The apparent name of the overriding repo in the root module.
    pub overriding_repo_name: String,
    /// Whether this override should apply to an existing repo.
    pub must_exist: bool,
    /// The location of the `override_repo` or `inject_repo` call.
    pub location: Location,
}

/// The usage of a module extension in one module: all proxies from `use_extension` calls that
/// refer to the same extension (or isolate). Port of `ModuleExtensionUsage`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionUsage.java:37 ModuleExtensionUsage
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleExtensionUsage {
    /// An unresolved label pointing to the Starlark file where the module extension is defined.
    pub extension_bzl_file: String,
    /// The name of the extension.
    pub extension_name: String,
    /// Present iff the usage is created with `isolate = True`.
    pub isolation_key: Option<IsolationKey>,
    pub proxies: Vec<ModuleExtensionUsageProxy>,
    /// All the tags specified by this module for this extension.
    pub tags: Vec<Tag>,
    /// Overrides of repos generated by this extension, keyed by extension-local repo name. Only
    /// non-empty for root module usages.
    pub repo_overrides: IndexMap<String, RepoOverride>,
}

impl ModuleExtensionUsage {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionUsage.java:117 ModuleExtensionUsage.getHasDevUseExtension
    pub fn get_has_dev_use_extension(&self) -> bool {
        self.proxies.iter().any(|p| p.dev_dependency)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionUsage.java:124 ModuleExtensionUsage.getHasNonDevUseExtension
    pub fn get_has_non_dev_use_extension(&self) -> bool {
        self.proxies.iter().any(|p| !p.dev_dependency)
    }

    /// Returns a new usage with all information removed that does not influence the evaluation
    /// of the extension.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionUsage.java:155 ModuleExtensionUsage.trimForEvaluation
    pub fn trim_for_evaluation(&self) -> ModuleExtensionUsage {
        ModuleExtensionUsage {
            tags: self.tags.iter().map(Tag::trim_for_evaluation).collect(),
            proxies: Vec::new(),
            repo_overrides: IndexMap::new(),
            ..self.clone()
        }
    }
}
