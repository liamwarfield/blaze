//! Shared fixtures for the ported client rc tests (rc_file_test.cc, option_processor_test.cc).

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use blaze_options::rc::option_processor::{ClientEnvironment, RcStartupFlag};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh, empty directory under the system temp dir (Java tests: TEST_TMPDIR).
pub fn fresh_tmpdir(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "blaze_options_rc_tests_{}_{}_{}",
        std::process::id(),
        name,
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

pub fn join(a: &str, b: &str) -> String {
    blaze_options::rc::tokenize::join_path(a, b)
}

pub fn mkdirs(dir: &str) {
    std::fs::create_dir_all(dir).unwrap();
}

/// `blaze_util::WriteFile(contents, path, 0755)`.
pub fn write_file(contents: &str, path: &str) {
    write_file_mode(contents, path, 0o755);
}

pub fn write_file_mode(contents: &str, path: &str, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(path);
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// An environment with the given cwd and home and nothing else.
pub fn client_env(cwd: &str, home: &str) -> ClientEnvironment {
    ClientEnvironment {
        cwd: cwd.to_string(),
        home: home.to_string(),
        ..Default::default()
    }
}

/// Port of `OptionProcessor::PrintStartupOptionsProvenanceMessage` over the recorded startup
/// flags (all values in these tests use `--flag=value`, so no space-separated joining is needed).
// Java: src/main/cpp/option_processor.cc:576 OptionProcessor::PrintStartupOptionsProvenanceMessage
pub fn provenance_message(flags: &[RcStartupFlag]) -> String {
    fn print(out: &mut String, source: &str, options: &[String]) {
        if !source.is_empty() {
            out.push_str(&format!(
                "INFO: Reading 'startup' options from {}: {}\n",
                source,
                options.join(" ")
            ));
        }
    }
    let mut out = String::new();
    let mut most_recent_blazerc = String::new();
    let mut accumulated: Vec<String> = Vec::new();
    for flag in flags {
        if flag.source == most_recent_blazerc {
            accumulated.push(flag.value.clone());
        } else {
            print(&mut out, &most_recent_blazerc, &accumulated);
            accumulated = vec![flag.value.clone()];
            most_recent_blazerc = flag.source.clone();
        }
    }
    print(&mut out, &most_recent_blazerc, &accumulated);
    out
}

/// gmock `MatchesRegex` (full match).
pub fn matches_regex(s: &str, re: &str) -> bool {
    regex_full(re).is_match(s)
}

/// gmock `ContainsRegex`.
pub fn contains_regex(s: &str, re: &str) -> bool {
    regex::Regex::new(&format!("(?s){}", re))
        .unwrap()
        .is_match(s)
}

fn regex_full(re: &str) -> regex::Regex {
    regex::Regex::new(&format!("(?s)^(?:{})$", re)).unwrap()
}
