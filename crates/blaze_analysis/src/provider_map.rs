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
// PORT: skipped, Java class hierarchy (a native provider is keyed by its Rust type, which has no subclasses): TransitiveInfoProviderEffectiveClassHelper.<init>, TransitiveInfoProviderEffectiveClassHelper.get
// PORT: skipped, Java class hierarchy: TransitiveInfoProviderEffectiveClassHelper.findEffectiveProviderClass, TransitiveInfoProviderEffectiveClassHelper.getDirectImplementations
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
/// Also `TransitiveInfoProviderMapImpl`, its only implementation (an `IndexMap` keeps the
/// insertion order that the Java arrays keep).
// PORT: skipped, serialization (remote analysis caching is not ported): TransitiveInfoProviderMapImpl.valueSharingCodec
// PORT: skipped, serialization: TransitiveInfoProviderMapImpl.ValueSharingCodec.autoRegister, TransitiveInfoProviderMapImpl.ValueSharingCodec.getEncodedClass
// PORT: skipped, serialization: TransitiveInfoProviderMapImpl.ValueSharingCodec.serialize, TransitiveInfoProviderMapImpl.ValueSharingCodec.deserializeDeferred
// PORT: skipped, serialization: TransitiveInfoProviderMapImpl.ValueSharingCodec.DeferredKeysCodec.getEncodedClass, TransitiveInfoProviderMapImpl.ValueSharingCodec.DeferredKeysCodec.autoRegister
// PORT: skipped, serialization: TransitiveInfoProviderMapImpl.ValueSharingCodec.DeferredKeysCodec.serialize, TransitiveInfoProviderMapImpl.ValueSharingCodec.DeferredKeysCodec.deserializeDeferred
// PORT: skipped, serialization: TransitiveInfoProviderMapImpl.ValueSharingCodec.DeferredKeysCodec.ArrayFieldSetter.<init>, TransitiveInfoProviderMapImpl.ValueSharingCodec.DeferredKeysCodec.ArrayFieldSetter.set
// PORT: skipped, serialization: TransitiveInfoProviderMapImpl.Codec.getEncodedClass, TransitiveInfoProviderMapImpl.Codec.serialize, TransitiveInfoProviderMapImpl.Codec.deserializeDeferred
// PORT: skipped, serialization: TransitiveInfoProviderMapImpl.DeserializationBuilder.call, TransitiveInfoProviderMapImpl.DeserializationBuilder.setKeys, TransitiveInfoProviderMapImpl.DeserializationBuilder.setValues
// Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapImpl.java:40 TransitiveInfoProviderMapImpl
// Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMap.java:23 TransitiveInfoProviderMap
#[derive(Clone, Debug, Default)]
pub struct TransitiveInfoProviderMap {
    map: IndexMap<ProviderMapKey, ProviderInstance>,
}

impl TransitiveInfoProviderMap {
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapImpl.java:45 TransitiveInfoProviderMapImpl.<init>
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapImpl.java:49 TransitiveInfoProviderMapImpl.empty
    pub fn builder() -> TransitiveInfoProviderMapBuilder {
        TransitiveInfoProviderMapBuilder::default()
    }

    /// Java `getProvider(Class)`.
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapImpl.java:70 TransitiveInfoProviderMapImpl.getProvider
    pub fn get_provider<P: TransitiveInfoProvider>(&self) -> Option<&P> {
        match self.map.get(&ProviderMapKey::Class(TypeId::of::<P>()))? {
            ProviderInstance::Native(p) => p.as_any().downcast_ref::<P>(),
            _ => None,
        }
    }

    /// Java `get(Provider.Key)`.
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapImpl.java:79 TransitiveInfoProviderMapImpl.get
    pub fn get(&self, key: &ProviderKey) -> Option<&OwnedFrozenValue> {
        match self.map.get(&ProviderMapKey::Info(key.clone()))? {
            ProviderInstance::Info(v) => Some(v),
            _ => None,
        }
    }

