//! Ports of `lib/analysis/actions/FileWriteAction.java` (with its nested `RegularFileWriteAction`
//! and `CompressedFileWriteAction`), `Compression.java` and the `OnDemandString` contents
//! (`lib/util/OnDemandString.java`).

use std::any::Any;
use std::fmt;
use std::io::{self, Write};
use std::sync::Arc;

use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::{Fingerprint, check_state};
use indexmap::IndexMap;

use crate::actions::action::{
    AbstractAction, Action, ComputeKeyError, FileContentsProvider, InputMetadataProvider,
};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;

use super::abstract_file_write_action::{
    self, AbstractFileWriteAction, DeterministicWriter, FnDeterministicWriter,
    NewDeterministicWriterError, new_abstract_file_write_action,
};

/// Whether or not transparent compression is possible.
// Java: com/google/devtools/build/lib/analysis/actions/Compression.java:17 Compression
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Compression {
    /// No compression, ever.
    DISALLOW,
    /// May compress.
    ALLOW,
}

/// A string whose computation is deferred until it is needed (`toString()`).
// Java: com/google/devtools/build/lib/util/OnDemandString.java:21 OnDemandString
pub trait OnDemandString: Send + Sync {
    /// Java `toString()`: computes the string.
    fn to_string(&self) -> String;
}

/// The contents of a `FileWriteAction`: Java's `CharSequence`, which is either an eager `String`
/// or a lazily computed `OnDemandString`.
#[derive(Clone)]
pub enum CharSequence {
    String(String),
    OnDemand(Arc<dyn OnDemandString>),
}

impl CharSequence {
    /// Java `CharSequence.toString()` (forces an `OnDemandString`).
    pub fn to_string_java(&self) -> String {
        match self {
            CharSequence::String(s) => s.clone(),
            CharSequence::OnDemand(s) => OnDemandString::to_string(&**s),
        }
    }
}

impl From<String> for CharSequence {
    fn from(s: String) -> CharSequence {
        CharSequence::String(s)
    }
}

impl From<&str> for CharSequence {
    fn from(s: &str) -> CharSequence {
        CharSequence::String(s.to_string())
    }
}

impl fmt::Debug for CharSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CharSequence::String(s) => fmt::Debug::fmt(s, f),
            CharSequence::OnDemand(_) => f.write_str("OnDemandString"),
        }
    }
}

/// Minimum length (in chars) for content to be eligible for compression.
// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:69 FileWriteAction.COMPRESS_CHARS_THRESHOLD
const COMPRESS_CHARS_THRESHOLD: usize = 256;

// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:250 RegularFileWriteAction.GUID
const REGULAR_GUID: &str = "332877c7-ca9f-4731-b387-54f620408522";

// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:285 CompressedFileWriteAction.GUID
const COMPRESSED_GUID: &str = "5bfba914-2251-11ee-be56-0242ac120002";

// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:286 CompressedFileWriteAction.GZIP_BYTES_BUFFER
const GZIP_BYTES_BUFFER: usize = 8192;

/// The two concrete Java subclasses.
#[derive(Debug)]
enum FileWriteActionKind {
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:249 FileWriteAction.RegularFileWriteAction
    Regular {
        file_contents: CharSequence,
    },
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:284 FileWriteAction.CompressedFileWriteAction
    Compressed {
        compressed_bytes: Vec<u8>,
        uncompressed_size: usize,
    },
}

/// Action to write a file whose contents are known at analysis time.
///
/// The output file is generally encoded as UTF-8, but by an unusual path. BUILD files and
/// directory entries, which are actually UTF-8, are misinterpreted by Bazel as Latin1, so that
/// most Strings within the build language use this unusual representation. FileWriteAction writes
/// those Strings out again as Latin1. (PORT: StringEncoding identity; Rust strings already hold
/// the UTF-8 bytes, which are written as is.)
///
/// The contents may be lazily computed or compressed. If the object representing the contents is
/// a `String`, its length is greater than `COMPRESS_CHARS_THRESHOLD`, and compression is enabled,
/// then the gzipped bytestream of the contents will be stored in place of the string itself. This
/// compression is transparent and does not affect the output file.
///
/// Otherwise, if the object represents a lazy computation, it will not be forced until
/// [`FileWriteAction::get_file_contents`] is called.
// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:65 FileWriteAction
#[derive(Debug)]
pub struct FileWriteAction {
    base: AbstractAction,
    make_executable: bool,
    mnemonic: String,
    kind: FileWriteActionKind,
}

