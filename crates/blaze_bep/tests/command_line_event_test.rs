//! Port of `src/test/java/com/google/devtools/build/lib/runtime/CommandLineEventTest.java`:
//! tests for `CommandLineEvent`'s construction of the command lines.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_bep::buildeventstream::{
    ArtifactGroupNamer, BuildEvent, BuildEventContext, PathConverter,
};
use blaze_bep::runtime::command_line_event::{
    CanonicalCommandLineEvent, OriginalCommandLineEvent, ToolCommandLineEvent,
};
use blaze_optdefs::command::bazel_startup_options_module::BazelStartupOptionsModuleOptions;
use blaze_optdefs::command::blaze_server_startup_options::BlazeServerStartupOptions;
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::misc::build_event_context::OutputGroupFileMode;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_options::{OptionsClass, OptionsParser, PriorityCategory};
use blaze_protos::JavaMessage;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::build_event_id::Id;
use blaze_protos::command_line::command_line_section::SectionType;
use blaze_protos::command_line::{
    ChunkList, CommandLine, CommandLineSection, Option as ProtoOption, OptionList,
};

#[allow(dead_code, clippy::all)]
#[path = "../../blaze_options/tests/common/test_options.rs"]
mod test_options;

use test_options::TestOptions;

/// Java passes `null` as the `BuildEventContext`; none of these events reads it.
struct NullContext;

impl BuildEventContext for NullContext {
    fn path_converter(&self) -> &dyn PathConverter {
        panic!("NullPointerException: null BuildEventContext")
    }
    fn artifact_group_namer(&self) -> &dyn ArtifactGroupNamer {
        panic!("NullPointerException: null BuildEventContext")
    }
    fn get_options(&self) -> &BuildEventProtocolOptions {
        panic!("NullPointerException: null BuildEventContext")
    }
    fn get_file_mode_for_output_group(&self, _output_group: &str) -> OutputGroupFileMode {
        panic!("NullPointerException: null BuildEventContext")
    }
}

/// `event.asStreamProto(null).getStructuredCommandLine()`.
fn structured_command_line(event: &dyn BuildEvent) -> CommandLine {
    match event.as_stream_proto(&NullContext).unwrap().payload {
        Some(Payload::StructuredCommandLine(line)) => line,
        // getStructuredCommandLine() of an unset oneof is the default instance.
        _ => CommandLine::default(),
    }
}

/// `section.getChunkList().getChunkList()`.
fn chunks(section: &CommandLineSection) -> &[String] {
    match &section.section_type {
        Some(SectionType::ChunkList(list)) => &list.chunk,
        _ => &[],
    }
}

/// `section.getOptionList().getOptionList()`.
fn options(section: &CommandLineSection) -> &[ProtoOption] {
    match &section.section_type {
        Some(SectionType::OptionList(list)) => &list.option,
        _ => &[],
    }
}

/// `OptionsParser.builder().optionsClasses(classes...).build()`.
fn parser(classes: &[&'static blaze_options::OptionsClassInfo]) -> OptionsParser {
    OptionsParser::builder()
        .options_classes(classes)
        .build()
        .unwrap()
}

fn tool_command_line(parser: &OptionsParser) -> ToolCommandLineEvent {
    ToolCommandLineEvent::from_option_value(
        &parser
            .get_options::<CommonCommandOptions>()
            .unwrap()
            .tool_command_line,
    )
}

/// Guava `BaseEncoding.base64().encode`.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn check_command_line_section_labels(line: &CommandLine) {
    assert_eq!(line.sections.len(), 5);

    assert_eq!(line.sections[0].section_label, "executable");
    assert_eq!(line.sections[1].section_label, "startup options");
    assert_eq!(line.sections[2].section_label, "command");
    assert_eq!(line.sections[3].section_label, "command options");
    assert_eq!(line.sections[4].section_label, "residual");
}

#[test]
fn test_mostly_empty_original_command_line() {
    let fake_startup_options = parser(&[BlazeServerStartupOptions::class()]);
    let fake_command_options = parser(&[TestOptions::class()]);

    let line = structured_command_line(&OriginalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        fake_command_options.get_residue(),
        true,
        Vec::new(),
        BTreeMap::new(),
        Some(Vec::new()),
    ));

    assert_eq!(line.command_line_label, "original");
    check_command_line_section_labels(&line);
    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 0);
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");
    assert_eq!(options(&line.sections[3]).len(), 0);
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

