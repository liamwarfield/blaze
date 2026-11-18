//! Port of com.google.devtools.build.lib.query2 (query, output formatters) and a temporary dev
//! entry point (`src/bin/blaze-query-dev.rs`).
//!
//! Modules follow the Java packages: `common` (query2/common), `query` (query2/query, with
//! `query::output` and `query::aspectresolvers`), `commands` (runtime/commands, the `query`
//! command), `pkgcache` (the lib/pkgcache
//! interfaces query consumes) and `compat` (lib/packages helpers only query uses, to move to
//! blaze_packages).

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod commands;
pub mod common;
pub mod compat;
pub mod pkgcache;
pub mod query;
