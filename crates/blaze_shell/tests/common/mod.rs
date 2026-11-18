//! Shared helpers for the ported tests.
#![allow(dead_code)]

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

use indexmap::IndexMap;

/// The panic message of `f`, which must panic.
pub fn panic_message<R>(f: impl FnOnce() -> R) -> String {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(prev);
    match r {
        Ok(_) => panic!("expected a panic"),
        Err(payload) => payload_string(&*payload),
    }
}

pub fn payload_string(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "<non-string panic>".to_string()
    }
}

/// `System.getenv()`.
pub fn getenv() -> IndexMap<String, String> {
    std::env::vars().collect()
}

pub fn map(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

pub fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}
