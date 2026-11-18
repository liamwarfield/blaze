//! The install archive embedded in the binary (built by `build.rs` from the oracle's install
//! base; see there for the format and Blaze's install key).

use blaze_client::archive_utils::EmbeddedArchive;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/embedded_archive.rs"));
}

/// The zstd-compressed tar of the install base.
static DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/install.tar.zst"));

/// Blaze's embedded install archive.
pub fn archive() -> EmbeddedArchive {
    EmbeddedArchive::new(
        DATA,
        generated::CONTENTS.iter().map(|s| s.to_string()).collect(),
        generated::INSTALL_MD5,
        generated::BUILD_LABEL,
        generated::SERVER_JAR,
    )
}
