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

use std::cell::Ref;
// BLAZE PATCH 0025: `use std::cell::RefCell;` removed (the payload is MutableSetData now).
use std::cell::RefMut;
use std::convert::Infallible;

use dupe::Dupe;
use either::Either;

use super::value::FrozenSetData;
use super::value::MutableSet; // BLAZE PATCH 0025
use super::value::SetData;
use super::value::frozen_set_error; // BLAZE PATCH 0025
use crate::coerce::coerce;
use crate::typing::Ty;
use crate::values::FrozenValue;
use crate::values::UnpackValue;
use crate::values::Value;
use crate::values::ValueError;
use crate::values::layout::value::ValueLike;
use crate::values::set::value::SetGen;
use crate::values::type_repr::SetType;
use crate::values::type_repr::StarlarkTypeRepr;

/// Define the set type.
pub struct SetRef<'v> {
    pub(crate) aref: Either<Ref<'v, SetData<'v>>, &'v SetData<'v>>,
}

impl<'v> Clone for SetRef<'v> {
    fn clone(&self) -> Self {
        match &self.aref {
            Either::Left(x) => SetRef {
                aref: Either::Left(Ref::clone(x)),
            },
            Either::Right(x) => SetRef {
                aref: Either::Right(*x),
            },
        }
    }
}

impl<'v> Dupe for SetRef<'v> {}

impl<'v> SetRef<'v> { // BLAZE PATCH 0015
    /// Downcast the value to a set (frozen or not), like `ListRef::from_value`.
    pub fn from_value(x: Value<'v>) -> Option<SetRef<'v>> { // BLAZE PATCH 0015
        Self::unpack_value_opt(x) // BLAZE PATCH 0015
    } // BLAZE PATCH 0015

    /// Number of elements.
    pub fn len(&self) -> usize { // BLAZE PATCH 0015
        self.aref.content.len() // BLAZE PATCH 0015
    } // BLAZE PATCH 0015

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool { // BLAZE PATCH 0015
        self.aref.content.is_empty() // BLAZE PATCH 0015
    } // BLAZE PATCH 0015

    /// The elements in insertion order (no heap needed, unlike `Value::iterate`).
    pub fn iter<'a>(&'a self) -> impl ExactSizeIterator<Item = Value<'v>> + 'a { // BLAZE PATCH 0015
        self.aref.iter() // BLAZE PATCH 0015
    } // BLAZE PATCH 0015
} // BLAZE PATCH 0015

/// Allocates a new (mutable) set holding `values` in order, dropping duplicates (Bazel
/// `StarlarkSet.copyOf` / `immutableCopyOf`; the heap freezes it with its module). Fails if a
/// value is unhashable.
pub fn alloc_set<'v>( // BLAZE PATCH 0015
    heap: crate::values::Heap<'v>, // BLAZE PATCH 0015
    values: impl IntoIterator<Item = Value<'v>>, // BLAZE PATCH 0015
) -> crate::Result<Value<'v>> { // BLAZE PATCH 0015
    let mut data = SetData::default(); // BLAZE PATCH 0015
    for v in values { // BLAZE PATCH 0015
        data.add_hashed(v.get_hashed()?); // BLAZE PATCH 0015
    } // BLAZE PATCH 0015
    Ok(heap.alloc(data)) // BLAZE PATCH 0015
} // BLAZE PATCH 0015

/// Mutably borrowed `Set`.
pub struct SetMut<'v> {
    pub(crate) aref: RefMut<'v, SetData<'v>>,
}

impl<'v> SetMut<'v> {
    /// Downcast the value to a mutable set reference.
    #[inline]
    pub fn from_value(x: Value<'v>) -> anyhow::Result<SetMut<'v>> {
        #[derive(thiserror::Error, Debug)]
        #[error("Value is not set, value type: `{0}`")]
        struct NotSetError(&'static str);

        #[cold]
        #[inline(never)]
        fn error<'v>(x: Value<'v>) -> anyhow::Error {
            if x.downcast_ref::<SetGen<FrozenSetData>>().is_some() {
                // BLAZE PATCH 0025: Java's text (Starlark.checkMutable), as 0023 for list/dict.
                frozen_set_error()
            } else {
                NotSetError(x.get_type()).into()
            }
        }

        let ptr = x.downcast_ref::<MutableSet<'v>>(); // BLAZE PATCH 0025
        match ptr {
            None => Err(error(x)),
            // BLAZE PATCH 0025: Java Starlark.checkMutable checks frozenness first.
            Some(ptr) if ptr.0.immutable.get() => Err(frozen_set_error()),
            Some(ptr) => match ptr.0.cell.try_borrow_mut() { // BLAZE PATCH 0025
                Ok(x) => Ok(SetMut { aref: x }),
                Err(_) => Err(ValueError::Bazel("set value is temporarily immutable due to active for-loop iteration".to_owned()).into()), // BLAZE PATCH 0051
            },
        }
    }
}

impl<'v> StarlarkTypeRepr for SetRef<'v> {
    type Canonical = <SetType<FrozenValue> as StarlarkTypeRepr>::Canonical;

    fn starlark_type_repr() -> Ty {
        <Self::Canonical as StarlarkTypeRepr>::starlark_type_repr()
    }
}

impl<'v> UnpackValue<'v> for SetRef<'v> {
    type Error = Infallible;

    fn unpack_value_impl(value: Value<'v>) -> Result<Option<SetRef<'v>>, Infallible> {
        let result = if let Some(value) = value.unpack_frozen() {
            value
                .downcast_ref::<SetGen<FrozenSetData>>()
                .map(|x| SetRef {
                    aref: Either::Right(coerce(&x.0)),
                })
        } else {
            value
                .downcast_ref::<MutableSet<'v>>() // BLAZE PATCH 0025
                .map(|ptr| SetRef {
                    aref: Either::Left(ptr.0.cell.borrow()), // BLAZE PATCH 0025
                })
        };
        Ok(result)
    }
}