impl FileWriteAction {
    // PORT: `FileWriteAction.createFile(RuleContext, String, CharSequence, boolean)` needs
    // `RuleContext.getPackageRelativeArtifact`; the analysis lead creates the artifact and calls
    // `create_from_context`.

    /// Creates a new FileWriteAction instance with inputs and empty content.
    ///
    /// This is useful for producing an artifact that, if built, will ensure that the generating
    /// actions for its inputs are run. The output file is non-executable.
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:108 FileWriteAction.createEmptyWithInputs
    pub fn create_empty_with_inputs(
        owner: ActionOwner,
        inputs: NestedSet<Artifact>,
        output: Artifact,
    ) -> FileWriteAction {
        FileWriteAction::create_internal(
            owner,
            inputs,
            output,
            CharSequence::String(String::new()),
            false,
            Compression::DISALLOW,
            abstract_file_write_action::MNEMONIC.to_string(),
        )
    }

    /// Creates a new FileWriteAction instance with direct control over whether or not transparent
    /// compression may be used.
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:124 FileWriteAction.create
    pub fn create(
        owner: ActionOwner,
        output: Artifact,
        file_contents: impl Into<CharSequence>,
        make_executable: bool,
        allow_compression: Compression,
    ) -> FileWriteAction {
        FileWriteAction::create_internal(
            owner,
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            output,
            file_contents.into(),
            make_executable,
            allow_compression,
            abstract_file_write_action::MNEMONIC.to_string(),
        )
    }

    /// `create(ActionConstructionContext context, Artifact output, CharSequence fileContents,
    /// boolean makeExecutable)`. There are no inputs.
    ///
    /// PORT: `action_owner` is `context.getActionOwner()`; no reference to the context is kept in
    /// Java either.
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:151 FileWriteAction.create
    pub fn create_from_context(
        action_owner: ActionOwner,
        output: Artifact,
        file_contents: impl Into<CharSequence>,
        make_executable: bool,
    ) -> FileWriteAction {
        FileWriteAction::create_from_context_with_mnemonic(
            action_owner,
            output,
            file_contents,
            make_executable,
            abstract_file_write_action::MNEMONIC,
        )
    }

