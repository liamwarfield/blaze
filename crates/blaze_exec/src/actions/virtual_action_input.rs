//! Port of `lib/actions/cache/VirtualActionInput.java`.

use std::any::Any;
use std::fmt;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use blaze_vfs::{Path, PathFragment};

use super::action_input::{ActionInput, ActionInputRef};
use super::file_artifact_value::FileArtifactValue;

/// An `ActionInput` that does not exist on the file system but can be written to a stream
/// (parameter files, runfiles manifests, empty runfiles files).
// Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:37 VirtualActionInput
pub trait VirtualActionInput: ActionInput {
    /// `DeterministicWriter.writeTo(OutputStream)`.
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()>;

    /// `DeterministicWriter.getBytes()`.
    fn get_bytes(&self) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.write_to(&mut buf)?;
        Ok(buf)
    }

    /// `getMetadata()`: the metadata if available.
    // Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:126 VirtualActionInput.getMetadata
    fn get_metadata(&self) -> io::Result<Option<FileArtifactValue>> {
        Ok(None)
    }

    /// `instanceof VirtualActionInput.EmptyActionInput`.
    fn is_empty_action_input(&self) -> bool {
        false
    }

    /// `atomicallyWriteRelativeTo(Path execRoot)`: writes the input below `exec_root` so that no
    /// reader observes an incomplete file; returns its digest.
    // Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:63 VirtualActionInput.atomicallyWriteRelativeTo
    fn atomically_write_relative_to(&self, exec_root: &Path) -> io::Result<Vec<u8>> {
        if self.is_empty_action_input() {
            return Ok(Vec::new());
        }
        let output_path = exec_root.get_relative(self.get_exec_path());
        self.atomically_write_to(&output_path)
    }

    /// `atomicallyWriteTo(Path outputPath)`.
    // Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:72 VirtualActionInput.atomicallyWriteTo
    fn atomically_write_to(&self, output_path: &Path) -> io::Result<Vec<u8>> {
        if self.is_empty_action_input() {
            return Ok(Vec::new());
        }
        let tmp_path = output_path.get_file_system_path(&format!(
            "{}.tmp.{}",
            output_path.get_path_string(),
            TMP_SUFFIX.fetch_add(1, Ordering::SeqCst)
        ));
        if let Some(parent) = tmp_path.get_parent_directory() {
            parent.create_directory_and_parents()?;
        }
        tmp_path.delete()?;
        let result = (|| {
            let digest = virtual_write_to_path(self, &tmp_path)?;
            tmp_path.rename_to(output_path)?;
            Ok(digest)
        })();
        if result.is_err() {
            let _ = tmp_path.delete();
        }
        result
    }
}

/// The next unique filename suffix to use when writing to a temporary path.
static TMP_SUFFIX: AtomicU32 = AtomicU32::new(0);

/// `VirtualActionInput.writeTo(Path target)`: writes the contents, makes the file executable and
/// returns the digest.
// Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:103 VirtualActionInput.writeTo
pub fn virtual_write_to_path<V: VirtualActionInput + ?Sized>(
    input: &V,
    target: &Path,
) -> io::Result<Vec<u8>> {
    let bytes = input.get_bytes()?;
    {
        let mut out = target.get_output_stream()?;
        out.write_all(&bytes)?;
        out.flush()?;
    }
    target.set_executable(true)?;
    // `HashingOutputStream(fs.getDigestFunction().getHashFunction(), out)`.
    Ok(target
        .get_file_system()
        .get_digest_function()
        .hash_bytes(&bytes))
}

trait PathExt {
    fn get_file_system_path(&self, s: &str) -> Path;
}

impl PathExt for Path {
    fn get_file_system_path(&self, s: &str) -> Path {
        Path::create(PathFragment::create(s), self.get_file_system())
    }
}

/// `VirtualActionInput.EmptyActionInput`: an empty file in a runfiles or fileset tree with no
/// corresponding artifact. It has no exec path.
// Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:146 VirtualActionInput.EmptyActionInput
#[derive(Debug)]
pub struct EmptyActionInput {
    _private: (),
}

impl ActionInput for EmptyActionInput {
    // Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:162 VirtualActionInput.EmptyActionInput.getExecPath
    fn get_exec_path(&self) -> &PathFragment {
        panic!("UnsupportedOperationException: empty virtual artifact doesn't have an execpath")
    }
    // Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:157 VirtualActionInput.EmptyActionInput.getExecPathString
    fn get_exec_path_string(&self) -> String {
        panic!("UnsupportedOperationException: empty virtual artifact doesn't have an execpath")
    }
    // Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:138 VirtualActionInput.isDirectory
    fn is_directory(&self) -> bool {
        false
    }
    // Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:143 VirtualActionInput.isSymlink
    fn is_symlink(&self) -> bool {
        false
    }
    fn as_virtual_action_input(&self) -> Option<&dyn VirtualActionInput> {
        Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// (`atomicallyWriteRelativeTo` / `atomicallyWriteTo` return the empty digest through
// `is_empty_action_input`; `getBytes()` is the empty write.)
// Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:167 VirtualActionInput.EmptyActionInput.atomicallyWriteRelativeTo
// Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:172 VirtualActionInput.EmptyActionInput.atomicallyWriteTo
// Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:182 VirtualActionInput.EmptyActionInput.getBytes
impl VirtualActionInput for EmptyActionInput {
    // Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:177 VirtualActionInput.EmptyActionInput.writeTo
    fn write_to(&self, _out: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }
    fn is_empty_action_input(&self) -> bool {
        true
    }
}

impl fmt::Display for EmptyActionInput {
    // Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:187 VirtualActionInput.EmptyActionInput.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EmptyActionInput")
    }
}

/// `VirtualActionInput.EMPTY_MARKER`.
// Java: com/google/devtools/build/lib/actions/cache/VirtualActionInput.java:42 VirtualActionInput.EMPTY_MARKER
pub fn empty_marker() -> ActionInputRef {
    static EMPTY: std::sync::OnceLock<ActionInputRef> = std::sync::OnceLock::new();
    EMPTY
        .get_or_init(|| ActionInputRef(Arc::new(EmptyActionInput { _private: () })))
        .clone()
}

/// A virtual input with fixed contents at a fixed exec path. Port of the test helper
/// `ActionsTestUtil.createVirtualActionInput`, also used for parameter files whose contents are
/// already known (`CommandLines.ParamFileActionInput` lives in `blaze_actions`).
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java ActionsTestUtil.createVirtualActionInput
#[derive(Debug, Clone)]
pub struct BasicVirtualActionInput {
    exec_path: PathFragment,
    contents: Vec<u8>,
}

impl BasicVirtualActionInput {
    /// Creates the input.
    pub fn new(exec_path: PathFragment, contents: Vec<u8>) -> BasicVirtualActionInput {
        BasicVirtualActionInput {
            exec_path,
            contents,
        }
    }
}

impl ActionInput for BasicVirtualActionInput {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        false
    }
    fn is_symlink(&self) -> bool {
        false
    }
    fn as_virtual_action_input(&self) -> Option<&dyn VirtualActionInput> {
        Some(self)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl VirtualActionInput for BasicVirtualActionInput {
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&self.contents)
    }
}
