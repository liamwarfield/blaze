//! Port of src/test/java/com/google/devtools/build/lib/runtime/BlazeOptionHandlerTest.java.
//!
//! The Java fixture builds a full BlazeRuntime with a mock `build` command; here the handler is
//! built directly with the same options classes (TestOptions, the rc-relevant subset of
//! CommonCommandOptions, ClientOptions), the same command and product name.

mod common;

use blaze_options::rc::client_options::{ClientOptions, OptionOverride};
use blaze_options::rc::handler::{
    BlazeOptionHandler, Command, CommandToRcArgs, HandlerEvent, RcChunkOfArgs, StoredEventHandler,
    structure_rc_options_and_configs,
};
use blaze_options::{OptionsClass, OptionsParser};
use common::common_command_options::CommonCommandOptions;
use common::test_options::TestOptions;
use indexmap::IndexSet;

static BUILD: Command = Command {
    name: "build",
    inherits_options_from: &[],
};

fn new_parser() -> OptionsParser {
    OptionsParser::builder()
        .options_classes(&[
            TestOptions::class(),
            CommonCommandOptions::class(),
            ClientOptions::class(),
        ])
        .allow_residue(true)
        .with_alias_flag(None)
        .skip_starlark_option_prefixes()
        .build()
        .unwrap()
}

fn handler(parser: &mut OptionsParser) -> BlazeOptionHandler<'_> {
    BlazeOptionHandler::new(
        "bazel",
        &BUILD,
        ["build"],
        vec![
            TestOptions::class(),
            CommonCommandOptions::class(),
            ClientOptions::class(),
        ],
        parser,
    )
}

fn chunk(rc: &str, args: &[&str]) -> RcChunkOfArgs {
    RcChunkOfArgs::new(rc, args.iter().map(|s| s.to_string()).collect())
}

fn put(m: &mut CommandToRcArgs, key: &str, c: RcChunkOfArgs) {
    m.entry(key.to_string()).or_default().push(c);
}

fn structured_args_from_2_simple_rcs_with_only_residue() -> CommandToRcArgs {
    let mut m = CommandToRcArgs::new();
    put(&mut m, "build", chunk("rc1", &["a"]));
    put(&mut m, "build:config", chunk("rc1", &["b"]));
    put(&mut m, "common", chunk("rc2", &["c"]));
    put(&mut m, "build", chunk("rc2", &["d", "e"]));
    put(&mut m, "c1:other", chunk("rc2", &["f", "g"]));
    m
}

fn structured_args_from_2_simple_rcs_with_flags() -> CommandToRcArgs {
    let mut m = CommandToRcArgs::new();
    put(
        &mut m,
        "build",
        chunk("rc1", &["--test_multiple_string=foo"]),
    );
    put(
        &mut m,
        "build:config",
        chunk("rc1", &["--test_multiple_string=config"]),
    );
    put(
        &mut m,
        "common",
        chunk("rc2", &["--test_multiple_string=common"]),
    );
    put(
        &mut m,
        "build",
        chunk("rc2", &["--test_multiple_string=bar"]),
    );
    put(
        &mut m,
        "c1:other",
        chunk("rc2", &["--test_multiple_string=other"]),
    );
    m
}

fn structured_args_from_imported_rcs_with_only_residue() -> CommandToRcArgs {
    let mut m = structured_args_from_2_simple_rcs_with_only_residue();
    put(&mut m, "build", chunk("rc1", &["h"]));
    m
}

fn structured_args_for_different_platforms() -> CommandToRcArgs {
    let mut m = CommandToRcArgs::new();
    put(&mut m, "build:linux", chunk("rc1", &["command_linux"]));
    put(&mut m, "build:windows", chunk("rc1", &["command_windows"]));
    put(&mut m, "build:macos", chunk("rc1", &["command_macos"]));
    put(&mut m, "build:freebsd", chunk("rc1", &["command_freebsd"]));
    put(&mut m, "build:openbsd", chunk("rc1", &["command_openbsd"]));
    put(
        &mut m,
        "build:platform_config",
        chunk("rc1", &["--enable_platform_specific_config"]),
    );
    m
}

