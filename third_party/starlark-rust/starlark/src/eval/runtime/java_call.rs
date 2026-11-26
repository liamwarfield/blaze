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

//! BLAZE PATCH 0061: where Bazel locates an error raised while a call binds its arguments, and
//! errors Java raises in the caller before the callee is entered.
//!
//! Java: net/starlark/java/eval/Eval.java:672 Eval.evalCall. The callee is looked up
//! (`Starlark.getStarlarkCallable`: "'T' object is not callable") before any argument is
//! evaluated and before the call's location is set. A call with only positional arguments goes
//! through `evalPositionalOnlyCall`, whose `catch` sets the error location to the `(`. Any other
//! call sets the frame's location to the `(`, then binds each argument with
//! `ArgumentProcessor.addPositionalArg` / `addNamedArg`, whose errors (a parameter's type, an
//! unexpected or repeated keyword) leave `evalCall` without a location of their own: the
//! innermost enclosing node that catches and sets one gives it (`execAssignment`: the
//! assignment operator; `execFor`: the `for`; a comprehension's `for` clause: its `for`; a dict
//! comprehension's value: its `:`), else the frame keeps the `(`. `*x` that is not iterable and
//! `**x` that is not a dict fail in the caller at the `*` / `**`, without the callee's frame.

use std::fmt;

use dupe::Dupe;

use crate as starlark;
use crate::codemap::Span;
use crate::eval::runtime::arguments::FunctionError;
use crate::eval::runtime::frame_span::FrameSpan;
use crate::eval::runtime::frozen_file_span::FrozenFileSpan;
use crate::register_starlark_any;
use crate::values::any::FrozenAnyValue;
use crate::values::unpack::BazelParamTypeError;

/// The spans Java's `evalCall` uses for one call expression.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Dupe, starlark_derive::StarlarkPagable)]
pub(crate) struct JavaCallSpans {
    /// The innermost enclosing node that sets the location of an error the call's argument
    /// binding raises; `None` when there is none (the `(` of the call stays).
    pub(crate) handler: Option<FrozenFileSpan>,
    /// The `*` of a `*args` argument.
    pub(crate) star: Option<FrozenFileSpan>,
    /// The `**` of a `**kwargs` argument.
    pub(crate) star_star: Option<FrozenFileSpan>,
    /// The call has positional arguments only (Java `evalPositionalOnlyCall`).
    pub(crate) positional_only: bool,
}

register_starlark_any!(JavaCallSpans);

/// [`JavaCallSpans`] of a call's [`FrameSpan`] (none for other spans).
#[derive(Copy, Clone, Dupe, Debug, Default, starlark_derive::StarlarkPagable)]
pub(crate) struct JavaCall(pub(crate) Option<FrozenAnyValue<JavaCallSpans>>);

impl PartialEq for JavaCall {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => **a == **b,
            _ => false,
        }
    }
}

impl Eq for JavaCall {}

/// "'T' object is not callable" (Starlark.java:904 getStarlarkCallable), raised before the
/// callee is entered.
#[derive(Debug)]
pub(crate) struct JavaNotCallable(pub(crate) String);

impl fmt::Display for JavaNotCallable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JavaNotCallable {}

/// An error of a builtin's argument binding raised by host code (Java
/// `BuiltinFunction.checkParamValue` and the other `addPositionalArg` / `addNamedArg` checks
/// that a host builtin performs itself). See [`crate::values::bazel_argument_binding_error`].
#[derive(Debug)]
pub(crate) struct JavaArgBinding(pub(crate) String);

impl fmt::Display for JavaArgBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JavaArgBinding {}

enum Kind {
    /// Raised while binding arguments (`addPositionalArg` / `addNamedArg`).
    Binding,
    /// `*x`: not iterable.
    Star,
    /// `**x`: not a dict, or a key that is not a string.
    StarStar,
    /// The callee is not callable.
    NotCallable,
}

fn kind(e: &crate::Error) -> Option<Kind> {
    match e.kind() {
        crate::ErrorKind::Function(a) => match a.downcast_ref::<FunctionError>()? {
            FunctionError::JavaBinding(_) | FunctionError::RepeatedKwarg { .. } => {
                Some(Kind::Binding)
            }
            FunctionError::JavaStar(_) => Some(Kind::Star),
            FunctionError::JavaStarStar(_) => Some(Kind::StarStar),
            _ => None,
        },
        crate::ErrorKind::Value(a) => {
            if a.downcast_ref::<BazelParamTypeError>().is_some()
                || a.downcast_ref::<JavaArgBinding>().is_some()
            {
                Some(Kind::Binding)
            } else if a.downcast_ref::<JavaNotCallable>().is_some() {
                Some(Kind::NotCallable)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Moves the error of a call at `span` to where Java reports it. Called once, when the call
/// stack is first captured, so the last frame is the callee of that call.
pub(crate) fn relocate(e: &mut crate::Error, span: &FrameSpan) {
    let Some(java) = span.java_call.0 else {
        return;
    };
    let Some(kind) = kind(e) else {
        return;
    };
    let at = |s: FrozenFileSpan| (s.span(), s.file());
    let target = match kind {
        Kind::Binding => {
            if java.positional_only {
                return;
            }
            let Some(h) = java.handler else {
                return;
            };
            if let Some(last) = e.call_stack_mut().frames.last_mut() {
                last.location = Some(h.to_file_span());
            }
            // A builtin's error has no span yet: give it the call site's, so that it stays a
            // builtin's error (`Error in NAME:`). A `def`'s keeps its own (Java shows the `def`).
            if e.span().is_none() {
                let (s, file) = at(h);
                e.set_span(s, &file);
            }
            return;
        }
        Kind::Star => java.star,
        Kind::StarStar => java.star_star,
        Kind::NotCallable => Some(java.handler.unwrap_or(span.span)),
    };
    let Some(target) = target else {
        return;
    };
    e.call_stack_mut().frames.pop();
    let (s, file) = at(target);
    e.replace_span(s, &file);
}

/// The span of the first `needle` in the source between `from` and `to` (comments skipped).
pub(crate) fn find_token(
    codemap: &crate::codemap::CodeMap,
    from: crate::codemap::Pos,
    to: crate::codemap::Pos,
    needle: &str,
) -> Option<Span> {
    if to < from {
        return None;
    }
    let text = codemap.source_span(Span::new(from, to));
    let mut in_comment = false;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            in_comment = false;
        } else if !in_comment {
            if c == b'#' {
                in_comment = true;
            } else if text[i..].starts_with(needle) {
                let at = from + i as u32;
                return Some(Span::new(at, at + needle.len() as u32));
            }
        }
        i += 1;
    }
    None
}

/// The span of the last `needle` before `to`, searching back from `to` to `from`.
pub(crate) fn find_token_back(
    codemap: &crate::codemap::CodeMap,
    from: crate::codemap::Pos,
    to: crate::codemap::Pos,
    needle: &str,
) -> Option<Span> {
    if to < from {
        return None;
    }
    let text = codemap.source_span(Span::new(from, to));
    let i = text.rfind(needle)?;
    let at = from + i as u32;
    Some(Span::new(at, at + needle.len() as u32))
}
