//! Ports `packages/Provider.java` (`Provider.Key`, with `BuiltinProvider.Key` and
//! `StarlarkProvider.Key`) and `packages/StarlarkProviderIdentifier.java`.

use std::fmt;

use blaze_cmdline::Label;

/// A serializable identity of a provider (Java `Provider.Key`).
// Java: com/google/devtools/build/lib/packages/Provider.java:64 Provider.Key
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProviderKey {
    /// Java `BuiltinProvider.Key(name, providerClass)`; equality is by provider class.
    /// `provider_class` is the Java simple name of the provider's value class (e.g.
    /// `DefaultInfo`, `OutputGroupInfo`), the class `BuiltinProvider` is parameterized with.
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:118 BuiltinProvider.Key
    Builtin {
        name: String,
        provider_class: String,
    },
    /// Java `StarlarkProvider.Key(BzlLoadValue.Key, exportedName)`.
    // PORT: the BzlLoadValue.Key is represented by the .bzl label; BzlLoadValue keys of one
    // label differ only by kind (BUILD / bzlmod / builtins), which cannot define the same
    // provider name in one build.
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:607 StarlarkProvider.Key
    Starlark {
        extension_label: Label,
        exported_name: String,
    },
}

impl ProviderKey {
    /// Java `StarlarkProvider.Key.getExtensionLabel` (`None` for builtins).
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:615 StarlarkProvider.Key.getExtensionLabel
    pub fn get_extension_label(&self) -> Option<&Label> {
        match self {
            ProviderKey::Starlark {
                extension_label, ..
            } => Some(extension_label),
            ProviderKey::Builtin { .. } => None,
        }
    }

    /// Java `Provider.Key.fingerprint`.
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:645 StarlarkProvider.Key.fingerprint
    pub fn fingerprint(&self, fp: &mut blaze_util::Fingerprint) {
        match self {
            // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:136 BuiltinProvider.Key.fingerprint
            ProviderKey::Builtin { name, .. } => {
                // True => native
                fp.add_boolean(true);
                fp.add_string(name);
            }
            ProviderKey::Starlark {
                extension_label,
                exported_name,
            } => {
                // False => Not native.
                fp.add_boolean(false);
                fp.add_string(&extension_label.get_canonical_form());
                fp.add_string(exported_name);
            }
        }
    }
}

impl fmt::Display for ProviderKey {
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:655 StarlarkProvider.Key.toString
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:153 BuiltinProvider.Key.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderKey::Builtin { name, .. } => f.write_str(name),
            ProviderKey::Starlark { exported_name, .. } => f.write_str(exported_name),
        }
    }
}

/// Identifies a provider that a Starlark rule may advertise or require (Java
/// `StarlarkProviderIdentifier`).
// Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:27 StarlarkProviderIdentifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StarlarkProviderIdentifier {
    key: ProviderKey,
}

impl StarlarkProviderIdentifier {
    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:32 StarlarkProviderIdentifier.forKey
    pub fn for_key(key: ProviderKey) -> Self {
        StarlarkProviderIdentifier { key }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:37 StarlarkProviderIdentifier.getKey
    pub fn get_key(&self) -> &ProviderKey {
        &self.key
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:64 StarlarkProviderIdentifier.KeyedIdentifier.fingerprint
    pub fn fingerprint(&self, fp: &mut blaze_util::Fingerprint) {
        fp.add_boolean(false);
        self.key.fingerprint(fp);
    }
}

impl fmt::Display for StarlarkProviderIdentifier {
    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:86 StarlarkProviderIdentifier.KeyedIdentifier.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.key)
    }
}
