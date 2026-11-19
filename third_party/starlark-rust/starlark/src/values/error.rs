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

//! Define a common set of errors.

use thiserror::Error;

use crate::values::StarlarkValue;
use crate::values::Value;

/// Common errors returned by Starlark evaluation.
#[derive(Debug, Error)]
#[allow(missing_docs)] // Self-explanatory.
pub enum ValueError {
    #[error("Operation `{op}` not supported on type `{typ}`")]
    OperationNotSupported { op: String, typ: String },
    #[error("Operation `{op}` not supported for types `{left}` and `{right}`")]
    OperationNotSupportedBinary {
        op: String,
        left: String,
        right: String,
    },
    #[error("Cannot divide by zero")]
    DivisionByZero,
    #[error("Integer overflow")]
    IntegerOverflow,
    #[error("Negative shift count")]
    NegativeShiftCount,
    #[error("Type of parameters mismatch")]
    IncorrectParameterType,
    #[error("Type of parameter `{0}` doesn't match")]
    IncorrectParameterTypeNamed(String),
    #[error("Missing this parameter")]
    MissingThis,
    #[error("Missing required parameter `{0}`")]
    MissingRequired(String),
    #[error("Index `{0}` is out of bound")]
    IndexOutOfBound(i32),
    #[error("Key `{0}` was not found")]
    KeyNotFound(String),
    #[error("Immutable")]
    CannotMutateImmutableValue,
    #[error("This operation mutates an iterable for an iterator while iterating.")]
    MutationDuringIteration,
    #[error("Object of type `{0}` has no attribute `{1}`")]
    NoAttr(String, String),
    #[error("Object of type `{0}` has no attribute `{1}`, did you mean `{2}`?")]
    NoAttrDidYouMean(String, String, String),
    /// BLAZE PATCH 0051: an error whose text is Bazel's (the Java site is named where it is
    /// created).
    #[error("{0}")]
    Bazel(String),
}

impl From<ValueError> for crate::Error {
    fn from(e: ValueError) -> Self {
        crate::Error::new_kind(crate::ErrorKind::Value(anyhow::Error::new(e)))
    }
}

#[derive(Debug, Error)]
pub(crate) enum ControlError {
    /// BLAZE PATCH 0051: Java's text (StarlarkValue.java:120 checkHashable).
    #[error("unhashable type: '{0}'")]
    NotHashableValue(String),
    #[error("Too many recursion levels")]
    TooManyRecursionLevel,
}

impl ValueError {
    #[cold]
    pub(crate) fn unsupported_owned<T>(
        left: &str,
        op: &str,
        right: Option<&str>,
    ) -> crate::Result<T> {
        // BLAZE PATCH 0051: Java's texts for the operations Starlark defines.
        if let Some(message) = Self::bazel_unsupported_message(left, op, right) {
            return Err(ValueError::Bazel(message).into());
        }
        match right {
            None => Err(ValueError::OperationNotSupported {
                op: op.to_owned(),
                typ: left.to_owned(),
            }
            .into()),
            Some(right) => Err(ValueError::OperationNotSupportedBinary {
                op: op.to_owned(),
                left: left.to_owned(),
                right: right.to_owned(),
            }
            .into()),
        }
    }

    /// BLAZE PATCH 0051: Bazel's text for an unsupported operation `op` on values of type `left`
    /// (and `right`), or `None` for an operation Bazel does not have.
    #[cold]
    pub(crate) fn bazel_unsupported_message(
        left: &str,
        op: &str,
        right: Option<&str>,
    ) -> Option<String> {
        Some(match (op, right) {
            // Java: net/starlark/java/eval/EvalUtils.java:472 EvalUtils.unaryOp
            ("+" | "-" | "~", None) => format!("unsupported unary operation: {op}{left}"),
            // Java: net/starlark/java/eval/Starlark.java:904 Starlark.getStarlarkCallable
            ("call()", None) => format!("'{left}' object is not callable"),
            // Java: net/starlark/java/eval/Starlark.java:284 Starlark.toIterable
            ("(iter)", None) => format!("type '{left}' is not iterable"),
            // Java: net/starlark/java/eval/Starlark.java:672 Starlark.slice
            ("[::]", None) => format!("invalid slice operand: {left}"),
            // Java: net/starlark/java/eval/EvalUtils.java:499 EvalUtils.index
            ("[]", Some(right)) => format!("type '{left}' has no operator []({right})"),
            // Java: net/starlark/java/eval/Starlark.java:516 Starlark.compareUnchecked
            ("compare" | "cmp()" | "<>", Some(right)) => {
                format!("unsupported comparison: {left} <=> {right}")
            }
            // Java: net/starlark/java/eval/EvalUtils.java:416 EvalUtils.binaryOp
            (
                "+" | "-" | "*" | "/" | "//" | "%" | "&" | "|" | "^" | "<<" | ">>" | "in",
                Some(right),
            ) => format!("unsupported binary operation: {left} {op} {right}"),
            // Java: net/starlark/java/eval/Eval.java:486 inplaceBinaryOp falls back to `|`.
            ("|=", Some(right)) => format!("unsupported binary operation: {left} | {right}"),
            // Java: net/starlark/java/eval/MethodLibrary.java len (the parameter's type check).
            ("len()", None) => format!(
                "in call to len(), parameter 'x' got value of type '{left}', want 'iterable or string'"
            ),
            // Java: net/starlark/java/eval/EvalUtils.java:534 EvalUtils.setField
            (op, None) if op.starts_with('.') && op.ends_with('=') && op.len() > 2 => format!(
                "cannot set .{} field of {left} value",
                &op[1..op.len() - 1]
            ),
            _ => return None,
        })
    }

    /// BLAZE PATCH 0051: Java's error for a missing field or method `name` of `x`, with a
    /// spelling suggestion among `dir(x)` (sorted, Java's TreeSet).
    // Java: net/starlark/java/eval/Starlark.java:1014 Starlark.getattr
    #[cold]
    pub(crate) fn bazel_no_attr(x: Value, name: &str) -> crate::Error {
        let mut dir = x.dir_attr();
        dir.sort();
        dir.dedup();
        ValueError::Bazel(format!(
            "'{}' value has no field or method '{}'{}",
            x.get_type(),
            name,
            crate::errors::spell_checker::did_you_mean(name, dir.iter().map(|s| s.as_str()))
        ))
        .into()
    }

    /// Helper to create an [`OperationNotSupported`](ValueError::OperationNotSupported) error.
    #[cold]
    pub fn unsupported<'v, T, V: StarlarkValue<'v>>(_left: &V, op: &str) -> crate::Result<T> {
        Self::unsupported_owned(V::TYPE, op, None)
    }

    #[cold]
    #[allow(dead_code)] // BLAZE PATCH 0008
    pub(crate) fn unsupported_type<T>(left: Value, op: &str) -> crate::Result<T> {
        Self::unsupported_owned(left.get_type(), op, None)
    }

    /// Helper to create an [`OperationNotSupported`](ValueError::OperationNotSupportedBinary) error.
    #[cold]
    pub fn unsupported_with<'v, T, V: StarlarkValue<'v>>(
        _left: &V,
        op: &str,
        right: Value,
    ) -> crate::Result<T> {
        Self::unsupported_owned(V::TYPE, op, Some(right.get_type()))
    }
}