#[test]
fn test_mostly_empty_canonical_command_line() {
    let fake_startup_options = parser(&[BlazeServerStartupOptions::class()]);
    let fake_command_options = parser(&[TestOptions::class()]);

    let line = structured_command_line(&CanonicalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        fake_command_options.get_residue(),
        false,
        BTreeMap::new(),
        BTreeMap::new(),
        fake_command_options.as_list_of_canonical_options(),
        /* replaceable= */ false,
    ));

    assert_eq!(line.command_line_label, "canonical");
    check_command_line_section_labels(&line);

    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 1);
    assert_eq!(
        options(&line.sections[1])[0].combined_form,
        "--ignore_all_rc_files"
    );
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");
    assert_eq!(options(&line.sections[3]).len(), 0);
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

#[test]
fn test_active_bazelrcs_original_command_line() {
    let mut fake_startup_options = parser(&[
        BlazeServerStartupOptions::class(),
        BazelStartupOptionsModuleOptions::class(),
    ]);
    fake_startup_options
        .parse(&["--bazelrc=/some/path", "--bazelrc", "/some/other/path"])
        .unwrap();
    let fake_command_options = parser(&[TestOptions::class()]);

    let line = structured_command_line(&OriginalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        fake_command_options.get_residue(),
        false,
        fake_command_options.as_list_of_canonical_options(),
        BTreeMap::new(),
        None,
    ));

    assert_eq!(line.command_line_label, "original");
    check_command_line_section_labels(&line);

    // Expect the provided rc-related startup options are correctly listed
    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 2);
    assert_eq!(
        options(&line.sections[1])[0].combined_form,
        "--bazelrc=/some/path"
    );
    assert_eq!(
        options(&line.sections[1])[1].combined_form,
        "--bazelrc /some/other/path"
    );
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");
    assert_eq!(options(&line.sections[3]).len(), 0);
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

#[test]
fn test_passed_in_bazelrcs_original_command_line() {
    let fake_startup_options = parser(&[
        BlazeServerStartupOptions::class(),
        BazelStartupOptionsModuleOptions::class(),
    ]);
    let fake_command_options = parser(&[TestOptions::class()]);

    let line = structured_command_line(&OriginalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        fake_command_options.get_residue(),
        false,
        fake_command_options.as_list_of_canonical_options(),
        BTreeMap::new(),
        Some(vec![
            ("".to_string(), "--bazelrc=/some/path".to_string()),
            ("".to_string(), "--master_bazelrc".to_string()),
            ("".to_string(), "--bazelrc=/some/other/path".to_string()),
            (
                "".to_string(),
                "--invocation_policy=notARealPolicy".to_string(),
            ),
        ]),
    ));

    assert_eq!(line.command_line_label, "original");
    check_command_line_section_labels(&line);

    // Expect the provided rc-related startup options are correctly listed
    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 4);
    assert_eq!(
        options(&line.sections[1])[0].combined_form,
        "--bazelrc=/some/path"
    );
    assert_eq!(
        options(&line.sections[1])[1].combined_form,
        "--master_bazelrc"
    );
    assert_eq!(
        options(&line.sections[1])[2].combined_form,
        "--bazelrc=/some/other/path"
    );
    assert_eq!(
        options(&line.sections[1])[3].combined_form,
        "--invocation_policy=notARealPolicy"
    );
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");
    assert_eq!(options(&line.sections[3]).len(), 0);
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

#[test]
fn test_bazelrcs_canonical_command_line() {
    let mut fake_startup_options = parser(&[
        BlazeServerStartupOptions::class(),
        BazelStartupOptionsModuleOptions::class(),
    ]);
    fake_startup_options
        .parse(&["--bazelrc=/some/path", "--bazelrc", "/some/other/path"])
        .unwrap();
    let fake_command_options = parser(&[TestOptions::class()]);

    let line = structured_command_line(&CanonicalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        fake_command_options.get_residue(),
        false,
        BTreeMap::new(),
        BTreeMap::new(),
        fake_command_options.as_list_of_canonical_options(),
        /* replaceable= */ false,
    ));

    assert_eq!(line.command_line_label, "canonical");
    check_command_line_section_labels(&line);

    // Expect the provided rc-related startup options are removed and replaced with the
    // rc-prevention options.
    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 1);
    assert_eq!(
        options(&line.sections[1])[0].combined_form,
        "--ignore_all_rc_files"
    );
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");
    assert_eq!(options(&line.sections[3]).len(), 0);
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

