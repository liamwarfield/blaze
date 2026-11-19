//! BZL_COMPILE: reading, parsing and statically checking a .bzl (or .scl, prelude, builtins)
//! file.
//!
//! Java: com/google/devtools/build/lib/skyframe/{BzlCompileFunction,BzlCompileValue}.java. The
//! compiled program is blaze_starlark's [`CompiledFile`] (a starlark-rust `AstModule` plus
//! Bazel's static checks); name resolution happens when BZL_LOAD executes it.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::{Event, ExtendedEventHandler, Location};
use blaze_skyframe::{
    ComputeError, Environment, Key, SkyFunctionException, SkyFunctionName, Transience,
};
use blaze_starlark::eval::{CompiledFile, compile_with_options};
use blaze_starlark::file_kind::FileOptions;
use blaze_starlark::{FileKind, StarlarkSemantics, SyntaxError};
use blaze_util::unported;
use blaze_vfs::{DigestHashFunction, Root, RootedPath};

use crate::file::FileKey;
use crate::io::IOException;
use crate::precomputed::STARLARK_SEMANTICS;
use crate::repos::is_builtins_repo;
use crate::sky_functions;
use crate::starlark_util;

/// Types of bzl files we may encounter.
// Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:128 BzlCompileValue.Kind
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BzlCompileKind {
    /// A regular .bzl file loaded on behalf of a BUILD or WORKSPACE file.
    // The reason we can share a single key type for these environments is that they have the
    // same symbol names, even though their symbol definitions (particularly for the "native"
    // object) differ. (See also #11954, which aims to make even the symbol definitions the
    // same.)
    Normal,
    /// A .bzl file loaded during evaluation of the `@_builtins` pseudo-repository.
    Builtins,
    /// The prelude file, whose declarations are implicitly loaded by all BUILD files.
    Prelude,
    /// A virtual empty file that does not correspond to a lookup in the filesystem. This is used
    /// for the default prelude contents, when the real prelude's contents should be ignored (in
    /// particular, when its package is missing).
    EmptyPrelude,
}

/// SkyKey for retrieving a compiled .bzl program. `Debug` prints `BZL_COMPILE:[root]label`.
// Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:151 BzlCompileValue.Key
#[derive(Clone)]
pub struct BzlCompileKey {
    /// The root in which the .bzl file is to be found. `None` for EMPTY_PRELUDE.
    root: Option<Root>,
    /// The label of the .bzl to be retrieved. `None` for EMPTY_PRELUDE.
    label: Option<Label>,
    kind: BzlCompileKind,
}

impl BzlCompileKey {
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:162 BzlCompileValue.Key.<init>
    fn new(root: Option<Root>, label: Option<Label>, kind: BzlCompileKind) -> Self {
        if kind != BzlCompileKind::EmptyPrelude {
            assert!(root.is_some(), "NullPointerException");
            assert!(label.is_some(), "NullPointerException");
        }
        BzlCompileKey { root, label, kind }
    }

    /// Constructs a key for loading a regular (non-prelude) .bzl.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:250 BzlCompileValue.key
    pub fn key(root: Root, label: Label) -> Self {
        Self::new(Some(root), Some(label), BzlCompileKind::Normal)
    }

    /// Constructs a key for loading a builtins .bzl.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:255 BzlCompileValue.keyForBuiltins
    pub fn key_for_builtins(root: Root, label: Label) -> Self {
        Self::new(Some(root), Some(label), BzlCompileKind::Builtins)
    }

    /// Constructs a key for loading the prelude .bzl.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:261 BzlCompileValue.keyForBuildPrelude
    pub fn key_for_build_prelude(root: Root, label: Label) -> Self {
        Self::new(Some(root), Some(label), BzlCompileKind::Prelude)
    }

    /// The unique SkyKey of EMPTY_PRELUDE kind.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:267 BzlCompileValue.EMPTY_PRELUDE_KEY
    pub fn empty_prelude_key() -> Self {
        Self::new(None, None, BzlCompileKind::EmptyPrelude)
    }

    /// Returns whether this key is for a `@_builtins` .bzl file.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:182 BzlCompileValue.Key.isBuiltins
    pub fn is_builtins(&self) -> bool {
        self.kind == BzlCompileKind::Builtins
    }

