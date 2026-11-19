/*
 * Copyright 2019 The Starlark in Rust Authors.
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

use std::num::NonZeroI32;

use starlark_derive::starlark_module;

use crate as starlark;
use crate::environment::GlobalsBuilder;
use crate::values::range::Range;

#[starlark_module]
pub(crate) fn register_range(globals: &mut GlobalsBuilder) {
    /// [range](
    /// https://github.com/bazelbuild/starlark/blob/master/spec.md#range
    /// ): return a range of integers
    ///
    /// `range` returns a tuple of integers defined by the specified interval
    /// and stride.
    ///
    /// ```python
    /// range(stop)                             # equivalent to range(0, stop)
    /// range(start, stop)                      # equivalent to range(start, stop, 1)
    /// range(start, stop, step)
    /// ```
    ///
    /// `range` requires between one and three integer arguments.
    /// With one argument, `range(stop)` returns the ascending sequence of
    /// non-negative integers less than `stop`.
    /// With two arguments, `range(start, stop)` returns only integers not less
    /// than `start`.
    ///
    /// With three arguments, `range(start, stop, step)` returns integers
    /// formed by successively adding `step` to `start` until the value meets or
    /// passes `stop`. A call to `range` fails if the value of `step` is
    /// zero.
    ///
    /// ```
    /// # starlark::assert::all_true(r#"
    /// list(range(10))                         == [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
    /// list(range(3, 10))                      == [3, 4, 5, 6, 7, 8, 9]
    /// list(range(3, 10, 2))                   == [3, 5, 7, 9]
    /// list(range(10, 3, -2))                  == [10, 8, 6, 4]
    /// # "#);
    /// ```
    #[starlark(as_type = Range, speculative_exec_safe)]
    fn range<'v>( // BLAZE PATCH 0052
        #[starlark(require = pos)] start_or_stop: crate::values::Value<'v>, // BLAZE PATCH 0052
        #[starlark(require = pos)] stop: Option<crate::values::Value<'v>>, // BLAZE PATCH 0052
        #[starlark(require = pos)] step: Option<crate::values::Value<'v>>, // BLAZE PATCH 0052
    ) -> anyhow::Result<Range> {
        // BLAZE PATCH 0052: Java MethodLibrary.range (MethodLibrary.java:765): the parameters
        // are ints (checked in order), then converted with toInt("start"/"stop"/"step").
        let is_int = |v: crate::values::Value<'v>| { // BLAZE PATCH 0052
            crate::values::types::int::int_or_big::StarlarkIntRef::unpack(v).is_some() // BLAZE PATCH 0052
        }; // BLAZE PATCH 0052
        for (name, v) in [("start_or_stop", Some(start_or_stop)), ("stop", stop), ("step", step)] { // BLAZE PATCH 0052
            if let Some(v) = v { // BLAZE PATCH 0052
                if !is_int(v) { // BLAZE PATCH 0052
                    return Err(anyhow::anyhow!( // BLAZE PATCH 0052
                        "in call to range(), parameter '{name}' got value of type '{}', want 'int'", // BLAZE PATCH 0052
                        v.get_type() // BLAZE PATCH 0052
                    )); // BLAZE PATCH 0052
                } // BLAZE PATCH 0052
            } // BLAZE PATCH 0052
        } // BLAZE PATCH 0052
        let to_int = |v, what| crate::values::index::java_to_int(v, what).map_err(|e| e.into_anyhow()); // BLAZE PATCH 0052
        let (start, stop) = match stop { // BLAZE PATCH 0052
            None => (0, to_int(start_or_stop, "stop")?), // BLAZE PATCH 0052
            Some(stop) => (to_int(start_or_stop, "start")?, to_int(stop, "stop")?), // BLAZE PATCH 0052
        }; // BLAZE PATCH 0052
        let step = match step { // BLAZE PATCH 0052
            None => 1, // BLAZE PATCH 0052
            Some(v) => to_int(v, "step")?, // BLAZE PATCH 0052
        }; // BLAZE PATCH 0052
        let step = match NonZeroI32::new(step) {
            Some(step) => step,
            None => {
                // BLAZE PATCH 0051: Java's text (MethodLibrary.java:779 range).
                return Err(anyhow::anyhow!("step cannot be 0"));
            }
        };
        // BLAZE PATCH 0051: Java RangeList's constructor (RangeList.java:89) rejects a length
        // beyond int32.
        let r = Range::new(start, stop, step); // BLAZE PATCH 0051
        if crate::values::StarlarkValue::length(&r).is_err() { // BLAZE PATCH 0051
            return Err(anyhow::anyhow!("len({r}) exceeds signed 32-bit range")); // BLAZE PATCH 0051
        } // BLAZE PATCH 0051
        Ok(r) // BLAZE PATCH 0051
    }
}
