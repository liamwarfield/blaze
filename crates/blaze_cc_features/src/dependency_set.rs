//! Port of `com.google.devtools.build.lib.util.DependencySet`: the `.d` file parser and writer.
//!
//! PORT: the Java class lives in `lib/util`; it is here because `blaze_util` cannot depend on
//! `blaze_vfs`, and its only user is the C++ compile action.

use std::io::{self, Read, Write};

use blaze_vfs::{Path, PathFragment};

/// Representation of a set of file dependencies for a given output file. There are generally one
/// input dependency and a bunch of include dependencies. The files are stored as `Path`s and may
/// be relative or absolute.
///
/// The serialized format read and written is equivalent and compatible with the ".d" file
/// produced by the -MM for a given out (.o) file.
///
/// The file format looks like:
///
/// ```text
/// {outfile}:  \
///  {infile} \
///   {include} \
///   ... \
///   {include}
/// ```
// Java: com/google/devtools/build/lib/util/DependencySet.java:55 DependencySet
#[derive(Debug)]
pub struct DependencySet {
    /// The set of dependent files that this DependencySet embodies, in the order read.
    dependencies: Vec<Path>,
    root: Path,
    output_file_name: Option<String>,
}

impl PartialEq for DependencySet {
    // Java: com/google/devtools/build/lib/util/DependencySet.java:245 DependencySet.equals
    fn eq(&self, other: &Self) -> bool {
        self.dependencies == other.dependencies
    }
}

impl DependencySet {
    /// Constructs a new empty DependencySet instance.
    // Java: com/google/devtools/build/lib/util/DependencySet.java:76 DependencySet.<init>
    pub fn new(root: Path) -> DependencySet {
        DependencySet {
            dependencies: Vec::new(),
            root,
            output_file_name: None,
        }
    }

    /// Get output file name for which dependencies are included in this DependencySet.
    // Java: com/google/devtools/build/lib/util/DependencySet.java:67 DependencySet.getOutputFileName
    pub fn get_output_file_name(&self) -> Option<&str> {
        self.output_file_name.as_deref()
    }

    // Java: com/google/devtools/build/lib/util/DependencySet.java:71 DependencySet.setOutputFileName
    pub fn set_output_file_name(&mut self, output_file_name: &str) {
        self.output_file_name = Some(output_file_name.to_string());
    }

    /// Gets the dependencies in `Path` form.
    // Java: com/google/devtools/build/lib/util/DependencySet.java:84 DependencySet.getDependencies
    pub fn get_dependencies(&self) -> &[Path] {
        &self.dependencies
    }

    /// Adds a given collection of dependencies in Path form to this DependencySet instance.
    // Java: com/google/devtools/build/lib/util/DependencySet.java:93 DependencySet.addDependencies
    pub fn add_dependencies(&mut self, deps: &[Path]) {
        for d in deps {
            blaze_util::check_argument!(d.starts_with(&self.root));
            self.dependencies.push(d.clone());
        }
    }

    /// Adds a given dependency to this DependencySet instance.
    // Java: com/google/devtools/build/lib/util/DependencySet.java:101 DependencySet.addDependency
    fn add_dependency(&mut self, dep: &str) {
        // PORT: translatePath is the identity off Windows.
        let dep_path = self.root.get_relative_str(dep);
        self.dependencies.push(dep_path);
    }

    /// Reads a dotd file into this DependencySet instance.
    // Java: com/google/devtools/build/lib/util/DependencySet.java:115 DependencySet.read
    pub fn read(&mut self, dotd_file: &Path) -> io::Result<&mut Self> {
        let mut content = Vec::new();
        dotd_file.get_input_stream()?.read_to_end(&mut content)?;
        match self.process(content) {
            Ok(_) => Ok(self),
            Err(e) => Err(io::Error::other(format!(
                "Error processing {}: {}",
                dotd_file.get_path_string(),
                e
            ))),
        }
    }