    /// `create(ActionConstructionContext context, Artifact output, CharSequence fileContents,
    /// boolean makeExecutable, String mnemonic)`. There are no inputs; compression is allowed.
    ///
    /// PORT: `action_owner` is `context.getActionOwner()`.
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:171 FileWriteAction.create
    pub fn create_from_context_with_mnemonic(
        action_owner: ActionOwner,
        output: Artifact,
        file_contents: impl Into<CharSequence>,
        make_executable: bool,
        mnemonic: &str,
    ) -> FileWriteAction {
        FileWriteAction::create_internal(
            action_owner,
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            output,
            file_contents.into(),
            make_executable,
            Compression::ALLOW,
            mnemonic.to_string(),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:187 FileWriteAction.createInternal
    fn create_internal(
        owner: ActionOwner,
        inputs: NestedSet<Artifact>,
        output: Artifact,
        file_contents: CharSequence,
        make_executable: bool,
        allow_compression: Compression,
        mnemonic: String,
    ) -> FileWriteAction {
        // PORT: Java's `length()` counts the chars of the internal (Latin-1-of-UTF-8) string,
        // which is the UTF-8 byte length.
        let kind = match file_contents {
            CharSequence::String(s)
                if allow_compression == Compression::ALLOW
                    && s.len() > COMPRESS_CHARS_THRESHOLD =>
            {
                compressed_kind(&s)
            }
            other => FileWriteActionKind::Regular {
                file_contents: other,
            },
        };
        // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:205 FileWriteAction.<init>
        FileWriteAction {
            base: new_abstract_file_write_action(owner, inputs, output),
            make_executable,
            mnemonic,
            kind,
        }
    }

    /// Returns the string contents to be written.
    ///
    /// Note that if the string is lazily computed or compressed, calling this method will force
    /// its computation or decompression. No attempt is made by FileWriteAction to cache the
    /// result.
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:242 FileWriteAction.getFileContents
    pub fn get_file_contents(&self) -> String {
        match &self.kind {
            // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:266 RegularFileWriteAction.getFileContents
            FileWriteActionKind::Regular { file_contents } => file_contents.to_string_java(),
            // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:320 CompressedFileWriteAction.getFileContents
            FileWriteActionKind::Compressed {
                compressed_bytes,
                uncompressed_size,
            } => {
                let uncompressed_bytes = match gunzip(compressed_bytes) {
                    Ok(b) => b,
                    // This should be impossible since we're reading from a byte array.
                    Err(e) => panic!("IllegalStateException: {}", e),
                };
                check_state!(
                    uncompressed_bytes.len() == *uncompressed_size,
                    "Corrupt byte buffer in FileWriteAction"
                );
                // PORT: StringEncoding identity (`StringUnsafe.newInstance(bytes, LATIN1)`); the
                // bytes came from a Rust String.
                String::from_utf8(uncompressed_bytes)
                    .expect("FileWriteAction compressed contents are UTF-8")
            }
        }
    }

    /// `getFileContents(@Nullable EventHandler)`.
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:217 FileWriteAction.getFileContents
    pub fn get_file_contents_with_event_handler(&self) -> String {
        self.get_file_contents()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:245 FileWriteAction.getStarlarkContent
    pub fn get_starlark_content(&self) -> String {
        self.get_file_contents()
    }

    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:222 FileWriteAction.makeExecutable
    pub fn make_executable(&self) -> bool {
        self.make_executable
    }

    /// Whether the contents are stored gzip-compressed (Java: the `CompressedFileWriteAction`
    /// subclass).
    pub fn is_compressed(&self) -> bool {
        matches!(self.kind, FileWriteActionKind::Compressed { .. })
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:291 CompressedFileWriteAction.<init>
fn compressed_kind(file_contents: &str) -> FileWriteActionKind {
    // Grab the string's internal byte array. Calling getBytes() makes a copy, which can cause
    // memory spikes resulting in OOMs (b/290807073). Do not mutate this!
    // PORT: StringEncoding identity: the internal Latin-1 bytes are the UTF-8 bytes.
    let data_to_compress = file_contents.as_bytes();
    let compressed_bytes = gzip(data_to_compress);
    FileWriteActionKind::Compressed {
        compressed_bytes,
        uncompressed_size: data_to_compress.len(),
    }
}

impl Action for FileWriteAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:227 FileWriteAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        self.mnemonic.clone()
    }

    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        match &self.kind {
            // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:276 RegularFileWriteAction.computeKey
            FileWriteActionKind::Regular { .. } => {
                fp.add_string(REGULAR_GUID)
                    .add_boolean(self.make_executable)
                    // PORT: StringEncoding. The contents are a Bazel internal string, which
                    // Java's addString UTF-8-encodes once more (checked against the oracle).
                    .add_internal_string(&self.get_file_contents());
            }
            // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:351 CompressedFileWriteAction.computeKey
            FileWriteActionKind::Compressed {
                compressed_bytes, ..
            } => {
                fp.add_string(COMPRESSED_GUID)
                    .add_boolean(self.make_executable)
                    .add_bytes(compressed_bytes);
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:101 AbstractFileWriteAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        abstract_file_write_action::raw_progress_message(self)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:125 AbstractFileWriteAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        abstract_file_write_action::exec_properties()
    }

    fn as_file_contents_provider(&self) -> Option<&dyn FileContentsProvider> {
        Some(self)
    }
}

impl AbstractFileWriteAction for FileWriteAction {
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:222 FileWriteAction.makeExecutable
    fn make_executable(&self) -> bool {
        self.make_executable
    }

    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        match &self.kind {
            // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:271 RegularFileWriteAction.newDeterministicWriter
            FileWriteActionKind::Regular { .. } => {
                Ok(Box::new(FnDeterministicWriter(|out: &mut dyn Write| {
                    // PORT: StringEncoding identity (`StringUnsafe.getInternalStringBytes`).
                    out.write_all(self.get_file_contents().as_bytes())
                })))
            }
            // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:341 CompressedFileWriteAction.newDeterministicWriter
            FileWriteActionKind::Compressed {
                compressed_bytes, ..
            } => Ok(Box::new(FnDeterministicWriter(
                move |out: &mut dyn Write| {
                    let bytes = gunzip(compressed_bytes)?;
                    out.write_all(&bytes)
                },
            ))),
        }
    }
}

impl FileContentsProvider for FileWriteAction {
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:217 FileWriteAction.getFileContents
    fn get_file_contents(&self) -> Result<String, io::Error> {
        Ok(FileWriteAction::get_file_contents(self))
    }

    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:222 FileWriteAction.makeExecutable
    fn make_executable(&self) -> bool {
        self.make_executable
    }
}

// ---------------------------------------------------------------------------------------------
// java.util.zip.GZIPOutputStream / GZIPInputStream
// ---------------------------------------------------------------------------------------------

/// `new GZIPOutputStream(byteStream, GZIP_BYTES_BUFFER)`, one `write(data)`, `close()`.
///
/// PORT: the compressed bytes are part of the action key, so they must be the bytes Java
/// produces. Java's `Deflater` calls the zlib that the JDK links; Bazel's embedded JDK links the
/// system `libz.so.1` dynamically (on Fedora: zlib-ng-compat). The pure-Rust deflaters (miniz_oxide,
/// zlib-rs) produce different bytes for some inputs, so this calls the system zlib through FFI,
/// with Java's parameters (level `DEFAULT_COMPRESSION`, raw deflate `-MAX_WBITS`, memLevel 8,
/// default strategy) and Java's call sequence (`NO_FLUSH` until the input is consumed, then
/// `FINISH`), framed with Java's gzip header and trailer.
// Java: java/util/zip/GZIPOutputStream.java GZIPOutputStream.<init>/write/finish
fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 100 + 18);
    // GZIPOutputStream.writeHeader: magic, CM=DEFLATED, FLG=0, MTIME=0, XFL=0, OS=255 (unknown,
    // JDK 16+).
    out.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff]);
    out.extend_from_slice(&zlib::deflate_raw(data, GZIP_BYTES_BUFFER));
    // GZIPOutputStream.writeTrailer: CRC-32 and the uncompressed size (mod 2^32), little-endian.
    out.extend_from_slice(&zlib::crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// `new GZIPInputStream(new ByteArrayInputStream(compressed), GZIP_BYTES_BUFFER)` read to the
/// end, for the streams [`gzip`] writes (no optional header fields, single member).
// Java: java/util/zip/GZIPInputStream.java GZIPInputStream.readHeader/read/readTrailer
fn gunzip(compressed: &[u8]) -> io::Result<Vec<u8>> {
    if compressed.len() < 18 || compressed[0] != 0x1f || compressed[1] != 0x8b {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Not in GZIP format",
        ));
    }
    if compressed[2] != 8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unsupported compression method",
        ));
    }
    if compressed[3] != 0 {
        // PORT: only the header GZIPOutputStream writes (FLG=0) is supported.
        blaze_util::unported!("GZIPInputStream.readHeader", "gzip header flags");
    }
    let body = &compressed[10..];
    let (data, consumed) = zlib::inflate_raw(body, GZIP_BYTES_BUFFER)?;
    let trailer = &body[consumed..];
    if trailer.len() < 8 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Unexpected end of ZLIB input stream",
        ));
    }
    let crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    let size = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
    if crc != zlib::crc32(&data) || size != data.len() as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Corrupt GZIP trailer",
        ));
    }
    Ok(data)
}