/// Compares two multimaps the way Guava's ListMultimap.equals does (key order ignored).
fn assert_multimap_eq(a: &CommandToRcArgs, b: &CommandToRcArgs) {
    assert_eq!(a.len(), b.len(), "{:?} vs {:?}", a, b);
    for (k, v) in a {
        assert_eq!(Some(v), b.get(k), "key {}", k);
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn commands(v: &[&str]) -> IndexSet<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn parse_options(args: &[&str]) -> (OptionsParser, StoredEventHandler, Vec<String>) {
    let mut parser = new_parser();
    let mut events = StoredEventHandler::default();
    let notes = {
        let mut h = handler(&mut parser);
        let _ = h.parse_options(&strs(args), &mut events);
        h.get_rcfile_notes().to_vec()
    };
    (parser, events, notes)
}

fn multiple(parser: &OptionsParser) -> Vec<String> {
    parser
        .get_options::<TestOptions>()
        .unwrap()
        .test_multiple_string
}

// Java: BlazeOptionHandlerTest.testStructureRcOptionsAndConfigs_argumentless
#[test]
fn test_structure_rc_options_and_configs_argumentless() {
    let mut events = StoredEventHandler::default();
    let structured = structure_rc_options_and_configs(
        &mut events,
        &strs(&["rc1", "rc2"]),
        &[],
        &commands(&["build", "c1"]),
    )
    .unwrap();
    assert!(structured.is_empty());
    assert!(events.is_empty());
}

#[test]
fn test_structure_rc_options_and_configs_config_only() {
    let mut events = StoredEventHandler::default();
    structure_rc_options_and_configs(
        &mut events,
        &strs(&["rc1", "rc2"]),
        &[OptionOverride::new(0, "build:none", "a")],
        &commands(&["build"]),
    )
    .unwrap();
    assert!(events.is_empty());
}

#[test]
fn test_structure_rc_options_and_configs_invalid_command() {
    let mut events = StoredEventHandler::default();
    structure_rc_options_and_configs(
        &mut events,
        &strs(&["rc1", "rc2"]),
        &[OptionOverride::new(0, "c1", "a")],
        &commands(&["build"]),
    )
    .unwrap();
    assert!(events.get_events().contains(&HandlerEvent::warn(
        "while reading option defaults file 'rc1':\n  invalid command name 'c1'."
    )));
}

#[test]
fn test_structure_rc_options_and_configs_two_rcs() {
    let mut events = StoredEventHandler::default();
    let structured = structure_rc_options_and_configs(
        &mut events,
        &strs(&["rc1", "rc2"]),
        &[
            OptionOverride::new(0, "build", "a"),
            OptionOverride::new(0, "build:config", "b"),
            OptionOverride::new(1, "common", "c"),
            OptionOverride::new(1, "build", "d"),
            OptionOverride::new(1, "build", "e"),
            OptionOverride::new(1, "c1:other", "f"),
            OptionOverride::new(1, "c1:other", "g"),
        ],
        &commands(&["build", "c1"]),
    )
    .unwrap();
    assert_multimap_eq(
        &structured,
        &structured_args_from_2_simple_rcs_with_only_residue(),
    );
    assert!(events.is_empty());
}

#[test]
fn test_structure_rc_options_and_configs_imported_rcs() {
    let mut events = StoredEventHandler::default();
    let structured = structure_rc_options_and_configs(
        &mut events,
        &strs(&["rc1", "rc2"]),
        &[
            OptionOverride::new(0, "build", "a"),
            OptionOverride::new(0, "build:config", "b"),
            OptionOverride::new(1, "common", "c"),
            OptionOverride::new(1, "build", "d"),
            OptionOverride::new(1, "build", "e"),
            OptionOverride::new(1, "c1:other", "f"),
            OptionOverride::new(1, "c1:other", "g"),
            OptionOverride::new(0, "build", "h"),
        ],
        &commands(&["build", "c1"]),
    )
    .unwrap();
    assert_multimap_eq(
        &structured,
        &structured_args_from_imported_rcs_with_only_residue(),
    );
    assert!(events.is_empty());
}

#[test]
fn test_structure_rc_options_and_configs_bad_override_index() {
    let mut events = StoredEventHandler::default();
    let structured = structure_rc_options_and_configs(
        &mut events,
        &strs(&["rc1", "rc2"]),
        &[
            OptionOverride::new(0, "build", "a"),
            OptionOverride::new(0, "build:config", "b"),
            OptionOverride::new(2, "c4:other", "z"),
            OptionOverride::new(-1, "c3:other", "q"),
            OptionOverride::new(1, "common", "c"),
            OptionOverride::new(1, "build", "d"),
            OptionOverride::new(1, "build", "e"),
            OptionOverride::new(1, "c1:other", "f"),
            OptionOverride::new(1, "c1:other", "g"),
        ],
        &commands(&["build", "c1"]),
    )
    .unwrap();
    assert_multimap_eq(
        &structured,
        &structured_args_from_2_simple_rcs_with_only_residue(),
    );
    let bogus = HandlerEvent::warn(
        "inconsistency in generated command line args. Ignoring bogus argument\n",
    );
    assert_eq!(
        events.get_events().iter().filter(|e| **e == bogus).count(),
        2
    );
}

#[test]
fn test_parse_rc_options_empty() {
    let mut parser = new_parser();
    let mut events = StoredEventHandler::default();
    handler(&mut parser)
        .parse_rc_options(&mut events, &CommandToRcArgs::new())
        .unwrap();
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
}

#[test]
fn test_parse_rc_options_flat_rcs_residue() {
    let mut parser = new_parser();
    let mut events = StoredEventHandler::default();
    handler(&mut parser)
        .parse_rc_options(
            &mut events,
            &structured_args_from_2_simple_rcs_with_only_residue(),
        )
        .unwrap();
    assert!(events.is_empty());
    assert_eq!(parser.get_residue(), strs(&["c", "a", "d", "e"]));
}

#[test]
fn test_parse_rc_options_flat_rcs_flags() {
    let mut parser = new_parser();
    let mut events = StoredEventHandler::default();
    handler(&mut parser)
        .parse_rc_options(&mut events, &structured_args_from_2_simple_rcs_with_flags())
        .unwrap();
    assert!(events.is_empty());
    assert_eq!(multiple(&parser), strs(&["common", "foo", "bar"]));
}

#[test]
fn test_parse_rc_options_imported_rcs_residue() {
    let mut parser = new_parser();
    let mut events = StoredEventHandler::default();
    handler(&mut parser)
        .parse_rc_options(
            &mut events,
            &structured_args_from_imported_rcs_with_only_residue(),
        )
        .unwrap();
    assert!(events.is_empty());
    assert_eq!(parser.get_residue(), strs(&["c", "a", "d", "e", "h"]));
}

#[test]
fn test_expand_config_options_configless() {
    let mut parser = new_parser();
    let mut events = StoredEventHandler::default();
    handler(&mut parser)
        .expand_config_options(
            &mut events,
            &structured_args_from_2_simple_rcs_with_only_residue(),
        )
        .unwrap();
    assert!(parser.get_residue().is_empty());
}

#[test]
fn test_expand_config_options_with_config() {
    let mut parser = new_parser();
    parser.parse(&["--config=config"]).unwrap();
    let mut events = StoredEventHandler::default();
    let notes = {
        let mut h = handler(&mut parser);
        h.expand_config_options(
            &mut events,
            &structured_args_from_2_simple_rcs_with_only_residue(),
        )
        .unwrap();
        h.get_rcfile_notes().to_vec()
    };
    assert_eq!(parser.get_residue(), strs(&["b"]));
    assert_eq!(
        notes,
        strs(&["Found applicable config definition build:config in file rc1: b"])
    );
}

#[test]
fn test_expand_config_options_with_platform_specific_config_enabled() {
    let mut parser = new_parser();
    parser
        .parse(&["--enable_platform_specific_config"])
        .unwrap();
    let mut events = StoredEventHandler::default();
    handler(&mut parser)
        .expand_config_options(&mut events, &structured_args_for_different_platforms())
        .unwrap();
    // Linux only.
    assert_eq!(parser.get_residue(), strs(&["command_linux"]));
}

#[test]
fn test_expand_config_options_with_platform_specific_config_enabled_in_config() {
    let mut parser = new_parser();
    parser.parse(&["--config=platform_config"]).unwrap();
    let mut events = StoredEventHandler::default();
    handler(&mut parser)
        .expand_config_options(&mut events, &structured_args_for_different_platforms())
        .unwrap();
    assert_eq!(parser.get_residue(), strs(&["command_linux"]));
}

#[test]
fn test_expand_config_options_with_platform_specific_config_enabled_when_nothing_specified() {
    let mut parser = new_parser();
    parser
        .parse(&["--enable_platform_specific_config"])
        .unwrap();
    let mut events = StoredEventHandler::default();
    handler(&mut parser)
        .parse_rc_options(&mut events, &CommandToRcArgs::new())
        .unwrap();
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
}

#[test]
fn test_expand_config_options_with_config_for_unapplicable_command() {
    let mut parser = new_parser();
    parser.parse(&["--config=other"]).unwrap();
    let mut events = StoredEventHandler::default();
    let (e, notes) = {
        let mut h = handler(&mut parser);
        let e = h
            .expand_config_options(
                &mut events,
                &structured_args_from_2_simple_rcs_with_only_residue(),
            )
            .unwrap_err();
        (e, h.get_rcfile_notes().to_vec())
    };
    assert!(parser.get_residue().is_empty());
    assert!(notes.is_empty());
    assert!(
        e.get_message()
            .contains("Config value 'other' is not defined in any .rc file")
    );
}

#[test]
fn test_expand_config_options_skipped_args_order_preserved() {
    let mut rc_content = CommandToRcArgs::new();
    put(
        &mut rc_content,
        "build:config1",
        chunk("rc1", &["--//f=2", "--//f=3"]),
    );
    put(
        &mut rc_content,
        "build:config2b",
        chunk("rc1", &["--//f=6"]),
    );
    put(
        &mut rc_content,
        "build:config2",
        chunk("rc1", &["--config=config2a", "--config=config2b"]),
    );
    put(
        &mut rc_content,
        "build:config2a",
        chunk("rc1", &["--//f=5"]),
    );
    let mut parser = new_parser();
    parser
        .parse(&[
            "--test_multiple_string=1",
            "--//f=1",
            "--test_multiple_string=2",
            "--config=config1",
            "--test_multiple_string=3",
            "--//f=4",
            "--test_multiple_string=4",
            "--config=config2",
            "--test_multiple_string=5",
            "--//f=7",
            "--test_multiple_string=6",
        ])
        .unwrap();
    let mut events = StoredEventHandler::default();
    handler(&mut parser)
        .expand_config_options(&mut events, &rc_content)
        .unwrap();

    assert_eq!(
        parser.get_skipped_args(),
        strs(&[
            "--//f=1", "--//f=2", "--//f=3", "--//f=4", "--//f=5", "--//f=6", "--//f=7"
        ])
    );
    let expected = strs(&[
        "--test_multiple_string=1",
        "--test_multiple_string=2",
        "--config=config1",
        "--test_multiple_string=3",
        "--test_multiple_string=4",
        "--config=config2",
        "--config=config2a",
        "--config=config2b",
        "--test_multiple_string=5",
        "--test_multiple_string=6",
    ]);
    let canonical: Vec<String> = parser
        .as_list_of_canonical_options()
        .iter()
        .map(|p| p.get_canonical_form())
        .collect();
    assert_eq!(canonical, expected);
    let complete: Vec<String> = parser
        .as_complete_list_of_parsed_options()
        .iter()
        .map(|p| p.get_canonical_form())
        .collect();
    assert_eq!(complete, expected);
}

#[test]
fn test_undefined_config() {
    let mut parser = new_parser();
    parser.parse(&["--config=invalid"]).unwrap();
    let mut events = StoredEventHandler::default();
    let e = handler(&mut parser)
        .expand_config_options(&mut events, &CommandToRcArgs::new())
        .unwrap_err();
    assert!(
        e.get_message()
            .contains("Config value 'invalid' is not defined in any .rc file")
    );
}

#[test]
fn test_parse_options_argless() {
    let (parser, events, notes) = parse_options(&["build"]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert!(notes.is_empty());
}

#[test]
fn test_parse_options_residue() {
    let (parser, events, notes) = parse_options(&["build", "res"]);
    assert!(events.is_empty());
    assert!(parser.get_residue().contains(&"res".to_string()));
    assert!(notes.is_empty());
}

#[test]
fn test_parse_options_explicit_option() {
    let (parser, events, notes) = parse_options(&["build", "--test_multiple_string=explicit"]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert!(notes.is_empty());
    assert_eq!(multiple(&parser), strs(&["explicit"]));
}

#[test]
fn test_parse_options_rc_option() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--test_multiple_string=rc_a",
        "--default_override=0:build=--test_multiple_string=rc_b",
        "--rc_source=/somewhere/.blazerc",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc_a --test_multiple_string=rc_b"
        ])
    );
    assert_eq!(multiple(&parser), strs(&["rc_a", "rc_b"]));
}

