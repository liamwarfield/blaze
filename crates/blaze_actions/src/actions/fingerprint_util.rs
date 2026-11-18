//! `Fingerprint.addPath`, which Java defines on `Fingerprint` itself.
//!
//! PORT: `Fingerprint` lives in blaze_util, below blaze_vfs (docs/research/06 §5.3: "Path methods
//! become an extension trait"). The extension lives here, where the action keys use it.

use blaze_util::Fingerprint;
use blaze_vfs::{Path, PathFragment};

/// `Fingerprint.addPath(Path)` / `addPath(PathFragment)`.
pub trait FingerprintPathExt {
    // Java: com/google/devtools/build/lib/util/Fingerprint.java:258 Fingerprint.addPath
    fn add_path(&mut self, input: &Path) -> &mut Self;
    // Java: com/google/devtools/build/lib/util/Fingerprint.java:264 Fingerprint.addPath
    fn add_path_fragment(&mut self, input: &PathFragment) -> &mut Self;
}

impl FingerprintPathExt for Fingerprint {
    // PORT: path strings are in Bazel's internal (Latin-1-of-UTF-8) form in Java, which
    // `addString` encodes again; `add_internal_string` reproduces that (StringEncoding).
    fn add_path(&mut self, input: &Path) -> &mut Self {
        self.add_internal_string(input.get_path_string())
    }

    fn add_path_fragment(&mut self, input: &PathFragment) -> &mut Self {
        self.add_internal_string(input.get_path_string())
    }
}

/// `Fingerprint.addPaths(Collection<PathFragment>)`.
// Java: com/google/devtools/build/lib/util/Fingerprint.java:316 Fingerprint.addPaths
pub fn add_paths(fp: &mut Fingerprint, inputs: &[PathFragment]) {
    fp.add_int(inputs.len() as i32);
    for input in inputs {
        fp.add_path_fragment(input);
    }
}
