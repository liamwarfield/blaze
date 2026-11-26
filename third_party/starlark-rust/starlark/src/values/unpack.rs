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
                receiver: None, // BLAZE PATCH 0058
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
    /// BLAZE PATCH 0058: `type()` of the method's receiver (`NativeMeth::invoke`); `None` for a
    /// function.
    pub(crate) receiver: Option<String>,
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
            java_param_type_error_message(
                function,
                self.receiver.as_deref(),
                &self.param,
                &self.ty
            )
        )
    }
}

impl BazelParamTypeError {
    /// `e` with the builtin's name set, if `e` is a parameter type error without one.
    pub(crate) fn with_function(e: crate::Error, function: &str) -> crate::Error {
        Self::with_function_receiver(e, function, None)
    }

    /// BLAZE PATCH 0058: [`Self::with_function`] for a method, with its receiver's `type()`.
    pub(crate) fn with_function_receiver(
        e: crate::Error,
        function: &str,
        receiver: Option<&str>,
    ) -> crate::Error {
        if let crate::ErrorKind::Value(a) = e.kind() {
            if let Some(p) = a.downcast_ref::<BazelParamTypeError>() {
                if p.function.is_none() {
                    let mut p = p.clone();
                    p.function = Some(function.to_owned());
                    p.receiver = receiver.map(str::to_owned); // BLAZE PATCH 0058
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

/// BLAZE PATCH 0058: Java's `ParamDescriptor.getTypeErrorMessage` of every Starlark method
/// parameter of Bazel 9.3.2 (`receiver`, `method`, `param`, `want`; receiver is the
/// `@StarlarkBuiltin` name of the class the method is called on, inherited methods included, or
/// empty for global functions; want `-` for a parameter that accepts `Object`), printed by
/// calling Bazel's `MethodDescriptor.of` and `getTypeErrorMessage` on the oracle's server jar
/// (`scripts/java-param-types/run`).
static JAVA_PARAM_TYPES: std::sync::LazyLock<
    std::collections::HashMap<(&'static str, &'static str), Vec<(&'static str, &'static str)>>,
> = std::sync::LazyLock::new(|| {
    let mut m: std::collections::HashMap<_, Vec<_>> = std::collections::HashMap::new();
    for line in include_str!("java_param_types.tsv").lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if let [receiver, method, param, want] = cols[..] {
            m.entry((method, param)).or_default().push((receiver, want));
        }
    }
    m
});

/// BLAZE PATCH 0058: the "want" part of Java's parameter type error for a builtin named
/// `function` (a method of a value of type `receiver`, or a function when `receiver` is
/// `None`), from Bazel's annotations when they decide it: the receiver's entry (a function's is
/// the receiver `""`), else the one text all of Bazel's methods of that name give for that
/// parameter. Otherwise (a builtin Bazel does not have, or a parameter Bazel declares
/// `Object`) [`java_allowed_types`]' derivation.
// Java: net/starlark/java/eval/ParamDescriptor.java:123 ParamDescriptor.getTypeErrorMessage
pub(crate) fn java_param_type_error_message(
    function: &str,
    receiver: Option<&str>,
    param: &str,
    ty: &Ty,
) -> String {
    if let Some(entries) = JAVA_PARAM_TYPES.get(&(function, param)) {
        // A method call takes its receiver's row; a function call the row of Java's global
        // functions (receiver ""), as `configure_features(requested_features=)`.
        let own = receiver.unwrap_or(""); // BLAZE PATCH 0058 (round 2)
        let chosen = match entries.iter().find(|(rc, _)| *rc == own) {
            Some((_, want)) => Some(*want),
            None if entries.iter().all(|(_, w)| *w == entries[0].1) => Some(entries[0].1),
            None => None,
        };
        if let Some(want) = chosen {
            if want != "-" {
                return want.to_owned();
            }
        }
    }
    java_allowed_types(function, param, ty)
}

/// BLAZE PATCH 0062: whether `v` is one of Java's allowed classes of the builtin's parameter, as
/// the table lists them (`Starlark.classType` names); `None` when the table has no text for it.
// Java: net/starlark/java/eval/BuiltinFunction.java:388 checkParamValue
pub(crate) fn java_param_accepts<'v>(
    function: &str,
    receiver: Option<&str>,
    param: &str,
    v: Value<'v>,
) -> Option<bool> {
    let entries = JAVA_PARAM_TYPES.get(&(function, param))?;
    let own = receiver.unwrap_or("");
    let want = match entries.iter().find(|(rc, _)| *rc == own) {
        Some((_, w)) => *w,
        None if entries.iter().all(|(_, w)| *w == entries[0].1) => entries[0].1,
        None => return None,
    };
    if want == "-" {
        return Some(true);
    }
    let names = want.replace(", or ", ", ").replace(" or ", ", ");
    let ty = v.get_type();
    Some(names.split(", ").any(|n| match n {
        "NoneType" => v.is_none(),
        "string" => v.unpack_str().is_some(),
        "sequence" => ty == "list" || ty == "tuple",
        "iterable" => matches!(ty, "list" | "tuple" | "dict" | "set" | "range" | "depset"),
        "callable" => v.vtable().starlark_value.HAS_invoke,
        "unknown" | "Object" => true,
        other => ty == other,
    }))
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
        // BLAZE PATCH 0061: a binding error (Bazel's location for it, `eval::runtime::java_call`).
        Some(v) => Err(bazel_argument_binding_error(format!(
            "in call to {function}(), parameter '{param}' got value of type '{}', want 'callable or NoneType'", // BLAZE PATCH 0061
            v.get_type()
        ))),
    }
}

/// BLAZE PATCH 0061: an error a host builtin raises while checking its arguments the way Java's
/// `BuiltinFunction` binds them (`checkParamValue`: "in call to F(), parameter 'P' got value of
/// type 'T', want 'W'"). Bazel reports such an error at the call's enclosing statement when the
/// call has named or `*`/`**` arguments (`eval::runtime::java_call`); a plain error would stay
/// at the `(`.
pub fn bazel_argument_binding_error(message: String) -> crate::Error {
    crate::Error::new_value(crate::eval::runtime::java_call::JavaArgBinding(message))
}
