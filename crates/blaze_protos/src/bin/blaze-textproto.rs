//! `blaze-textproto`: Blaze's `TextFormat` printer and parser over binary protos, for
//! `scripts/textproto/compare` (which checks the output byte for byte against Bazel's).
//!
//! Usage:
//!   blaze-textproto print <framing> <message type> <binary file>
//!   blaze-textproto roundtrip <framing> <message type> <binary file>
//!   blaze-textproto reprint <framing> <message type> <text file>
//!   blaze-textproto bep-text-to-json <text file>
//!   blaze-textproto help
//!
//! Framings (how Bazel writes the binary and the text of the same output):
//!   bep     binary: `writeDelimitedTo` records (`--build_event_binary_file`); text: each record
//!           as `TextFormatFileTransport` writes it: `event {\n` + printToString + `}\n\n`.
//!   cquery  binary: top-level fields of the type, each `writeTo` of a message holding one field
//!           (`cquery --output=proto`); text: `TextFormat.printer().print` of each such message
//!           (`cquery --output=textproto`).
//!   aquery  binary: top-level fields of the type (`aquery --output=proto`,
//!           `CodedOutputStream.writeMessage(fieldNumber, msg)`); text: `<field name> {\n` +
//!           `msg.toString()` + `}\n` per field (`aquery --output=textproto`).
//!   single  binary: one message; text: `TextFormat.printer().printToString`.
//!
//! `print` writes the text to stdout. `roundtrip` parses each printed record back with
//! `TextFormat.getParser().merge` and checks that it equals the record (`Message.equals`); it
//! prints `records N, equal N, different N` and exits 1 on a difference.
//! `reprint` splits a text output into its records, parses each with the parser and prints it
//! again with the printer, framed as Bazel frames it (stdout); for Bazel's text the result must
//! be the same bytes. (`cquery` and `single`: the whole text is one message of the type.)
//! `bep-text-to-json` parses a `--build_event_text_file` with the parser and writes each event as
//! `--build_event_json_file` does (JsonFormat, no whitespace, SpawnExec registry).

use std::io::Write as _;
use std::process::ExitCode;

use blaze_protos::build_event_stream::BuildEvent;
use blaze_protos::descriptors::{Descriptor, pool};
use blaze_protos::dynamic_message::DynamicMessage;
use blaze_protos::json::{JsonFormat, TypeRegistry as JsonTypeRegistry};
use blaze_protos::text_format::{self, default_format_printer, get_parser, printer};
use blaze_protos::tools::protos::SpawnExec;

