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

//! Parameter conversion utilities for `starlark_module` macros.

use std::convert::Infallible;
use std::fmt::Debug;

use anyhow::Context;
use either::Either;
use starlark_syntax::StarlarkResultExt;

use crate::typing::Ty;
use crate::values::Value;
use crate::values::type_repr::StarlarkTypeRepr;

/// Error that can be returned by [`UnpackValue`].
pub trait UnpackValueError: Debug + Send + Sync + 'static {
    /// Convert into a crate error.
    fn into_error(this: Self) -> crate::Error;
}

impl UnpackValueError for crate::Error {
    #[cold]
    fn into_error(this: Self) -> crate::Error {
        this
    }
}

impl UnpackValueError for anyhow::Error {
    #[cold]
    fn into_error(this: Self) -> crate::Error {
        crate::Error::new_value(this)
    }
}

impl UnpackValueError for Infallible {
    #[cold]
    fn into_error(this: Self) -> crate::Error {
        match this {}
    }
}

impl<A: UnpackValueError, B: UnpackValueError> UnpackValueError for Either<A, B> {
    #[cold]
    fn into_error(this: Self) -> crate::Error {
        match this {
            Either::Left(a) => UnpackValueError::into_error(a),
            Either::Right(b) => UnpackValueError::into_error(b),
        }
    }
}

/// Never error.
pub trait UnpackValueErrorInfallible: UnpackValueError {
    /// Convert into a never type.
    fn into_infallible(this: Self) -> !;
}

impl UnpackValueErrorInfallible for Infallible {
    fn into_infallible(this: Self) -> ! {
        match this {}
    }
}

impl<A: UnpackValueErrorInfallible, B: UnpackValueErrorInfallible> UnpackValueErrorInfallible
    for Either<A, B>
{
    fn into_infallible(this: Self) -> ! {
        match this {
            Either::Left(a) => UnpackValueErrorInfallible::into_infallible(a),
            Either::Right(b) => UnpackValueErrorInfallible::into_infallible(b),
        }
    }
}

