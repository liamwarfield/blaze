//! Port of `remote/options/RemoteBuildEventUploadMode.java`.

use blaze_options::option_enum;

// Java: com/google/devtools/build/lib/remote/options/RemoteBuildEventUploadMode.java:17 RemoteBuildEventUploadMode
option_enum! {
    /// Describes how to upload local files referenced in BEP to remote cache.
    pub enum RemoteBuildEventUploadMode {
        // Uploads all local files to remote cache.
        ALL,
        // Only upload important local files (e.g. test logs, timing profile) to remote cache.
        MINIMAL,
    }
}
