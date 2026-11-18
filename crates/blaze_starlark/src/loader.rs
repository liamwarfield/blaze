//! `load()` resolution for one evaluation.
//!
//! Bazel resolves every `load()` label of a file before executing it (BzlLoadFunction /
//! PackageFunction request the BZL_LOAD values first), so the evaluator only needs a lookup
//! table from the literal load string to the already-frozen module.

use std::collections::HashMap;

use starlark::environment::FrozenModule;
use starlark::eval::FileLoader;
use starlark::syntax::AstModule;

use crate::error::Location;

/// A load string as written in the file, with the location of the string literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadRequest {
    pub module: String,
    pub location: Location,
    /// Symbols loaded: (local name, exported name).
    pub symbols: Vec<(String, String)>,
}

/// The loads of a parsed file, in source order (Java `StarlarkFile` load statements).
pub fn load_requests(ast: &AstModule) -> Vec<LoadRequest> {
    ast.loads()
        .into_iter()
        .map(|l| LoadRequest {
            module: l.module_id.to_owned(),
            location: Location::of_span_start(&l.span),
            symbols: l
                .symbols
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect(),
        })
        .collect()
}

/// Map from load string to frozen module.
#[derive(Default, Clone)]
pub struct LoadMap {
    modules: HashMap<String, FrozenModule>,
}

impl LoadMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, load_string: impl Into<String>, module: FrozenModule) {
        self.modules.insert(load_string.into(), module);
    }

    pub fn get(&self, load_string: &str) -> Option<&FrozenModule> {
        self.modules.get(load_string)
    }
}

impl FileLoader for LoadMap {
    fn load(&self, path: &str) -> starlark::Result<FrozenModule> {
        self.modules.get(path).cloned().ok_or_else(|| {
            // Callers resolve every load first; reaching this is a Blaze bug.
            starlark::Error::new_other(anyhow::anyhow!(
                "internal error: load '{path}' was not resolved before evaluation"
            ))
        })
    }
}
