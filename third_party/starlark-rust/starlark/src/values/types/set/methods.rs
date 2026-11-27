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

//! Methods for the `set` type.

use std::mem;

#[allow(unused_imports)] // BLAZE PATCH 0007
use either::Either;
use starlark_derive::starlark_module;
use starlark_map::Hashed;
use starlark_map::small_set::SmallSet;
use starlark_syntax::value_error;

use crate as starlark;
use crate::environment::MethodsBuilder;
use crate::values::Heap;
use crate::values::UnpackValue;
use crate::values::Value;
use crate::values::ValueOfUnchecked;
use crate::values::none::NoneType;
use crate::values::set::refs::SetMut;
use crate::values::set::refs::SetRef;
use crate::values::set::value::SetData;
use crate::values::typing::StarlarkIter;
use crate::values::dict::DictRef; // BLAZE PATCH 0007
use crate::values::list::ListRef; // BLAZE PATCH 0007
use crate::values::range::Range; // BLAZE PATCH 0007
use crate::values::ValueLike; // BLAZE PATCH 0007
use crate::values::tuple::TupleRef; // BLAZE PATCH 0007
use crate::values::tuple::UnpackTuple; // BLAZE PATCH 0007

#[allow(dead_code)] // BLAZE PATCH 0007: replaced by `hashable_collection`
enum SetFromValue<'v> {
    Set(SmallSet<Value<'v>>),
    Ref(SetRef<'v>),
}

#[allow(dead_code)] // BLAZE PATCH 0007
impl<'v> SetFromValue<'v> {
    fn from_value(
        value: ValueOfUnchecked<'v, StarlarkIter<Value<'v>>>,
        heap: Heap<'v>,
    ) -> crate::Result<Self> {
        match SetRef::unpack_value_opt(value.get()) {
            Some(v) => Ok(SetFromValue::Ref(v)),
            _ => {
                let mut set = SmallSet::default();
                for elem in value.get().iterate(heap)? {
                    set.insert_hashed(elem.get_hashed()?);
                }
                Ok(SetFromValue::Set(set))
            }
        }
    }

    fn get(&self) -> &SmallSet<Value<'v>> {
        match self {
            SetFromValue::Set(set) => set,
            SetFromValue::Ref(set) => &set.aref.content,
        }
    }

    fn into_set(self) -> SmallSet<Value<'v>> {
        match self {
            SetFromValue::Set(set) => set,
            SetFromValue::Ref(set) => set.aref.content.clone(),
        }
    }

    fn is_empty(&self) -> bool {
        self.get().is_empty()
    }

    fn contains_hashed(&self, value: Hashed<Value<'v>>) -> bool {
        self.get().contains_hashed(value.as_ref())
    }
}

// BLAZE PATCH 0007: Java StarlarkSet.toHashableCollection (StarlarkSet.java:818): a set, a dict
// (its keys), or a list, tuple or range whose elements are hashable; anything else fails with
// "for <what> got value of type '<type>', want a collection of hashable elements".
fn hashable_collection<'v>( // BLAZE PATCH 0007
    other: Value<'v>, // BLAZE PATCH 0007
    what: &str, // BLAZE PATCH 0007
    heap: Heap<'v>, // BLAZE PATCH 0007
) -> crate::Result<Vec<Hashed<Value<'v>>>> { // BLAZE PATCH 0007
    if let Some(set) = SetRef::unpack_value_opt(other) { // BLAZE PATCH 0007
        return Ok(set.aref.content.iter_hashed().map(|h| h.copied()).collect()); // BLAZE PATCH 0007
    } // BLAZE PATCH 0007
    if let Some(dict) = DictRef::from_value(other) { // BLAZE PATCH 0007
        return Ok(dict.iter_hashed().map(|(k, _)| k).collect()); // BLAZE PATCH 0007
    } // BLAZE PATCH 0007
    // Java StarlarkSet.toHashableCollection: any Map gives its keys. // BLAZE PATCH 0068
    if let Some(entries) = other.get_ref().java_map_entries(heap) { // BLAZE PATCH 0068
        return entries?.into_iter().map(|(k, _)| k.get_hashed()).collect(); // BLAZE PATCH 0068
    } // BLAZE PATCH 0068
    let collection = ListRef::from_value(other).is_some() // BLAZE PATCH 0007
        || TupleRef::from_value(other).is_some() // BLAZE PATCH 0007
        || ValueLike::downcast_ref::<Range>(other).is_some(); // BLAZE PATCH 0007
    if !collection { // BLAZE PATCH 0007
        return Err(value_error!( // BLAZE PATCH 0007
            "for {} got value of type '{}', want a collection of hashable elements", // BLAZE PATCH 0007
            what, // BLAZE PATCH 0007
            other.get_type() // BLAZE PATCH 0007
        )); // BLAZE PATCH 0007
    } // BLAZE PATCH 0007
    other.iterate(heap)?.map(|v| v.get_hashed()).collect() // BLAZE PATCH 0007
} // BLAZE PATCH 0007