#[test]
fn test_parse_options_multiple_rcs() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--test_multiple_string=rc1_a",
        "--default_override=1:build=--test_multiple_string=rc2",
        "--default_override=0:build=--test_multiple_string=rc1_b",
        "--rc_source=/somewhere/.blazerc",
        "--rc_source=/some/other/.blazerc",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc1_a",
            "Reading rc options for 'build' from /some/other/.blazerc:\n  'build' options: --test_multiple_string=rc2",
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc1_b",
        ])
    );
    assert_eq!(multiple(&parser), strs(&["rc1_a", "rc2", "rc1_b"]));
}

#[test]
fn test_parse_options_multiple_rcs_with_multiple_commands() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--test_multiple_string=rc1_a",
        "--default_override=1:build=--test_multiple_string=rc2",
        "--default_override=1:common=--test_multiple_string=rc2_common",
        "--default_override=0:build=--test_multiple_string=rc1_b",
        "--default_override=0:common=--test_multiple_string=rc1_common",
        "--rc_source=/somewhere/.blazerc",
        "--rc_source=/some/other/.blazerc",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /some/other/.blazerc:\n  Inherited 'common' options: --test_multiple_string=rc2_common",
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  Inherited 'common' options: --test_multiple_string=rc1_common",
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc1_a",
            "Reading rc options for 'build' from /some/other/.blazerc:\n  'build' options: --test_multiple_string=rc2",
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc1_b",
        ])
    );
    assert_eq!(
        multiple(&parser),
        strs(&["rc2_common", "rc1_common", "rc1_a", "rc2", "rc1_b"])
    );
}

