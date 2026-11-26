//! Messages built by hand and compared with what Java protobuf produces for the same messages
//! (`scripts/JavaSynthetic.java`, run against the oracle's server jar by
//! `scripts/record-synthetic`). These cover cases the oracle runs do not reach.

use blaze_protos::JavaMessage;
use blaze_protos::build_event_stream::{
    ActionExecuted, BuildEvent, BuildMetadata, BuildMetrics, build_event, build_metadata,
};
use blaze_protos::google::protobuf::{Any, Duration, Timestamp};
use blaze_protos::json::{JsonFormat, TypeRegistry};
use blaze_protos::tools::protos::{Digest, File, SpawnExec};

macro_rules! data {
    ($name:literal) => {
        include_bytes!(concat!("data/synthetic/", $name)).as_slice()
    };
}

fn any_of<M: JavaMessage>(m: &M) -> Any {
    // Any.pack
    Any {
        type_url: format!("type.googleapis.com/{}", M::FULL_NAME),
        value: m.to_byte_array(),
    }
}

fn registry() -> TypeRegistry {
    TypeRegistry::new_builder().add::<SpawnExec>().build()
}

fn oneof_event() -> BuildEvent {
    BuildEvent {
        last_message: true,
        payload: Some(build_event::Payload::BuildMetrics(BuildMetrics::default())),
        ..Default::default()
    }
}

fn map_metadata() -> BuildMetadata {
    let entry = |k: &str, v: &str| build_metadata::MetadataEntry {
        key: k.to_string(),
        value: v.to_string(),
    };
    BuildMetadata {
        metadata: vec![
            entry("z", "1"),
            entry("", ""),
            entry("a=b", "<&>'\u{e9}\u{2028}\n"),
        ],
    }
}

fn any_action() -> ActionExecuted {
    let packed = SpawnExec {
        command_args: vec!["echo".to_string()],
        timeout_millis: -5,
        exit_code: -1,
        mnemonic: "M".to_string(),
        ..Default::default()
    };
    ActionExecuted {
        strategy_details: vec![
            any_of(&packed),
            any_of(&Duration {
                seconds: -1,
                nanos: -500_000_000,
            }),
            any_of(&Timestamp {
                seconds: 1,
                nanos: 1000,
            }),
            Any::default(),
        ],
        start_time: Some(Timestamp {
            seconds: 1_700_000_000,
            nanos: 120_000_000,
        }),
        ..Default::default()
    }
}

#[test]
fn oneof_member_is_written_at_its_own_field_number() {
    let event = oneof_event();
    assert_eq!(event.to_byte_array(), data!("oneof_order.bin"));
    // prost writes the oneof at its lowest field number (3), before field 20.
    assert_ne!(
        prost::Message::encode_to_vec(&event),
        data!("oneof_order.bin")
    );
    assert_eq!(event.get_serialized_size(), data!("oneof_order.bin").len());
    let json = JsonFormat::printer().print(&event).unwrap();
    assert_eq!(json.as_bytes(), data!("oneof_order.json"));
}

#[test]
fn map_entries_keep_order_and_default_keys() {
    let m = map_metadata();
    assert_eq!(m.to_byte_array(), data!("map_entries.bin"));
    assert_eq!(
        JsonFormat::printer().print(&m).unwrap().as_bytes(),
        data!("map_entries.json")
    );
    let compact = JsonFormat::printer().omitting_insignificant_whitespace();
    assert_eq!(
        compact.print(&m).unwrap().as_bytes(),
        data!("map_entries.compact.json")
    );
}

#[test]
fn empty_map_and_message_layout() {
    let empty = BuildMetadata::default();
    assert_eq!(
        JsonFormat::printer().print(&empty).unwrap().as_bytes(),
        data!("map_empty.json")
    );
    let always = JsonFormat::printer().always_print_fields_with_no_presence();
    assert_eq!(
        always.print(&empty).unwrap().as_bytes(),
        data!("map_empty.always.json")
    );
}

#[test]
fn always_print_fields_with_no_presence() {
    let always = JsonFormat::printer().always_print_fields_with_no_presence();
    let exec = SpawnExec {
        inputs: vec![
            File {
                path: "p".to_string(),
                digest: Some(Digest::default()),
                ..Default::default()
            },
            File::default(),
        ],
        ..Default::default()
    };
    assert_eq!(
        always.print(&exec).unwrap().as_bytes(),
        data!("spawn_exec.always.json")
    );
    assert_eq!(
        always.print(&SpawnExec::default()).unwrap().as_bytes(),
        data!("spawn_exec_default.always.json")
    );
}

