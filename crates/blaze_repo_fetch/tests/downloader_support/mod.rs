//! Shared helpers for the downloader tests: ports of `HttpParser`, `DownloaderTestUtils`,
//! `ManualClock`, `ManualSleeper`, and a recording event handler.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use blaze_events::Event;
use blaze_events::handler::{EventHandler, ExtendedEventHandler, Postable};
use blaze_events::ui::Clock;
use blaze_repo_fetch::downloader::http_connector::Sleeper;
use parking_lot::Mutex;

/// Records handled events and posts.
#[derive(Default)]
pub struct RecordingHandler {
    pub events: Mutex<Vec<Event>>,
    pub posts: Mutex<Vec<Arc<dyn Postable>>>,
}

impl RecordingHandler {
    pub fn new() -> Arc<RecordingHandler> {
        Arc::new(RecordingHandler::default())
    }

    pub fn messages(&self) -> Vec<String> {
        self.events
            .lock()
            .iter()
            .map(|e| e.get_message().into_owned())
            .collect()
    }
}

impl EventHandler for RecordingHandler {
    fn handle(&self, event: &Event) {
        self.events.lock().push(event.clone());
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for RecordingHandler {
    fn post(&self, obj: Arc<dyn Postable>) {
        self.posts.lock().push(obj);
    }
}

/// Port of `ManualClock` (starts at 0).
#[derive(Default)]
pub struct ManualClock {
    millis: Mutex<i64>,
}

impl ManualClock {
    pub fn new() -> Arc<ManualClock> {
        Arc::new(ManualClock::default())
    }

    pub fn advance_millis(&self, ms: i64) -> i64 {
        let mut m = self.millis.lock();
        *m += ms;
        *m
    }
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        *self.millis.lock()
    }

    fn nano_time(&self) -> i64 {
        *self.millis.lock() * 1_000_000
    }
}

type Scheduled = (i64, Box<dyn FnOnce() + Send>);

/// Port of `ManualSleeper`: sleeping advances the clock and runs due scheduled runnables.
pub struct ManualSleeper {
    clock: Arc<ManualClock>,
    scheduled: Mutex<Vec<Scheduled>>,
}

impl ManualSleeper {
    pub fn new(clock: Arc<ManualClock>) -> Arc<ManualSleeper> {
        Arc::new(ManualSleeper {
            clock,
            scheduled: Mutex::new(Vec::new()),
        })
    }

    pub fn schedule_runnable(&self, runnable: impl FnOnce() + Send + 'static, delay_ms: i64) {
        let at = self.clock.current_time_millis() + delay_ms;
        self.scheduled.lock().push((at, Box::new(runnable)));
    }
}

impl Sleeper for ManualSleeper {
    fn sleep_millis(&self, millis: i64) {
        assert!(millis >= 0, "sleeper can't time travel");
        let now = self.clock.advance_millis(millis);
        let due: Vec<Scheduled> = {
            let mut s = self.scheduled.lock();
            let (due, rest): (Vec<Scheduled>, Vec<Scheduled>) =
                s.drain(..).partition(|(t, _)| now >= *t);
            *s = rest;
            due
        };
        for (_, r) in due {
            r();
        }
    }
}

/// `DownloaderTestUtils.sendLines`: joins with CRLF (no trailing CRLF).
pub fn send_lines(socket: &mut TcpStream, data: &[&str]) {
    let s = data.join("\r\n");
    let bytes: Vec<u8> = s.chars().map(|c| c as u32 as u8).collect();
    let _ = socket.write_all(&bytes);
    let _ = socket.flush();
}

/// Port of `HttpParser.readHttpRequest`: parses the request line and headers (keys lowercased;
/// `x-method`, `x-request-uri`, `x-version`).
pub fn read_http_request(stream: &mut TcpStream) -> std::io::Result<HashMap<String, Vec<String>>> {
    #[derive(PartialEq, Clone, Copy)]
    enum State {
        Method,
        Uri,
        Version,
        Hkey,
        Hsep,
        Hval,
        Cr1,
        Lf1,
        Lf2,
    }
    let malformed = || std::io::Error::other("Malformed Request");
    let mut output = HashMap::new();
    let mut builder = String::new();
    let mut state = State::Method;
    let mut key = String::new();
    let mut byte = [0u8; 1];
    loop {
        if stream.read(&mut byte)? == 0 {
            return Err(std::io::Error::other("eof"));
        }
        let c = byte[0] as char;
        let mut st = state;
        loop {
            match st {
                State::Method => {
                    if c == ' ' {
                        if builder.is_empty() {
                            return Err(malformed());
                        }
                        output.insert("x-method".to_string(), vec![std::mem::take(&mut builder)]);
                        state = State::Uri;
                    } else if c != '\r' && c != '\n' {
                        builder.push(c.to_ascii_uppercase());
                    }
                }
                State::Uri => {
                    if c == ' ' {
                        if builder.is_empty() {
                            return Err(malformed());
                        }
                        output.insert(
                            "x-request-uri".to_string(),
                            vec![std::mem::take(&mut builder)],
                        );
                        state = State::Version;
                    } else {
                        builder.push(c);
                    }
                }
                State::Version => {
                    if c == '\r' || c == '\n' {
                        output.insert("x-version".to_string(), vec![std::mem::take(&mut builder)]);
                        state = if c == '\r' { State::Cr1 } else { State::Lf1 };
                    } else {
                        builder.push(c.to_ascii_uppercase());
                    }
                }
                State::Cr1 => {
                    if c == '\n' {
                        state = State::Lf1;
                    } else {
                        return Err(malformed());
                    }
                }
                State::Lf1 => {
                    if c == '\r' {
                        state = State::Lf2;
                    } else if c == '\n' {
                        return Ok(output);
                    } else if c == ' ' || c == '\t' {
                        return Err(malformed());
                    } else {
                        state = State::Hkey;
                        st = State::Hkey;
                        continue;
                    }
                }
                State::Hkey => {
                    if c == ':' {
                        key = std::mem::take(&mut builder);
                        state = State::Hsep;
                    } else {
                        builder.push(c.to_ascii_lowercase());
                    }
                }
                State::Hsep => {
                    if c != ' ' && c != '\t' {
                        state = State::Hval;
                        st = State::Hval;
                        continue;
                    }
                }
                State::Hval => {
                    if c == '\r' || c == '\n' {
                        output.insert(key.clone(), vec![std::mem::take(&mut builder)]);
                        state = if c == '\r' { State::Cr1 } else { State::Lf1 };
                    } else {
                        builder.push(c);
                    }
                }
                State::Lf2 => {
                    if c == '\n' {
                        return Ok(output);
                    }
                    return Err(malformed());
                }
            }
            break;
        }
    }
}