#[test]
fn test_parse_options_rc_option_and_explicit() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--test_multiple_string=rc",
        "--rc_source=/somewhere/.blazerc",
        "--test_multiple_string=explicit",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc"
        ])
    );
    assert_eq!(multiple(&parser), strs(&["rc", "explicit"]));
}

#[test]
fn test_parse_options_multi_command_rc_option_and_explicit() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--test_multiple_string=rc_build_1",
        "--default_override=0:common=--test_multiple_string=rc_common",
        "--default_override=0:build=--test_multiple_string=rc_build_2",
        "--rc_source=/somewhere/.blazerc",
        "--test_multiple_string=explicit",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  Inherited 'common' options: --test_multiple_string=rc_common",
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc_build_1 --test_multiple_string=rc_build_2",
        ])
    );
    assert_eq!(
        multiple(&parser),
        strs(&["rc_common", "rc_build_1", "rc_build_2", "explicit"])
    );
}

#[test]
fn test_parse_options_multiple_rcs_with_multiple_commands_plus_explicit_option() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--test_multiple_string=rc1_a",
        "--default_override=1:build=--test_multiple_string=rc2",
        "--test_multiple_string=explicit",
        "--default_override=1:common=--test_multiple_string=rc2_common",
        "--default_override=0:build=--test_multiple_string=rc1_b",
        "--default_override=0:common=--test_multiple_string=rc1_common",
        "--rc_source=/somewhere/.blazerc",
        "--rc_source=/some/other/.blazerc",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /some/other/.blazerc:\n  Inherited 'common' options: --test_multiple_string=rc2_common",
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  Inherited 'common' options: --test_multiple_string=rc1_common",
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc1_a",
            "Reading rc options for 'build' from /some/other/.blazerc:\n  'build' options: --test_multiple_string=rc2",
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc1_b",
        ])
    );
    assert_eq!(
        multiple(&parser),
        strs(&[
            "rc2_common",
            "rc1_common",
            "rc1_a",
            "rc2",
            "rc1_b",
            "explicit"
        ])
    );
}

