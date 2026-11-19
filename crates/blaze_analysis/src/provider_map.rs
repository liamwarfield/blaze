//! Port of `analysis/TransitiveInfoProviderMap.java`, `TransitiveInfoProviderMapBuilder.java` and
//! `TransitiveInfoProvider.java`.

use std::any::{Any, TypeId};
use std::fmt;
use std::sync::Arc;

use blaze_packages::rule_class::ProviderKey;
use indexmap::IndexMap;
use starlark::values::OwnedFrozenValue;

use crate::info::info_provider_key;

/// A native-only provider (Java `TransitiveInfoProvider` implementations that are not `Info`:
/// `FileProvider`, `FilesToRunProvider`, `RunfilesProvider`, `VisibilityProvider`,
/// `LicensesProvider`, `RequiredConfigFragmentsProvider`, ...). Keyed by its Rust type.
// Java: com/google/devtools/build/lib/analysis/TransitiveInfoProvider.java:30 TransitiveInfoProvider
pub trait TransitiveInfoProvider: Any + Send + Sync + fmt::Debug {
    /// The Java simple class name (for debugging and error messages).
    fn java_class_name(&self) -> &'static str;
    fn as_any(&self) -> &dyn Any;
}

/// A key of [`TransitiveInfoProviderMap`]: Java uses `Class` (native providers), `Provider.Key`
/// (infos) and `String` (legacy struct fields).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ProviderMapKey {
    Class(TypeId),
    Info(ProviderKey),
    Legacy(String),
}

/// A value of [`TransitiveInfoProviderMap`].
#[derive(Clone)]
pub enum ProviderInstance {
    /// An `Info` (frozen Starlark value; see `crate::info`).
    Info(OwnedFrozenValue),
    /// A native `TransitiveInfoProvider`.
    Native(Arc<dyn TransitiveInfoProvider>),
    /// A legacy struct field value (frozen Starlark value).
    Legacy(OwnedFrozenValue),
}

impl fmt::Debug for ProviderInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderInstance::Info(v) => write!(f, "Info({})", v.value()),
            ProviderInstance::Native(p) => write!(f, "Native({:?})", p),
            ProviderInstance::Legacy(v) => write!(f, "Legacy({})", v.value()),
        }
    }
}

/// Provides a mapping between a `TransitiveInfoProvider` class (or `Provider.Key`, or legacy
/// key string) and an instance, in insertion order.
// Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMap.java:23 TransitiveInfoProviderMap
#[derive(Clone, Debug, Default)]
pub struct TransitiveInfoProviderMap {
    map: IndexMap<ProviderMapKey, ProviderInstance>,
}

impl TransitiveInfoProviderMap {
    pub fn builder() -> TransitiveInfoProviderMapBuilder {
        TransitiveInfoProviderMapBuilder::default()
    }

    /// Java `getProvider(Class)`.
    pub fn get_provider<P: TransitiveInfoProvider>(&self) -> Option<&P> {
        match self.map.get(&ProviderMapKey::Class(TypeId::of::<P>()))? {
            ProviderInstance::Native(p) => p.as_any().downcast_ref::<P>(),
            _ => None,
        }
    }

    /// Java `get(Provider.Key)`.
    pub fn get(&self, key: &ProviderKey) -> Option<&OwnedFrozenValue> {
        match self.map.get(&ProviderMapKey::Info(key.clone()))? {
            ProviderInstance::Info(v) => Some(v),
            _ => None,
        }
    }

    /// Java `get(String legacyKey)`.
    pub fn get_legacy(&self, key: &str) -> Option<&OwnedFrozenValue> {
        match self.map.get(&ProviderMapKey::Legacy(key.to_owned()))? {
            ProviderInstance::Legacy(v) => Some(v),
            _ => None,
        }
    }

    /// Java `getProviderCount()`.
    pub fn get_provider_count(&self) -> usize {
        self.map.len()
    }

    /// Java `getProviderKeyAt(i)` / `getProviderInstanceAt(i)`.
    pub fn entries(&self) -> impl Iterator<Item = (&ProviderMapKey, &ProviderInstance)> {
        self.map.iter()
    }

