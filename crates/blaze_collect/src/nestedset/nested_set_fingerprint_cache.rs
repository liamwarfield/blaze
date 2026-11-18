//! Port of `NestedSetFingerprintCache.java`.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_util::check_argument;
use blaze_util::fingerprint::{DigestHashFunction, Fingerprint};
use parking_lot::Mutex;

use super::NestedSet;
use super::digest_map::DigestMap;
use super::nested_set::Children;
use crate::command_line_item::{
    CommandLineItem, DEFAULT, ExceptionlessMapFn, MapFn, MapFnError, ParametrizedMapFn,
};

// Java: com/google/devtools/build/lib/collect/nestedset/NestedSetFingerprintCache.java:32 EMPTY_SET_DIGEST
const EMPTY_SET_DIGEST: i32 = 104_395_303;

/// Java keys `mapFnToDigestMap` on the map fn object: identity for singleton map fns,
/// `equals`/`hashCode` for `ParametrizedMapFn`s.
enum MapFnKey {
    /// Concrete type and address of a `'static` singleton.
    Instance(TypeId, usize),
    Parametrized(TypeId, &'static dyn ParametrizedMapFn),
}

impl PartialEq for MapFnKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (MapFnKey::Instance(t1, a1), MapFnKey::Instance(t2, a2)) => t1 == t2 && a1 == a2,
            (MapFnKey::Parametrized(t1, p1), MapFnKey::Parametrized(t2, p2)) => {
                let p2: &dyn Any = *p2;
                t1 == t2 && p1.equals(p2)
            }
            _ => false,
        }
    }
}

impl Eq for MapFnKey {}

impl Hash for MapFnKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            MapFnKey::Instance(t, a) => {
                t.hash(state);
                a.hash(state);
            }
            MapFnKey::Parametrized(t, p) => {
                t.hash(state);
                p.hash_code().hash(state);
            }
        }
    }
}

struct CacheState {
    /// Memoize the subresults. We have to have one cache per type of command item map function.
    map_fn_to_digest_map: HashMap<MapFnKey, Arc<DigestMap>>,
    seen_map_fns: HashSet<TypeId>,
    seen_parametrized_map_fns: HashMap<TypeId, i32>,
}

impl CacheState {
    fn new() -> Self {
        CacheState {
            map_fn_to_digest_map: HashMap::new(),
            seen_map_fns: HashSet::new(),
            seen_parametrized_map_fns: HashMap::new(),
        }
    }
}

/// Computes fingerprints for nested sets, reusing sub-computations from children.
///
/// Map fns are passed as `&'static` references: Java requires non-parametrized map fns to be
/// singletons and keys its memo tables on the instance, so a `'static` address is the instance
/// identity. Parametrized map fns are keyed by `ParametrizedMapFn::equals`/`hash_code`.
// Java: com/google/devtools/build/lib/collect/nestedset/NestedSetFingerprintCache.java:31 NestedSetFingerprintCache
pub struct NestedSetFingerprintCache {
    state: Mutex<CacheState>,
    /// Test hook replacing Java's overridable `addToFingerprint` (NestedSetFingerprintCacheTest).
    #[cfg(test)]
    pub(crate) fingerprinted: Mutex<Vec<String>>,
}

impl NestedSetFingerprintCache {
    pub fn new() -> Self {
        NestedSetFingerprintCache {
            state: Mutex::new(CacheState::new()),
            #[cfg(test)]
            fingerprinted: Mutex::new(Vec::new()),
        }
    }