#[test]
fn test_parse_options_explicit_config() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--test_multiple_string=rc",
        "--default_override=0:build:conf=--test_multiple_string=config",
        "--rc_source=/somewhere/.blazerc",
        "--test_multiple_string=explicit",
        "--config=conf",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc",
            "Found applicable config definition build:conf in file /somewhere/.blazerc: --test_multiple_string=config",
        ])
    );
    assert_eq!(multiple(&parser), strs(&["rc", "explicit", "config"]));
}

#[test]
fn test_parse_options_rc_specified_config() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--config=conf",
        "--default_override=0:build=--test_multiple_string=rc",
        "--default_override=0:build:conf=--test_multiple_string=config",
        "--rc_source=/somewhere/.blazerc",
        "--test_multiple_string=explicit",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --config=conf --test_multiple_string=rc",
            "Found applicable config definition build:conf in file /somewhere/.blazerc: --test_multiple_string=config",
        ])
    );
    assert_eq!(multiple(&parser), strs(&["config", "rc", "explicit"]));
}

#[test]
fn test_parse_options_recursive_config() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--config=conf",
        "--default_override=0:build=--test_multiple_string=rc",
        "--default_override=0:build:other=--test_multiple_string=other",
        "--default_override=0:build:conf=--test_multiple_string=config1",
        "--default_override=0:build:conf=--config=other",
        "--default_override=0:common:other=--test_multiple_string=othercommon",
        "--rc_source=/somewhere/.blazerc",
        "--test_multiple_string=explicit",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --config=conf --test_multiple_string=rc",
            "Found applicable config definition build:conf in file /somewhere/.blazerc: --test_multiple_string=config1 --config=other",
            "Found applicable config definition common:other in file /somewhere/.blazerc: --test_multiple_string=othercommon",
            "Found applicable config definition build:other in file /somewhere/.blazerc: --test_multiple_string=other",
        ])
    );
    assert_eq!(
        multiple(&parser),
        strs(&["config1", "othercommon", "other", "rc", "explicit"])
    );
}

#[test]
fn test_parse_options_recursive_config_with_different_tokens() {
    let (_parser, events, _notes) = parse_options(&[
        "build",
        "--default_override=0:build=--test_multiple_string=rc",
        "--default_override=0:build:other=--test_multiple_string=other",
        "--default_override=0:build:conf=--test_multiple_string=config1",
        "--default_override=0:build:conf=--config",
        "--default_override=0:build:conf=other",
        "--rc_source=/somewhere/.blazerc",
        "--config=conf",
    ]);
    assert_eq!(
        events.get_events(),
        &[HandlerEvent::error(
            "In file /somewhere/.blazerc, the definition of config conf expands to another config that either has no value or is not in the form --config=value. For recursive config definitions, please do not provide the value in a separate token, such as in the form '--config value'."
        )]
    );
}

