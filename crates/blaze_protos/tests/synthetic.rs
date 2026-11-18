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
