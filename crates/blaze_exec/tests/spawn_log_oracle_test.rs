//! Checks the execution log writers against the oracle's `--execution_log_json_file` output
//! (`--execution_log_sort`, recorded by `scripts/oracle record` for each corpus project).
//!
//! Skips (with a message) when the oracle records are not present.

use std::io::{self, Cursor, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use blaze_exec::exec::io::{
    BinaryInputStreamWrapper, JsonInputStreamWrapper, JsonOutputStreamWrapper, MessageInputStream,
    MessageOutputStream,
};
use blaze_exec::exec::stable_sort::stable_sort;
use blaze_protos::JavaMessage;
use blaze_protos::tools::protos::SpawnExec;

const PROJECTS: [&str; 2] = ["brotli", "re2"];

fn oracle_execlog(project: &str) -> Option<Vec<u8>> {
    let home = std::env::var_os("HOME")?;
    let path = PathBuf::from(home)
        .join(".cache/blaze/oracle-records/oracle-9.3.2")
        .join(project)
        .join("raw/execlog.json");
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skipping {project}: {} not found", path.display());
            None
        }
    }
}

fn parse_json_log(bytes: &[u8]) -> Vec<SpawnExec> {
    let mut input = JsonInputStreamWrapper::<SpawnExec>::new(Box::new(Cursor::new(bytes.to_vec())));
    let mut execs = Vec::new();
    while let Some(ex) = input.read().unwrap() {
        execs.push(ex);
    }
    execs
}

#[derive(Clone, Default)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `JsonOutputStreamWrapper` prints each parsed `SpawnExec` exactly as Bazel wrote it.
#[test]
fn oracle_execlog_json_round_trips() {
    for project in PROJECTS {
        let Some(bytes) = oracle_execlog(project) else {
            continue;
        };
        let execs = parse_json_log(&bytes);
        assert!(!execs.is_empty());

        let buffer = SharedBuffer::default();
        let mut out = JsonOutputStreamWrapper::<SpawnExec>::new(Box::new(buffer.clone()));
        for ex in &execs {
            out.write(ex).unwrap();
        }
        out.close().unwrap();

        let actual = buffer.0.lock().unwrap().clone();
        assert!(
            actual == bytes,
            "{project}: JSON output differs from the oracle's execlog.json"
        );
    }
}

/// `StableSort` reproduces the oracle's (sorted) order from the spawns in reverse order.
#[test]
fn oracle_execlog_stable_sort_order() {
    for project in PROJECTS {
        let Some(bytes) = oracle_execlog(project) else {
            continue;
        };
        let execs = parse_json_log(&bytes);

        let mut binary = Vec::new();
        for ex in execs.iter().rev() {
            ex.write_delimited_to(&mut binary);
        }
        let mut input = BinaryInputStreamWrapper::<SpawnExec>::new(Box::new(Cursor::new(binary)));

        struct ListOutput(Vec<SpawnExec>);
        impl MessageOutputStream<SpawnExec> for ListOutput {
            fn write(&mut self, m: &SpawnExec) -> io::Result<()> {
                self.0.push(m.clone());
                Ok(())
            }
            fn close(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut out = ListOutput(Vec::new());
        stable_sort(&mut input, &mut out).unwrap();

        assert!(
            out.0 == execs,
            "{project}: sorted order differs from the oracle's execlog.json"
        );
    }
}