    /// Returns true if the requested file follows the .scl dialect.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:189 BzlCompileValue.Key.isSclDialect
    pub fn is_scl_dialect(&self) -> bool {
        self.label
            .as_ref()
            .is_some_and(|l| l.get_name().ends_with(".scl"))
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:193 BzlCompileValue.Key.isBuildPrelude
    pub fn is_build_prelude(&self) -> bool {
        self.kind == BzlCompileKind::Prelude || self.kind == BzlCompileKind::EmptyPrelude
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:197 BzlCompileValue.Key.getLabel
    pub fn get_label(&self) -> Option<&Label> {
        self.label.as_ref()
    }

    pub fn get_root(&self) -> Option<&Root> {
        self.root.as_ref()
    }

    pub fn kind(&self) -> BzlCompileKind {
        self.kind
    }

    /// Returns the dep to register for the underlying .bzl file (if any).
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:203 BzlCompileValue.Key.getBzlFileKey
    pub fn get_bzl_file_key(&self) -> Option<FileKey> {
        // `None` implies kind == EMPTY_PRELUDE.
        let label = self.label.as_ref()?;
        Some(FileKey::key(RootedPath::to_rooted_path(
            self.root.clone().expect("root is set with label"),
            &label.to_path_fragment(),
        )))
    }
}

// Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:217 BzlCompileValue.Key.equals
impl PartialEq for BzlCompileKey {
    fn eq(&self, other: &Self) -> bool {
        // Compare roots last since that's the more expensive step.
        self.kind == other.kind && self.label == other.label && self.root == other.root
    }
}

impl Eq for BzlCompileKey {}

// Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:212 BzlCompileValue.Key.hashCode
impl Hash for BzlCompileKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.root.hash(state);
        self.label.hash(state);
        self.kind.hash(state);
    }
}

// Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:236 BzlCompileValue.Key.toString
impl fmt::Debug for BzlCompileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:[", Self::FUNCTION)?;
        match &self.root {
            Some(r) => write!(f, "{}", r)?,
            None => f.write_str("null")?,
        }
        f.write_str("]")?;
        match &self.label {
            Some(l) => write!(f, "{}", l),
            None => f.write_str("null"),
        }
    }
}

impl Key for BzlCompileKey {
    type Value = BzlCompileValue;
    const FUNCTION: SkyFunctionName = sky_functions::BZL_COMPILE;
}

/// The result of BzlCompileFunction: `Success` when the file is compiled successfully, `Failure`
/// when the file does not exist, or had parser/resolver errors.
// Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:50 BzlCompileValue
#[derive(Clone, Debug)]
pub enum BzlCompileValue {
    /// The file compiled successfully.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:62 BzlCompileValue.Success
    Success {
        program: Arc<CompiledFile>,
        digest: Vec<u8>,
        /// Java `Program.getFilename()`: the name the file was compiled with.
        filename: String,
    },
    /// The file isn't found or has errors; `error_msg` is the reason.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:96 BzlCompileValue.Failure
    Failure {
        error_msg: String,
        /// A file with static errors, whose error list needs the predeclared names (Java's
        /// BzlCompileFunction has them; Blaze's BzlLoad does): BzlLoad reports Java's errors
        /// (`blaze_starlark::eval::compile_and_resolve`) with this.
        deferred: Option<Arc<DeferredCompile>>,
    },
}

/// The input of a compile whose errors BzlLoad reports (see `BzlCompileValue::Failure`).
#[derive(Debug)]
pub struct DeferredCompile {
    pub key: BzlCompileKey,
    pub kind: FileKind,
    pub options: FileOptions,
    pub input_name: String,
    pub source: String,
}

impl BzlCompileValue {
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:52 BzlCompileValue.lookupSuccessful
    pub fn lookup_successful(&self) -> bool {
        matches!(self, BzlCompileValue::Success { .. })
    }

    /// On success, the compiled program.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:54 BzlCompileValue.getProgram
    pub fn get_program(&self) -> &Arc<CompiledFile> {
        match self {
            BzlCompileValue::Success { program, .. } => program,
            BzlCompileValue::Failure { .. } => panic!(
                "IllegalStateException: attempted to retrieve .bzl program from an unsuccessful \
                 lookup"
            ),
        }
    }

    /// On success, the digest of the file's bytes. It feeds the transitive digests of .bzl
    /// modules (and so `$rule_implementation_hash`).
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:56 BzlCompileValue.getDigest
    pub fn get_digest(&self) -> &[u8] {
        match self {
            BzlCompileValue::Success { digest, .. } => digest,
            BzlCompileValue::Failure { .. } => panic!(
                "IllegalStateException: attempted to retrieve digest for unsuccessful lookup"
            ),
        }
    }

