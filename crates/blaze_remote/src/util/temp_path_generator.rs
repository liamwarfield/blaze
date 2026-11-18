//! Port of `lib/util/TempPathGenerator.java`.
//!
//! PORT: the Java class lives in `com.google.devtools.build.lib.util`, a package whose crate
//! (`blaze_util`) does not know `blaze_vfs::Path`; the remote package is its only user in the
//! ported code, so the port lives here.

use std::sync::atomic::{AtomicI32, Ordering};

use blaze_vfs::Path;

/// Generates temporary paths under a given directory.
// Java: com/google/devtools/build/lib/util/TempPathGenerator.java:22 TempPathGenerator
#[derive(Debug)]
pub struct TempPathGenerator {
    temp_dir: Path,
    index: AtomicI32,
}

impl TempPathGenerator {
    /// `new TempPathGenerator(tempDir)`.
    // Java: com/google/devtools/build/lib/util/TempPathGenerator.java:26 TempPathGenerator.<init>
    pub fn new(temp_dir: Path) -> TempPathGenerator {
        TempPathGenerator {
            temp_dir,
            index: AtomicI32::new(0),
        }
    }

    /// Generates a temporary path.
    // Java: com/google/devtools/build/lib/util/TempPathGenerator.java:31 TempPathGenerator.generateTempPath
    pub fn generate_temp_path(&self) -> Path {
        // `AtomicInteger.getAndIncrement()` wraps around on overflow, as `fetch_add` does.
        self.temp_dir.get_child(&format!(
            "{}.tmp",
            self.index.fetch_add(1, Ordering::SeqCst)
        ))
    }

    /// `getTempDir()`.
    // Java: com/google/devtools/build/lib/util/TempPathGenerator.java:35 TempPathGenerator.getTempDir
    pub fn get_temp_dir(&self) -> &Path {
        &self.temp_dir
    }
}
