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

use std::fmt;
use std::fmt::Display;

use dupe::Dupe;

use crate as starlark;
use crate::eval::runtime::frozen_file_span::FrozenFileSpan;
use crate::eval::runtime::inlined_frame::InlinedFrames;
use crate::register_starlark_any;

/// Span of the call frame (including inlined call frames).
#[derive(
    Debug,
    Clone,
    Copy,
    Dupe,
    PartialEq,
    Eq,
    Default,
    starlark_derive::StarlarkPagable
)]
pub(crate) struct FrameSpan {
    pub(crate) span: FrozenFileSpan,
    /// Parent frames.
    pub(crate) inlined_frames: InlinedFrames,
    /// BLAZE PATCH 0061: Java's call-site spans of a call expression's span.
    pub(crate) java_call: crate::eval::runtime::java_call::JavaCall, // BLAZE PATCH 0061
}

impl FrameSpan {
    pub(crate) const fn new(span: FrozenFileSpan) -> FrameSpan {
        FrameSpan {
            span,
            inlined_frames: InlinedFrames { frames: None },
            java_call: crate::eval::runtime::java_call::JavaCall(None), // BLAZE PATCH 0061
        }
    }

    /// The `:` between a dict entry's key (`self`) and its value (Java
    /// `DictExpression.Entry.getColonLocation`); the key's span if it cannot be found.
    pub(crate) fn dict_colon(&self, value: &FrameSpan) -> FrameSpan { // BLAZE PATCH 0028
        let file = self.span.file(); // BLAZE PATCH 0028
        let (from, to) = (self.span.span().end(), value.span.span().begin()); // BLAZE PATCH 0028
        if value.span.file() != file || to < from { // BLAZE PATCH 0028
            return *self; // BLAZE PATCH 0028
        } // BLAZE PATCH 0028
        let text = file.source_span(crate::codemap::Span::new(from, to)); // BLAZE PATCH 0028
        let mut in_comment = false; // BLAZE PATCH 0028
        for (i, c) in text.char_indices() { // BLAZE PATCH 0028
            match c { // BLAZE PATCH 0028
                '\n' => in_comment = false, // BLAZE PATCH 0028
                _ if in_comment => {} // BLAZE PATCH 0028
                '#' => in_comment = true, // BLAZE PATCH 0028
                ':' => { // BLAZE PATCH 0028
                    let at = from + i as u32; // BLAZE PATCH 0028
                    let colon = crate::codemap::Span::new(at, at + 1); // BLAZE PATCH 0028
                    return FrameSpan { // BLAZE PATCH 0028
                        span: FrozenFileSpan::new_unchecked(file, colon), // BLAZE PATCH 0028
                        inlined_frames: self.inlined_frames, // BLAZE PATCH 0028
                        java_call: crate::eval::runtime::java_call::JavaCall(None), // BLAZE PATCH 0061
                    }; // BLAZE PATCH 0028
                } // BLAZE PATCH 0028
                _ => {} // BLAZE PATCH 0028
            } // BLAZE PATCH 0028
        } // BLAZE PATCH 0028
        *self // BLAZE PATCH 0028
    } // BLAZE PATCH 0028

    /// An empty span at the start of this one, with the same inlined frames: errors located
    /// there render at the node's start (Java `Node.getStartLocation`).
    pub(crate) fn begin_point(&self) -> FrameSpan { // BLAZE PATCH 0028
        FrameSpan { // BLAZE PATCH 0028
            span: self.span.begin_point(), // BLAZE PATCH 0028
            inlined_frames: self.inlined_frames, // BLAZE PATCH 0028
            java_call: crate::eval::runtime::java_call::JavaCall(None), // BLAZE PATCH 0061
        } // BLAZE PATCH 0028
    } // BLAZE PATCH 0028

    pub(crate) fn end_span(&self) -> FrameSpan {
        FrameSpan {
            span: self.span.end_span(),
            inlined_frames: self.inlined_frames,
            java_call: crate::eval::runtime::java_call::JavaCall(None), // BLAZE PATCH 0061
        }
    }
}

impl Display for FrameSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO(nga): either display inlined frames or remove this.
        Display::fmt(&self.span, f)
    }
}

impl FrameSpan {
    pub(crate) fn merge(&self, other: &FrameSpan) -> FrameSpan {
        // TODO(nga): merge inlined frames.
        FrameSpan {
            span: self.span.merge(&other.span),
            inlined_frames: self.inlined_frames,
            java_call: crate::eval::runtime::java_call::JavaCall(None), // BLAZE PATCH 0061
        }
    }
}

register_starlark_any!(FrameSpan);