    /// Parses a .d file.
    // Java: com/google/devtools/build/lib/util/DependencySet.java:131 DependencySet.process
    pub fn process(&mut self, mut content: Vec<u8>) -> io::Result<&mut Self> {
        let n = content.len();
        if n > 0 && content[n - 1] != b'\n' {
            return Err(io::Error::other("File does not end in a newline"));
            // From now on, we can safely peek ahead one character when not at a newline.
        }
        // Our write position in content[]; we use the prefix as working space to build strings.
        let mut w = 0usize;
        // Have we seen a leading "mumble.o:" on this line yet?  If not, we ignore any
        // dependencies we parse.  This is bug-for-bug compatibility with our MSVC wrapper, which
        // generates invalid .d files :(
        let mut saw_target = false;
        let mut r = 0usize;
        while r < n {
            let c = content[r];
            r += 1;
            match c {
                b' ' => {
                    // If we haven't yet seen the colon delimiting the target name, keep scanning.
                    // We do this to cope with "foo.o : \" which is valid Makefile syntax produced
                    // by the cuda compiler.
                    if saw_target && w > 0 {
                        let dep = latin1(&content[..w]);
                        self.add_dependency(&dep);
                        w = 0;
                    }
                }
                b'\r' => {
                    // Ignore, should be followed by a \n.
                }
                b'\n' => {
                    // This closes a filename.
                    // (Arguably if !sawTarget && w > 0 we should report an error, as that
                    // suggests the .d file is malformed.)
                    if saw_target && w > 0 {
                        let dep = latin1(&content[..w]);
                        self.add_dependency(&dep);
                    }
                    w = 0;
                    saw_target = false; // reset for new line
                }
                b':' => {
                    // Normally this indicates the target name, but it might be part of a filename
                    // on Windows.  Peek ahead at the next character.
                    match content[r] {
                        b' ' | b'\n' | b'\r' => {
                            if w > 0 {
                                self.output_file_name = Some(latin1(&content[..w]));
                                w = 0;
                                saw_target = true;
                            }
                        }
                        _ => {
                            content[w] = c; // copy a colon to filename
                            w += 1;
                        }
                    }
                }
                b'\\' => {
                    // Peek ahead at the next character.
                    match content[r] {
                        // Backslashes are taken literally except when followed by whitespace.
                        // See the Windows tests for some of the nonsense we have to tolerate.
                        b' ' => {
                            content[w] = b' '; // copy a space to the filename
                            w += 1;
                            r += 1; // skip over the space
                        }
                        b'\n' => {
                            r += 1; // skip over the newline
                        }
                        b'\r' => {
                            // One backslash can escape \r\n, so peek one more character.
                            r += 1;
                            if content[r] == b'\n' {
                                r += 1;
                            }
                        }
                        _ => {
                            content[w] = c; // copy a backlash to the filename
                            w += 1;
                        }
                    }
                }
                b'$' if content[r] == b'$' => {
                    content[w] = b'$';
                    w += 1;
                    r += 1;
                }
                // I don't think a lone '$' can ever happen, but fall through nevertheless...
                _ => {
                    content[w] = c;
                    w += 1;
                }
            }
        }
        Ok(self)
    }

    /// Writes this DependencySet object for a specified output file under the root dir, and with
    /// a given suffix.
    // Java: com/google/devtools/build/lib/util/DependencySet.java:231 DependencySet.write
    pub fn write(&self, out_file: &Path, suffix: &str) -> io::Result<()> {
        let dotd_file = out_file.get_relative(&replace_extension(out_file.as_fragment(), suffix));

        let mut out = dotd_file.get_output_stream()?;
        let mut text = format!("{}: ", out_file.relative_to(&self.root).get_path_string());
        for d in &self.dependencies {
            text.push_str(" \\\n  ");
            text.push_str(d.get_path_string()); // should already be root relative
        }
        text.push('\n');
        out.write_all(text.as_bytes())?;
        out.flush()
    }
}

/// Java: `new String(bytes, ISO_8859_1)`.
// PORT: StringEncoding identity: the bytes are taken as UTF-8 (Bazel's internal form); invalid
// UTF-8 is replaced.
fn latin1(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Java: `FileSystemUtils.replaceExtension(PathFragment, String)`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:173 FileSystemUtils.replaceExtension
fn replace_extension(path: &PathFragment, new_extension: &str) -> PathFragment {
    path.replace_name(&format!(
        "{}{}",
        remove_extension(path.get_base_name()),
        new_extension
    ))
    .unwrap_or_else(|| panic!("NullPointerException"))
}

// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:128 FileSystemUtils.removeExtension
fn remove_extension(filename: &str) -> &str {
    let Some(last_dot_index) = filename.rfind('.') else {
        return filename;
    };
    if let Some(last_slash_index) = filename.rfind('/')
        && last_slash_index > last_dot_index
    {
        return filename;
    }
    &filename[..last_dot_index]
}