    /// Fingerprints with `CommandLineItem.MapFn.DEFAULT`.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetFingerprintCache.java:40 NestedSetFingerprintCache.addNestedSetToFingerprint
    pub fn add_nested_set_to_fingerprint<T>(
        &self,
        fingerprint: &mut Fingerprint,
        nested_set: &NestedSet<T>,
    ) -> Result<(), MapFnError>
    where
        T: CommandLineItem + Clone + Eq + Hash + Send + Sync + 'static,
    {
        self.add_nested_set_to_fingerprint_with_map_fn(&DEFAULT, fingerprint, nested_set)
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetFingerprintCache.java:45 NestedSetFingerprintCache.addNestedSetToFingerprint
    pub fn add_nested_set_to_fingerprint_exceptionless<T>(
        &self,
        map_fn: &'static dyn ExceptionlessMapFn<T>,
        fingerprint: &mut Fingerprint,
        nested_set: &NestedSet<T>,
    ) where
        T: Clone + Eq + Hash + Send + Sync + 'static,
    {
        let map_fn: &'static dyn MapFn<T> = map_fn;
        if let Err(e) =
            self.add_nested_set_to_fingerprint_with_map_fn(map_fn, fingerprint, nested_set)
        {
            // addNestedSetToFingerprint only throws these exceptions if mapFn does.
            panic!("IllegalStateException: {e}");
        }
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetFingerprintCache.java:57 NestedSetFingerprintCache.addNestedSetToFingerprint
    pub fn add_nested_set_to_fingerprint_with_map_fn<T>(
        &self,
        map_fn: &'static dyn MapFn<T>,
        fingerprint: &mut Fingerprint,
        nested_set: &NestedSet<T>,
    ) -> Result<(), MapFnError>
    where
        T: Clone + Eq + Hash + Send + Sync + 'static,
    {
        // Only top-level nested sets can be empty, so we can bail here
        if nested_set.is_empty() {
            fingerprint.add_int(EMPTY_SET_DIGEST);
            return Ok(());
        }
        let digest_map = self.compute_digest_map_if_absent(map_fn);
        fingerprint.add_int(nested_set.get_order().ordinal());
        let children = nested_set.get_children();
        self.add_children_to_fingerprint(map_fn, fingerprint, &digest_map, children)
    }

    /// Java: `mapFnToDigestMap.computeIfAbsent(mapFn, this::newDigestMap)`.
    fn compute_digest_map_if_absent<T: 'static>(
        &self,
        map_fn: &'static dyn MapFn<T>,
    ) -> Arc<DigestMap> {
        let as_any: &'static dyn Any = map_fn;
        let type_id = as_any.type_id();
        let key = match map_fn.as_parametrized_map_fn() {
            Some(p) => MapFnKey::Parametrized(type_id, p),
            None => {
                MapFnKey::Instance(type_id, map_fn as *const dyn MapFn<T> as *const () as usize)
            }
        };
        let mut state = self.state.lock();
        if let Some(digest_map) = state.map_fn_to_digest_map.get(&key) {
            return Arc::clone(digest_map);
        }
        let digest_map = Arc::new(Self::new_digest_map(&mut state, map_fn, type_id));
        state
            .map_fn_to_digest_map
            .insert(key, Arc::clone(&digest_map));
        digest_map
    }

    /// Returns a description of the nested set for the purpose of reporting fingerprint
    /// mismatches.
    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetFingerprintCache.java:71 NestedSetFingerprintCache.describedNestedSetFingerprint
    pub fn described_nested_set_fingerprint<T: Clone + Eq + Hash + 'static>(
        map_fn: &dyn ExceptionlessMapFn<T>,
        nested_set: &NestedSet<T>,
    ) -> String {
        if nested_set.is_empty() {
            return "<empty>".to_string();
        }
        let mut sb = String::new();
        sb.push_str("order: ");
        write!(sb, "{}", nested_set.get_order()).unwrap();
        sb.push_str(
            " (fingerprinting considers internal nested set structure, which is not reflected in \
             values reported below)\n",
        );
        let list = nested_set.to_list();
        writeln!(sb, "size: {}", list.len()).unwrap();
        for item in list.iter() {
            sb.push_str("  ");
            let result = map_fn.expand_to_command_line(item, &mut |s| {
                sb.push_str(s);
                sb.push_str(", ");
            });
            if let Err(e) = result {
                panic!("IllegalStateException: ExceptionlessMapFn threw: {e}");
            }
            sb.push('\n');
        }
        sb
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetFingerprintCache.java:92 NestedSetFingerprintCache.clear
    pub fn clear(&self) {
        let mut state = self.state.lock();
        *state = CacheState::new();
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetFingerprintCache.java:99 NestedSetFingerprintCache.addToFingerprint
    fn add_children_to_fingerprint<T>(
        &self,
        map_fn: &dyn MapFn<T>,
        fingerprint: &mut Fingerprint,
        digest_map: &DigestMap,
        children: &Children<T>,
    ) -> Result<(), MapFnError>
    where
        T: Clone + Eq + Hash + Send + Sync + 'static,
    {
        match children {
            Children::Array(array) => {
                if !digest_map.read_digest(array, fingerprint) {
                    let mut children_fingerprint = Fingerprint::new();
                    for child in array.iter() {
                        self.add_children_to_fingerprint(
                            map_fn,
                            &mut children_fingerprint,
                            digest_map,
                            child,
                        )?;
                    }
                    digest_map.insert_and_read_digest(
                        array,
                        &mut children_fingerprint,
                        fingerprint,
                    );
                }
                Ok(())
            }
            Children::Leaf(object) => self.add_to_fingerprint(map_fn, fingerprint, object),
            Children::Empty => unreachable!("only top-level nested sets can be empty"),
        }
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetFingerprintCache.java:119 NestedSetFingerprintCache.addToFingerprint
    fn add_to_fingerprint<T: 'static>(
        &self,
        map_fn: &dyn MapFn<T>,
        fingerprint: &mut Fingerprint,
        object: &T,
    ) -> Result<(), MapFnError> {
        // Java: `mapFn.expandToCommandLine(object, fingerprint::addString)`. Invariant: the strings
        // a map function produces (plain strings, exec paths, `map_each` results) are internal
        // strings (`blaze_util::string_encoding`), as in Java; see `Fingerprint::add_string`.
        map_fn.expand_to_command_line(object, &mut |s| {
            fingerprint.add_string(s);
        })?;
        #[cfg(test)]
        if let Some(s) = (object as &dyn Any).downcast_ref::<String>() {
            self.fingerprinted.lock().push(s.clone());
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/collect/nestedset/NestedSetFingerprintCache.java:129 NestedSetFingerprintCache.newDigestMap
    fn new_digest_map<T: 'static>(
        state: &mut CacheState,
        map_fn: &dyn MapFn<T>,
        map_fn_class: TypeId,
    ) -> DigestMap {
        if let Some(parametrized) = map_fn.as_parametrized_map_fn() {
            let count = state
                .seen_parametrized_map_fns
                .entry(map_fn_class)
                .or_insert(0);
            *count += 1;
            let occurrences = *count;
            check_argument!(
                occurrences <= parametrized.max_instances_allowed(),
                "Too many instances of CommandLineItem.ParametrizedMapFn '{}' detected. Please \
                 construct fewer instances.",
                map_fn.class_name()
            );
        } else {
            check_argument!(
                state.seen_map_fns.insert(map_fn_class),
                "Illegal mapFn implementation: '{}'. CommandLineItem.MapFn instances must be \
                 singletons.Please see CommandLineItem.ParametrizedMapFn for an alternative.",
                map_fn.class_name()
            );
        }
        // TODO(b/112460990): Use the value from DigestHashFunction.getDefault(), but check for
        // contention.
        DigestMap::new(DigestHashFunction::Sha256, 1024)
    }
}

#[cfg(test)]
mod nested_set_fingerprint_cache_test {
    //! Port of `NestedSetFingerprintCacheTest.java`.
    use super::*;
    use crate::NestedSetBuilder;
    use std::collections::HashMap;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn s(x: &str) -> String {
        x.to_string()
    }

    fn counts(cache: &NestedSetFingerprintCache) -> HashMap<String, usize> {
        let mut m = HashMap::new();
        for x in cache.fingerprinted.lock().iter() {
            *m.entry(x.clone()).or_insert(0) += 1;
        }
        m
    }

    #[test]
    fn test_basic() {
        let cache = NestedSetFingerprintCache::new();
        let nested_set = NestedSetBuilder::<String>::stable_order()
            .add(s("a"))
            .add(s("b"))
            .build();

        // This test does reimplement the inner algorithm of the cache, but serves
        // as a simple check that the basic operations do something sensible
        let mut fingerprint = Fingerprint::new();
        fingerprint.add_int(nested_set.get_order().ordinal());
        let mut sub_fingerprint = Fingerprint::new();
        sub_fingerprint.add_string("a");
        sub_fingerprint.add_string("b");
        fingerprint.add_bytes(&sub_fingerprint.digest_and_reset());
        let control_digest = fingerprint.hex_digest_and_reset();

        let mut nested_set_fingerprint = Fingerprint::new();
        cache
            .add_nested_set_to_fingerprint(&mut nested_set_fingerprint, &nested_set)
            .unwrap();
        let nested_set_digest = nested_set_fingerprint.hex_digest_and_reset();

        assert_eq!(control_digest, nested_set_digest);
    }

    #[test]
    fn test_only_fingerprinted_once_per_string() {
        let cache = NestedSetFingerprintCache::new();
        // Leaving leaf nodes with a single item will defeat this check
        // The nested set builder will effectively inline single-item objects into their parent,
        // meaning they will get hashed multiple times.
        let a = NestedSetBuilder::<String>::stable_order()
            .add(s("a0"))
            .add(s("a1"))
            .build();
        let b = NestedSetBuilder::<String>::stable_order()
            .add(s("b0"))
            .add(s("b1"))
            .build();
        let c = NestedSetBuilder::<String>::stable_order()
            .add(s("c"))
            .add_transitive(&a)
            .add_transitive(&b)
            .build();
        let d = NestedSetBuilder::<String>::stable_order()
            .add(s("d"))
            .add_transitive(&a)
            .add_transitive(&b)
            .build();
        let e = NestedSetBuilder::<String>::stable_order()
            .add(s("e"))
            .add_transitive(&c)
            .add_transitive(&d)
            .build();
        cache
            .add_nested_set_to_fingerprint(&mut Fingerprint::new(), &e)
            .unwrap();
        let counts = counts(&cache);
        let mut keys: Vec<&str> = counts.keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(keys, vec!["a0", "a1", "b0", "b1", "c", "d", "e"]);
        for count in counts.values() {
            assert_eq!(*count, 1);
        }
    }

    fn mapped(s: &String, args: &mut dyn FnMut(&str)) {
        args(&format!("{s}_mapped"));
    }

    #[test]
    fn test_map_fn() {
        let cache = NestedSetFingerprintCache::new();
        // Make sure that the map function assigns completely different key spaces
        let a = NestedSetBuilder::<String>::stable_order()
            .add(s("a0"))
            .add(s("a1"))
            .build();

        let mut default_map_fn_fingerprint = Fingerprint::new();
        cache
            .add_nested_set_to_fingerprint(&mut default_map_fn_fingerprint, &a)
            .unwrap();
        let mut explicit_default_map_fn_fingerprint = Fingerprint::new();
        cache
            .add_nested_set_to_fingerprint_with_map_fn(
                &DEFAULT,
                &mut explicit_default_map_fn_fingerprint,
                &a,
            )
            .unwrap();
        let mut mapped_fingerprint = Fingerprint::new();
        cache.add_nested_set_to_fingerprint_exceptionless(&mapped, &mut mapped_fingerprint, &a);

        let default_map_fn_digest = default_map_fn_fingerprint.hex_digest_and_reset();
        let explicit_default_map_fn_digest =
            explicit_default_map_fn_fingerprint.hex_digest_and_reset();
        let mapped_digest = mapped_fingerprint.hex_digest_and_reset();
        assert_eq!(default_map_fn_digest, explicit_default_map_fn_digest);
        assert_ne!(mapped_digest, default_map_fn_digest);

        let counts = counts(&cache);
        let mut keys: Vec<&str> = counts.keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(keys, vec!["a0", "a1"]);
        for count in counts.values() {
            assert_eq!(*count, 2);
        }
    }

    struct IntParametrizedMapFn {
        i: i32,
    }

    impl MapFn<String> for IntParametrizedMapFn {
        fn expand_to_command_line(
            &self,
            object: &String,
            args: &mut dyn FnMut(&str),
        ) -> Result<(), MapFnError> {
            args(&format!("{object}{}", self.i));
            Ok(())
        }

        fn as_parametrized_map_fn(&self) -> Option<&dyn ParametrizedMapFn> {
            Some(self)
        }
    }

    impl ExceptionlessMapFn<String> for IntParametrizedMapFn {}

    impl ParametrizedMapFn for IntParametrizedMapFn {
        fn equals(&self, other: &dyn Any) -> bool {
            match other.downcast_ref::<IntParametrizedMapFn>() {
                Some(that) => self.i == that.i,
                None => false,
            }
        }

        fn hash_code(&self) -> i32 {
            self.i
        }

        fn max_instances_allowed(&self) -> i32 {
            2
        }
    }

    fn simple_expand(o: &String, args: &mut dyn FnMut(&str)) {
        args(&format!("{o}_mapped"));
    }

    fn simple_expand2(o: &String, args: &mut dyn FnMut(&str)) {
        args(&format!("{o}_mapped2"));
    }

    fn assert_illegal_argument(f: impl FnOnce()) {
        let err = catch_unwind(AssertUnwindSafe(f)).expect_err("expected IllegalArgumentException");
        let msg = err
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(msg.starts_with("IllegalArgumentException: "), "{msg}");
    }

    #[test]
    fn test_multiple_instances_of_map_fn_throws() {
        let cache = NestedSetFingerprintCache::new();
        let nested_set = NestedSetBuilder::<String>::stable_order()
            .add(s("a0"))
            .add(s("a1"))
            .build();

        // Make sure a normal method reference doesn't get denied.
        for _ in 0..2 {
            cache.add_nested_set_to_fingerprint_exceptionless(
                &simple_expand,
                &mut Fingerprint::new(),
                &nested_set,
            );
        }

        // Try again to make sure a second function item is a distinct map fn.
        for _ in 0..2 {
            cache.add_nested_set_to_fingerprint_exceptionless(
                &simple_expand2,
                &mut Fingerprint::new(),
                &nested_set,
            );
        }

        // Make sure a non-capturing lambda doesn't get denied
        type ExpandFn = fn(&String, &mut dyn FnMut(&str));
        static NON_CAPTURING: ExpandFn =
            |s: &String, args: &mut dyn FnMut(&str)| args(&format!("{s}_mapped"));
        for _ in 0..2 {
            cache.add_nested_set_to_fingerprint_exceptionless(
                &NON_CAPTURING,
                &mut Fingerprint::new(),
                &nested_set,
            );
        }

        // Make sure a ParametrizedMapFn doesn't get denied until it exceeds its instance count
        // PORT: map fns are `&'static`; parametrized instances are leaked like interned ones.
        let p1: &'static IntParametrizedMapFn = Box::leak(Box::new(IntParametrizedMapFn { i: 1 }));
        let p2: &'static IntParametrizedMapFn = Box::leak(Box::new(IntParametrizedMapFn { i: 2 }));
        let p3: &'static IntParametrizedMapFn = Box::leak(Box::new(IntParametrizedMapFn { i: 3 }));
        cache.add_nested_set_to_fingerprint_exceptionless(p1, &mut Fingerprint::new(), &nested_set);
        cache.add_nested_set_to_fingerprint_exceptionless(p2, &mut Fingerprint::new(), &nested_set);
        assert_illegal_argument(|| {
            cache.add_nested_set_to_fingerprint_exceptionless(
                p3,
                &mut Fingerprint::new(),
                &nested_set,
            )
        });

        // Make sure a capturing method reference gets denied: two instances of the same
        // capturing closure type are distinct map fns.
        assert_illegal_argument(|| {
            for _ in 0..2 {
                let str_ = s("hello");
                let expand: &'static _ = Box::leak(Box::new(
                    move |other: &String, args: &mut dyn FnMut(&str)| {
                        args(&format!("{str_}{other}"))
                    },
                ));
                cache.add_nested_set_to_fingerprint_exceptionless(
                    expand,
                    &mut Fingerprint::new(),
                    &nested_set,
                );
            }
        });

        // Do make sure that a capturing lambda gets denied.
        assert_illegal_argument(|| {
            for i in 0..2 {
                let captured_variable = i;
                let f: &'static _ =
                    Box::leak(Box::new(move |s: &String, args: &mut dyn FnMut(&str)| {
                        args(&format!("{s}{captured_variable}"))
                    }));
                cache.add_nested_set_to_fingerprint_exceptionless(
                    f,
                    &mut Fingerprint::new(),
                    &nested_set,
                );
            }
        });
    }

    #[test]
    fn test_parametrized_map_fn_equal_instances_share_cache() {
        // Not a Java test: equal parametrized instances are one map fn (Java HashMap semantics).
        let cache = NestedSetFingerprintCache::new();
        let nested_set = NestedSetBuilder::<String>::stable_order()
            .add(s("a0"))
            .add(s("a1"))
            .build();
        for _ in 0..3 {
            let p: &'static IntParametrizedMapFn =
                Box::leak(Box::new(IntParametrizedMapFn { i: 1 }));
            cache.add_nested_set_to_fingerprint_exceptionless(
                p,
                &mut Fingerprint::new(),
                &nested_set,
            );
        }
    }
}
