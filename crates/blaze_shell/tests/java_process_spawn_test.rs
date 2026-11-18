//! PORT: Blaze only. How `JavaProcessBuilder.start` launches children: like the JDK's
//! `posix_spawn` launch mechanism, without copying (and accounting) the parent's address space,
//! and optionally in a session of its own (the daemonized server's children).

use std::io::Read;

use blaze_shell::java_process::{JavaProcessBuilder, set_start_children_in_new_session};

fn run(argv: &[&str]) -> (i32, String) {
    let mut builder = JavaProcessBuilder::new();
    builder.command(argv.iter().map(|s| s.to_string()).collect());
    builder.environment_put("PATH", "/usr/bin:/bin");
    let process = builder.start().expect("start");
    let mut out = String::new();
    process
        .get_input_stream()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    (process.wait_for().unwrap(), out)
}

/// The numbers a child shell prints.
fn own_stat_fields(script: &str) -> Vec<i64> {
    let (code, out) = run(&["/bin/sh", "-c", script]);
    assert_eq!(code, 0, "{out}");
    out.split_whitespace().map(|f| f.parse().unwrap()).collect()
}

// Both cases run in one test: the session setting is process-wide.
#[test]
fn children_start_in_a_new_session_only_when_asked() {
    // pid, sid of the shell itself.
    let script = "read -r line < /proc/$$/stat; set -- ${line##*) }; echo $$ $4";
    // SAFETY: plain syscall.
    let parent_sid = unsafe { libc::getsid(0) } as i64;
    let fields = own_stat_fields(script);
    assert_eq!(fields[1], parent_sid, "default: the parent's session");

    set_start_children_in_new_session(true);
    let fields = own_stat_fields(script);
    set_start_children_in_new_session(false);
    assert_eq!(fields[1], fields[0], "a session leader of its own");
}

#[test]
fn children_start_while_hundreds_of_large_stacks_are_reserved() {
    // 400 parked threads with 256 MB stacks reserve 100 GB of writable private memory; a fork
    // must account all of it under the kernel's heuristic overcommit and fails with ENOMEM on a
    // machine with less RAM + swap. posix_spawn does not copy the address space.
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    let mut threads = Vec::new();
    for _ in 0..400 {
        let rx = rx.clone();
        threads.push(
            std::thread::Builder::new()
                .stack_size(256 << 20)
                .spawn(move || {
                    let _ = rx.lock().unwrap().recv();
                })
                .expect("thread"),
        );
    }
    let (code, out) = run(&["echo", "started"]);
    assert_eq!((code, out.as_str()), (0, "started\n"));
    drop(tx);
    for t in threads {
        t.join().unwrap();
    }
}