#[test]
fn any_with_registry() {
    let action = any_action();
    assert_eq!(action.to_byte_array(), data!("any.bin"));
    let pretty = JsonFormat::printer().using_type_registry(registry());
    assert_eq!(pretty.print(&action).unwrap().as_bytes(), data!("any.json"));
    let compact = JsonFormat::printer()
        .using_type_registry(registry())
        .omitting_insignificant_whitespace();
    assert_eq!(
        compact.print(&action).unwrap().as_bytes(),
        data!("any.compact.json")
    );
}

#[test]
fn any_without_registry_fails() {
    let err = JsonFormat::printer().print(&any_action()).unwrap_err();
    let java = std::str::from_utf8(data!("any.unregistered.error")).unwrap();
    assert_eq!(
        format!(
            "com.google.protobuf.InvalidProtocolBufferException: {}",
            err.get_message()
        ),
        java
    );
}

#[test]
fn json_parse_round_trips() {
    let parser = JsonFormat::parser().using_type_registry(registry());
    let mut action = ActionExecuted::default();
    parser
        .merge(std::str::from_utf8(data!("any.json")).unwrap(), &mut action)
        .unwrap();
    assert_eq!(action, any_action());
    let mut m = BuildMetadata::default();
    parser
        .merge(
            std::str::from_utf8(data!("map_entries.json")).unwrap(),
            &mut m,
        )
        .unwrap();
    assert_eq!(m, map_metadata());
    let mut event = BuildEvent::default();
    parser
        .merge(
            std::str::from_utf8(data!("oneof_order.json")).unwrap(),
            &mut event,
        )
        .unwrap();
    assert_eq!(event, oneof_event());
}

#[allow(deprecated)] // aspect_name, aspect_parameters (Java sets them too)
fn extra_action_spawn_info() -> blaze_protos::blaze::ExtraActionInfo {
    use blaze_protos::blaze::{EnvironmentVariable, ExtraActionInfo, SpawnInfo, extra_action_info};
    ExtraActionInfo {
        owner: Some("//pkg:gen".to_owned()),
        id: Some("0123abcd".to_owned()),
        mnemonic: Some("Genrule".to_owned()),
        aspect_name: Some("MyAspect".to_owned()),
        aspect_parameters: vec![extra_action_info::AspectParametersEntry {
            key: "p".to_owned(),
            value: Some(extra_action_info::StringList {
                value: vec!["v1".to_owned(), "v2".to_owned()],
            }),
        }],
        spawn_info: Some(SpawnInfo {
            argument: vec![
                "/bin/bash".to_owned(),
                "-c".to_owned(),
                "echo \"é\" > $@".to_owned(),
            ],
            variable: vec![EnvironmentVariable {
                name: "PATH".to_owned(),
                value: "/bin".to_owned(),
            }],
            input_file: vec!["pkg/in.txt".to_owned()],
            output_file: vec!["bazel-out/k8-fastbuild/bin/pkg/out.txt".to_owned()],
        }),
        ..Default::default()
    }
}

/// Extensions of `ExtraActionInfo` are written after its fields, in number order, and printed
/// by `TextFormat` as `[full.name]` blocks (print_action's output).
#[test]
fn extra_action_info_extensions() {
    use blaze_protos::blaze::{
        CppCompileInfo, DetailedExtraActionInfo, ExtraActionInfo, ExtraActionSummary, PythonInfo,
    };
    let spawn = extra_action_spawn_info();
    assert_eq!(spawn.to_byte_array(), data!("extra_action_info.bin"));
    let cpp = ExtraActionInfo {
        owner: Some("//pkg:lib".to_owned()),
        id: Some("ffff".to_owned()),
        mnemonic: Some("CppCompile".to_owned()),
        python_info: Some(PythonInfo {
            source_file: vec!["a.py".to_owned()],
            dep_file: vec![],
        }),
        cpp_compile_info: Some(CppCompileInfo {
            tool: Some("/usr/bin/gcc".to_owned()),
            compiler_option: vec!["-c".to_owned()],
            source_file: Some("pkg/a.cc".to_owned()),
            output_file: Some("bazel-out/k8-fastbuild/bin/pkg/_objs/lib/a.o".to_owned()),
            sources_and_headers: vec!["pkg/a.cc".to_owned()],
            variable: vec![],
        }),
        ..Default::default()
    };
    let summary = ExtraActionSummary {
        action: vec![
            DetailedExtraActionInfo {
                triggering_file: None,
                action: spawn,
            },
            DetailedExtraActionInfo {
                triggering_file: Some("pkg/a.cc".to_owned()),
                action: cpp,
            },
        ],
    };
    assert_eq!(summary.to_byte_array(), data!("extra_action_summary.bin"));
    let text = blaze_protos::text_format::printer().print_to_string(&summary);
    assert_eq!(text.as_bytes(), data!("extra_action_summary.txt"));
}