    /// On failure, the reason.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:58 BzlCompileValue.getError
    pub fn get_error(&self) -> &str {
        match self {
            BzlCompileValue::Failure { error_msg, .. } => error_msg,
            BzlCompileValue::Success { .. } => panic!(
                "IllegalStateException: attempted to retrieve unsuccessful lookup reason for \
                 successful lookup"
            ),
        }
    }

    /// Constructs a value from a failure before parsing a file.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:118 BzlCompileValue.noFile
    pub fn no_file(error_msg: String) -> BzlCompileValue {
        BzlCompileValue::Failure {
            error_msg,
            deferred: None,
        }
    }

    /// The compile BzlLoad reports the errors of, if any.
    pub fn deferred(&self) -> Option<&Arc<DeferredCompile>> {
        match self {
            BzlCompileValue::Failure { deferred, .. } => deferred.as_ref(),
            BzlCompileValue::Success { .. } => None,
        }
    }

    /// Constructs a value from a compiled .bzl program.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileValue.java:123 BzlCompileValue.withProgram
    pub fn with_program(
        program: CompiledFile,
        digest: Vec<u8>,
        filename: String,
    ) -> BzlCompileValue {
        BzlCompileValue::Success {
            program: Arc::new(program),
            digest,
            filename,
        }
    }

    /// On success, the file name the program was compiled with (Java `Program.getFilename()`).
    pub fn get_filename(&self) -> &str {
        match self {
            BzlCompileValue::Success { filename, .. } => filename,
            BzlCompileValue::Failure { .. } => panic!(
                "IllegalStateException: attempted to retrieve .bzl program from an unsuccessful \
                 lookup"
            ),
        }
    }
}

/// A copy of a compiled program, for executing it (execution consumes the program).
pub fn clone_compiled_file(program: &CompiledFile) -> CompiledFile {
    CompiledFile {
        kind: program.kind,
        ast: program.ast.clone(),
        loads: program.loads.clone(),
        build_facts: program.build_facts.clone(),
        // The copy is for execution; Java's syntax tree stays with (and is taken by the resolve
        // of) the original.
        java_tree: std::sync::Mutex::new(None),
    }
}

/// Reading a .bzl file failed. `Display` is the cause's message (Java `super(cause.getMessage(),
/// cause)`).
// Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:292 BzlCompileFunction.FailedIOException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct FailedIOException {
    message: String,
    cause: IOException,
    transience: Transience,
}

impl FailedIOException {
    pub fn new(cause: IOException, transience: Transience) -> Self {
        FailedIOException {
            message: cause.get_message().to_owned(),
            cause,
            transience,
        }
    }