/// The command options shared by the two `testOptionsAtVariousPriorities_*` tests.
fn options_at_various_priorities() -> OptionsParser {
    let mut fake_command_options = parser(&[TestOptions::class()]);
    fake_command_options
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line"),
            vec![
                "--test_string=foo".to_string(),
                "--test_multiple_string=bar".to_string(),
            ],
        )
        .unwrap();
    fake_command_options
        .parse_with_priority(
            PriorityCategory::INVOCATION_POLICY,
            Some("fake invocation policy"),
            vec!["--expanded_c=2".to_string()],
        )
        .unwrap();
    fake_command_options
        .parse_with_priority(
            PriorityCategory::RC_FILE,
            Some("fake rc file"),
            vec!["--test_multiple_string=baz".to_string()],
        )
        .unwrap();
    fake_command_options
}

#[test]
fn test_options_at_various_priorities_original_command_line() {
    let fake_startup_options = parser(&[BlazeServerStartupOptions::class()]);
    let fake_command_options = options_at_various_priorities();

    let line = structured_command_line(&OriginalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        fake_command_options.get_residue(),
        false,
        fake_command_options.as_list_of_canonical_options(),
        BTreeMap::new(),
        Some(Vec::new()),
    ));

    assert_eq!(line.command_line_label, "original");
    check_command_line_section_labels(&line);

    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 0);
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");
    // Expect the rc file options and invocation policy options to not be listed with the explicit
    // command line options.
    assert_eq!(options(&line.sections[3]).len(), 2);
    assert_eq!(
        options(&line.sections[3])[0].combined_form,
        "--test_string=foo"
    );
    assert_eq!(
        options(&line.sections[3])[1].combined_form,
        "--test_multiple_string=bar"
    );
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

#[test]
fn test_options_at_various_priorities_canonical_command_line() {
    let fake_startup_options = parser(&[BlazeServerStartupOptions::class()]);
    let fake_command_options = options_at_various_priorities();

    let line = structured_command_line(&CanonicalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        Vec::new(),
        false,
        BTreeMap::new(),
        BTreeMap::new(),
        fake_command_options.as_list_of_canonical_options(),
        /* replaceable= */ false,
    ));

    assert_eq!(line.command_line_label, "canonical");
    check_command_line_section_labels(&line);

    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 1);
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");
    // In the canonical line, expect the options in priority order.
    assert_eq!(options(&line.sections[3]).len(), 4);
    assert_eq!(
        options(&line.sections[3])[0].combined_form,
        "--test_multiple_string=baz"
    );
    assert_eq!(
        options(&line.sections[3])[1].combined_form,
        "--test_string=foo"
    );
    assert_eq!(
        options(&line.sections[3])[2].combined_form,
        "--test_multiple_string=bar"
    );
    assert_eq!(
        options(&line.sections[3])[3].combined_form,
        "--expanded_c=2"
    );
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

#[test]
fn test_expansion_option_original_command_line() {
    let fake_startup_options = parser(&[BlazeServerStartupOptions::class()]);
    let mut fake_command_options = parser(&[TestOptions::class()]);
    fake_command_options
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line"),
            vec!["--test_expansion".to_string()],
        )
        .unwrap();

    let line = structured_command_line(&OriginalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        fake_command_options.get_residue(),
        false,
        fake_command_options.as_list_of_explicit_options(),
        fake_command_options
            .get_explicit_starlark_options(OriginalCommandLineEvent::command_line_priority),
        None,
    ));

    assert_eq!(line.command_line_label, "original");
    check_command_line_section_labels(&line);

    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 0);
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");
    // Expect the rc file option to not be listed with the explicit command line options.
    assert_eq!(options(&line.sections[3]).len(), 1);
    assert_eq!(
        options(&line.sections[3])[0].combined_form,
        "--test_expansion"
    );
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

#[test]
fn test_expansion_option_canonical_command_line() {
    let fake_startup_options = parser(&[BlazeServerStartupOptions::class()]);
    let mut fake_command_options = parser(&[TestOptions::class()]);
    fake_command_options
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line"),
            vec!["--test_expansion".to_string()],
        )
        .unwrap();

    let line = structured_command_line(&CanonicalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        Vec::new(),
        false,
        BTreeMap::new(),
        BTreeMap::new(),
        fake_command_options.as_list_of_canonical_options(),
        /* replaceable= */ false,
    ));

    assert_eq!(line.command_line_label, "canonical");
    check_command_line_section_labels(&line);

    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 1);
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");

    assert_eq!(options(&line.sections[3]).len(), 4);
    assert_eq!(
        options(&line.sections[3])[0].combined_form,
        "--noexpanded_a"
    );
    assert_eq!(
        options(&line.sections[3])[1].combined_form,
        "--expanded_b=false"
    );
    assert_eq!(
        options(&line.sections[3])[2].combined_form,
        "--expanded_c 42"
    );
    assert_eq!(
        options(&line.sections[3])[3].combined_form,
        "--expanded_d bar"
    );
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

