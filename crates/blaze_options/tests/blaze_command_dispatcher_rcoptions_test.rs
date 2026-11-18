//! Port of src/test/java/com/google/devtools/build/lib/runtime/BlazeCommandDispatcherRcoptionsTest.java.
//!
//! PORT: the Java tests run commands through BlazeCommandDispatcher and read the command's
//! printed output. The dispatcher is not ported; these tests drive the same option parsing
//! (BlazeOptionHandler.parseOptions with ClientOptions + the command's options) and check the
//! values the command would print.

mod common;

use blaze_options::rc::client_options::ClientOptions;
use blaze_options::rc::handler::{BlazeOptionHandler, Command, StoredEventHandler};
use blaze_options::{OptionsClass, OptionsParser, options};
use common::common_command_options::CommonCommandOptions;

options! {
    pub struct FooOptions {
        #[option(
            name = "numoption",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub num_option: i32,

        #[option(
            name = "stringoption",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "[unspecified]",
        )]
        pub string_option: String,
    }
}

static REPORT_NUM: Command = Command {
    name: "reportnum",
    inherits_options_from: &[],
};
static REPORT_ALL: Command = Command {
    name: "reportall",
    inherits_options_from: &[],
};
static REPORT_ALL_INHERITED: Command = Command {
    name: "reportallinherited",
    inherits_options_from: &[&REPORT_ALL],
};

/// Parses `cmd_line` (command name first) for `command` and returns its FooOptions.
fn exec(command: &'static Command, commands: &[&str], cmd_line: &[&str]) -> FooOptions {
    // Every command gets CommonCommandOptions (BlazeCommandUtils.COMMON_COMMAND_OPTIONS).
    let classes = vec![
        FooOptions::class(),
        CommonCommandOptions::class(),
        ClientOptions::class(),
    ];
    let mut parser = OptionsParser::builder()
        .options_classes(&classes)
        .build()
        .unwrap();
    let mut events = StoredEventHandler::default();
    {
        let mut h = BlazeOptionHandler::new(
            "bazel",
            command,
            commands.iter().copied(),
            classes.clone(),
            &mut parser,
        );
        let args: Vec<String> = cmd_line.iter().map(|s| s.to_string()).collect();
        h.parse_options(&args, &mut events).unwrap();
    }
    assert!(events.is_empty(), "{:?}", events);
    parser.get_options::<FooOptions>().unwrap()
}

#[test]
fn test_common_used() {
    let out = exec(
        &REPORT_NUM,
        &["reportnum"],
        &[
            "reportnum",
            "--rc_source=/home/jrluser/.blazerc",
            "--default_override=0:common=--numoption=99",
        ],
    );
    assert_eq!(
        out.num_option.to_string(),
        "99",
        "Common options should be used"
    );
}

#[test]
fn test_specific_options_win() {
    let out = exec(
        &REPORT_NUM,
        &["reportnum"],
        &[
            "reportnum",
            "--rc_source=/home/jrluser/.blazerc",
            "--default_override=0:reportnum=--numoption=42",
            "--default_override=0:common=--numoption=99",
        ],
    );
    assert_eq!(out.num_option.to_string(), "42");
}

#[test]
fn test_specific_options_win_other_order() {
    let out = exec(
        &REPORT_NUM,
        &["reportnum"],
        &[
            "reportnum",
            "--rc_source=/home/jrluser/.blazerc",
            "--default_override=0:common=--numoption=99",
            "--default_override=0:reportnum=--numoption=42",
        ],
    );
    assert_eq!(out.num_option.to_string(), "42");
}

#[test]
fn test_options_combined() {
    let out = exec(
        &REPORT_ALL,
        &["reportnum", "reportall"],
        &[
            "reportall",
            "--rc_source=/etc/bazelrc",
            "--default_override=0:common=--stringoption=foo",
            "--rc_source=/home/jrluser/.blazerc",
            "--default_override=1:common=--numoption=99",
        ],
    );
    assert_eq!(
        format!("{} {}", out.num_option, out.string_option),
        "99 foo"
    );
}

#[test]
fn test_options_combined_with_override() {
    let out = exec(
        &REPORT_ALL,
        &["reportnum", "reportall"],
        &[
            "reportall",
            "--rc_source=/etc/bazelrc",
            "--default_override=0:common=--stringoption=foo",
            "--default_override=0:common=--numoption=42",
            "--rc_source=/home/jrluser/.blazerc",
            "--default_override=1:common=--numoption=99",
        ],
    );
    assert_eq!(
        format!("{} {}", out.num_option, out.string_option),
        "99 foo"
    );
}

#[test]
fn test_options_combined_with_override_other_name() {
    let out = exec(
        &REPORT_ALL,
        &["reportnum", "reportall"],
        &[
            "reportall",
            "--rc_source=/home/jrluser/.blazerc",
            "--default_override=0:common=--stringoption=foo",
            "--default_override=0:common=--numoption=42",
            "--rc_source=/etc/bazelrc",
            "--default_override=1:common=--numoption=99",
        ],
    );
    assert_eq!(
        format!("{} {}", out.num_option, out.string_option),
        "99 foo"
    );
}

fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, x.clone());
            out.push(p);
        }
    }
    out
}

#[test]
fn test_inherited_options_with_specific_override() {
    let blazerc_opts: Vec<Vec<&str>> = vec![
        vec![
            "--rc_source=/doesnt/matter/0/bazelrc",
            "--default_override=0:common=--stringoption=common",
            "--default_override=0:common=--numoption=42",
        ],
        vec![
            "--rc_source=/doesnt/matter/1/bazelrc",
            "--default_override=0:reportall=--stringoption=reportall",
        ],
        vec![
            "--rc_source=/doesnt/matter/2/bazelrc",
            "--default_override=0:reportallinherited=--stringoption=reportallinherited",
        ],
    ];
    for e in permutations(&blazerc_opts) {
        let mut cmd_line = vec!["reportallinherited"];
        let ordered: Vec<&str> = e.into_iter().flatten().collect();
        cmd_line.extend(ordered.iter().copied());
        let out = exec(
            &REPORT_ALL_INHERITED,
            &["reportnum", "reportall", "reportallinherited"],
            &cmd_line,
        );
        assert_eq!(
            format!("{} {}", out.num_option, out.string_option),
            "42 reportallinherited",
            "The more specific option should override, irrespective of source file or order. {:?}",
            ordered
        );
    }
}