/// How to convert a [`Value`] to a Rust type. Required for all arguments in
/// a [`#[starlark_module]`](macro@crate::starlark_module) definition.
///
/// Note for simple references it often can be implemented with `#[starlark_value(UnpackValue)]`,
/// for example:
///
/// ```
/// # use allocative::Allocative;
/// # use starlark::any::ProvidesStaticType;
/// # use starlark::values::{NoSerialize, StarlarkPagable, StarlarkValue, starlark_value};
///
/// #[derive(
///     Debug,
///     derive_more::Display,
///     Allocative,
///     NoSerialize,
///     StarlarkPagable,
///     ProvidesStaticType
/// )]
/// struct MySimpleValue;
///
/// #[starlark_value(type = "MySimpleValue", UnpackValue, StarlarkTypeRepr)]
/// impl<'v> StarlarkValue<'v> for MySimpleValue {}
/// ```
///
/// Whereas for types that aren't also [`StarlarkValue`](crate::values::StarlarkValue) you can define:
///
/// ```
/// # use either::Either;
/// # use starlark::typing::Ty;
/// # use starlark::values::{UnpackValue, Value};
/// # use starlark::values::type_repr::StarlarkTypeRepr;
///
/// struct BoolOrInt(i32);
///
/// impl StarlarkTypeRepr for BoolOrInt {
///     type Canonical = <Either<bool, i32> as StarlarkTypeRepr>::Canonical;
///
///     fn starlark_type_repr() -> Ty {
///         Either::<bool, i32>::starlark_type_repr()
///     }
/// }
///
/// impl<'v> UnpackValue<'v> for BoolOrInt {
///     type Error = starlark::Error;
///
///     fn unpack_value_impl(value: Value<'v>) -> starlark::Result<Option<Self>> {
///         if let Some(x) = value.unpack_bool() {
///             Ok(Some(BoolOrInt(x as i32)))
///         } else {
///             let Some(x) = i32::unpack_value(value)? else {
///                 return Ok(None);
///             };
///             Ok(Some(BoolOrInt(x)))
///         }
///     }
/// }
/// ```
pub trait UnpackValue<'v>: Sized + StarlarkTypeRepr {
    /// Error returned when type matches, but conversion fails.
    ///
    /// Typically [`starlark::Error`](crate::Error), [`anyhow::Error`], or [`Infallible`].
    type Error: UnpackValueError;

    /// Given a [`Value`], try and unpack it into the given type,
    /// which may involve some element of conversion.
    ///
    /// Return `None` if the value is not of expected type (as described by [`StarlarkTypeRepr`],
    /// and return `Err` if the value is of expected type, but conversion cannot be performed.
    /// For example, when unpacking an integer to `String`, return `None`,
    /// and when unpacking a large integer to `i32`, return `Err`.
    ///
    /// This function is needs to be implemented, but usually not meant to be called directly.
    /// Consider using [`unpack_value`](UnpackValue::unpack_value),
    /// [`unpack_value_err`](UnpackValue::unpack_value_err),
    /// [`unpack_value_opt`](UnpackValue::unpack_value_opt) instead.
    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error>;

    /// Given a [`Value`], try and unpack it into the given type,
    /// which may involve some element of conversion.
    ///
    /// Return `None` if the value is not of expected type (as described by [`StarlarkTypeRepr`],
    /// and return `Err` if the value is of expected type, but conversion cannot be performed.
    /// For example, when unpacking an integer to `String`, return `None`,
    /// and when unpacking a large integer to `i32`, return `Err`.
    fn unpack_value(value: Value<'v>) -> Result<Option<Self>, crate::Error> {
        Self::unpack_value_impl(value).map_err(Self::Error::into_error)
    }

    /// Unpack a value if unpacking is infallible.
    fn unpack_value_opt(value: Value<'v>) -> Option<Self>
    where
        Self::Error: UnpackValueErrorInfallible,
    {
        match Self::unpack_value_impl(value) {
            Ok(x) => x,
            Err(e) => Self::Error::into_infallible(e),
        }
    }

    /// Unpack a value, but return error instead of `None` if unpacking fails.
    #[inline]
    fn unpack_value_err(value: Value<'v>) -> crate::Result<Self> {
        #[cold]
        fn error<'v>(value: Value<'v>, ty: fn() -> Ty) -> crate::Error {
            #[derive(thiserror::Error, Debug)]
            #[error("Expected `{0}`, but got `{1}`")]
            struct IncorrectType(Ty, String);

            crate::Error::new_value(IncorrectType(ty(), value.to_string_for_type_error()))
        }

        Self::unpack_value(value)?.ok_or_else(|| error(value, Self::starlark_type_repr))
    }

    /// Unpack value, but instead of `None` return error about incorrect argument type.
    #[inline]
    fn unpack_param(value: Value<'v>) -> crate::Result<Self> {
        #[cold]
        fn error<'v>(value: Value<'v>, ty: fn() -> Ty) -> crate::Error {
            #[derive(thiserror::Error, Debug)]
            #[error("Type of parameters mismatch, expected `{0}`, actual `{1}`")]
            struct IncorrectParameterTypeWithExpected(Ty, String);

            crate::Error::new_value(IncorrectParameterTypeWithExpected(
                ty(),
                value.to_string_for_type_error(),
            ))
        }

        Self::unpack_value(value)?.ok_or_else(|| error(value, Self::starlark_type_repr))
    }

    /// Unpack value, but instead of `None` return error about incorrect named argument type.
    #[inline]
    fn unpack_named_param(value: Value<'v>, param_name: &str) -> crate::Result<Self> {
        #[cold]
        fn error<'v>(value: Value<'v>, param_name: &str, ty: fn() -> Ty) -> crate::Error {
            // BLAZE PATCH 0052: Java's text (BuiltinFunction.java:404 checkParamValue); the
            // function name is filled in by the native function's call (`NativeFunc::invoke`).
            crate::Error::new_value(BazelParamTypeError {
                function: None,
                param: param_name.to_owned(),
                actual: value.get_type().to_owned(),
                ty: ty(),
            })
        }

        // BLAZE PATCH 0052: an int beyond the parameter's 32-bit range is Java's
        // `Starlark.toInt(x, param)` error (Starlark.java:755, StarlarkInt.java:339).
        if let Err(e) = Self::unpack_value(value) { // BLAZE PATCH 0052
            if crate::values::types::int::int_or_big::StarlarkIntRef::unpack(value).is_some() { // BLAZE PATCH 0052
                let _ = e; // BLAZE PATCH 0052
                return Err(crate::Error::new_value(anyhow::anyhow!( // BLAZE PATCH 0052
                    "got {value} for {param_name}, want value in signed 32-bit range" // BLAZE PATCH 0052
                ))); // BLAZE PATCH 0052
            } // BLAZE PATCH 0052
        } // BLAZE PATCH 0052
        Self::unpack_value(value)
            .into_anyhow_result()
            .with_context(|| {
                format!(
                    "Error unpacking value for parameter `{}` of type `{}",
                    param_name,
                    Self::starlark_type_repr()
                )
            })?
            .ok_or_else(|| error(value, param_name, Self::starlark_type_repr))
    }
}