    /// Java `get(String legacyKey)`.
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapImpl.java:85 TransitiveInfoProviderMapImpl.get
    pub fn get_legacy(&self, key: &str) -> Option<&OwnedFrozenValue> {
        match self.map.get(&ProviderMapKey::Legacy(key.to_owned()))? {
            ProviderInstance::Legacy(v) => Some(v),
            _ => None,
        }
    }

    /// Java `getProviderCount()`.
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapImpl.java:91 TransitiveInfoProviderMapImpl.getProviderCount
    pub fn get_provider_count(&self) -> usize {
        self.map.len()
    }

    /// Java `getProviderKeyAt(i)` / `getProviderInstanceAt(i)`.
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapImpl.java:96 TransitiveInfoProviderMapImpl.getProviderKeyAt
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapImpl.java:101 TransitiveInfoProviderMapImpl.getProviderInstanceAt
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
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:47 TransitiveInfoProviderMapBuilder.put
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:82 TransitiveInfoProviderMapBuilder.add
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
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:62 TransitiveInfoProviderMapBuilder.put
    pub fn put_info(&mut self, info: OwnedFrozenValue) -> &mut Self {
        let key = info_provider_key(info.value())
            .unwrap_or_else(|| panic!("IllegalArgumentException: not an Info: {}", info.value()));
        self.map
            .insert(ProviderMapKey::Info(key), ProviderInstance::Info(info));
        self
    }

    /// Java `put(String legacyKey, Object value)`.
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:74 TransitiveInfoProviderMapBuilder.put
    pub fn put_legacy(&mut self, key: &str, value: OwnedFrozenValue) -> &mut Self {
        self.map.insert(
            ProviderMapKey::Legacy(key.to_owned()),
            ProviderInstance::Legacy(value),
        );
        self
    }

    /// Java `addAll(TransitiveInfoProviderMap)`.
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:87 TransitiveInfoProviderMapBuilder.addAll
    pub fn add_all(&mut self, other: &TransitiveInfoProviderMap) -> &mut Self {
        for (k, v) in &other.map {
            self.map.insert(k.clone(), v.clone());
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:38 TransitiveInfoProviderMapBuilder.contains
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
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:34 TransitiveInfoProviderMapBuilder.contains
    pub fn contains_native<P: TransitiveInfoProvider>(&self) -> bool {
        self.map
            .contains_key(&ProviderMapKey::Class(TypeId::of::<P>()))
    }

    /// Java `contains(Provider.Key)`.
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:42 TransitiveInfoProviderMapBuilder.contains
    pub fn contains_info(&self, key: &ProviderKey) -> bool {
        self.map.contains_key(&ProviderMapKey::Info(key.clone()))
    }

    /// Java `getProvider(Class)`.
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:95 TransitiveInfoProviderMapBuilder.getProvider
    pub fn get_provider<P: TransitiveInfoProvider>(&self) -> Option<&P> {
        match self.map.get(&ProviderMapKey::Class(TypeId::of::<P>()))? {
            ProviderInstance::Native(p) => p.as_any().downcast_ref::<P>(),
            _ => None,
        }
    }

    /// Java `getProvider(Provider.Key)`.
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:100 TransitiveInfoProviderMapBuilder.getProvider
    pub fn get_info(&self, key: &ProviderKey) -> Option<&OwnedFrozenValue> {
        match self.map.get(&ProviderMapKey::Info(key.clone()))? {
            ProviderInstance::Info(v) => Some(v),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapBuilder.java:104 TransitiveInfoProviderMapBuilder.build
    // Java: com/google/devtools/build/lib/analysis/TransitiveInfoProviderMapImpl.java:53 TransitiveInfoProviderMapImpl.create
    pub fn build(&self) -> TransitiveInfoProviderMap {
        TransitiveInfoProviderMap {
            map: self.map.clone(),
        }
    }
}
