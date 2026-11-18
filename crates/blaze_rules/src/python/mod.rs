//! Python rule natives (agent PY; crates/blaze_analysis/DESIGN.md): `py_builtins`
//! (`rules/python/PyBuiltins.java`, `bazel/rules/python/BazelPyBuiltins.java`) and
//! `rules/python/PythonUtils.java`.
//!
//! The `py` and `bazel_py` configuration fragments' Starlark values are in
//! `crate::fragments::python_configuration`.

pub mod py_builtins;
pub mod python_utils;

pub use py_builtins::{PyBuiltins, bazel_py_builtins, py_builtins_value};
