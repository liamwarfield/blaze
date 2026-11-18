//! Port of `remote/options/RemoteOutputsMode.java`.

use blaze_options::option_enum;

// Java: com/google/devtools/build/lib/remote/options/RemoteOutputsMode.java:18 RemoteOutputsMode
option_enum! {
    /// Describes what kind of remote build outputs to download locally.
    pub enum RemoteOutputsMode {
        // Download all remote outputs locally.
        ALL,
        // Generally don't download remote action outputs. The only outputs that are being
        // downloaded are: stdout, stderr and .d and .jdeps files for C++ and Java compilation
        // actions.
        MINIMAL,
        // Downloads outputs of top level targets. Top level targets are targets specified on the
        // command line. If a top level target has runfile dependencies it will also download
        // those. Intermediate outputs are generally not downloaded (See `MINIMAL`.
        TOPLEVEL,
    }
}