// BLAZE PATCH 0007
fn retain_in<'v>(set: &SmallSet<Value<'v>>, keep: &SmallSet<Value<'v>>) -> SmallSet<Value<'v>> { // BLAZE PATCH 0007
    let mut r = SmallSet::new(); // BLAZE PATCH 0007
    for h in set.iter_hashed() { // BLAZE PATCH 0007
        if keep.contains_hashed(h) { // BLAZE PATCH 0007
            r.insert_hashed(h.copied()); // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
    } // BLAZE PATCH 0007
    r // BLAZE PATCH 0007
} // BLAZE PATCH 0007

// BLAZE PATCH 0007
fn to_small_set<'v>(elems: Vec<Hashed<Value<'v>>>) -> SmallSet<Value<'v>> { // BLAZE PATCH 0007
    let mut set = SmallSet::with_capacity(elems.len()); // BLAZE PATCH 0007
    for h in elems { // BLAZE PATCH 0007
        set.insert_hashed(h); // BLAZE PATCH 0007
    } // BLAZE PATCH 0007
    set // BLAZE PATCH 0007
} // BLAZE PATCH 0007

/// BLAZE PATCH 0007: the augmented assignments that mutate a set in place. // BLAZE PATCH 0007
#[derive(Clone, Copy)] // BLAZE PATCH 0007
pub(crate) enum SetInplaceOp { // BLAZE PATCH 0007
    /// `|=` (Java StarlarkSet.update). // BLAZE PATCH 0007
    Or, // BLAZE PATCH 0007
    /// `&=` (Java StarlarkSet.intersectionUpdate). // BLAZE PATCH 0007
    And, // BLAZE PATCH 0007
    /// `^=` (Java StarlarkSet.symmetricDifferenceUpdate). // BLAZE PATCH 0007
    Xor, // BLAZE PATCH 0007
    /// `-=` (Java StarlarkSet.differenceUpdate). // BLAZE PATCH 0007
    Sub, // BLAZE PATCH 0007
} // BLAZE PATCH 0007