/// The system zlib (`libz.so.1`), as `java.util.zip.Deflater`/`Inflater`/`CRC32` call it.
mod zlib {
    use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
    use std::io;

    /// zlib's `z_stream` (identical layout in zlib-ng-compat).
    #[repr(C)]
    struct ZStream {
        next_in: *const u8,
        avail_in: c_uint,
        total_in: c_ulong,
        next_out: *mut u8,
        avail_out: c_uint,
        total_out: c_ulong,
        msg: *const c_char,
        state: *mut c_void,
        zalloc: *const c_void,
        zfree: *const c_void,
        opaque: *mut c_void,
        data_type: c_int,
        adler: c_ulong,
        reserved: c_ulong,
    }

    #[link(name = "z")]
    unsafe extern "C" {
        fn zlibVersion() -> *const c_char;
        fn deflateInit2_(
            strm: *mut ZStream,
            level: c_int,
            method: c_int,
            window_bits: c_int,
            mem_level: c_int,
            strategy: c_int,
            version: *const c_char,
            stream_size: c_int,
        ) -> c_int;
        fn deflate(strm: *mut ZStream, flush: c_int) -> c_int;
        fn deflateEnd(strm: *mut ZStream) -> c_int;
        fn inflateInit2_(
            strm: *mut ZStream,
            window_bits: c_int,
            version: *const c_char,
            stream_size: c_int,
        ) -> c_int;
        fn inflate(strm: *mut ZStream, flush: c_int) -> c_int;
        fn inflateEnd(strm: *mut ZStream) -> c_int;
        #[link_name = "crc32"]
        fn zlib_crc32(crc: c_ulong, buf: *const u8, len: c_uint) -> c_ulong;
    }