#[test]
fn test_option_with_implicit_requirement_original_command_line() {
    let fake_startup_options = parser(&[BlazeServerStartupOptions::class()]);
    let mut fake_command_options = parser(&[TestOptions::class()]);
    fake_command_options
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line"),
            vec!["--test_implicit_requirement=foo".to_string()],
        )
        .unwrap();

    let line = structured_command_line(&OriginalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        Vec::new(),
        false,
        fake_command_options.as_list_of_canonical_options(),
        BTreeMap::new(),
        Some(Vec::new()),
    ));

    assert_eq!(line.command_line_label, "original");
    check_command_line_section_labels(&line);

    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 0);
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");
    assert_eq!(options(&line.sections[3]).len(), 1);
    assert_eq!(
        options(&line.sections[3])[0].combined_form,
        "--test_implicit_requirement=foo"
    );
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

#[test]
fn test_option_with_implicit_requirement_canonical_command_line() {
    let fake_startup_options = parser(&[BlazeServerStartupOptions::class()]);
    let mut fake_command_options = parser(&[TestOptions::class()]);
    fake_command_options
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line"),
            vec!["--test_implicit_requirement=foo".to_string()],
        )
        .unwrap();

    let line = structured_command_line(&CanonicalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "someCommandName".to_string(),
        Vec::new(),
        false,
        BTreeMap::new(),
        BTreeMap::new(),
        fake_command_options.as_list_of_canonical_options(),
        /* replaceable= */ false,
    ));

    assert_eq!(line.command_line_label, "canonical");
    check_command_line_section_labels(&line);

    // Unlike expansion flags, implicit requirements are not listed separately.
    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 1);
    assert_eq!(chunks(&line.sections[2])[0], "someCommandName");
    assert_eq!(options(&line.sections[3]).len(), 1);
    assert_eq!(
        options(&line.sections[3])[0].combined_form,
        "--test_implicit_requirement=foo"
    );
    assert_eq!(chunks(&line.sections[4]).len(), 0);
}

#[test]
fn test_default_tool_command_line() {
    let parser = parser(&[CommonCommandOptions::class()]);
    let event = tool_command_line(&parser);
    // Test that the actual default value is an empty command line.
    assert_eq!(structured_command_line(&event), CommandLine::default());
}

#[test]
#[ignore = "needs CommandLine.parseFrom in blaze_optdefs ToolCommandLineEventConverter (base64 proto input is unported!)"]
fn test_labeless_parsing_of_compiled_tool_command_line() {
    let mut parser = parser(&[CommonCommandOptions::class()]);
    let original = CommandLine {
        sections: vec![CommandLineSection::default()],
        ..Default::default()
    };
    let arg = format!(
        "--experimental_tool_command_line={}",
        base64_encode(&original.to_byte_array())
    );
    parser.parse(&[arg.as_str()]).unwrap();

    let event = tool_command_line(&parser);
    let id = match event.get_event_id().id {
        Some(Id::StructuredCommandLine(id)) => id,
        other => panic!("unexpected id {other:?}"),
    };
    let line = structured_command_line(&event);

    assert_eq!(id.command_line_label, "tool");
    assert_eq!(line.sections.len(), 1);
}

#[test]
#[ignore = "needs CommandLine.parseFrom in blaze_optdefs ToolCommandLineEventConverter (base64 proto input is unported!)"]
fn test_parsing_of_compiled_tool_command_line() {
    let mut parser = parser(&[CommonCommandOptions::class()]);
    let original = CommandLine {
        command_line_label: "something meaningful".to_string(),
        sections: vec![
            CommandLineSection {
                section_label: "command".to_string(),
                section_type: Some(SectionType::ChunkList(ChunkList {
                    chunk: vec!["aCommand".to_string()],
                })),
            },
            CommandLineSection {
                section_label: "someArguments".to_string(),
                section_type: Some(SectionType::ChunkList(ChunkList {
                    chunk: vec!["arg1".to_string(), "arg2".to_string()],
                })),
            },
            CommandLineSection {
                section_label: "someOptions".to_string(),
                section_type: Some(SectionType::OptionList(OptionList::default())),
            },
        ],
    };
    let arg = format!(
        "--experimental_tool_command_line={}",
        base64_encode(&original.to_byte_array())
    );
    parser.parse(&[arg.as_str()]).unwrap();

    let event = tool_command_line(&parser);
    let id = match event.get_event_id().id {
        Some(Id::StructuredCommandLine(id)) => id,
        other => panic!("unexpected id {other:?}"),
    };
    let line = structured_command_line(&event);

    assert_eq!(id.command_line_label, "tool");
    assert_eq!(line.command_line_label, "something meaningful");
    assert_eq!(line.sections.len(), 3);
    assert!(matches!(
        line.sections[0].section_type,
        Some(SectionType::ChunkList(_))
    ));
    assert_eq!(chunks(&line.sections[0]).len(), 1);
    assert_eq!(chunks(&line.sections[0])[0], "aCommand");
    assert!(matches!(
        line.sections[1].section_type,
        Some(SectionType::ChunkList(_))
    ));
    assert_eq!(chunks(&line.sections[1]).len(), 2);
    assert_eq!(chunks(&line.sections[1])[0], "arg1");
    assert_eq!(chunks(&line.sections[1])[1], "arg2");
    assert!(matches!(
        line.sections[2].section_type,
        Some(SectionType::OptionList(_))
    ));
    assert_eq!(options(&line.sections[2]).len(), 0);
}

