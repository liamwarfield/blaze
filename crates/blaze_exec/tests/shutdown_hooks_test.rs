//! The shutdown hooks of `blaze_exec::worker::shutdown_hooks` (no Java test; Java's hooks are
//! `Runtime` hooks). A separate test binary: running the hooks puts the process into shutdown.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use blaze_exec::worker::shutdown_hooks;

// Review cgroups round 2 (open item): a panic in one hook must not keep the others from running
// (each Java hook is a thread of its own).
#[test]
fn a_panicking_hook_does_not_skip_the_others() {
    let ran = Arc::new(AtomicUsize::new(0));
    let r1 = ran.clone();
    shutdown_hooks::add_shutdown_hook(Arc::new(move || {
        r1.fetch_add(1, Ordering::SeqCst);
    }));
    shutdown_hooks::add_shutdown_hook(Arc::new(|| panic!("hook failure")));
    let r3 = ran.clone();
    shutdown_hooks::add_shutdown_hook(Arc::new(move || {
        r3.fetch_add(1, Ordering::SeqCst);
    }));
    // PORT: the test does not take the shutdown lock (`begin_shutdown`): only a thread that
    // ends the process may hold it, and the test harness's main thread runs the `atexit`
    // handler, which would wait for it.
    shutdown_hooks::run_shutdown_hooks();
    assert_eq!(ran.load(Ordering::SeqCst), 2);
}