    pub fn get_cause(&self) -> &IOException {
        &self.cause
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:301 BzlCompileFunction.FailedIOException.getTransience
    pub fn get_transience(&self) -> Transience {
        self.transience
    }
}

/// A Skyframe function that compiles the .bzl file denoted by a Label.
///
/// Given a [`Label`] referencing a Starlark file, BzlCompileFunction loads, parses, resolves, and
/// compiles it. The Label must be absolute, and must not reference the special `external`
/// package. If the file (or the package containing it) doesn't exist, the function doesn't fail,
/// but instead returns a specific `NO_FILE` [`BzlCompileValue`].
// Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:53 BzlCompileFunction
pub struct BzlCompileFunction {
    hash_function: DigestHashFunction,
}

impl BzlCompileFunction {
    /// `hash_function` is the file system's digest function (Java
    /// `getDigestFunction().getHashFunction()`).
    ///
    /// PORT: Java also takes the `BazelStarlarkEnvironment` for the predeclared names used in
    /// name resolution; starlark-rust resolves names when BZL_LOAD executes the module, so they
    /// are not needed here.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:58 BzlCompileFunction.<init>
    pub fn new(hash_function: DigestHashFunction) -> Self {
        BzlCompileFunction { hash_function }
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:65 BzlCompileFunction.compute
    pub fn compute(
        &self,
        key: &BzlCompileKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<BzlCompileValue>, ComputeError> {
        match Self::compute_inline(key, env, self.hash_function)? {
            Ok(v) => Ok(v),
            // Java: FunctionException(FailedIOException), with the exception's transience.
            Err(e) => {
                let transience = e.get_transience();
                Err(SkyFunctionException::new(e, transience).into())
            }
        }
    }

    /// Compiles `key` inline (BZL_LOAD's inlining mode uses this too). The inner `Err` is Java's
    /// checked `FailedIOException`.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:76 BzlCompileFunction.computeInline
    pub fn compute_inline(
        key: &BzlCompileKey,
        env: &mut Environment<'_>,
        hash_function: DigestHashFunction,
    ) -> Result<Result<Option<BzlCompileValue>, FailedIOException>, ComputeError> {
        let bytes: Vec<u8>;
        let mut digest: Option<Vec<u8>>;
        let input_name: String;
        if key.kind == BzlCompileKind::EmptyPrelude {
            // Default prelude is empty.
            bytes = Vec::new();
            digest = None;
            input_name = "<default prelude>".to_owned();
        } else {
            let label = key.label.as_ref().expect("label");
            // Obtain the file.
            let rooted_path = RootedPath::to_rooted_path(
                key.root.clone().expect("root"),
                &label.to_path_fragment(),
            );
            let file_sky_key = FileKey::key(rooted_path.clone());
            let file_value = match env.get_value_or_throw::<FileKey, IOException>(file_sky_key)? {
                Ok(v) => v,
                Err(e) => return Ok(Err(FailedIOException::new(e, Transience::Persistent))),
            };
            let Some(file_value) = file_value else {
                return Ok(Ok(None));
            };

            if file_value.exists() {
                if !file_value.is_file() {
                    return Ok(Ok(Some(if file_value.is_directory() {
                        BzlCompileValue::no_file(format!("cannot load '{}': is a directory", label))
                    } else {
                        BzlCompileValue::no_file(format!(
                            "cannot load '{}': not a regular file (dangling link?)",
                            label
                        ))
                    })));
                }

                // Read the file.
                // PORT: Java reads special files with readContent and others with
                // readWithKnownFileSize; both read the whole file.
                let path = rooted_path.as_path();
                let read = (|| -> std::io::Result<Vec<u8>> {
                    let mut input = path.get_input_stream()?;
                    let mut contents = Vec::new();
                    std::io::Read::read_to_end(&mut input, &mut contents)?;
                    Ok(contents)
                })();
                bytes = match read {
                    Ok(b) => b,
                    Err(e) => {
                        return Ok(Err(FailedIOException::new(
                            IOException::from_io(&e),
                            Transience::Transient,
                        )));
                    }
                };
                digest = file_value.get_digest().map(|d| d.to_vec()); // may be null
                input_name = path.to_string();
            } else if key.kind == BzlCompileKind::Prelude {
                // A non-existent prelude is fine.
                bytes = Vec::new();
                digest = None;
                input_name = "<default prelude>".to_owned();
            } else {
                return Ok(Ok(Some(BzlCompileValue::no_file(format!(
                    "cannot load '{}': no such file",
                    label
                )))));
            }
        }

        // Compute digest if we didn't already get it from a fileValue.
        if digest.is_none() {
            digest = Some(
                hash_function
                    .hash_reader(&mut bytes.as_slice())
                    .expect("hashing bytes in memory cannot fail"),
            );
        }
        let digest = digest.expect("set above");

        let Some(semantics) = STARLARK_SEMANTICS.get(env)? else {
            return Ok(Ok(None));
        };

        // PORT: Java picks the predeclared environment here (scl toplevels, builtins env, or the
        // uninjected BUILD-bzl env with AutoloadSymbols, requesting the AUTOLOAD_SYMBOLS
        // precomputed value). starlark-rust resolves names at execution, so BZL_LOAD supplies the
        // environment instead.

        // We have all deps. Parse, resolve, and return.
        let source = match starlark_util::create_parser_input(
            bytes,
            &input_name,
            semantics.incompatible_enforce_starlark_utf8,
            env.listener(),
        ) {
            Ok(s) => s,
            // createParserInput() reports its own error message
            Err(_) => {
                return Ok(Ok(Some(BzlCompileValue::no_file(format!(
                    "compilation of '{}' failed",
                    input_name
                )))));
            }
        };

        let use_type_syntax = Self::should_use_type_syntax(&semantics, key);
        // Type checking requires the syntax flag to be enabled, though technically it shouldn't
        // matter to semantics if unannotated code is considered untyped.
        let do_type_checking = use_type_syntax && semantics.experimental_starlark_type_checking;
        if do_type_checking {
            // Java: `.resolveTypeSyntax(true).tolerateInvalidTypeExpressions(false)` and the
            // static type checker (Program.java:108).
            unported!("BzlCompileFunction.compute: --experimental_starlark_type_checking");
        }

        // PORT: Java's FileOptions (loadBindsGlobally for the prelude, ASCII-only string literals
        // for .scl) are blaze_starlark's FileKind options; allowTypeSyntax depends on the flags
        // and is set here.
        let file_kind = if key.is_scl_dialect() {
            FileKind::Scl
        } else if key.is_build_prelude() {
            FileKind::BuildPrelude
        } else if key.kind == BzlCompileKind::Builtins {
            FileKind::BuiltinsBzl
        } else {
            FileKind::Bzl
        };
        let options = FileOptions {
            allow_type_syntax: use_type_syntax,
            ..file_kind.file_options()
        };

        match compile_with_options(file_kind, options, &input_name.clone(), source.clone()) {
            Ok(program) => Ok(Ok(Some(BzlCompileValue::with_program(
                program, digest, input_name,
            )))),
            Err(e) => {
                let _: &[SyntaxError] = match &e.kind {
                    blaze_starlark::error::EvalErrorKind::Syntax(errors) => errors,
                    _ => unreachable!("compile only fails with syntax errors"),
                };
                // PORT: Java's error list (Program.compileFile: the parse errors and every
                // resolver error) needs the predeclared names, which Blaze's BzlLoad picks; it
                // reports the errors (addSyntaxErrorsToListener) from `deferred`.
                let label = key.label.as_ref().expect("the empty prelude compiles");
                Ok(Ok(Some(BzlCompileValue::Failure {
                    error_msg: format!(
                        "compilation of module '{}'{} failed",
                        label.to_path_fragment(),
                        if is_builtins_repo(label.get_repository()) {
                            " (internal)"
                        } else {
                            ""
                        }
                    ),
                    deferred: Some(Arc::new(DeferredCompile {
                        key: key.clone(),
                        kind: file_kind,
                        options,
                        input_name,
                        source,
                    })),
                })))
            }
        }
    }

    /// Whether the file should permit type syntax (annotations, etc.) based on flags and the type
    /// of file.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:249 BzlCompileFunction.shouldUseTypeSyntax
    fn should_use_type_syntax(semantics: &StarlarkSemantics, key: &BzlCompileKey) -> bool {
        let type_syntax_flag = semantics.experimental_starlark_type_syntax;
        let allowlist = &semantics.experimental_starlark_types_allowed_paths;

        let ok_filetype =
            // annotations in prelude not allowed (it has null key.label)
            !key.is_build_prelude()
            // annotations in SCL now allowed (not yet compatible with Go-Starlark interpreter)
            && !key.is_scl_dialect();

        if type_syntax_flag && ok_filetype {
            let label = key.label.as_ref().expect("non-prelude keys have a label");
            if allowlist.is_empty()
                || allowlist
                    .iter()
                    .any(|s| label.get_canonical_form().starts_with(s.as_str()))
            {
                return true;
            }
        }
        false
    }

    /// Replays the syntax errors from a file onto an event handler, adding more context if
    /// necessary.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:268 BzlCompileFunction.addSyntaxErrorsToListener
    pub(crate) fn add_syntax_errors_to_listener(
        handler: &dyn ExtendedEventHandler,
        errors: &[SyntaxError],
        key: &BzlCompileKey,
    ) {
        starlark_util::replay_syntax_errors_on(handler, errors);
        // If type annotations are disallowed, it could either be because the required flags
        // aren't enabled or because the filetype disallows it.
        for err in errors {
            // PORT: starlark-rust's text for Java's ": type annotations are disallowed".
            if err.message.contains(": type annotations are disallowed")
                || err
                    .message
                    .contains("type annotations are not allowed in this dialect")
            {
                let file_loc = Location::new(err.location.file.clone(), 0, 0);
                let explanation = if key.is_scl_dialect() {
                    "Type annotations are not permitted in .scl files."
                } else {
                    "Type annotations syntax can be enabled with \
                     --experimental_starlark_type_syntax and/or \
                     --experimental_starlark_types_allowed_paths."
                };
                handler.handle(&Event::error_at(Some(file_loc), explanation));
            }
        }
    }
}
