//! Prints a `--remote_grpc_log` file (length-delimited `remote_logging.LogEntry` messages) as
//! JSON, one entry per line, for scripts/remote/compare-grpc-log.
//!
//! Not a port of a Java class (Bazel ships `//src/tools/remote:remote_client` for this).

use std::process::ExitCode;

use blaze_protos::java::parse_delimited_from;
use blaze_protos::json::{JsonFormat, TypeRegistry};
use blaze_remote::reapi::remote_logging::LogEntry;
use blaze_remote::reapi::{ExecuteOperationMetadata, ExecuteResponse};

const USAGE: &str = "\
usage: blaze-remote-grpc-log <grpc.log>
       blaze-remote-grpc-log help

Reads a file written with --remote_grpc_log (by Bazel or by Blaze) and prints every LogEntry as
one line of JSON (JsonFormat, whitespace omitted), in file order.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 || args[1] == "help" || args[1] == "--help" {
        print!("{USAGE}");
        return ExitCode::from(if args.len() == 2 { 0 } else { 2 });
    }
    let bytes = match std::fs::read(&args[1]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {}: {e}", args[1]);
            return ExitCode::from(1);
        }
    };
    let registry = TypeRegistry::new_builder()
        .add::<ExecuteResponse>()
        .add::<ExecuteOperationMetadata>()
        .build();
    let printer = JsonFormat::printer()
        .using_type_registry(registry)
        .omitting_insignificant_whitespace();
    let mut buf: &[u8] = &bytes;
    loop {
        match parse_delimited_from::<LogEntry>(&mut buf) {
            Ok(Some(entry)) => match printer.print(&entry) {
                Ok(json) => println!("{json}"),
                Err(e) => {
                    eprintln!("cannot print an entry: {}", e.get_message());
                    return ExitCode::from(1);
                }
            },
            Ok(None) => break,
            Err(e) => {
                eprintln!("malformed log entry: {e}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}
