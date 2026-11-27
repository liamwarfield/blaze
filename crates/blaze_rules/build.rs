//! Fails the build when flate2's zlib is not the system library.
//!
//! The gzip outputs (genquery's `compressed_output`, `command.profile.gz`) must have Bazel's
//! bytes, and Bazel's are those of the host's `libz.so.1`, which the JDK's `libzip.so` links
//! (on this machine zlib-ng-compat; stock zlib compresses differently, docs/reviews/
//! 2026-11-26-query-2-response.md). flate2's `zlib` feature (workspace `Cargo.toml`) uses
//! libz-sys, which links the system zlib when it finds it but silently builds its bundled stock
//! zlib otherwise (no system zlib, `LIBZ_SYS_STATIC=1`, or its `static` feature). Only the
//! bundled build reports `cargo:root`, which reaches this script (a direct dependent of the
//! `links = "z"` package) as `DEP_Z_ROOT`.
fn main() {
    println!("cargo:rerun-if-env-changed=DEP_Z_ROOT");
    if let Ok(root) = std::env::var("DEP_Z_ROOT") {
        panic!(
            "libz-sys built its bundled zlib (in {root}) instead of linking the system libz: the \
             gzip outputs would differ from Bazel's on this host. Install the zlib development \
             files (pkg-config `zlib`) and unset LIBZ_SYS_STATIC."
        );
    }
}