const USAGE: &str = "usage:
  blaze-textproto print <bep|cquery|aquery|single> <message type> <binary file>
  blaze-textproto roundtrip <bep|cquery|aquery|single> <message type> <binary file>
  blaze-textproto reprint <bep|cquery|aquery|single> <message type> <text file>
  blaze-textproto bep-text-to-json <text file>
  blaze-textproto help";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("help") | Some("--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("print") | Some("roundtrip") if args.len() == 4 => {
            let descriptor = match pool().find_message_type_by_name(&args[2]) {
                Some(d) => d,
                None => {
                    eprintln!("unknown message type {}", args[2]);
                    return ExitCode::from(2);
                }
            };
            let data = std::fs::read(&args[3]).expect("read binary file");
            let records = match split(&args[1], descriptor, &data) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(2);
                }
            };
            if args[0] == "print" {
                let mut out = std::io::stdout().lock();
                for r in &records {
                    out.write_all(r.framed_text().as_bytes()).unwrap();
                }
                ExitCode::SUCCESS
            } else {
                roundtrip(&records)
            }
        }
        Some("reprint") if args.len() == 4 => {
            let descriptor = match pool().find_message_type_by_name(&args[2]) {
                Some(d) => d,
                None => {
                    eprintln!("unknown message type {}", args[2]);
                    return ExitCode::from(2);
                }
            };
            let text = String::from_utf8(std::fs::read(&args[3]).expect("read text file"))
                .expect("ASCII text");
            match reprint(&args[1], descriptor, &text) {
                Ok(out) => {
                    std::io::stdout().lock().write_all(out.as_bytes()).unwrap();
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("bep-text-to-json") if args.len() == 2 => bep_text_to_json(&args[1]),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// One record of the output: the message and how its text is framed.
struct Record {
    message: DynamicMessage,
    framing: Framing,
}

enum Framing {
    Bep,
    Cquery,
    Aquery(String),
    Single,
}

impl Record {
    fn text(&self) -> String {
        match self.framing {
            // `Message.toString()`.
            Framing::Aquery(_) => default_format_printer().print_dynamic_to_string(&self.message),
            _ => printer().print_dynamic_to_string(&self.message),
        }
    }

    fn framed_text(&self) -> String {
        let text = self.text();
        match &self.framing {
            Framing::Bep => format!("event {{\n{text}}}\n\n"),
            Framing::Aquery(label) => format!("{label} {{\n{text}}}\n"),
            Framing::Cquery | Framing::Single => text,
        }
    }
}

fn split(
    framing: &str,
    descriptor: &'static Descriptor,
    data: &[u8],
) -> Result<Vec<Record>, String> {
    let mut records = Vec::new();
    match framing {
        "bep" => {
            let mut pos = 0;
            while pos < data.len() {
                let (len, n) = read_varint(&data[pos..]).ok_or("truncated length")?;
                pos += n;
                let end = pos + len as usize;
                let bytes = data.get(pos..end).ok_or("truncated record")?;
                records.push(Record {
                    message: parse(descriptor, bytes)?,
                    framing: Framing::Bep,
                });
                pos = end;
            }
        }
        "cquery" | "aquery" => {
            let mut pos = 0;
            while pos < data.len() {
                let start = pos;
                let (tag, n) = read_varint(&data[pos..]).ok_or("truncated tag")?;
                pos += n;
                let number = (tag >> 3) as i32;
                if tag & 7 != 2 {
                    return Err(format!(
                        "field {number}: wire type {} is not a message",
                        tag & 7
                    ));
                }
                let (len, n) = read_varint(&data[pos..]).ok_or("truncated length")?;
                pos += n;
                let end = pos + len as usize;
                let body = data.get(pos..end).ok_or("truncated field")?;
                pos = end;
                if framing == "cquery" {
                    records.push(Record {
                        message: parse(descriptor, &data[start..end])?,
                        framing: Framing::Cquery,
                    });
                } else {
                    let field = descriptor
                        .find_field_by_number(number)
                        .ok_or_else(|| format!("{} has no field {number}", descriptor.full_name))?;
                    records.push(Record {
                        message: parse(field.get_message_type(), body)?,
                        framing: Framing::Aquery(field.name.clone()),
                    });
                }
            }
        }
        "single" => records.push(Record {
            message: parse(descriptor, data)?,
            framing: Framing::Single,
        }),
        other => return Err(format!("unknown framing {other}")),
    }
    Ok(records)
}

fn parse(descriptor: &'static Descriptor, bytes: &[u8]) -> Result<DynamicMessage, String> {
    DynamicMessage::parse_from(descriptor, bytes).map_err(|e| e.get_message().to_string())
}

fn read_varint(b: &[u8]) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for (i, &byte) in b.iter().enumerate().take(10) {
        v |= ((byte & 0x7f) as u64) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

fn roundtrip(records: &[Record]) -> ExitCode {
    let mut equal = 0;
    let mut different = 0;
    for (i, r) in records.iter().enumerate() {
        let text = r.text();
        let mut parsed = DynamicMessage::new(r.message.get_descriptor_for_type());
        let ok = match get_parser().merge_dynamic(&text, &mut parsed) {
            Ok(()) => {
                // `equals` as Java defines it (map fields as maps: the printer sorts entries).
                parsed.java_equals(&r.message)
            }
            Err(e) => {
                eprintln!("record {i}: ParseException: {e}");
                false
            }
        };
        if ok {
            equal += 1;
        } else {
            different += 1;
            if different <= 5 {
                eprintln!("record {i} differs after parse(print(m)):\n{text}");
            }
        }
    }
    println!(
        "records {}, equal {equal}, different {different}",
        records.len()
    );
    if different == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn bep_text_to_json(path: &str) -> ExitCode {
    let text = std::fs::read(path).expect("read text file");
    // The printer escapes every non-ASCII byte: the file is ASCII.
    let text = String::from_utf8(text).expect("ASCII text");
    let json_printer = JsonFormat::printer()
        .using_type_registry(JsonTypeRegistry::new_builder().add::<SpawnExec>().build())
        .omitting_insignificant_whitespace();
    let mut out = std::io::stdout().lock();
    let mut rest = text.as_str();
    let mut n = 0;
    while !rest.is_empty() {
        let Some(body) = rest.strip_prefix("event {\n") else {
            eprintln!("event {n}: expected \"event {{\"");
            return ExitCode::FAILURE;
        };
        let end = if body.starts_with("}\n\n") {
            Some(0)
        } else {
            body.find("\n}\n\n").map(|i| i + 1)
        };
        let Some(end) = end else {
            eprintln!("event {n}: no end");
            return ExitCode::FAILURE;
        };
        let event_text = &body[..end];
        rest = &body[end + 3..];
        let event: BuildEvent = match text_format::parse(event_text) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("event {n}: ParseException: {e}");
                return ExitCode::FAILURE;
            }
        };
        let json = match json_printer.print(&event) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("event {n}: {e}");
                return ExitCode::FAILURE;
            }
        };
        out.write_all(json.as_bytes()).unwrap();
        out.write_all(b"\n").unwrap();
        n += 1;
    }
    ExitCode::SUCCESS
}

/// Parses each record of `text` and prints it again (see `reprint` in the usage).
fn reprint(framing: &str, descriptor: &'static Descriptor, text: &str) -> Result<String, String> {
    let parse_text = |d: &'static Descriptor, t: &str| -> Result<DynamicMessage, String> {
        let mut m = DynamicMessage::new(d);
        get_parser()
            .merge_dynamic(t, &mut m)
            .map_err(|e| format!("ParseException: {e}"))?;
        Ok(m)
    };
    let mut out = String::new();
    match framing {
        "bep" => {
            let mut rest = text;
            while !rest.is_empty() {
                let body = rest
                    .strip_prefix("event {\n")
                    .ok_or("expected \"event {\"")?;
                let end = if body.starts_with("}\n\n") {
                    0
                } else {
                    body.find("\n}\n\n").ok_or("no end of event")? + 1
                };
                let m = parse_text(descriptor, &body[..end])?;
                out.push_str(&format!(
                    "event {{\n{}}}\n\n",
                    printer().print_dynamic_to_string(&m)
                ));
                rest = &body[end + 3..];
            }
        }
        "aquery" => {
            // Records start with `<field name> {` at column 0.
            let mut records: Vec<(String, String)> = Vec::new();
            for line in text.split_inclusive('\n') {
                let label = line
                    .strip_suffix(" {\n")
                    .filter(|l| descriptor.find_field_by_name(l).is_some());
                match label {
                    Some(l) if records.last().is_none_or(|(_, body)| body.ends_with("}\n")) => {
                        records.push((l.to_string(), String::new()))
                    }
                    _ => records
                        .last_mut()
                        .ok_or("text before the first record")?
                        .1
                        .push_str(line),
                }
            }
            for (label, body) in records {
                let field = descriptor
                    .find_field_by_name(&label)
                    .expect("label is a field");
                let inner = body.strip_suffix("}\n").ok_or("record without end")?;
                let m = parse_text(field.get_message_type(), inner)?;
                out.push_str(&format!(
                    "{label} {{\n{}}}\n",
                    default_format_printer().print_dynamic_to_string(&m)
                ));
            }
        }
        "cquery" | "single" => {
            let m = parse_text(descriptor, text)?;
            out.push_str(&printer().print_dynamic_to_string(&m));
        }
        other => return Err(format!("unknown framing {other}")),
    }
    Ok(out)
}