#[test]
fn test_parse_options_complex_config_order() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--test_multiple_string=rc1",
        "--default_override=0:build=--config=foo",
        "--default_override=0:build=--test_multiple_string=rc2",
        "--default_override=0:common:baz=--test_multiple_string=baz1",
        "--default_override=0:build:baz=--test_multiple_string=baz2",
        "--default_override=0:common:foo=--test_multiple_string=foo1",
        "--default_override=0:common:foo=--config=bar",
        "--default_override=0:build:foo=--test_multiple_string=foo3",
        "--default_override=0:common:foo=--test_multiple_string=foo2",
        "--default_override=0:build:foo=--test_multiple_string=foo4",
        "--default_override=0:common:bar=--test_multiple_string=bar1",
        "--default_override=0:build:bar=--test_multiple_string=bar2",
        "--rc_source=/somewhere/.blazerc",
        "--test_multiple_string=explicit1",
        "--config=baz",
        "--test_multiple_string=explicit2",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --test_multiple_string=rc1 --config=foo --test_multiple_string=rc2",
            "Found applicable config definition common:foo in file /somewhere/.blazerc: --test_multiple_string=foo1 --config=bar --test_multiple_string=foo2",
            "Found applicable config definition common:bar in file /somewhere/.blazerc: --test_multiple_string=bar1",
            "Found applicable config definition build:bar in file /somewhere/.blazerc: --test_multiple_string=bar2",
            "Found applicable config definition build:foo in file /somewhere/.blazerc: --test_multiple_string=foo3 --test_multiple_string=foo4",
            "Found applicable config definition common:baz in file /somewhere/.blazerc: --test_multiple_string=baz1",
            "Found applicable config definition build:baz in file /somewhere/.blazerc: --test_multiple_string=baz2",
        ])
    );
    assert_eq!(
        multiple(&parser),
        strs(&[
            "rc1",
            "foo1",
            "bar1",
            "bar2",
            "foo2",
            "foo3",
            "foo4",
            "rc2",
            "explicit1",
            "baz1",
            "baz2",
            "explicit2"
        ])
    );
}

#[test]
fn test_parse_options_repeat_sub_config() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--config=foo",
        "--default_override=0:build=--test_multiple_string=rc",
        "--default_override=0:build:foo=--test_multiple_string=foo",
        "--default_override=0:build:foo=--config=bar",
        "--default_override=0:build:foo=--config=bar",
        "--default_override=0:build:bar=--test_multiple_string=bar",
        "--rc_source=/somewhere/.blazerc",
        "--test_multiple_string=explicit",
    ]);
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        events.get_events(),
        &[HandlerEvent::warn(
            "The following configs were expanded more than once: [bar]. For repeatable flags, repeats are counted twice and may lead to unexpected behavior."
        )]
    );
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --config=foo --test_multiple_string=rc",
            "Found applicable config definition build:foo in file /somewhere/.blazerc: --test_multiple_string=foo --config=bar --config=bar",
            "Found applicable config definition build:bar in file /somewhere/.blazerc: --test_multiple_string=bar",
            "Found applicable config definition build:bar in file /somewhere/.blazerc: --test_multiple_string=bar",
        ])
    );
    assert_eq!(
        multiple(&parser),
        strs(&["foo", "bar", "bar", "rc", "explicit"])
    );
}

#[test]
fn test_parse_options_repeat_config() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build:foo=--test_multiple_string=foo",
        "--default_override=0:build:foo=--config=bar",
        "--default_override=0:build:bar=--test_multiple_string=bar",
        "--default_override=0:build:baz=--test_multiple_string=baz",
        "--rc_source=/somewhere/.blazerc",
        "--config=foo",
        "--config=baz",
        "--config=foo",
        "--config=bar",
    ]);
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        events.get_events(),
        &[HandlerEvent::warn(
            "The following configs were expanded more than once: [foo, bar]. For repeatable flags, repeats are counted twice and may lead to unexpected behavior."
        )]
    );
    assert_eq!(
        notes,
        strs(&[
            "Found applicable config definition build:foo in file /somewhere/.blazerc: --test_multiple_string=foo --config=bar",
            "Found applicable config definition build:bar in file /somewhere/.blazerc: --test_multiple_string=bar",
            "Found applicable config definition build:baz in file /somewhere/.blazerc: --test_multiple_string=baz",
            "Found applicable config definition build:foo in file /somewhere/.blazerc: --test_multiple_string=foo --config=bar",
            "Found applicable config definition build:bar in file /somewhere/.blazerc: --test_multiple_string=bar",
            "Found applicable config definition build:bar in file /somewhere/.blazerc: --test_multiple_string=bar",
        ])
    );
    assert_eq!(
        multiple(&parser),
        strs(&["foo", "bar", "baz", "foo", "bar", "bar"])
    );
}