impl<'v> UnpackValue<'v> for Value<'v> {
    type Error = Infallible;

    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(Some(value))
    }
}

impl<'v, TLeft: UnpackValue<'v>, TRight: UnpackValue<'v>> UnpackValue<'v>
    for Either<TLeft, TRight>
{
    type Error = Either<TLeft::Error, TRight::Error>;

    // Only implemented for types that implement [`UnpackValue`]. Nonsensical for other types.
    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        match TLeft::unpack_value_impl(value).map_err(Either::Left)? {
            Some(left) => Ok(Some(Self::Left(left))),
            _ => Ok(TRight::unpack_value_impl(value)
                .map_err(Either::Right)?
                .map(Self::Right)),
        }
    }
}

/// BLAZE PATCH 0052: a builtin's argument of the wrong type, Java's
/// "in call to F(), parameter 'P' got value of type 'T', want 'W'"
/// (BuiltinFunction.java:404 checkParamValue, ParamDescriptor.java:123 getTypeErrorMessage).
#[derive(Debug, Clone)]
pub(crate) struct BazelParamTypeError {
    /// The builtin's name; set by the call (`NativeFunc::invoke`, `NativeMeth::invoke`).
    pub(crate) function: Option<String>,
    pub(crate) param: String,
    /// `type(value)`.
    pub(crate) actual: String,
    /// The parameter's starlark-rust type.
    pub(crate) ty: Ty,
}

impl std::error::Error for BazelParamTypeError {}

impl std::fmt::Display for BazelParamTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let function = self.function.as_deref().unwrap_or("function");
        write!(
            f,
            "in call to {function}(), parameter '{}' got value of type '{}', want '{}'",
            self.param,
            self.actual,
            java_allowed_types(function, &self.param, &self.ty)
        )
    }
}

impl BazelParamTypeError {
    /// `e` with the builtin's name set, if `e` is a parameter type error without one.
    pub(crate) fn with_function(e: crate::Error, function: &str) -> crate::Error {
        if let crate::ErrorKind::Value(a) = e.kind() {
            if let Some(p) = a.downcast_ref::<BazelParamTypeError>() {
                if p.function.is_none() {
                    let mut p = p.clone();
                    p.function = Some(function.to_owned());
                    return crate::Error::new_value(p);
                }
            }
        }
        // BLAZE PATCH 0056: the builtin's name in its duplicate keyword error.
        if let crate::ErrorKind::Function(a) = e.kind() {
            if let Some(crate::eval::runtime::arguments::FunctionError::RepeatedKwarg {
                function: None,
                name,
            }) = a.downcast_ref()
            {
                return crate::eval::runtime::arguments::FunctionError::RepeatedKwarg {
                    function: Some(function.to_owned()),
                    name: name.clone(),
                }
                .into();
            }
        }
        e
    }
}

