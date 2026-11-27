//! Ports `packages/Provider.java` (`Provider.Key`, with `BuiltinProvider.Key` and
//! `StarlarkProvider.Key`) and `packages/StarlarkProviderIdentifier.java`.

use std::fmt;

use blaze_cmdline::Label;

/// A serializable identity of a provider (Java `Provider.Key`).
// Java: com/google/devtools/build/lib/packages/Provider.java:64 Provider.Key
#[derive(Debug, Clone)]
pub enum ProviderKey {
    /// Java `BuiltinProvider.Key(name, providerClass)`; equality is by provider class.
    /// `provider_class` is the Java simple name of the provider's value class (e.g.
    /// `DefaultInfo`, `OutputGroupInfo`), the class `BuiltinProvider` is parameterized with.
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:118 BuiltinProvider.Key
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:122 BuiltinProvider.Key.Key
    // PORT: skipped, no caller in the Bazel binary: BuiltinProvider.Key.getProviderClass
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

impl PartialEq for ProviderKey {
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:147 BuiltinProvider.Key.equals
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:634 StarlarkProvider.Key.equals
    fn eq(&self, other: &ProviderKey) -> bool {
        match (self, other) {
            // A builtin key is equal to another by its provider class alone.
            (
                ProviderKey::Builtin { provider_class, .. },
                ProviderKey::Builtin {
                    provider_class: other_class,
                    ..
                },
            ) => provider_class == other_class,
            (
                ProviderKey::Starlark {
                    extension_label,
                    exported_name,
                },
                ProviderKey::Starlark {
                    extension_label: other_label,
                    exported_name: other_name,
                },
            ) => extension_label == other_label && exported_name == other_name,
            _ => false,
        }
    }
}

impl Eq for ProviderKey {}

impl std::hash::Hash for ProviderKey {
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:142 BuiltinProvider.Key.hashCode
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:629 StarlarkProvider.Key.hashCode
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            ProviderKey::Builtin { provider_class, .. } => {
                0u8.hash(state);
                provider_class.hash(state);
            }
            ProviderKey::Starlark {
                extension_label,
                exported_name,
            } => {
                1u8.hash(state);
                extension_label.hash(state);
                exported_name.hash(state);
            }
        }
    }
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
// Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:54 StarlarkProviderIdentifier.KeyedIdentifier.<init>
// Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:69 StarlarkProviderIdentifier.KeyedIdentifier.hashCode
// Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:74 StarlarkProviderIdentifier.KeyedIdentifier.equals
// PORT: skipped, identifiers are not interned (derived equality on the key): StarlarkProviderIdentifier.KeyedIdentifier.intern
pub struct StarlarkProviderIdentifier {
    key: ProviderKey,
}

impl StarlarkProviderIdentifier {
    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:32 StarlarkProviderIdentifier.forKey
    pub fn for_key(key: ProviderKey) -> Self {
        StarlarkProviderIdentifier { key }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:37 StarlarkProviderIdentifier.getKey
    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:58 StarlarkProviderIdentifier.KeyedIdentifier.getKey
    pub fn get_key(&self) -> &ProviderKey {
        &self.key
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:64 StarlarkProviderIdentifier.KeyedIdentifier.fingerprint
    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:39 StarlarkProviderIdentifier.fingerprint
    pub fn fingerprint(&self, fp: &mut blaze_util::Fingerprint) {
        fp.add_boolean(false);
        self.key.fingerprint(fp);
    }
}

impl fmt::Display for StarlarkProviderIdentifier {
    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:86 StarlarkProviderIdentifier.KeyedIdentifier.toString
    // Java: com/google/devtools/build/lib/packages/StarlarkProviderIdentifier.java:47 StarlarkProviderIdentifier.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.key)
    }
}