    const Z_NO_FLUSH: c_int = 0;
    const Z_FINISH: c_int = 4;
    const Z_OK: c_int = 0;
    const Z_STREAM_END: c_int = 1;
    const Z_BUF_ERROR: c_int = -5;
    const Z_DEFLATED: c_int = 8;
    // Java: java/util/zip/Deflater.java Deflater.DEFAULT_COMPRESSION
    const Z_DEFAULT_COMPRESSION: c_int = -1;
    // Java: java/util/zip/Deflater.java Deflater.DEFAULT_STRATEGY
    const Z_DEFAULT_STRATEGY: c_int = 0;
    const MAX_WBITS: c_int = 15;
    // JDK Deflater.c: DEF_MEM_LEVEL
    const DEF_MEM_LEVEL: c_int = 8;

    /// `new Deflater(DEFAULT_COMPRESSION, /* nowrap= */ true)` driven by `DeflaterOutputStream`:
    /// `write(input)` deflates with `NO_FLUSH` until the input is consumed, `finish()` deflates
    /// with `FINISH` until the stream ends, each call into a `buffer_size` buffer.
    pub fn deflate_raw(input: &[u8], buffer_size: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; buffer_size];
        // SAFETY: the stream is zero-initialized as zlib requires (Z_NULL allocators), the
        // input and output pointers stay valid for the duration of each call, and the stream is
        // ended before returning.
        unsafe {
            let mut strm: ZStream = std::mem::zeroed();
            let rc = deflateInit2_(
                &mut strm,
                Z_DEFAULT_COMPRESSION,
                Z_DEFLATED,
                -MAX_WBITS,
                DEF_MEM_LEVEL,
                Z_DEFAULT_STRATEGY,
                zlibVersion(),
                std::mem::size_of::<ZStream>() as c_int,
            );
            assert_eq!(rc, Z_OK, "IllegalStateException: deflateInit2 failed");
            let mut remaining = input;
            // Java: DeflaterOutputStream.write: `while (!def.needsInput()) deflate();`
            while !remaining.is_empty() {
                let chunk = remaining.len().min(c_uint::MAX as usize);
                strm.next_in = remaining.as_ptr();
                strm.avail_in = chunk as c_uint;
                while strm.avail_in > 0 {
                    strm.next_out = buf.as_mut_ptr();
                    strm.avail_out = buf.len() as c_uint;
                    let rc = deflate(&mut strm, Z_NO_FLUSH);
                    assert!(
                        rc == Z_OK || rc == Z_BUF_ERROR,
                        "IllegalStateException: deflate failed: {}",
                        rc
                    );
                    out.extend_from_slice(&buf[..buf.len() - strm.avail_out as usize]);
                }
                remaining = &remaining[chunk..];
            }
            // Java: DeflaterOutputStream.finish: `def.finish(); while (!def.finished()) deflate();`
            loop {
                strm.next_out = buf.as_mut_ptr();
                strm.avail_out = buf.len() as c_uint;
                let rc = deflate(&mut strm, Z_FINISH);
                assert!(
                    rc == Z_OK || rc == Z_STREAM_END || rc == Z_BUF_ERROR,
                    "IllegalStateException: deflate failed: {}",
                    rc
                );
                out.extend_from_slice(&buf[..buf.len() - strm.avail_out as usize]);
                if rc == Z_STREAM_END {
                    break;
                }
            }
            deflateEnd(&mut strm);
        }
        out
    }

    /// Raw-inflates one deflate stream at the start of `input`; returns the data and the number
    /// of input bytes the stream used.
    pub fn inflate_raw(input: &[u8], buffer_size: usize) -> io::Result<(Vec<u8>, usize)> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; buffer_size];
        // SAFETY: as in `deflate_raw`.
        unsafe {
            let mut strm: ZStream = std::mem::zeroed();
            let rc = inflateInit2_(
                &mut strm,
                -MAX_WBITS,
                zlibVersion(),
                std::mem::size_of::<ZStream>() as c_int,
            );
            assert_eq!(rc, Z_OK, "IllegalStateException: inflateInit2 failed");
            let avail = input.len().min(c_uint::MAX as usize);
            strm.next_in = input.as_ptr();
            strm.avail_in = avail as c_uint;
            let result = loop {
                strm.next_out = buf.as_mut_ptr();
                strm.avail_out = buf.len() as c_uint;
                let rc = inflate(&mut strm, Z_NO_FLUSH);
                out.extend_from_slice(&buf[..buf.len() - strm.avail_out as usize]);
                if rc == Z_STREAM_END {
                    break Ok(avail - strm.avail_in as usize);
                }
                if rc == Z_BUF_ERROR && strm.avail_in == 0 {
                    break Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Unexpected end of ZLIB input stream",
                    ));
                }
                if rc != Z_OK && rc != Z_BUF_ERROR {
                    break Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid deflate stream ({})", rc),
                    ));
                }
            };
            inflateEnd(&mut strm);
            result.map(|consumed| (out, consumed))
        }
    }

    /// `java.util.zip.CRC32` over `data`.
    pub fn crc32(data: &[u8]) -> u32 {
        let mut crc: c_ulong = 0;
        for chunk in data.chunks(1 << 30) {
            // SAFETY: `chunk` is a valid slice of `chunk.len()` bytes.
            crc = unsafe { zlib_crc32(crc, chunk.as_ptr(), chunk.len() as c_uint) };
        }
        crc as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gzip_round_trip() {
        let data: String = (0..1000).map(|i| format!("line {}\n", i)).collect();
        let z = gzip(data.as_bytes());
        assert_eq!(&z[..10], &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff]);
        assert_eq!(gunzip(&z).unwrap(), data.as_bytes());
        assert_eq!(gunzip(&gzip(b"")).unwrap(), b"");
    }
}
