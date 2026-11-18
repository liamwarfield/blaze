//! Kinds of Starlark files and their parse options.
//!
//! Java: net/starlark/java/syntax/FileOptions.java (options), and the call sites that choose them:
//! com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:199 (.bzl, .scl, prelude),
//! com/google/devtools/build/lib/skyframe/PackageFunction.java:1355 (BUILD),
//! bazel/bzlmod/CompiledModuleFile.java:80 (MODULE.bazel), skyframe/RepoFileFunction.java:131
//! (REPO.bazel), bazel/bzlmod/VendorFileFunction.java:176 (VENDOR.bazel).

use starlark::syntax::Dialect;
use starlark::syntax::DialectTypes;

/// What a Starlark source file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// BUILD / BUILD.bazel.
    Build,
    /// A .bzl file loaded from BUILD, .bzl, MODULE extensions or repository rules.
    Bzl,
    /// A .scl file (ASCII-only string literals).
    Scl,
    /// The BUILD prelude (`--prelude_label`); loads bind globally.
    BuildPrelude,
    /// `@_builtins` .bzl files (builtins injection).
    BuiltinsBzl,
    /// MODULE.bazel (and files it `include()`s).
    ModuleFile,
    /// REPO.bazel.
    RepoFile,
    /// VENDOR.bazel.
    VendorFile,
}

/// Java: net/starlark/java/syntax/FileOptions.java FileOptions (the flags Blaze can honour).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileOptions {
    pub allow_load_private_symbols: bool,
    pub allow_toplevel_rebinding: bool,
    pub load_binds_globally: bool,
    pub require_load_statements_first: bool,
    pub string_literals_are_ascii_only: bool,
    pub allow_type_syntax: bool,
}

impl FileOptions {
    /// Java: FileOptions.java DEFAULT (builder defaults).
    pub const DEFAULT: FileOptions = FileOptions {
        allow_load_private_symbols: false,
        allow_toplevel_rebinding: false,
        load_binds_globally: false,
        require_load_statements_first: true,
        string_literals_are_ascii_only: false,
        allow_type_syntax: false,
    };
}

impl FileKind {
    /// Classifies a file by its base name. `.bzl` vs `.scl` by extension; anything else `None`.
    pub fn from_file_name(name: &str) -> Option<FileKind> {
        match name {
            "BUILD" | "BUILD.bazel" => Some(FileKind::Build),
            "MODULE.bazel" => Some(FileKind::ModuleFile),
            "REPO.bazel" => Some(FileKind::RepoFile),
            "VENDOR.bazel" => Some(FileKind::VendorFile),
            _ if name.ends_with(".bzl") => Some(FileKind::Bzl),
            _ if name.ends_with(".scl") => Some(FileKind::Scl),
            // `build_file = "foo.BUILD"` in repository rules.
            _ if name.ends_with(".BUILD") => Some(FileKind::Build),
            _ => None,
        }
    }

    /// The Java FileOptions for this kind.
    pub fn file_options(self) -> FileOptions {
        match self {
            // Java: PackageFactory BUILD options (PackageFunction.java:1355).
            FileKind::Build => FileOptions {
                require_load_statements_first: false,
                load_binds_globally: true,
                allow_toplevel_rebinding: true,
                ..FileOptions::DEFAULT
            },
            // Java: BzlCompileFunction.java:199 (loadBindsGlobally only for the prelude; .scl
            // literals ASCII-only). Type syntax depends on the flags and the file's label
            // (BzlCompileFunction.shouldUseTypeSyntax): BZL_COMPILE sets `allow_type_syntax`
            // and compiles with `eval::compile_with_options`.
            FileKind::Bzl | FileKind::BuiltinsBzl => FileOptions::DEFAULT,
            FileKind::Scl => FileOptions {
                string_literals_are_ascii_only: true,
                ..FileOptions::DEFAULT
            },
            FileKind::BuildPrelude => FileOptions {
                load_binds_globally: true,
                ..FileOptions::DEFAULT
            },
            FileKind::ModuleFile | FileKind::RepoFile | FileKind::VendorFile => {
                FileOptions::DEFAULT
            }
        }
    }

    /// The starlark-rust parser dialect for this kind. Bazel-specific rules starlark-rust has no
    /// switch for are AST checks in `crate::syntax`.
    pub fn dialect(self) -> Dialect {
        self.dialect_with_options(self.file_options())
    }

    /// The starlark-rust parser dialect for this kind with explicit options (the options that
    /// depend on flags, such as `allow_type_syntax`).
    ///
    /// `allow_type_syntax` maps to `DialectTypes::ParseOnly`, which the vendored crate's patch
    /// 0020 makes Bazel 9.3.2's mode without type checking: annotations are arbitrary
    /// expressions that are neither resolved nor evaluated (Java FileOptions
    /// `allowTypeSyntax && !resolveTypeSyntax && tolerateInvalidTypeExpressions`).
    pub fn dialect_with_options(self, opts: FileOptions) -> Dialect {
        Dialect {
            enable_def: true,
            enable_lambda: true,
            enable_load: true,
            // Bazel accepts `def f(a, *, b)`; it has no `/` marker.
            enable_keyword_only_arguments: true,
            enable_positional_only_arguments: false,
            enable_types: if opts.allow_type_syntax {
                DialectTypes::ParseOnly
            } else {
                DialectTypes::Disable
            },
            // .bzl load bindings are file-local, never re-exported. BUILD files bind loads
            // globally, but nothing loads a BUILD file.
            enable_load_reexport: false,
            // Bazel's Resolver rejects top-level `if`/`for` in every file. Files with a
            // DotBazelFileSyntaxChecker parse them so the checker can give its own message.
            enable_top_level_stmt: self.has_dot_bazel_checker(),
            enable_f_strings: false,
            ..Dialect::Standard
        }
    }

    /// Whether Bazel runs `DotBazelFileSyntaxChecker` on this kind (see `crate::syntax`).
    pub fn has_dot_bazel_checker(self) -> bool {
        matches!(
            self,
            FileKind::Build | FileKind::ModuleFile | FileKind::RepoFile | FileKind::VendorFile
        )
    }

    /// Java `DotBazelFileSyntaxChecker` arguments: (where, canLoadBzl, allowLiteralStarStarArgs).
    pub fn dot_bazel_checker_config(self) -> Option<(&'static str, bool, bool)> {
        match self {
            FileKind::Build => Some(("BUILD files", true, false)),
            FileKind::ModuleFile => Some(("MODULE.bazel files", false, true)),
            FileKind::RepoFile => Some(("REPO.bazel files", false, false)),
            FileKind::VendorFile => Some(("VENDOR.bazel files", false, false)),
            _ => None,
        }
    }
}
