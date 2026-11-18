//! Golden tests: bytes produced by the oracle (Bazel 9.3.2, `scripts/record-goldens`) are
//! decoded with the generated types, then re-encoded (binary) or re-printed (JSON) by this crate,
//! and must match the oracle's files byte for byte.

use blaze_protos::JavaMessage;
use blaze_protos::analysis::{ActionGraphContainer, CqueryResult};
use blaze_protos::blaze_query::{QueryResult, Target};
use blaze_protos::build_event_stream::BuildEvent;
use blaze_protos::java::{decode_latin1, encode_latin1, parse_delimited_from, write_message_field};
use blaze_protos::json::{JsonFormat, TypeRegistry};
use blaze_protos::tools::protos::SpawnExec;
use prost::Message;

const QUERY_PB: &[u8] = include_bytes!("data/query.pb");
const QUERY_STREAMED_PB: &[u8] = include_bytes!("data/query.streamed.pb");
const QUERY_STREAMED_JSONL: &str = include_str!("data/query.streamed.jsonl");
const CQUERY_PB: &[u8] = include_bytes!("data/cquery.pb");
const CQUERY_JSON: &[u8] = include_bytes!("data/cquery.json");
const AQUERY_PB: &[u8] = include_bytes!("data/aquery.pb");
const AQUERY_JSON: &[u8] = include_bytes!("data/aquery.json");
const EXECLOG_PB: &[u8] = include_bytes!("data/execlog.pb");
const EXECLOG_JSON: &str = include_str!("data/execlog.json");
const BEP_PB: &[u8] = include_bytes!("data/bep.pb");
const BEP_JSON: &str = include_str!("data/bep.json");

fn delimited<M: JavaMessage>(mut data: &[u8]) -> Vec<M> {
    let mut out = Vec::new();
    while let Some(m) = parse_delimited_from::<M>(&mut data).unwrap() {
        out.push(m);
    }
    out
}

fn write_delimited<M: JavaMessage>(messages: &[M]) -> Vec<u8> {
    let mut buf = Vec::new();
    for m in messages {
        m.write_delimited_to(&mut buf);
    }
    buf
}

/// Fails with the first differing offset, which is easier to read than two byte dumps.
fn assert_bytes_eq(actual: &[u8], expected: &[u8], what: &str) {
    if actual != expected {
        let at = actual
            .iter()
            .zip(expected)
            .position(|(a, b)| a != b)
            .unwrap_or(actual.len().min(expected.len()));
        let lo = at.saturating_sub(80);
        panic!(
            "{what}: differs at byte {at} (len {} vs oracle {})\n ours: {}\n java: {}",
            actual.len(),
            expected.len(),
            String::from_utf8_lossy(&actual[lo..(at + 80).min(actual.len())]),
            String::from_utf8_lossy(&expected[lo..(at + 80).min(expected.len())]),
        );
    }
}

/// Splits a stream of `CodedOutputStream.writeMessage(field, msg)` records.
fn message_fields(mut data: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let key = prost::encoding::decode_varint(&mut data).unwrap();
        assert_eq!(key & 7, 2, "length-delimited");
        let len = prost::encoding::decode_varint(&mut data).unwrap() as usize;
        out.push(((key >> 3) as u32, &data[..len]));
        data = &data[len..];
    }
    out
}

/// JSON values concatenated with optional whitespace between them.
fn json_values(text: &str) -> Vec<serde_json::Value> {
    serde_json::Deserializer::from_str(text)
        .into_iter::<serde_json::Value>()
        .map(Result::unwrap)
        .collect()
}

fn bep_registry() -> TypeRegistry {
    // BuildEventServiceModule.makeJsonTypeRegistry
    TypeRegistry::new_builder().add::<SpawnExec>().build()
}

#[test]
fn query_proto_round_trip() {
    let result = QueryResult::decode(QUERY_PB).unwrap();
    assert_eq!(result.target.len(), 2);
    assert_bytes_eq(&result.to_byte_array(), QUERY_PB, "query --output=proto");
}

#[test]
fn query_streamed_proto_round_trip() {
    let targets = delimited::<Target>(QUERY_STREAMED_PB);
    assert_eq!(targets.len(), 2);
    assert_bytes_eq(
        &write_delimited(&targets),
        QUERY_STREAMED_PB,
        "query --output=streamed_proto",
    );
}

#[test]
fn query_streamed_jsonproto_matches_oracle() {
    // StreamedJSONProtoOutputFormatter
    let printer = JsonFormat::printer().omitting_insignificant_whitespace();
    let mut out = String::new();
    for target in delimited::<Target>(QUERY_STREAMED_PB) {
        out.push_str(&printer.print(&target).unwrap());
        out.push('\n');
    }
    assert_bytes_eq(
        out.as_bytes(),
        QUERY_STREAMED_JSONL.as_bytes(),
        "query --output=streamed_jsonproto",
    );
}

#[test]
fn query_streamed_jsonproto_parse_round_trip() {
    let printer = JsonFormat::printer().omitting_insignificant_whitespace();
    let parser = JsonFormat::parser();
    let mut out = String::new();
    for line in QUERY_STREAMED_JSONL.lines() {
        let mut target = Target::default();
        parser.merge(line, &mut target).unwrap();
        out.push_str(&printer.print(&target).unwrap());
        out.push('\n');
    }
    assert_bytes_eq(
        out.as_bytes(),
        QUERY_STREAMED_JSONL.as_bytes(),
        "parse+print",
    );
}