/// BLAZE PATCH 0052: Java `ParamDescriptor.getTypeErrorMessage` for a parameter of a builtin:
/// the Java class names of its allowed types, "a", "a or b", "a, b, or c".
///
/// PORT: Java lists the types in the order of the parameter's `@Param(allowedTypes)`, which
/// starlark-rust's types do not keep. The parameters of the core builtins whose order differs
/// from the rule below are listed (from MethodLibrary.java and StringModule.java); for the rest
/// the types are starlark-rust's, with `NoneType` last (Java's usual order).
pub(crate) fn java_allowed_types(function: &str, param: &str, ty: &Ty) -> String {
    let listed: Option<&[&str]> = match (function, param) {
        ("int" | "float", "x") => Some(&["string", "bool", "int", "float"]),
        ("abs", "x") => Some(&["int", "float"]),
        ("startswith" | "endswith", "sub") => Some(&["string", "tuple"]),
        _ => None,
    };
    let names: Vec<String> = match listed {
        Some(l) => l.iter().map(|s| (*s).to_owned()).collect(),
        None => {
            let text = ty.to_string();
            let mut parts = Vec::new();
            let mut depth = 0;
            let mut cur = String::new();
            for c in text.chars() {
                match c {
                    '[' | '(' => depth += 1,
                    ']' | ')' => depth -= 1,
                    _ => {}
                }
                if c == '|' && depth == 0 {
                    parts.push(std::mem::take(&mut cur));
                } else {
                    cur.push(c);
                }
            }
            parts.push(cur);
            let mut names: Vec<String> = parts
                .iter()
                .map(|p| {
                    let p = p.trim();
                    let base = p.split('[').next().unwrap_or(p);
                    match base {
                        "str" => "string",
                        "None" => "NoneType",
                        "typing.Iterable" => "iterable",
                        "typing.Callable" => "callable",
                        "typing.Any" => "Object",
                        other => other,
                    }
                    .to_owned()
                })
                .collect();
            names.dedup();
            if let Some(i) = names.iter().position(|n| n == "NoneType") {
                let n = names.remove(i);
                names.push(n);
            }
            names
        }
    };
    let n = names.len();
    let mut buf = String::new();
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            buf.push_str(if n == 2 {
                " or "
            } else if i < n - 1 {
                ", "
            } else {
                ", or "
            });
        }
        buf.push_str(name);
    }
    buf
}

/// BLAZE PATCH 0052: Java's check of a builtin parameter declared `StarlarkIterable` (strings
/// are not iterable): "in call to F(), parameter 'P' got value of type 'T', want 'iterable'".
pub(crate) fn java_check_iterable<'v>(
    function: &str,
    param: &str,
    value: Value<'v>,
    heap: crate::values::Heap<'v>,
) -> crate::Result<()> {
    if value.unpack_str().is_none() && value.iterate(heap).is_ok() {
        return Ok(());
    }
    Err(crate::Error::new_other(anyhow::anyhow!(
        "in call to {function}(), parameter '{param}' got value of type '{}', want 'iterable'",
        value.get_type()
    )))
}

/// BLAZE PATCH 0056: Java's check of a builtin parameter declared
/// `allowedTypes = {StarlarkCallable, NoneType}` (`key` of sorted, min, max):
/// "in call to F(), parameter 'P' got value of type 'T', want 'callable or NoneType'"
/// (BuiltinFunction.java:404 checkParamValue). Returns the callable, `None` for `None`.
pub(crate) fn java_check_callable_or_none<'v>(
    function: &str,
    param: &str,
    value: Option<Value<'v>>,
) -> crate::Result<Option<Value<'v>>> {
    match value {
        None => Ok(None),
        Some(v) if v.is_none() => Ok(None),
        Some(v) if v.vtable().starlark_value.HAS_invoke => Ok(Some(v)),
        Some(v) => Err(crate::Error::new_other(anyhow::anyhow!(
            "in call to {function}(), parameter '{param}' got value of type '{}', want 'callable or NoneType'",
            v.get_type()
        ))),
    }
}