/// BLAZE PATCH 0007: Java Eval.inplaceBinaryOp (Eval.java:498-536): `set op= set` mutates the // BLAZE PATCH 0007
/// left set and evaluates to it; `None` when either operand is not a set (a plain binary op). // BLAZE PATCH 0007
pub(crate) fn set_inplace_op<'v>( // BLAZE PATCH 0007
    lhs: Value<'v>, // BLAZE PATCH 0007
    rhs: Value<'v>, // BLAZE PATCH 0007
    op: SetInplaceOp, // BLAZE PATCH 0007
) -> Option<crate::Result<Value<'v>>> { // BLAZE PATCH 0007
    SetRef::unpack_value_opt(lhs)?; // BLAZE PATCH 0007
    let other: Vec<Hashed<Value<'v>>> = SetRef::unpack_value_opt(rhs)? // BLAZE PATCH 0007
        .aref // BLAZE PATCH 0007
        .content // BLAZE PATCH 0007
        .iter_hashed() // BLAZE PATCH 0007
        .map(|h| h.copied()) // BLAZE PATCH 0007
        .collect(); // BLAZE PATCH 0007
    let run = || -> crate::Result<Value<'v>> { // BLAZE PATCH 0007
        let mut set = SetMut::from_value(lhs)?; // BLAZE PATCH 0007
        let content = &mut set.aref.content; // BLAZE PATCH 0007
        match op { // BLAZE PATCH 0007
            SetInplaceOp::Or => { // BLAZE PATCH 0007
                for h in other { // BLAZE PATCH 0007
                    content.insert_hashed(h); // BLAZE PATCH 0007
                } // BLAZE PATCH 0007
            } // BLAZE PATCH 0007
            SetInplaceOp::And => { // BLAZE PATCH 0007
                *content = retain_in(content, &to_small_set(other)); // BLAZE PATCH 0007
            } // BLAZE PATCH 0007
            SetInplaceOp::Xor => { // BLAZE PATCH 0007
                let original = content.clone(); // BLAZE PATCH 0007
                for h in other { // BLAZE PATCH 0007
                    if original.contains_hashed(h.as_ref()) { // BLAZE PATCH 0007
                        content.shift_remove_hashed(h.as_ref()); // BLAZE PATCH 0007
                    } else { // BLAZE PATCH 0007
                        content.insert_hashed(h); // BLAZE PATCH 0007
                    } // BLAZE PATCH 0007
                } // BLAZE PATCH 0007
            } // BLAZE PATCH 0007
            SetInplaceOp::Sub => { // BLAZE PATCH 0007
                for h in other { // BLAZE PATCH 0007
                    content.shift_remove_hashed(h.as_ref()); // BLAZE PATCH 0007
                } // BLAZE PATCH 0007
            } // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
        Ok(lhs) // BLAZE PATCH 0007
    }; // BLAZE PATCH 0007
    Some(run()) // BLAZE PATCH 0007
} // BLAZE PATCH 0007

