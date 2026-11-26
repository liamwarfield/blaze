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

//! Guard to check we don't recurse too deeply with nested operations like Equals.

use std::cell::Cell;

use crate::hint::unlikely;
use crate::values::error::ControlError;

// Maximum recursion level for comparison
// TODO(dmarting): those are rather short, maybe make it configurable?
// BLAZE PATCH 0060: Java compares nested values without a depth limit (a JVM stack overflow is
// a Bazel crash; Bazel 9.3.2 compares two 5000-deep lists with `==`). Blaze evaluates on
// threads with a 256 MiB stack (blaze_starlark eval::EVAL_STACK_SIZE); this guard only keeps a
// native stack overflow from aborting the process, in debug and release builds alike.
const MAX_RECURSION: u32 = 20000; // BLAZE PATCH 0060

// A thread-local counter is used to detect too deep recursion.
//
// Thread-local is chosen instead of explicit function "recursion" parameter
// for two reasons:
// * It's possible to propagate stack depth across external functions like
//   `Display::to_string` where passing a stack depth parameter is hard
// * We need to guarantee that stack depth is not lost in complex invocation
//   chains like function calls compare which calls native function which calls
//   starlark function which calls to_str. We could change all evaluation stack
//   signatures to accept some "context" parameters, but passing it as
//   thread-local is easier.
thread_local! {
    static STACK_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Stored previous stack depth before calling `try_inc`.
///
/// Stores that previous stack depths back to thread-local on drop.
#[must_use]
// QUESTION: Is this useful? If we build a deep structure so deep equals
//           dies, won't we just die in `drop` anyway?
pub struct StackGuard {
    prev_depth: u32,
}

impl Drop for StackGuard {
    fn drop(&mut self) {
        STACK_DEPTH.with(|stack_depth| {
            stack_depth.set(self.prev_depth);
        });
    }
}

/// Increment stack depth.
fn inc() -> StackGuard {
    STACK_DEPTH.with(|stack_depth| {
        let prev_depth = stack_depth.get();
        stack_depth.set(prev_depth + 1);
        StackGuard { prev_depth }
    })
}

/// Check stack depth does not exceed configured max stack depth.
fn check() -> anyhow::Result<()> {
    if unlikely(STACK_DEPTH.with(|stack_depth| stack_depth.get()) >= MAX_RECURSION) {
        return Err(ControlError::TooManyRecursionLevel.into());
    }
    // BLAZE PATCH 0060 (round 2): equality, comparison and hashing also run on threads with
    // smaller stacks than the 256 MiB evaluation threads (Blaze's execution-phase evaluator
    // threads have 16 MiB, its gRPC command threads 8 MiB or more): stop while at least
    // `STACK_MARGIN` bytes of the current thread's stack are left, whatever the depth.
    if unlikely(remaining_stack().is_some_and(|r| r < STACK_MARGIN)) { // BLAZE PATCH 0060
        return Err(ControlError::TooManyRecursionLevel.into()); // BLAZE PATCH 0060
    } // BLAZE PATCH 0060
    Ok(())
}

/// BLAZE PATCH 0060: stack left when a guarded recursion stops (the error path and the deepest
/// recursion between two guarded calls must fit in it).
const STACK_MARGIN: usize = 1 << 20;

thread_local! {
    /// BLAZE PATCH 0060: the lowest address of the current thread's stack (0: not looked up yet;
    /// `usize::MAX`: unknown).
    static STACK_LOW: Cell<usize> = const { Cell::new(0) };
}

/// BLAZE PATCH 0060: bytes of the current thread's stack below the caller's frame, from glibc's
/// `pthread_getattr_np` (Blaze runs on Linux x86-64 only); `None` where it is unknown.
#[inline(never)]
fn remaining_stack() -> Option<usize> {
    let marker = 0u8;
    let sp = std::hint::black_box(&marker) as *const u8 as usize;
    let low = STACK_LOW.with(|low| {
        if low.get() == 0 {
            low.set(stack_low().unwrap_or(usize::MAX));
        }
        low.get()
    });
    if low == usize::MAX || sp < low {
        return None;
    }
    Some(sp - low)
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn stack_low() -> Option<usize> {
    use std::os::raw::c_int;
    use std::os::raw::c_void;
    /// glibc's `pthread_attr_t` (56 bytes on x86-64), as opaque storage.
    #[repr(C, align(16))]
    struct PthreadAttr([u8; 128]);
    unsafe extern "C" {
        fn pthread_self() -> usize;
        fn pthread_getattr_np(thread: usize, attr: *mut PthreadAttr) -> c_int;
        fn pthread_attr_getstack(
            attr: *const PthreadAttr,
            stackaddr: *mut *mut c_void,
            stacksize: *mut usize,
        ) -> c_int;
        fn pthread_attr_destroy(attr: *mut PthreadAttr) -> c_int;
    }
    let mut attr = PthreadAttr([0; 128]);
    // SAFETY: glibc fills `attr` (large enough) for the calling thread; it is destroyed below.
    unsafe {
        if pthread_getattr_np(pthread_self(), &mut attr) != 0 {
            return None;
        }
        let mut addr: *mut c_void = std::ptr::null_mut();
        let mut size: usize = 0;
        let r = pthread_attr_getstack(&attr, &mut addr, &mut size);
        pthread_attr_destroy(&mut attr);
        if r != 0 || addr.is_null() {
            return None;
        }
        Some(addr as usize)
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn stack_low() -> Option<usize> {
    None
}

/// Try increment stack depth.
///
/// Return opaque object which resets stack to previous value
/// on `drop`.
///
/// If stack depth exceeds configured limit, return error.
pub(crate) fn stack_guard() -> anyhow::Result<StackGuard> {
    check()?;
    Ok(inc())
}
