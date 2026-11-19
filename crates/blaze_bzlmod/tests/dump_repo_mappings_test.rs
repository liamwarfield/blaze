//! `ModCommand.dumpRepoMappings` and the `dump_repo_mapping` branch of `execInternal`. Bazel has
//! no JUnit test for them; the expected bytes are the oracle's (Bazel 9.3.2) stdout and stderr
//! for `bazel mod dump_repo_mapping` on the brotli corpus project (scripts/mod/compare, cases
//! brotli/dump_main, brotli/dump_multi and brotli/dump_unknown_two).

use std::sync::Mutex;

use blaze_bzlmod::modcommand::mod_command::{
    ModCommandResult, dump_repo_mappings, exec_dump_repo_mapping,
};
use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{RepositoryMapping, RepositoryName};
use blaze_events::{Event, EventHandler, ExtendedEventHandler, Postable};
use blaze_optdefs::misc::mod_options::{Charset, ModOptions};

fn repo(name: &str) -> RepositoryName {
    RepositoryName::create(name).unwrap()
}

/// The main repository's mapping of brotli (`module(repo_name = "org_brotli")`, one
/// `bazel_dep` on rules_cc), in Bazel's entry order.
fn brotli_main_mapping() -> RepositoryMapping {
    RepositoryMapping::create(
        [
            ("", MAIN.clone()),
            ("org_brotli", MAIN.clone()),
            ("rules_cc", repo("rules_cc+")),
            ("bazel_tools", repo("bazel_tools")),
        ],
        MAIN.clone(),
    )
}

#[derive(Default)]
struct Collect(Mutex<Vec<String>>);

impl EventHandler for Collect {
    fn handle(&self, event: &Event) {
        self.0.lock().unwrap().push(event.get_message().to_string());
    }
}

impl ExtendedEventHandler for Collect {
    fn post(&self, _: std::sync::Arc<dyn Postable>) {}
}

#[test]
fn dump_repo_mappings_main_repo() {
    let mapping = brotli_main_mapping();
    let mut out = Vec::new();
    dump_repo_mappings(&[&mapping], Charset::UTF8, &mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\"\":\"\",\"org_brotli\":\"\",\"rules_cc\":\"rules_cc+\",\"bazel_tools\":\"bazel_tools\"}\n"
    );
}

#[test]
fn dump_repo_mappings_one_line_per_mapping() {
    let main = brotli_main_mapping();
    let mut out = Vec::new();
    dump_repo_mappings(&[&main, &main], Charset::ASCII, &mut out).unwrap();
    let line = "{\"\":\"\",\"org_brotli\":\"\",\"rules_cc\":\"rules_cc+\",\"bazel_tools\":\"bazel_tools\"}\n";
    assert_eq!(String::from_utf8(out).unwrap(), format!("{line}{line}"));
}

#[test]
fn exec_dump_repo_mapping_reports_missing_repos_in_argument_order() {
    let rules_cc = RepositoryMapping::create([("rules_cc", repo("rules_cc+"))], repo("rules_cc+"));
    let names = [repo("rules_cc+"), repo("nope1"), repo("nope2")];
    let reporter = Collect::default();
    let mut out = Vec::new();
    let result = exec_dump_repo_mapping(
        &names,
        &[Some(&rules_cc), None, None],
        &ModOptions::get_default_options(),
        &reporter,
        &mut out,
    );
    let message =
        "Repositories not found: nope1, nope2. Type 'bazel help mod' for syntax and help.";
    assert_eq!(
        result,
        ModCommandResult::Failure {
            message: message.to_string(),
            code: "INVALID_ARGUMENTS",
            exit_code: 2,
        }
    );
    assert_eq!(*reporter.0.lock().unwrap(), vec![message.to_string()]);
    assert!(out.is_empty());
}
