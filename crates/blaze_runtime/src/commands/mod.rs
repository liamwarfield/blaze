//! Port of `com.google.devtools.build.lib.runtime.commands`: the `version`, `info`, `clean`,
//! `shutdown` and `help` commands.

pub mod clean;
pub mod help;
pub mod info;
pub mod shutdown;
pub mod version;
// W4 analysis lead: query/cquery/aquery.
pub mod aquery;
pub mod cquery;
pub mod query;