    pub fn contains(&self, key: &ProviderMapKey) -> bool {
        self.map.contains_key(key)
    }
}

/// Java `TransitiveInfoProviderMapBuilder`.
// Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:27 TransitiveInfoProviderMapBuilder
#[derive(Clone, Debug, Default)]
pub struct TransitiveInfoProviderMapBuilder {
    map: IndexMap<ProviderMapKey, ProviderInstance>,
}

impl TransitiveInfoProviderMapBuilder {
    /// Java `put(TransitiveInfoProvider)` / `add(...)`: keyed by the provider's class.
    pub fn add_native<P: TransitiveInfoProvider>(&mut self, provider: P) -> &mut Self {
        self.map.insert(
            ProviderMapKey::Class(TypeId::of::<P>()),
            ProviderInstance::Native(Arc::new(provider)),
        );
        self
    }

    /// Like [`Self::add_native`] for a shared provider.
    pub fn add_native_arc<P: TransitiveInfoProvider>(&mut self, provider: Arc<P>) -> &mut Self {
        self.map.insert(
            ProviderMapKey::Class(TypeId::of::<P>()),
            ProviderInstance::Native(provider),
        );
        self
    }

    /// Java `put(Info)`: keyed by `info.getProvider().getKey()`. Panics if `info` is not an Info.
    pub fn put_info(&mut self, info: OwnedFrozenValue) -> &mut Self {
        let key = info_provider_key(info.value())
            .unwrap_or_else(|| panic!("IllegalArgumentException: not an Info: {}", info.value()));
        self.map
            .insert(ProviderMapKey::Info(key), ProviderInstance::Info(info));
        self
    }

    /// Java `put(String legacyKey, Object value)`.
    pub fn put_legacy(&mut self, key: &str, value: OwnedFrozenValue) -> &mut Self {
        self.map.insert(
            ProviderMapKey::Legacy(key.to_owned()),
            ProviderInstance::Legacy(value),
        );
        self
    }

    /// Java `addAll(TransitiveInfoProviderMap)`.
    pub fn add_all(&mut self, other: &TransitiveInfoProviderMap) -> &mut Self {
        for (k, v) in &other.map {
            self.map.insert(k.clone(), v.clone());
        }
        self
    }

    pub fn contains(&self, key: &ProviderMapKey) -> bool {
        self.map.contains_key(key)
    }

    /// Java `put(key, value)` with a key and instance taken from another map (Java
    /// `put(Class, TransitiveInfoProvider)`, `put(String, Object)`, `put(Info)` on the entries of
    /// `getProviderKeyAt(i)` / `getProviderInstanceAt(i)`).
    pub fn put_entry(&mut self, key: ProviderMapKey, instance: ProviderInstance) -> &mut Self {
        self.map.insert(key, instance);
        self
    }

    /// Java `contains(Class)`.
    pub fn contains_native<P: TransitiveInfoProvider>(&self) -> bool {
        self.map
            .contains_key(&ProviderMapKey::Class(TypeId::of::<P>()))
    }

    /// Java `contains(Provider.Key)`.
    pub fn contains_info(&self, key: &ProviderKey) -> bool {
        self.map.contains_key(&ProviderMapKey::Info(key.clone()))
    }

    /// Java `getProvider(Class)`.
    pub fn get_provider<P: TransitiveInfoProvider>(&self) -> Option<&P> {
        match self.map.get(&ProviderMapKey::Class(TypeId::of::<P>()))? {
            ProviderInstance::Native(p) => p.as_any().downcast_ref::<P>(),
            _ => None,
        }
    }

    /// Java `getProvider(Provider.Key)`.
    pub fn get_info(&self, key: &ProviderKey) -> Option<&OwnedFrozenValue> {
        match self.map.get(&ProviderMapKey::Info(key.clone()))? {
            ProviderInstance::Info(v) => Some(v),
            _ => None,
        }
    }

    pub fn build(&self) -> TransitiveInfoProviderMap {
        TransitiveInfoProviderMap {
            map: self.map.clone(),
        }
    }
}