#[test]
fn test_parse_options_config_cycle_length1() {
    let (_parser, events, _notes) = parse_options(&[
        "build",
        "--default_override=0:build=--config=foo",
        "--default_override=0:build=--test_multiple_string=rc",
        "--default_override=0:build:foo=--test_multiple_string=foo",
        "--default_override=0:build:foo=--config=foo",
        "--rc_source=/somewhere/.blazerc",
        "--test_multiple_string=explicit",
    ]);
    assert!(events.get_events().contains(&HandlerEvent::error(
        "Config expansion has a cycle: config value foo expands to itself, see inheritance chain [foo]"
    )));
}

#[test]
fn test_parse_options_config_cycle_length2() {
    let (_parser, events, _notes) = parse_options(&[
        "build",
        "--default_override=0:build=--config=foo",
        "--default_override=0:build=--test_multiple_string=rc",
        "--default_override=0:build:foo=--test_multiple_string=foo",
        "--default_override=0:build:foo=--config=bar",
        "--default_override=0:build:bar=--test_multiple_string=bar",
        "--default_override=0:build:bar=--config=foo",
        "--rc_source=/somewhere/.blazerc",
        "--test_multiple_string=explicit",
    ]);
    assert!(events.get_events().contains(&HandlerEvent::error(
        "Config expansion has a cycle: config value foo expands to itself, see inheritance chain [foo, bar]"
    )));
}

#[test]
fn test_parse_options_recursive_config_was_already_present() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build=--config=other",
        "--default_override=0:build=--config=conf",
        "--default_override=0:build=--test_multiple_string=rc",
        "--default_override=0:build:other=--test_multiple_string=other",
        "--default_override=0:build:conf=--test_multiple_string=config1",
        "--default_override=0:build:conf=--config=other",
        "--default_override=0:common:other=--test_multiple_string=othercommon",
        "--rc_source=/somewhere/.blazerc",
        "--test_multiple_string=explicit",
    ]);
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        events.get_events(),
        &[HandlerEvent::warn(
            "The following configs were expanded more than once: [other]. For repeatable flags, repeats are counted twice and may lead to unexpected behavior."
        )]
    );
    assert_eq!(
        notes,
        strs(&[
            "Reading rc options for 'build' from /somewhere/.blazerc:\n  'build' options: --config=other --config=conf --test_multiple_string=rc",
            "Found applicable config definition common:other in file /somewhere/.blazerc: --test_multiple_string=othercommon",
            "Found applicable config definition build:other in file /somewhere/.blazerc: --test_multiple_string=other",
            "Found applicable config definition build:conf in file /somewhere/.blazerc: --test_multiple_string=config1 --config=other",
            "Found applicable config definition common:other in file /somewhere/.blazerc: --test_multiple_string=othercommon",
            "Found applicable config definition build:other in file /somewhere/.blazerc: --test_multiple_string=other",
        ])
    );
    assert_eq!(
        multiple(&parser),
        strs(&[
            "othercommon",
            "other",
            "config1",
            "othercommon",
            "other",
            "rc",
            "explicit"
        ])
    );
}

const GREEK_ALPHABET_CHAIN: [&str; 23] = [
    "--default_override=0:build:alpha=--test_multiple_string=alpha",
    "--default_override=0:build:alpha=--config=beta",
    "--default_override=0:build:beta=--test_multiple_string=beta",
    "--default_override=0:build:beta=--config=gamma",
    "--default_override=0:build:gamma=--test_multiple_string=gamma",
    "--default_override=0:build:gamma=--config=delta",
    "--default_override=0:build:delta=--test_multiple_string=delta",
    "--default_override=0:build:delta=--config=epsilon",
    "--default_override=0:build:epsilon=--test_multiple_string=epsilon",
    "--default_override=0:build:epsilon=--config=zeta",
    "--default_override=0:build:zeta=--test_multiple_string=zeta",
    "--default_override=0:build:zeta=--config=eta",
    "--default_override=0:build:eta=--test_multiple_string=eta",
    "--default_override=0:build:eta=--config=theta",
    "--default_override=0:build:theta=--test_multiple_string=theta",
    "--default_override=0:build:theta=--config=iota",
    "--default_override=0:build:iota=--test_multiple_string=iota",
    "--default_override=0:build:iota=--config=kappa",
    "--default_override=0:build:kappa=--test_multiple_string=kappa",
    "--default_override=0:build:kappa=--config=lambda",
    "--default_override=0:build:lambda=--test_multiple_string=lambda",
    "--default_override=0:build:lambda=--config=mu",
    "--default_override=0:build:mu=--test_multiple_string=mu",
];

