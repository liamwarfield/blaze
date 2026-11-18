//! Port of `lib/exec/BinTools.java`: the binaries embedded in the install base
//! (`<install_base>/_embedded_binaries`: `linux-sandbox`, `process-wrapper`, `build-runfiles`,
//! ...).

use std::any::Any;
use std::io::{self, Read, Write};
use std::sync::Mutex;

use blaze_vfs::{DirentType, Path, PathFragment, Symlinks};
use indexmap::IndexMap;

use crate::actions::{
    ActionInput, ActionInputRef, FileArtifactValue, VirtualActionInput,
    virtual_action_input::virtual_write_to_path,
};

/// Initializes and provides the embedded tools.
// Java: com/google/devtools/build/lib/exec/BinTools.java:42 BinTools
#[derive(Debug)]
pub struct BinTools {
    embedded_binaries_root: Path,
    embedded_tools: Vec<String>,
    action_inputs: IndexMap<String, ActionInputRef>,
}

impl BinTools {
    // Java: com/google/devtools/build/lib/exec/BinTools.java:53 BinTools.<init>
    fn new(embedded_binaries_root: Path, tools: Vec<String>) -> BinTools {
        let embedded_tools: Vec<String> = tools
            .into_iter()
            .filter(|t| !t.starts_with("embedded_tools/"))
            .collect();
        let mut bt = BinTools {
            embedded_binaries_root,
            embedded_tools,
            action_inputs: IndexMap::new(),
        };
        let mut result = IndexMap::new();
        for embedded_path in &bt.embedded_tools {
            let path = bt.get_embedded_path(embedded_path).unwrap();
            let exec_path = PathFragment::create("_bin").get_relative_str(embedded_path);
            result.insert(
                embedded_path.clone(),
                ActionInputRef::new(PathActionInput::new(path, exec_path)),
            );
        }
        bt.action_inputs = result;
        bt
    }

    /// `forProduction(BlazeDirectories)`: scans `embedded_binaries_root` recursively.
    // Java: com/google/devtools/build/lib/exec/BinTools.java:75 BinTools.forProduction
    pub fn for_production(embedded_binaries_root: Path) -> io::Result<BinTools> {
        let mut tools = Vec::new();
        scan_directory_recursively(&mut tools, &embedded_binaries_root, "")?;
        Ok(BinTools::new(embedded_binaries_root, tools))
    }

    /// `forEmbeddedBin(Path, Iterable<String>)`.
    pub fn for_embedded_bin(embedded_binaries_root: Path, tools: Vec<String>) -> BinTools {
        BinTools::new(embedded_binaries_root, tools)
    }

    /// `forUnitTesting(Path execroot, Iterable<String> tools)`.
    pub fn for_unit_testing(execroot: &Path, tools: Vec<String>) -> BinTools {
        BinTools::new(execroot.get_relative_str("/fake/embedded/tools"), tools)
    }

    /// `getActionInput(embeddedPath)`.
    pub fn get_action_input(&self, embedded_path: &str) -> Option<ActionInputRef> {
        self.action_inputs.get(embedded_path).cloned()
    }

    /// `getEmbeddedPath(embedPath)`: `None` if the tool is not embedded.
    // Java: com/google/devtools/build/lib/exec/BinTools.java:136 BinTools.getEmbeddedPath
    pub fn get_embedded_path(&self, embed_path: &str) -> Option<Path> {
        if !self.embedded_tools.iter().any(|t| t == embed_path) {
            return None;
        }
        Some(self.embedded_binaries_root.get_relative_str(embed_path))
    }
}

// Java: com/google/devtools/build/lib/exec/BinTools.java:101 BinTools.scanDirectoryRecursively
fn scan_directory_recursively(
    result: &mut Vec<String>,
    root: &Path,
    relative: &str,
) -> io::Result<()> {
    for dirent in root.readdir(Symlinks::Nofollow)? {
        let child_relative = if relative.is_empty() {
            dirent.get_name().to_string()
        } else {
            format!("{}/{}", relative, dirent.get_name())
        };
        match dirent.get_type() {
            DirentType::File => result.push(child_relative),
            DirentType::Directory => scan_directory_recursively(
                result,
                &root.get_child(dirent.get_name()),
                &child_relative,
            )?,
            _ => {}
        }
    }
    Ok(())
}

/// `BinTools.PathActionInput`: an embedded tool staged at `_bin/<name>` in the exec root.
// Java: com/google/devtools/build/lib/exec/BinTools.java:146 BinTools.PathActionInput
#[derive(Debug)]
pub struct PathActionInput {
    path: Path,
    exec_path: PathFragment,
    state: Mutex<(Option<FileArtifactValue>, Option<Vec<u8>>)>,
}

impl PathActionInput {
    /// `new PathActionInput(path, execPath)`.
    pub fn new(path: Path, exec_path: PathFragment) -> PathActionInput {
        PathActionInput {
            path,
            exec_path,
            state: Mutex::new((None, None)),
        }
    }
}

impl ActionInput for PathActionInput {
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

impl VirtualActionInput for PathActionInput {
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        let mut input = self.path.get_input_stream()?;
        io::copy(&mut input, out)?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/exec/BinTools.java:166 BinTools.PathActionInput.atomicallyWriteTo
    fn atomically_write_to(&self, output_path: &Path) -> io::Result<Vec<u8>> {
        let mut state = self.state.lock().unwrap();
        if state.1.is_none() || !output_path.exists() {
            if let Some(parent) = output_path.get_parent_directory() {
                parent.create_directory_and_parents()?;
            }
            let digest = virtual_write_to_path(self, output_path)?;
            output_path.set_executable(true)?;
            state.1 = Some(digest);
        }
        Ok(state.1.clone().unwrap())
    }

    // Java: com/google/devtools/build/lib/exec/BinTools.java:183 BinTools.PathActionInput.getMetadata
    fn get_metadata(&self) -> io::Result<Option<FileArtifactValue>> {
        let mut state = self.state.lock().unwrap();
        if state.0.is_none() {
            let hash_fn = self.path.get_file_system().get_digest_function();
            let mut hasher = hash_fn.new_hasher();
            let mut bytes_copied: i64 = 0;
            let mut input = self.path.get_input_stream()?;
            let mut buffer = [0u8; 1024];
            loop {
                let len = input.read(&mut buffer)?;
                if len == 0 {
                    break;
                }
                hasher.update(&buffer[..len]);
                bytes_copied += len as i64;
            }
            state.0 = Some(FileArtifactValue::create_for_virtual_action_input(
                hasher.finish(),
                bytes_copied,
            ));
        }
        Ok(state.0.clone())
    }
}
