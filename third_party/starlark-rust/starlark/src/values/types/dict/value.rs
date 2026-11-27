/*
 * Copyright 2018 The Starlark in Rust Authors.
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::any::TypeId;
use std::cell::Cell; // BLAZE PATCH 0018
use std::cell::Ref;
use std::cell::RefCell;
use std::cell::RefMut;
use std::fmt;
use std::fmt::Debug;
use std::fmt::Display;
use std::hash::Hash;
use std::hash::Hasher;
use std::mem;
use std::ops::Deref;

use allocative::Allocative;
use display_container::fmt_keyed_container;
use serde::Serialize;
use starlark::register_avalue_simple_frozen;
use starlark_derive::StarlarkPagable;
use starlark_derive::starlark_value;
use starlark_map::Equivalent;

use crate as starlark;
use crate::any::ProvidesStaticType;
use crate::cast::transmute;
use crate::coerce::Coerce;
use crate::coerce::coerce;
use crate::collections::Hashed;
use crate::collections::SmallMap;
use crate::environment::Methods;
use crate::hint::unlikely;
use crate::static_starlark_value;
use crate::typing::Ty;
use crate::util::refcell::unleak_borrow;
use crate::values::AllocFrozenValue;
use crate::values::AllocValue;
use crate::values::Freeze;
use crate::values::FreezeResult;
use crate::values::Freezer;
use crate::values::FrozenHeap;
use crate::values::FrozenStringValue;
use crate::values::FrozenValue;
use crate::values::Heap;
use crate::values::StarlarkValue;
use crate::values::StringValue;
use crate::values::Trace;
use crate::values::Value;
use crate::values::ValueLike;
use crate::values::comparison::equals_small_map;
use crate::values::dict::DictRef;
use crate::values::error::ValueError;
use crate::values::string::str_type::hash_string_value;
use crate::values::type_repr::StarlarkTypeRepr;
use crate::values::types::dict::dict_type::DictType;

#[derive(
    Clone,
    Default,
    Trace,
    Debug,
    ProvidesStaticType,
    Allocative,
    StarlarkPagable
)]
pub(crate) struct DictGen<T>(pub(crate) T);

impl<'v, T: DictLike<'v>> Display for DictGen<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_keyed_container(f, "{", "}", ": ", self.0.content().iter())
    }
}

impl<'v> Display for Dict<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_keyed_container(f, "{", "}", ": ", self.iter())
    }
}

/// Define the dict type.
#[derive(Clone, Default, Trace, Debug, ProvidesStaticType, Allocative)]
#[repr(transparent)]
pub struct Dict<'v> {
    /// The data stored by the dictionary. The keys must all be hashable values.
    pub(crate) content: SmallMap<Value<'v>, Value<'v>>,
}

impl<'v> StarlarkTypeRepr for Dict<'v> {
    type Canonical = <DictType<FrozenValue, FrozenValue> as StarlarkTypeRepr>::Canonical;

    fn starlark_type_repr() -> Ty {
        Self::Canonical::starlark_type_repr()
    }
}

#[derive(Clone, Default, Debug, ProvidesStaticType, Allocative, StarlarkPagable)]
#[repr(transparent)]
pub(crate) struct FrozenDictData {
    /// The data stored by the dictionary. The keys must all be hashable values.
    pub(crate) content: SmallMap<FrozenValue, FrozenValue>,
}

/// Alias is used in `StarlarkDocs` derive.
pub(crate) type FrozenDict = DictGen<FrozenDictData>;

pub(crate) type MutableDict<'v> = DictGen<MutableDictData<'v>>; // BLAZE PATCH 0018

/// BLAZE PATCH 0018: the mutable dict's payload (was `RefCell<Dict>`), with a flag for an
/// immutable dict on a mutable heap (Java `Dict.immutableCopyOf`). Mutation fails; freezing makes
/// it an ordinary frozen dict.
#[derive(Trace, ProvidesStaticType, Allocative)]
pub(crate) struct MutableDictData<'v> {
    pub(crate) cell: RefCell<Dict<'v>>,
    #[allocative(skip)]
    pub(crate) immutable: Cell<bool>,
    /// BLAZE PATCH 0063: for a key-tracking dict (Java `Dict.ImmutableKeyTrackingDict`), the
    /// indexes of the keys accessed so far, in order of first access; `None` for other dicts.
    #[allocative(skip)]
    pub(crate) accessed_keys: RefCell<Option<Vec<usize>>>, // BLAZE PATCH 0063
}

// BLAZE PATCH 0018: same output as the `RefCell<Dict>` payload before.
impl<'v> Debug for MutableDictData<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.immutable.get() {
            f.debug_struct("ImmutableDict")
                .field("value", &self.cell)
                .finish()
        } else {
            Debug::fmt(&self.cell, f)
        }
    }
}

impl<'v> MutableDictData<'v> {
    // BLAZE PATCH 0018
    pub(crate) fn new(dict: Dict<'v>) -> Self {
        MutableDictData {
            cell: RefCell::new(dict),
            immutable: Cell::new(false),
            accessed_keys: RefCell::new(None), // BLAZE PATCH 0063
        }
    }

    /// BLAZE PATCH 0063: records an access of the key at `index` (Java
    /// `ImmutableKeyTrackingDict.accessedKeys.add`, an `ImmutableSet.Builder`).
    fn record_index(&self, index: usize) {
        if let Some(accessed) = self.accessed_keys.borrow_mut().as_mut() {
            if !accessed.contains(&index) {
                accessed.push(index);
            }
        }
    }
}

/// BLAZE PATCH 0063 (round 2): the keys Java's `x.equals(y)` reads from a key-tracking `y`.
/// `Dict.equals` is `contents.equals(y)`: an `ImmutableMap` `x` (`Maps.equalsImpl`) compares
/// `entrySet()`s, which records every key of `y`; a `LinkedHashMap` `x` (`AbstractMap.equals`)
/// returns at once on different sizes, else calls `y.get(k)` for its keys in order (recording a
/// present key) until the first missing key or unequal value.
// Java: net/starlark/java/eval/Dict.java:209 Dict.equals
fn record_equals_accesses<'v, X: DictLike<'v>>(
    x: &X,
    y: &MutableDictData<'v>,
    y_content: &SmallMap<Value<'v>, Value<'v>>,
) -> crate::Result<()> {
    if x.java_contents_immutable() {
        y.record_all_keys_access();
        return Ok(());
    }
    let x_content = x.content();
    if x_content.len() != y_content.len() {
        return Ok(());
    }
    for (xk, xv) in x_content.iter_hashed() {
        let Some(i) = y_content.get_index_of_hashed_by_value(xk.copied()) else {
            return Ok(());
        };
        y.record_index(i);
        let yv = *y_content.get_index(i).expect("index of a key").1;
        if !xv.equals(yv)? {
            return Ok(());
        }
    }
    Ok(())
}

/// BLAZE PATCH 0018: Java's error for mutating a frozen dict (Starlark.checkMutable).
pub(crate) fn frozen_dict_error() -> anyhow::Error {
    anyhow::anyhow!("trying to mutate a frozen dict value")
}

/// BLAZE PATCH 0018: an immutable dict with the given entries (Java
/// net/starlark/java/eval/Dict.java:495 `Dict.immutableCopyOf`, via `copyOf(null, ..)`). The
/// empty dict is the shared frozen empty dict (Java `Dict.empty()`). Otherwise the dict lives on
/// `heap` and keeps its keys and values as they are; every mutation fails with "trying to mutate
/// a frozen dict value", and [`dict_is_immutable`] reports it immutable.
pub fn alloc_immutable_dict<'v>(heap: Heap<'v>, dict: Dict<'v>) -> Value<'v> {
    if dict.content.is_empty() {
        return VALUE_EMPTY_FROZEN_DICT.to_frozen_value().to_value();
    }
    let data = MutableDictData::new(dict);
    data.immutable.set(true);
    heap.alloc_complex(DictGen(data))
}

/// BLAZE PATCH 0018: Java `Dict.isImmutable` for a dict value (Dict.java:180: its mutability is
/// frozen; keys and values are not looked at), `None` for other values. Frozen dicts and dicts
/// made by [`alloc_immutable_dict`] are immutable; other dicts are not.
pub fn dict_is_immutable(x: Value) -> Option<bool> {
    if let Some(d) = x.downcast_ref::<MutableDict>() {
        Some(d.0.immutable.get())
    } else if x.downcast_ref::<DictGen<FrozenDictData>>().is_some() {
        Some(true)
    } else {
        None
    }
}

/// BLAZE PATCH 0063: an immutable dict that records the keys accessed through it (Java
/// net/starlark/java/eval/Dict.java:789 `Dict.ImmutableKeyTrackingDict`, made by
/// `Dict.Builder.buildImmutableWithKeyTracking`). Only keys present in the dict are tracked:
/// `in` and `[]` and `get` record the key looked up when it is present (Java `containsKey`,
/// `get`); iterating, `repr`/`str`, copying (`dict(d)`, `|`, `|=`, `update(d)`) and `**d` record
/// every key (Java `keySet`, `entrySet`). `keys()`, `items()`, `values()`, `len` and `==` record
/// nothing (Java reads its `contents` there). Unlike Java's, an empty dict is a tracking dict too
/// (there is nothing to track in it).
pub fn alloc_immutable_key_tracking_dict<'v>(heap: Heap<'v>, dict: Dict<'v>) -> Value<'v> {
    let data = MutableDictData::new(dict);
    data.immutable.set(true);
    *data.accessed_keys.borrow_mut() = Some(Vec::new());
    heap.alloc_complex(DictGen(data))
}

/// BLAZE PATCH 0063: the keys accessed so far of a dict made by
/// [`alloc_immutable_key_tracking_dict`], in order of first access (Java
/// `ImmutableKeyTrackingDict.getAccessedKeys`); `None` for any other value.
pub fn dict_accessed_keys<'v>(x: Value<'v>) -> Option<Vec<Value<'v>>> {
    let d = x.downcast_ref::<MutableDict<'v>>()?;
    let accessed = d.0.accessed_keys.borrow();
    let accessed = accessed.as_ref()?;
    let content = d.0.cell.borrow();
    Some(
        accessed
            .iter()
            .map(|&i| *content.content.get_index(i).expect("a key of the dict").0)
            .collect(),
    )
}

/// BLAZE PATCH 0063: records every key of `x` as accessed when it is a key-tracking dict (Java
/// `keySet()` / `entrySet()` of `ImmutableKeyTrackingDict`).
pub fn dict_record_all_keys_accessed(x: Value) {
    if let Some(d) = x.downcast_ref::<MutableDict>() {
        d.0.record_all_keys_access();
    }
}

static_starlark_value!(
    pub(crate) VALUE_EMPTY_FROZEN_DICT: DictGen<FrozenDictData> = DictGen(FrozenDictData {
        content: SmallMap::new(),
    })
);

unsafe impl<'v> Coerce<Dict<'v>> for FrozenDictData {}

impl<'v> AllocValue<'v> for Dict<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(DictGen(MutableDictData::new(self))) // BLAZE PATCH 0018
    }
}

impl StarlarkTypeRepr for FrozenDictData {
    type Canonical = <DictType<FrozenValue, FrozenValue> as StarlarkTypeRepr>::Canonical;

    fn starlark_type_repr() -> Ty {
        Ty::dict(Ty::any(), Ty::any())
    }
}

impl AllocFrozenValue for FrozenDictData {
    fn alloc_frozen_value(self, heap: &FrozenHeap) -> FrozenValue {
        if self.content.is_empty() {
            VALUE_EMPTY_FROZEN_DICT.to_frozen_value()
        } else {
            heap.alloc_simple(DictGen(self))
        }
    }
}

impl<'v> Dict<'v> {
    pub(crate) fn is_dict_type(x: TypeId) -> bool {
        x == TypeId::of::<DictGen<FrozenDictData>>()
            || x == TypeId::of::<DictGen<MutableDictData<'static>>>() // BLAZE PATCH 0018
    }

    pub(crate) unsafe fn from_value_unchecked_mut(x: Value<'v>) -> RefMut<'v, Self> {
        unsafe {
            let dict = &x.downcast_ref_unchecked::<MutableDict<'v>>().0; // BLAZE PATCH 0018
            debug_assert!(!dict.immutable.get()); // BLAZE PATCH 0018
            dict.cell.borrow_mut() // BLAZE PATCH 0018
        }
    }
}

/// Helper type for lookups, not useful.
#[derive(Eq, PartialEq)]
pub struct ValueStr<'a>(pub(crate) &'a str);

impl<'a> Hash for ValueStr<'a> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_string_value(self.0, state)
    }
}

impl<'v> Equivalent<Value<'v>> for ValueStr<'_> {
    fn equivalent(&self, key: &Value<'v>) -> bool {
        key.unpack_str() == Some(self.0)
    }
}

impl Equivalent<FrozenValue> for ValueStr<'_> {
    fn equivalent(&self, key: &FrozenValue) -> bool {
        key.unpack_str() == Some(self.0)
    }
}

impl<'v> Dict<'v> {
    /// The result of calling `type()` on dictionaries.
    pub const TYPE: &'static str = "dict";

    /// Dict type string as Starlark frozen string value.
    pub fn get_type_value_static() -> FrozenStringValue {
        DictGen::<FrozenDictData>::get_type_value_static()
    }

    /// This function is deprecated.
    /// Use [`AllocDict`](crate::values::dict::AllocDict) or [`SmallMap`]
    /// to allocate a new dictionary on the heap.
    pub fn new(content: SmallMap<Value<'v>, Value<'v>>) -> Self {
        Self { content }
    }

    /// Number of elements in the dict.
    pub fn len(&self) -> usize {
        self.content.len()
    }

    /// Is the dict empty?
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Iterate through the key/value pairs in the dictionary.
    pub fn iter<'a>(&'a self) -> impl ExactSizeIterator<Item = (Value<'v>, Value<'v>)> + 'a {
        self.content.iter().map(|(l, r)| (*l, *r))
    }

    /// Iterate through the key/value pairs in the dictionary, but retaining the hash of the keys.
    pub fn iter_hashed<'a>(&'a self) -> impl Iterator<Item = (Hashed<Value<'v>>, Value<'v>)> + 'a
    where
        'v: 'a,
    {
        self.content.iter_hashed().map(|(l, r)| (l.copied(), *r))
    }

    /// Iterator over keys.
    pub fn keys<'a>(&'a self) -> impl Iterator<Item = Value<'v>> + 'a {
        self.content.keys().copied()
    }

    /// Iterator over keys.
    pub fn values<'a>(&'a self) -> impl Iterator<Item = Value<'v>> + 'a {
        self.content.values().copied()
    }

    /// Get the value associated with a particular key. Will be [`Err`] if the key is not hashable,
    /// and otherwise [`Some`] if the key exists in the dictionary and [`None`] otherwise.
    pub fn get(&self, key: Value<'v>) -> crate::Result<Option<Value<'v>>> {
        Ok(self.get_hashed(key.get_hashed()?))
    }

    /// Lookup the value by the given prehashed key.
    pub fn get_hashed(&self, key: Hashed<Value<'v>>) -> Option<Value<'v>> {
        self.content.get_hashed_by_value(key).copied()
    }

    /// Get the value associated with a particular string. Equivalent to allocating the
    /// string on the heap, turning it into a value, and looking up using that.
    pub fn get_str(&self, key: &str) -> Option<Value<'v>> {
        self.content.get(&ValueStr(key)).copied()
    }

    /// Like [`Dict::get_str`], but where you already have the hash.
    pub fn get_str_hashed(&self, key: Hashed<&str>) -> Option<Value<'v>> {
        self.content
            .get_hashed_by_value(Hashed::new_unchecked(key.hash(), ValueStr(key.key())))
            .copied()
    }

    /// Try to coerce all keys to strings.
    pub(crate) fn downcast_ref_key_string(&self) -> Option<&SmallMap<StringValue<'v>, Value<'v>>> {
        for &key in self.content.keys() {
            if unlikely(!key.is_str()) {
                return None;
            }
        }

        // Scary part: `SmallMap` has the same repr for `Value` and `StringValue`,
        // and we just checked above that all keys are strings.

        fn _assert_coerce<'v>(
            s: SmallMap<StringValue<'v>, Value<'v>>,
        ) -> SmallMap<Value<'v>, Value<'v>> {
            coerce(s)
        }

        Some(unsafe {
            transmute!(&SmallMap<Value, Value>, &SmallMap<StringValue, Value>, &self.content)
        })
    }

    /// Reserve capacity to insert `additional` elements without reallocating.
    pub fn reserve(&mut self, additional: usize) {
        self.content.reserve(additional);
    }

    /// Insert a key/value pair into the dictionary.
    pub fn insert_hashed(&mut self, key: Hashed<Value<'v>>, value: Value<'v>) -> Option<Value<'v>> {
        self.content.insert_hashed(key, value)
    }

    /// Remove given key from the dictionary.
    pub fn remove_hashed(&mut self, key: Hashed<Value<'v>>) -> Option<Value<'v>> {
        self.content.shift_remove_hashed(key.as_ref())
    }

    /// Remove all elements from the dictionary.
    pub fn clear(&mut self) {
        self.content.clear();
    }
}

impl FrozenDictData {
    /// Iterate through the key/value pairs in the dictionary.
    pub fn iter<'a>(&'a self) -> impl ExactSizeIterator<Item = (FrozenValue, FrozenValue)> + 'a {
        self.content.iter().map(|(l, r)| (*l, *r))
    }

    /// Get the value associated with a particular string. Equivalent to allocating the
    /// string on the heap, turning it into a value, and looking up using that.
    pub fn get_str(&self, key: &str) -> Option<FrozenValue> {
        self.content.get(&ValueStr(key)).copied()
    }
}

impl<'v> Freeze for MutableDict<'v> { // BLAZE PATCH 0018
    type Frozen = DictGen<FrozenDictData>;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<Self::Frozen> {
        let content = self.0.cell.into_inner().content.freeze(freezer)?; // BLAZE PATCH 0018
        Ok(DictGen(FrozenDictData { content }))
    }
}

trait DictLike<'v>: Debug + Allocative {
    type ContentRef<'a>: Deref<Target = SmallMap<Value<'v>, Value<'v>>>
    where
        Self: 'a,
        'v: 'a;
    fn content<'a>(&'a self) -> Self::ContentRef<'a>;
    // These functions are unsafe for the same reason
    // `StarlarkValue` iterator functions are unsafe.
    unsafe fn iter_start(&self);
    unsafe fn content_unchecked(&self) -> &SmallMap<Value<'v>, Value<'v>>;
    unsafe fn iter_stop(&self);
    fn set_at(&self, index: Hashed<Value<'v>>, value: Value<'v>) -> crate::Result<()>;
    /// BLAZE PATCH 0063: records an access of the key at `index` (key-tracking dicts only).
    fn record_key_index_access(&self, _index: usize) {}
    /// BLAZE PATCH 0063: records an access of every key (key-tracking dicts only).
    fn record_all_keys_access(&self) {}
    /// BLAZE PATCH 0063 (round 2): whether Java's `contents` of this dict is an `ImmutableMap`
    /// (a dict made immutable, `Dict.immutableCopyOf`) rather than a `LinkedHashMap` (a mutable
    /// dict, frozen or not).
    fn java_contents_immutable(&self) -> bool {
        false
    }
}

// BLAZE PATCH 0018: implemented on `MutableDictData` (was `RefCell<Dict>`, now `self.cell`).
impl<'v> DictLike<'v> for MutableDictData<'v> {
    type ContentRef<'a>
        = Ref<'a, SmallMap<Value<'v>, Value<'v>>>
    where
        Self: 'a,
        'v: 'a;

    fn content<'a>(&'a self) -> Ref<'a, SmallMap<Value<'v>, Value<'v>>> {
        Ref::map(self.cell.borrow(), |x| &x.content)
    }

    #[inline]
    unsafe fn iter_start(&self) {
        mem::forget(self.cell.borrow());
    }

    #[inline]
    unsafe fn iter_stop(&self) {
        unsafe {
            unleak_borrow(&self.cell);
        }
    }

    #[inline]
    unsafe fn content_unchecked(&self) -> &SmallMap<Value<'v>, Value<'v>> {
        unsafe {
            // SAFETY: this function contract is, caller must ensure that the value is borrowed.
            &self.cell.try_borrow_unguarded().ok().unwrap_unchecked().content
        }
    }

    // BLAZE PATCH 0063
    fn record_key_index_access(&self, index: usize) {
        self.record_index(index);
    }

    // BLAZE PATCH 0063 (round 2)
    fn java_contents_immutable(&self) -> bool {
        self.immutable.get()
    }

    // BLAZE PATCH 0063
    fn record_all_keys_access(&self) {
        if self.accessed_keys.borrow().is_some() {
            let len = self.cell.borrow().content.len();
            for i in 0..len {
                self.record_index(i);
            }
        }
    }

    fn set_at(&self, index: Hashed<Value<'v>>, alloc_value: Value<'v>) -> crate::Result<()> {
        if unlikely(self.immutable.get()) {
            return Err(crate::Error::new_other(frozen_dict_error())); // BLAZE PATCH 0018
        }
        match self.cell.try_borrow_mut() {
            Ok(mut xs) => {
                xs.content.insert_hashed(index, alloc_value);
                Ok(())
            }
            Err(_) => Err(crate::Error::new_other(ValueError::Bazel("dict value is temporarily immutable due to active for-loop iteration".to_owned()))), // BLAZE PATCH 0051
        }
    }
}

impl<'v> DictLike<'v> for FrozenDictData {
    type ContentRef<'a>
        = &'a SmallMap<Value<'v>, Value<'v>>
    where
        Self: 'a,
        'v: 'a;

    fn content<'a>(&'a self) -> &'a SmallMap<Value<'v>, Value<'v>> {
        coerce(&self.content)
    }

    unsafe fn iter_start(&self) {}

    unsafe fn iter_stop(&self) {}

    unsafe fn content_unchecked(&self) -> &SmallMap<Value<'v>, Value<'v>> {
        coerce(&self.content)
    }

    fn set_at(&self, _index: Hashed<Value<'v>>, _value: Value<'v>) -> crate::Result<()> {
        // BLAZE PATCH 0023: Java's text (Starlark.checkMutable).
        Err(crate::Error::new_other(frozen_dict_error()))
    }
}

// Register vtable for FrozenDict (special type not handled by #[starlark_value] macro, because V is not ValueLike).
register_avalue_simple_frozen!(FrozenDict);

starlark::methods_static!(DICT_METHODS = crate::values::types::dict::methods::dict_methods);

#[starlark_value(type = Dict::TYPE)]
impl<'v, T: DictLike<'v> + 'v> StarlarkValue<'v> for DictGen<T>
where
    Self: ProvidesStaticType<'v>,
{
    type Canonical = FrozenDict;

    fn get_methods() -> Option<&'static Methods> {
        Some(DICT_METHODS.methods())
    }

    fn collect_repr(&self, r: &mut String) {
        self.0.record_all_keys_access(); // BLAZE PATCH 0063: Java repr reads entrySet()
        // Fast path as repr() for dicts is quite hot
        r.push('{');
        for (i, (name, value)) in self.0.content().iter().enumerate() {
            if i != 0 {
                r.push_str(", ");
            }
            name.collect_repr(r);
            r.push_str(": ");
            value.collect_repr(r);
        }
        r.push('}');
    }

    fn collect_repr_cycle(&self, collector: &mut String) {
        collector.push_str("..."); // BLAZE PATCH 0031: Java Printer.repr (Printer.java:191)
    }

    fn to_bool(&self) -> bool {
        !self.0.content().is_empty()
    }

    fn equals(&self, other: Value<'v>) -> crate::Result<bool> {
        match DictRef::from_value(other) {
            None => Ok(false),
            Some(other_ref) => {
                // BLAZE PATCH 0063 (round 2): the reads of Java's `x.equals(y)` on a key-tracking `y`.
                if let Some(tracking) = other.downcast_ref::<MutableDict>() {
                    if tracking.0.accessed_keys.borrow().is_some() {
                        record_equals_accesses(&self.0, &tracking.0, &other_ref.content)?;
                    }
                }
                equals_small_map(&*self.0.content(), &other_ref.content, |x, y| x.equals(*y))
            }
        }
    }

    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> crate::Result<Value<'v>> {
        // BLAZE PATCH 0051: Java Dict.getIndex (Dict.java:696): an unhashable key is not found.
        let content = self.0.content();
        let found = match index.get_hashed() {
            Ok(k) => content.get_index_of_hashed_by_value(k), // BLAZE PATCH 0063
            Err(_) => None,
        };
        match found {
            Some(i) => {
                // BLAZE PATCH 0063: Java ImmutableKeyTrackingDict.get records a present key.
                let v = content.get_index(i).expect("index of a key").1.to_value();
                drop(content);
                self.0.record_key_index_access(i);
                Ok(v)
            }
            None => Err(ValueError::Bazel(format!(
                "key {} not found in dictionary",
                index.to_repr()
            ))
            .into()),
        }
    }

    fn length(&self) -> crate::Result<i32> {
        Ok(self.0.content().len() as i32)
    }

    fn is_in(&self, other: Value<'v>) -> crate::Result<bool> {
        // BLAZE PATCH 0063: Java ImmutableKeyTrackingDict.containsKey records a present key.
        let found = self
            .0
            .content()
            .get_index_of_hashed_by_value(other.get_hashed()?);
        match found {
            Some(i) => {
                self.0.record_key_index_access(i);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    unsafe fn iterate(&self, me: Value<'v>, _heap: Heap<'v>) -> crate::Result<Value<'v>> {
        self.0.record_all_keys_access(); // BLAZE PATCH 0063: Java iterator() reads keySet()
        unsafe {
            self.0.iter_start();
            Ok(me)
        }
    }

    unsafe fn iter_size_hint(&self, index: usize) -> (usize, Option<usize>) {
        debug_assert!(index <= self.0.content().len());
        let rem = self.0.content().len() - index;
        (rem, Some(rem))
    }

    unsafe fn iter_next(&self, index: usize, _heap: Heap<'v>) -> Option<Value<'v>> {
        unsafe { self.0.content_unchecked().keys().nth(index).copied() }
    }

    unsafe fn iter_stop(&self) {
        unsafe {
            self.0.iter_stop();
        }
    }

    fn set_at(&self, index: Value<'v>, alloc_value: Value<'v>) -> crate::Result<()> {
        let index = index.get_hashed()?;
        self.0.set_at(index, alloc_value)
    }

    fn bit_or(&self, rhs: Value<'v>, heap: Heap<'v>) -> crate::Result<Value<'v>> {
        // Java EvalUtils.binaryOp PIPE: `map | map` with any Map on the right. // BLAZE PATCH 0068
        let rhs = match crate::values::types::dict::globals::java_map_as_dict(rhs, heap) { // BLAZE PATCH 0068
            Some(m) => m?, // BLAZE PATCH 0068
            None => rhs, // BLAZE PATCH 0068
        }; // BLAZE PATCH 0068
        let rhs_value = rhs; // BLAZE PATCH 0063
        let rhs = DictRef::from_value(rhs)
            .map_or_else(|| ValueError::unsupported_with(self, "|", rhs), Ok)?;
        // BLAZE PATCH 0063: Java copies both operands through entrySet().
        self.0.record_all_keys_access();
        dict_record_all_keys_accessed(rhs_value);
        if self.0.content().is_empty() {
            return Ok(heap.alloc((*rhs).clone()));
        }
        // Might be faster if we preallocate the capacity, but then copying in the LHS
        // is more expensive and might oversize given the behaviour on duplicates.
        // If this becomes a bottleneck, benchmark.
        let mut items = self.0.content().clone();
        for (k, v) in rhs.iter_hashed() {
            items.insert_hashed(k, v);
        }
        Ok(heap.alloc(Dict::new(items)))
    }

    fn typechecker_ty(&self) -> Option<Ty> {
        Some(Ty::any_dict())
    }

    fn get_type_starlark_repr() -> Ty {
        Ty::any_dict()
    }

    fn try_freeze_directly(&self, _freezer: &Freezer<'_>) -> Option<FreezeResult<FrozenValue>> {
        if self.0.content().is_empty() {
            Some(Ok(VALUE_EMPTY_FROZEN_DICT.to_frozen_value()))
        } else {
            None
        }
    }
}

impl<'v, T: DictLike<'v>> Serialize for DictGen<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_map(self.0.content().iter())
    }
}

#[cfg(test)]
mod tests {
    use crate::assert;
    use crate::coerce::coerce;
    use crate::collections::SmallMap;
    use crate::values::Heap;
    use crate::values::dict::Dict;

    #[test]
    fn test_mutate_dict() {
        assert::is_true(
            r#"
x = {1: 2, 2: 4}
b1 = str(x) == "{1: 2, 2: 4}"
x[2] = 3
b2 = str(x) == "{1: 2, 2: 3}"
x[(3,4)] = 5
b3 = str(x) == "{1: 2, 2: 3, (3, 4): 5}"
b1 and b2 and b3
"#,
        );
    }

    #[test]
    fn test_get_str() -> crate::Result<()> {
        Heap::temp(|heap| {
            let k1 = heap.alloc_str("hello").get_hashed();
            let k2 = heap.alloc_str("world").get_hashed();
            let mut sm = SmallMap::new();
            sm.insert_hashed(k1, heap.alloc(12));
            sm.insert_hashed(k2, heap.alloc(56));
            let d = Dict::new(coerce(sm));

            assert_eq!(d.get(heap.alloc("hello"))?.unwrap().unpack_i32(), Some(12));
            assert_eq!(d.get(heap.alloc("foo"))?, None);
            assert_eq!(d.get_str("hello").unwrap().unpack_i32(), Some(12));
            assert_eq!(d.get_str("foo"), None);
            Ok(())
        })
    }

    #[test]
    fn test_repr_cycle() {
        assert::eq("d = {}; d[17] = d; repr(d)", "'{17: {...}}'");
        assert::eq("d = {}; d[17] = d; str(d)", "'{17: {...}}'");
    }
}