#[test]
fn test_parse_options_long_chain() {
    let mut args = vec!["build"];
    args.extend(GREEK_ALPHABET_CHAIN);
    args.push("--rc_source=/somewhere/.blazerc");
    args.push("--config=alpha");
    let (parser, events, notes) = parse_options(&args);
    assert!(parser.get_residue().is_empty());
    let greek = [
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
        "lambda", "mu",
    ];
    let expected_notes: Vec<String> = greek
        .iter()
        .enumerate()
        .map(|(i, g)| {
            if i + 1 < greek.len() {
                format!(
                    "Found applicable config definition build:{} in file /somewhere/.blazerc: --test_multiple_string={} --config={}",
                    g,
                    g,
                    greek[i + 1]
                )
            } else {
                format!(
                    "Found applicable config definition build:{} in file /somewhere/.blazerc: --test_multiple_string={}",
                    g, g
                )
            }
        })
        .collect();
    assert_eq!(notes, expected_notes);
    assert_eq!(multiple(&parser), strs(&greek));
    assert_eq!(
        events.get_events(),
        &[HandlerEvent::warn(
            "There is a recursive chain of configs 12 configs long: [alpha, beta, gamma, delta, epsilon, zeta, eta, theta, iota, kappa, lambda, mu]. This seems excessive, and might be hiding errors."
        )]
    );
}

#[test]
fn test_parse_options_2_long_chains() {
    let mut args = vec!["build"];
    args.extend(GREEK_ALPHABET_CHAIN);
    args.push("--rc_source=/somewhere/.blazerc");
    args.push("--config=alpha");
    args.push("--config=gamma");
    let (parser, events, _notes) = parse_options(&args);
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        multiple(&parser),
        strs(&[
            "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
            "lambda", "mu", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
            "lambda", "mu"
        ])
    );
    assert_eq!(
        events.get_events(),
        &[
            HandlerEvent::warn(
                "There is a recursive chain of configs 12 configs long: [alpha, beta, gamma, delta, epsilon, zeta, eta, theta, iota, kappa, lambda, mu]. This seems excessive, and might be hiding errors."
            ),
            HandlerEvent::warn(
                "There is a recursive chain of configs 10 configs long: [gamma, delta, epsilon, zeta, eta, theta, iota, kappa, lambda, mu]. This seems excessive, and might be hiding errors."
            ),
            HandlerEvent::warn(
                "The following configs were expanded more than once: [gamma, delta, epsilon, zeta, eta, theta, iota, kappa, lambda, mu]. For repeatable flags, repeats are counted twice and may lead to unexpected behavior."
            ),
        ]
    );
}

#[test]
fn test_warning_flag() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--unconditional_warning",
        "You are forcing this warning to print for no apparent reason",
    ]);
    assert_eq!(
        events.get_events(),
        &[HandlerEvent::warn(
            "You are forcing this warning to print for no apparent reason"
        )]
    );
    assert!(parser.get_residue().is_empty());
    assert!(notes.is_empty());
}

#[test]
fn test_warning_flag_by_config_not_triggered() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--default_override=0:build:conf=--unconditional_warning=config \"conf\" is deprecated, please stop using!",
        "--rc_source=/somewhere/.blazerc",
    ]);
    assert!(events.is_empty());
    assert!(parser.get_residue().is_empty());
    assert!(notes.is_empty());
}

#[test]
fn test_warning_flag_by_config_triggered() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--config=conf",
        "--default_override=0:build:conf=--unconditional_warning=config \"conf\" is deprecated, please stop using!",
        "--rc_source=/somewhere/.blazerc",
    ]);
    assert_eq!(
        events.get_events(),
        &[HandlerEvent::warn(
            "config \"conf\" is deprecated, please stop using!"
        )]
    );
    assert!(parser.get_residue().is_empty());
    assert_eq!(
        notes,
        strs(&[
            "Found applicable config definition build:conf in file /somewhere/.blazerc: --unconditional_warning=config \"conf\" is deprecated, please stop using!"
        ])
    );
}

#[test]
fn test_config_after_explicit() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--test_string=explicitValue",
        "--config=conf",
        "--default_override=0:build:conf=--test_string=fromConf",
        "--rc_source=/somewhere/.blazerc",
    ]);
    assert_eq!(
        events.get_events(),
        &[HandlerEvent::warn(
            "option '--config=conf' (source command line options) was expanded and now overrides the explicit option --test_string=explicitValue with --test_string=fromConf"
        )]
    );
    assert_eq!(
        parser.get_options::<TestOptions>().unwrap().test_string,
        "fromConf"
    );
    assert_eq!(
        notes,
        strs(&[
            "Found applicable config definition build:conf in file /somewhere/.blazerc: --test_string=fromConf"
        ])
    );
}

#[test]
fn test_explicit_overrides_config() {
    let (parser, events, notes) = parse_options(&[
        "build",
        "--config=conf",
        "--test_string=explicitValue",
        "--default_override=0:build:conf=--test_string=fromConf",
        "--rc_source=/somewhere/.blazerc",
    ]);
    assert!(events.is_empty());
    assert_eq!(
        parser.get_options::<TestOptions>().unwrap().test_string,
        "explicitValue"
    );
    assert_eq!(
        notes,
        strs(&[
            "Found applicable config definition build:conf in file /somewhere/.blazerc: --test_string=fromConf"
        ])
    );
}