#[starlark_module]
pub(crate) fn set_methods(builder: &mut MethodsBuilder) {
    fn clear(this: Value) -> anyhow::Result<NoneType> {
        let mut this = SetMut::from_value(this)?;
        this.aref.clear();
        Ok(NoneType)
    }

    // Java StarlarkSet.union (StarlarkSet.java:586): any number of collections.
    fn union<'v>( // BLAZE PATCH 0007
        this: SetRef<'v>, // BLAZE PATCH 0007
        #[starlark(args)] others: UnpackTuple<Value<'v>>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<SetData<'v>> { // BLAZE PATCH 0007
        let mut data = this.aref.content.clone(); // BLAZE PATCH 0007
        for other in others.items { // BLAZE PATCH 0007
            for h in hashable_collection(other, "union argument", heap)? { // BLAZE PATCH 0007
                data.insert_hashed(h); // BLAZE PATCH 0007
            } // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
        Ok(SetData { content: data }) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.intersection (StarlarkSet.java:615).
    fn intersection<'v>( // BLAZE PATCH 0007
        this: SetRef<'v>, // BLAZE PATCH 0007
        #[starlark(args)] others: UnpackTuple<Value<'v>>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<SetData<'v>> { // BLAZE PATCH 0007
        let mut data = this.aref.content.clone(); // BLAZE PATCH 0007
        for other in others.items { // BLAZE PATCH 0007
            let other = to_small_set(hashable_collection(other, "intersection argument", heap)?); // BLAZE PATCH 0007
            data = retain_in(&data, &other); // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
        Ok(SetData { content: data }) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.intersectionUpdate (StarlarkSet.java:644).
    fn intersection_update<'v>( // BLAZE PATCH 0007
        this: Value<'v>, // BLAZE PATCH 0007
        #[starlark(args)] others: UnpackTuple<Value<'v>>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<NoneType> { // BLAZE PATCH 0007
        SetMut::from_value(this)?; // BLAZE PATCH 0007
        for other in others.items { // BLAZE PATCH 0007
            let other = to_small_set(hashable_collection(other, "intersection_update argument", heap)?); // BLAZE PATCH 0007
            let mut set = SetMut::from_value(this)?; // BLAZE PATCH 0007
            set.aref.content = retain_in(&set.aref.content, &other); // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
        Ok(NoneType) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.symmetricDifference (StarlarkSet.java:730): one collection.
    fn symmetric_difference<'v>( // BLAZE PATCH 0007
        this: SetRef<'v>, // BLAZE PATCH 0007
        #[starlark(require = pos)] other: Value<'v>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<SetData<'v>> { // BLAZE PATCH 0007
        let other = hashable_collection(other, "symmetric_difference argument", heap)?; // BLAZE PATCH 0007
        let mut data = this.aref.content.clone(); // BLAZE PATCH 0007
        for h in other { // BLAZE PATCH 0007
            if this.aref.content.contains_hashed(h.as_ref()) { // BLAZE PATCH 0007
                data.shift_remove_hashed(h.as_ref()); // BLAZE PATCH 0007
            } else { // BLAZE PATCH 0007
                data.insert_hashed(h); // BLAZE PATCH 0007
            } // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
        Ok(SetData { content: data }) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.symmetricDifferenceUpdate (StarlarkSet.java:766).
    fn symmetric_difference_update<'v>( // BLAZE PATCH 0007
        this: Value<'v>, // BLAZE PATCH 0007
        #[starlark(require = pos)] other: Value<'v>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<NoneType> { // BLAZE PATCH 0007
        let original = SetMut::from_value(this)?.aref.content.clone(); // BLAZE PATCH 0007
        let other = hashable_collection(other, "symmetric_difference_update argument", heap)?; // BLAZE PATCH 0007
        let mut set = SetMut::from_value(this)?; // BLAZE PATCH 0007
        for h in other { // BLAZE PATCH 0007
            if original.contains_hashed(h.as_ref()) { // BLAZE PATCH 0007
                set.aref.content.shift_remove_hashed(h.as_ref()); // BLAZE PATCH 0007
            } else { // BLAZE PATCH 0007
                set.aref.content.insert_hashed(h); // BLAZE PATCH 0007
            } // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
        Ok(NoneType) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.addElement (StarlarkSet.java:482).
    fn add<'v>( // BLAZE PATCH 0007
        this: Value<'v>, // BLAZE PATCH 0007
        #[starlark(require = pos)] element: Value<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<NoneType> { // BLAZE PATCH 0007
        let mut this = SetMut::from_value(this)?; // BLAZE PATCH 0007
        let hashed = element.get_hashed()?; // BLAZE PATCH 0007
        this.aref.add_hashed(hashed); // BLAZE PATCH 0007
        Ok(NoneType) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.update (StarlarkSet.java:451): any number of collections.
    fn update<'v>( // BLAZE PATCH 0007
        this: Value<'v>, // BLAZE PATCH 0007
        #[starlark(args)] others: UnpackTuple<Value<'v>>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<NoneType> { // BLAZE PATCH 0007
        SetMut::from_value(this)?; // BLAZE PATCH 0007
        for other in others.items { // BLAZE PATCH 0007
            let elems = hashable_collection(other, "update argument", heap)?; // BLAZE PATCH 0007
            let mut set = SetMut::from_value(this)?; // BLAZE PATCH 0007
            for h in elems { // BLAZE PATCH 0007
                set.aref.add_hashed(h); // BLAZE PATCH 0007
            } // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
        Ok(NoneType) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.removeElement (StarlarkSet.java:501).
    fn remove<'v>( // BLAZE PATCH 0007
        this: Value<'v>, // BLAZE PATCH 0007
        #[starlark(require = pos)] element: Value<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<NoneType> { // BLAZE PATCH 0007
        let mut set = SetMut::from_value(this)?; // BLAZE PATCH 0007
        let hashed = element.get_hashed()?; // BLAZE PATCH 0007
        if set.aref.remove_hashed(hashed.as_ref()) { // BLAZE PATCH 0007
            Ok(NoneType) // BLAZE PATCH 0007
        } else { // BLAZE PATCH 0007
            mem::drop(set); // BLAZE PATCH 0007
            Err(value_error!("element {} not found in set", element.to_repr())) // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.discard (StarlarkSet.java:526).
    fn discard<'v>( // BLAZE PATCH 0007
        this: Value<'v>, // BLAZE PATCH 0007
        #[starlark(require = pos)] element: Value<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<NoneType> { // BLAZE PATCH 0007
        let mut set = SetMut::from_value(this)?; // BLAZE PATCH 0007
        let hashed = element.get_hashed()?; // BLAZE PATCH 0007
        set.aref.remove_hashed(hashed.as_ref()); // BLAZE PATCH 0007
        Ok(NoneType) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.pop (StarlarkSet.java:552): the first element in iteration order.
    fn pop<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> { // BLAZE PATCH 0007
        let mut set = SetMut::from_value(this)?; // BLAZE PATCH 0007
        match set.aref.content.shift_remove_index(0) { // BLAZE PATCH 0007
            Some(x) => Ok(x), // BLAZE PATCH 0007
            None => Err(value_error!("set is empty")), // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.difference (StarlarkSet.java:673).
    fn difference<'v>( // BLAZE PATCH 0007
        this: SetRef<'v>, // BLAZE PATCH 0007
        #[starlark(args)] others: UnpackTuple<Value<'v>>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<SetData<'v>> { // BLAZE PATCH 0007
        let mut data = this.aref.content.clone(); // BLAZE PATCH 0007
        for other in others.items { // BLAZE PATCH 0007
            for h in hashable_collection(other, "difference argument", heap)? { // BLAZE PATCH 0007
                data.shift_remove_hashed(h.as_ref()); // BLAZE PATCH 0007
            } // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
        Ok(SetData { content: data }) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.differenceUpdate (StarlarkSet.java:702; its error names
    // "intersection_update argument").
    fn difference_update<'v>( // BLAZE PATCH 0007
        this: Value<'v>, // BLAZE PATCH 0007
        #[starlark(args)] others: UnpackTuple<Value<'v>>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<NoneType> { // BLAZE PATCH 0007
        SetMut::from_value(this)?; // BLAZE PATCH 0007
        for other in others.items { // BLAZE PATCH 0007
            let elems = hashable_collection(other, "intersection_update argument", heap)?; // BLAZE PATCH 0007
            let mut set = SetMut::from_value(this)?; // BLAZE PATCH 0007
            for h in elems { // BLAZE PATCH 0007
                set.aref.content.shift_remove_hashed(h.as_ref()); // BLAZE PATCH 0007
            } // BLAZE PATCH 0007
        } // BLAZE PATCH 0007
        Ok(NoneType) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.isSuperset (StarlarkSet.java:409).
    fn issuperset<'v>( // BLAZE PATCH 0007
        this: SetRef<'v>, // BLAZE PATCH 0007
        #[starlark(require = pos)] other: Value<'v>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<bool> { // BLAZE PATCH 0007
        let other = hashable_collection(other, "issuperset argument", heap)?; // BLAZE PATCH 0007
        Ok(other.iter().all(|h| this.aref.content.contains_hashed(h.as_ref()))) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.isSubset (StarlarkSet.java:389).
    fn issubset<'v>( // BLAZE PATCH 0007
        this: SetRef<'v>, // BLAZE PATCH 0007
        #[starlark(require = pos)] other: Value<'v>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<bool> { // BLAZE PATCH 0007
        let other = to_small_set(hashable_collection(other, "issubset argument", heap)?); // BLAZE PATCH 0007
        Ok(this.aref.content.iter_hashed().all(|h| other.contains_hashed(h))) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007

    // Java StarlarkSet.isDisjoint (StarlarkSet.java:429).
    fn isdisjoint<'v>( // BLAZE PATCH 0007
        this: SetRef<'v>, // BLAZE PATCH 0007
        #[starlark(require = pos)] other: Value<'v>, // BLAZE PATCH 0007
        heap: Heap<'v>, // BLAZE PATCH 0007
    ) -> starlark::Result<bool> { // BLAZE PATCH 0007
        let other = hashable_collection(other, "isdisjoint argument", heap)?; // BLAZE PATCH 0007
        Ok(!other.iter().any(|h| this.aref.content.contains_hashed(h.as_ref()))) // BLAZE PATCH 0007
    } // BLAZE PATCH 0007
}

#[cfg(test)]
mod tests {
    use crate::assert;
    use crate::assert::Assert;

    #[test]
    fn test_empty() {
        assert::is_true("s = set(); len(s) == 0")
    }

    #[test]
    fn test_single() {
        assert::is_true("s = set([0, 1]); len(s) == 2")
    }

    #[test]
    fn test_eq() {
        assert::is_true("set([1, 2, 3]) == set([3, 2, 1])")
    }

    #[test]
    fn test_clear() {
        assert::is_true("s = set([1, 2, 3]); s.clear(); s == set()")
    }

    #[test]
    fn test_type() {
        assert::eq("type(set([1, 2, 3]))", "'set'")
    }

    #[test]
    fn test_iter() {
        assert::is_true("list([elem for elem in set([1, 2, 3])]) ==  [1, 2, 3]")
    }

    #[test]
    fn test_bool_true() {
        assert::is_true("bool(set([1, 2, 3]))")
    }

    #[test]
    fn test_bool_false() {
        assert::is_false("bool(set())")
    }

    #[test]
    fn test_union() {
        assert::eq(
            "set([1, 2, 3]).union(set([3, 4, 5]))",
            "set([1, 2, 3, 4, 5])",
        )
    }

    #[test]
    fn test_union_empty() {
        assert::eq("set([1, 2, 3]).union(set([]))", "set([1, 2, 3])")
    }

    #[test]
    fn test_union_iter() {
        assert::eq("set([1, 2, 3]).union([3, 4])", "set([1, 2, 3, 4])")
    }

    #[test]
    fn test_union_ordering_mixed() {
        assert::eq("list(set([1, 3, 5]).union(set([4, 3])))", "[1, 3, 5, 4]");
    }

    #[test]
    fn test_intersection() {
        assert::eq("set([1, 2, 3]).intersection(set([3, 4, 5]))", "set([3])")
    }

    #[test]
    fn test_intersection_empty() {
        assert::eq("set([1, 2, 3]).intersection(set([]))", "set([])")
    }

    #[test]
    fn test_intersection_iter() {
        assert::eq("set([1, 2, 3]).intersection([3, 4])", "set([3])")
    }

    #[test]
    fn test_intersection_order() {
        assert::eq("list(set([1, 2, 3]).intersection([4, 3, 1]))", "[1, 3]")
    }

    #[test]
    fn test_symmetric_difference() {
        assert::eq(
            "set([1, 2, 3]).symmetric_difference(set([3, 4, 5]))",
            "set([1, 2, 4, 5])",
        )
    }

    #[test]
    fn test_symmetric_difference_empty() {
        assert::eq(
            "set([1, 2, 3]).symmetric_difference(set([]))",
            "set([1, 2, 3])",
        )
    }

    #[test]
    fn test_symmetric_difference_iter() {
        assert::eq(
            "set([1, 2, 3]).symmetric_difference([3, 4])",
            "set([1, 2, 4])",
        )
    }

    #[test]
    fn test_symmetric_difference_ord() {
        assert::eq(
            "list(set([1, 2, 3, 7]).symmetric_difference(set([4, 3, 1])))",
            "[2, 7, 4]",
        )
    }

    #[test]
    fn test_add() {
        assert::eq(r#"x = set([1, 2, 3]);x.add(0);x"#, "set([0, 1, 2, 3])")
    }

    #[test]
    fn test_add_empty() {
        assert::eq(r#"x = set([]);x.add(0);x"#, "set([0])")
    }

    #[test]
    fn test_add_existing() {
        assert::eq(r#"x = set([0]);x.add(0);x"#, "set([0])")
    }

    #[test]
    fn test_add_order() {
        assert::eq(r#"x = set([1, 2, 3]);x.add(2);list(x)"#, "[1, 2, 3]");
        assert::eq(r#"x = set([1, 2, 3]);x.add(0);list(x)"#, "[1, 2, 3, 0]")
    }

    #[test]
    fn test_remove() {
        assert::eq("x = set([0, 1]);x.remove(1);x", "set([0])")
    }

    #[test]
    fn test_remove_empty() {
        assert::fail("set([]).remove(0)", "`0` not found in `set([])`");
    }

    #[test]
    fn test_remove_not_existing() {
        assert::fail("set([1]).remove(0)", "`0` not found in `set([1])`");
    }

    #[test]
    fn test_discard() {
        assert::eq("x = set([0, 1]);x.discard(1);x", "set([0])")
    }

    #[test]
    fn test_discard_multiple_times() {
        assert::eq("x = set([0, 1]); x.discard(0); x.discard(0); x", "set([1])");
    }

    #[test]
    fn test_pop() {
        assert::is_true("x = set([1, 0]); (x.pop() == 0 and x.pop() == 1 and x == set())");
    }

    #[test]
    fn test_pop_empty() {
        assert::fail("x = set([]); x.pop()", "pop from an empty set");
    }

    #[test]
    fn test_difference() {
        assert::eq("set([1, 2, 3]).difference(set([2]))", "set([1, 3])")
    }

    #[test]
    fn test_difference_iter() {
        assert::eq("set([1, 2, 3]).difference([3, 2])", "set([1])")
    }

    #[test]
    fn test_difference_order() {
        assert::eq("list(set([3, 2, 1]).difference([2]))", "[3, 1]")
    }

    #[test]
    fn test_difference_empty_lhs() {
        assert::eq("set([]).difference(set([2]))", "set([])")
    }

    #[test]
    fn test_difference_empty_rhs() {
        assert::eq("set([1, 2]).difference(set([]))", "set([2, 1])")
    }

    #[test]
    fn test_is_superset() {
        assert::is_true("set([1, 2, 3, 4]).issuperset(set([1, 3, 2]))")
    }

    #[test]
    fn test_is_not_superset() {
        assert::is_false("set([1, 2]).issuperset(set([1, 3, 5]))")
    }

    #[test]
    fn test_is_not_superset_empty_lhs() {
        assert::is_false("set([]).issuperset(set([1]))");
    }

    #[test]
    fn test_is_superset_empty_rhs() {
        assert::is_true("set([1, 2]).issuperset(set([]))");
        assert::is_true("set([]).issuperset(set([]))")
    }

    #[test]
    fn test_is_superset_iter() {
        assert::is_true("set([1, 2, 3]).issuperset([3, 1])")
    }

    #[test]
    fn test_is_subset() {
        assert::is_true("set([1, 2]).issubset(set([1, 3, 2]))")
    }

    #[test]
    fn test_is_not_subset() {
        assert::is_false("set([1, 2]).issubset(set([1, 3, 5]))")
    }

    #[test]
    fn test_is_subset_empty_lhs() {
        assert::is_true("set([]).issubset(set([1, 3, 5]))");
        assert::is_true("set([]).issubset(set([]))")
    }

    #[test]
    fn test_is_not_subset_empty_rhs() {
        assert::is_false("set([1, 2]).issubset(set([]))")
    }

    #[test]
    fn test_is_subset_iter() {
        assert::is_true("set([1, 2]).issubset([1, 3, 2])")
    }

    #[test]
    fn test_update() {
        assert::eq(
            "x = set([1, 3, 2]); x.update([4, 3]); list(x)",
            "[1, 3, 2, 4]",
        )
    }

    #[test]
    fn test_update_empty() {
        assert::eq("x = set([1, 3, 2]); x.update([]); list(x)", "[1, 3, 2]");
        assert::eq("x = set(); x.update(set([4, 3])); list(x)", "[4, 3]");
        assert::eq("x = set(); x.update([]); x", "set()");
    }

    #[test]
    fn test_update_self() {
        assert::eq("x = set([1, 3, 2]); x.update(x); list(x)", "[1, 3, 2]")
    }

    #[test]
    fn test_update_frozen_set_cannot_be_updated_with_self() {
        let mut a = Assert::new();
        a.module("s.star", "S = set([1, 2, 3])");
        a.fail(
            r#"
load('s.star', 'S')

S.update(S)
"#,
            "trying to mutate a frozen set value", // BLAZE PATCH 0025 (was "Immutable")
        );
    }
}
