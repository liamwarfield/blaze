//! `Fingerprint.addPath`, which Java defines on `Fingerprint` itself.
//!
//! # Strings in action keys
//!
//! Every string that reaches a `Fingerprint` in this crate (arguments, environment, execution
//! info, paths, file contents, labels, mnemonics) is hashed with `Fingerprint::add_string` or a
//! helper built on it, exactly where Java calls `addString`. The invariant that makes this equal
//! to Java is the string model of decisions.md 2026-11-18 "Strings": strings reaching these sites
//! are internal-form (`blaze_util::string_encoding`: one char `<= U+00FF` per byte), as the Java
//! strings at the same sites are. A caller that holds real Unicode (text from a proto or JSON)
//! converts with `string_encoding::unicode_to_internal` where Java does, not here.
//!
//! Until the decoding switch (`string_encoding::STRINGS_ARE_INTERNAL`) the strings are still
//! Unicode, and `Fingerprint::add_string` itself bridges that through the migration seam; no key
//! site in this crate converts or chooses an encoding. Where Java takes the bytes or the length of
//! an internal string (file contents, the compression threshold, name length limits), the sites
//! call the other seam functions (`internal_string_bytes`, `internal_string_from_bytes`,
//! `internal_length`). `tests/non_ascii_action_keys.rs` checks every site class against keys
//! recorded from the oracle, on both sides of the switch.
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
    // Invariant: path strings are internal strings (`blaze_util::string_encoding`), as in Java
    // (`addString(input.getPathString())`); see `Fingerprint::add_string`.
    fn add_path(&mut self, input: &Path) -> &mut Self {
        self.add_string(input.get_path_string())
    }

    fn add_path_fragment(&mut self, input: &PathFragment) -> &mut Self {
        self.add_string(input.get_path_string())
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
