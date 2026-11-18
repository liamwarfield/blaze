//! Port of `src/test/java/com/google/devtools/build/lib/shell/LoadTest.java`: Command execution
//! under load.

mod common;

use std::sync::Arc;

use blaze_shell::Command;

#[test]
fn test_load() {
    // create a temp file with some random numbers
    let temp_file = std::env::temp_dir().join(format!("LoadTest{}txt", std::process::id()));
    let mut contents = String::new();
    let mut state: u64 = std::process::id() as u64;
    for _ in 0..100 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        contents.push_str(&format!("{}\n", (state >> 11) as f64 / (1u64 << 53) as f64));
    }
    std::fs::write(&temp_file, contents).unwrap();

    // PORT: Java runs the test-only cat_file.cc binary; /bin/cat does the same.
    let command = Arc::new(Command::new(
        vec![
            "/bin/cat".to_string(),
            temp_file.to_string_lossy().into_owned(),
        ],
        &common::getenv(),
    ));
    let threads: Vec<_> = (0..10)
        .map(|_| {
            let command = command.clone();
            std::thread::spawn(move || -> Result<(), String> {
                for _ in 0..20 {
                    command.execute().map_err(|e| e.to_string())?;
                }
                Ok(())
            })
        })
        .collect();
    let exceptions: Vec<String> = threads
        .into_iter()
        .filter_map(|t| t.join().unwrap().err())
        .collect();
    let _ = std::fs::remove_file(&temp_file);
    if let Some(first) = exceptions.first() {
        panic!("{first}");
    }
}
