//! Port of `com.google.devtools.build.lib.remote.zstd`.
//!
//! PORT: the Java classes sit on zstd-jni's `ZstdOutputStreamNoFinalizer` /
//! `ZstdInputStreamNoFinalizer`; here they sit on the same libzstd streaming calls through the
//! `zstd` crate's raw stream API (`::zstd::stream::raw`), driven the way zstd-jni drives them.
//! Inside this crate the external crate is always spelled `::zstd`, this module `crate::zstd`.

pub mod zstd_compressing_input_stream;
pub mod zstd_decompressing_output_stream;

pub use zstd_compressing_input_stream::ZstdCompressingInputStream;
pub use zstd_decompressing_output_stream::ZstdDecompressingOutputStream;