#[test]
fn test_simple_string_tool_command_line() {
    let mut parser = parser(&[CommonCommandOptions::class()]);
    parser
        .parse(&["--experimental_tool_command_line=The quick brown fox jumps over the lazy dog"])
        .unwrap();

    let event = tool_command_line(&parser);
    let id = match event.get_event_id().id {
        Some(Id::StructuredCommandLine(id)) => id,
        other => panic!("unexpected id {other:?}"),
    };
    let line = structured_command_line(&event);

    assert_eq!(id.command_line_label, "tool");
    assert_eq!(line.command_line_label, "tool");
    assert_eq!(line.sections.len(), 1);
    assert!(matches!(
        line.sections[0].section_type,
        Some(SectionType::ChunkList(_))
    ));
    assert_eq!(
        chunks(&line.sections[0])[0],
        "The quick brown fox jumps over the lazy dog"
    );
}

/// The command options shared by the two `redactedResidual_includesTarget_*` tests.
fn redacted_residual_command_options() -> OptionsParser {
    let mut fake_command_options = parser(&[BuildEventProtocolOptions::class()]);
    fake_command_options
        .parse(&["--experimental_run_bep_event_include_residue=false"])
        .unwrap();
    fake_command_options.set_residue(
        vec!["//some:target".to_string(), "--sensitive_arg".to_string()],
        Vec::new(),
    );
    fake_command_options
}

#[test]
fn redacted_residual_includes_target_original_command_line() {
    let fake_startup_options = parser(&[BlazeServerStartupOptions::class()]);
    let fake_command_options = redacted_residual_command_options();

    let line = structured_command_line(&OriginalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "run".to_string(),
        fake_command_options.get_residue(),
        false,
        fake_command_options.as_list_of_canonical_options(),
        BTreeMap::new(),
        Some(Vec::new()),
    ));

    assert_eq!(line.command_line_label, "original");
    check_command_line_section_labels(&line);
    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 0);
    assert_eq!(chunks(&line.sections[2])[0], "run");
    assert_eq!(options(&line.sections[3]).len(), 1);
    assert_eq!(chunks(&line.sections[4]).len(), 2);
    assert_eq!(chunks(&line.sections[4])[0], "//some:target");
    assert_eq!(chunks(&line.sections[4])[1], "REDACTED");
}

#[test]
fn redacted_residual_includes_target_canonical_command_line() {
    let fake_startup_options = parser(&[BlazeServerStartupOptions::class()]);
    let fake_command_options = redacted_residual_command_options();

    let line = structured_command_line(&CanonicalCommandLineEvent::new(
        "testblaze".to_string(),
        Arc::new(fake_startup_options),
        "run".to_string(),
        fake_command_options.get_residue(),
        false,
        BTreeMap::new(),
        BTreeMap::new(),
        fake_command_options.as_list_of_canonical_options(),
        /* replaceable= */ false,
    ));

    assert_eq!(line.command_line_label, "canonical");
    check_command_line_section_labels(&line);
    assert_eq!(chunks(&line.sections[0])[0], "testblaze");
    assert_eq!(options(&line.sections[1]).len(), 1);
    assert_eq!(
        options(&line.sections[1])[0].combined_form,
        "--ignore_all_rc_files"
    );
    assert_eq!(chunks(&line.sections[2])[0], "run");
    assert_eq!(options(&line.sections[3]).len(), 1);
    assert_eq!(chunks(&line.sections[4]).len(), 2);
    assert_eq!(chunks(&line.sections[4])[0], "//some:target");
    assert_eq!(chunks(&line.sections[4])[1], "REDACTED");
}