#[test]
fn cquery_proto_round_trip() {
    let result = CqueryResult::decode(CQUERY_PB).unwrap();
    assert!(!result.results.is_empty());
    assert_bytes_eq(&result.to_byte_array(), CQUERY_PB, "cquery --output=proto");
}

#[test]
fn cquery_jsonproto_matches_oracle() {
    // cquery ProtoOutputFormatterCallback: jsonPrinter.appendTo(message), then '\n', through a
    // Latin-1 PrintStream (the cquery proto holds Unicode strings, so 'é' becomes byte 0xE9).
    let result = CqueryResult::decode(CQUERY_PB).unwrap();
    let mut out = String::new();
    JsonFormat::printer().append_to(&result, &mut out).unwrap();
    out.push('\n');
    assert_bytes_eq(
        &encode_latin1(&out),
        CQUERY_JSON,
        "cquery --output=jsonproto",
    );
}

#[test]
fn cquery_jsonproto_parse_round_trip() {
    let mut result = CqueryResult::default();
    JsonFormat::parser()
        .merge(&decode_latin1(CQUERY_JSON), &mut result)
        .unwrap();
    assert_eq!(result, CqueryResult::decode(CQUERY_PB).unwrap());
}

#[test]
fn aquery_proto_round_trip() {
    // StreamedConsumingOutputHandler writes each element as a field of ActionGraphContainer, in
    // emission order.
    let mut out = Vec::new();
    for (field, bytes) in message_fields(AQUERY_PB) {
        use blaze_protos::analysis as a;
        match field {
            1 => write_message_field(1, &a::Artifact::decode(bytes).unwrap(), &mut out),
            2 => write_message_field(2, &a::Action::decode(bytes).unwrap(), &mut out),
            3 => write_message_field(3, &a::Target::decode(bytes).unwrap(), &mut out),
            4 => write_message_field(4, &a::DepSetOfFiles::decode(bytes).unwrap(), &mut out),
            5 => write_message_field(5, &a::Configuration::decode(bytes).unwrap(), &mut out),
            6 => write_message_field(6, &a::AspectDescriptor::decode(bytes).unwrap(), &mut out),
            7 => write_message_field(7, &a::RuleClass::decode(bytes).unwrap(), &mut out),
            8 => write_message_field(8, &a::PathFragment::decode(bytes).unwrap(), &mut out),
            f => panic!("unexpected field {f}"),
        }
    }
    assert_bytes_eq(&out, AQUERY_PB, "aquery --output=proto");
}

#[test]
fn aquery_jsonproto_matches_oracle() {
    // MonolithicOutputHandler: jsonPrinter.appendTo(container), then println(), through a
    // Latin-1 PrintStream. The aquery proto holds Bazel's internal (Latin-1 view of UTF-8)
    // strings, so the Latin-1 stream turns them back into UTF-8 bytes.
    let container = ActionGraphContainer::decode(AQUERY_PB).unwrap();
    let mut out = String::new();
    JsonFormat::printer()
        .append_to(&container, &mut out)
        .unwrap();
    out.push('\n');
    assert_bytes_eq(
        &encode_latin1(&out),
        AQUERY_JSON,
        "aquery --output=jsonproto",
    );
}

#[test]
fn execlog_binary_round_trip() {
    let execs = delimited::<SpawnExec>(EXECLOG_PB);
    assert_eq!(execs.len(), 1);
    assert_bytes_eq(
        &write_delimited(&execs),
        EXECLOG_PB,
        "--execution_log_binary_file",
    );
}

#[test]
fn execlog_json_round_trip() {
    // The JSON log comes from a second build (the two flags are exclusive), so it is parsed
    // with JsonFormat.parser() and printed again with JsonOutputStreamWrapper's printer.
    let printer = JsonFormat::printer().always_print_fields_with_no_presence();
    let parser = JsonFormat::parser();
    let mut out = String::new();
    let values = json_values(EXECLOG_JSON);
    assert_eq!(values.len(), 1);
    for value in values {
        let mut exec = SpawnExec::default();
        parser.merge_value(&value, &mut exec).unwrap();
        out.push_str(&printer.print(&exec).unwrap());
    }
    assert_bytes_eq(
        out.as_bytes(),
        EXECLOG_JSON.as_bytes(),
        "--execution_log_json_file",
    );
}

#[test]
fn bep_binary_round_trip() {
    let events = delimited::<BuildEvent>(BEP_PB);
    assert!(events.len() > 10);
    assert_bytes_eq(
        &write_delimited(&events),
        BEP_PB,
        "--build_event_binary_file",
    );
}

#[test]
fn bep_json_matches_oracle() {
    // JsonFormatFileTransport, fed the same events as the binary file of the same build.
    let printer = JsonFormat::printer()
        .using_type_registry(bep_registry())
        .omitting_insignificant_whitespace();
    let mut out = String::new();
    for event in delimited::<BuildEvent>(BEP_PB) {
        out.push_str(&printer.print(&event).unwrap());
        out.push('\n');
    }
    assert_bytes_eq(
        out.as_bytes(),
        BEP_JSON.as_bytes(),
        "--build_event_json_file",
    );
}

#[test]
fn bep_json_parse_matches_binary() {
    let parser = JsonFormat::parser().using_type_registry(bep_registry());
    let mut events = Vec::new();
    for line in BEP_JSON.lines() {
        let mut event = BuildEvent::default();
        parser.merge(line, &mut event).unwrap();
        events.push(event);
    }
    assert_bytes_eq(&write_delimited(&events), BEP_PB, "BEP json -> binary");
}
