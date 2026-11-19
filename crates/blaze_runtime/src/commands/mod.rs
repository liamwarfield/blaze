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
// Integration lead: mod, build, test.
pub mod bep;
pub mod build;
pub mod mod_command;
pub mod target_patterns_helper;
// run.
pub mod run;
// cmds implementer: license, canonicalize-flags, fetch.
pub mod canonicalize;
pub mod fetch;
pub mod license;
