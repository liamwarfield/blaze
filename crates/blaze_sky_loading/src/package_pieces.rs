//! Lazy symbolic macro expansion (`--experimental_lazy_macro_expansion_packages`): the package
//! piece keys and values, and the functions computing them.
//!
//! Java: com/google/devtools/build/lib/skyframe/{PackagePieceValue, PackageDeclarationsValue,
//! PackageDeclarationsFunction, MacroInstanceValue, MacroInstanceFunction, EvalMacroFunction,
//! NonFinalizerPackagePiecesValue, NonFinalizerPackagePiecesFunction, PackagePieces}.java and
//! `PackageFunction.computePackageFromPackagePieces`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_events::EventHandler;
use blaze_packages::build::package_factory::PackageFactory;
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::exceptions::PackageLoadingCode;
use blaze_packages::macros::MacroClass;
use blaze_packages::macros::MacroInstance;
use blaze_packages::package::NameConflictException;
use blaze_packages::package::NoSuchPackagePieceException;
use blaze_packages::package::Package;
use blaze_packages::package::PackageDeclarations;
use blaze_packages::package::PackageMetadata;
use blaze_packages::package::PackagePiece;
use blaze_packages::package::PackagePieceIdentifier;
use blaze_packages::package::PackagePieceIdentifierForBuildFile;
use blaze_packages::package::PackagePieceIdentifierForMacro;
use blaze_packages::package::TargetDefinitionContext;
use blaze_packages::package::TargetRecorder;
use blaze_packages::target::Rule;
use blaze_packages::target::Target;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_skyframe::Key;
use blaze_skyframe::SkyError;
use blaze_skyframe::SkyFunctionException;
use blaze_skyframe::SkyFunctionName;
use blaze_skyframe::SkyKey;
use blaze_skyframe::Transience;
use blaze_skyframe::{ComputeError, FunctionHermeticity};
use blaze_starlark::EvalError;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::error::StackEntry;
use indexmap::IndexMap;

use crate::sky_functions;

// ---------------------------------------------------------------------------------------------
// Keys and values.
// ---------------------------------------------------------------------------------------------

/// The PACKAGE key of a BUILD file piece (Java uses `PackagePieceIdentifier.ForBuildFile` itself
/// as the key; `Debug` prints its `toString()`).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PackagePieceForBuildFileKey(pub PackagePieceIdentifierForBuildFile);

impl fmt::Debug for PackagePieceForBuildFileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl Key for PackagePieceForBuildFileKey {
    type Value = PackagePieceValueForBuildFile;
    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:65 PackagePieceIdentifier.ForBuildFile.functionName
    const FUNCTION: SkyFunctionName = sky_functions::PACKAGE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:56 SkyFunctions.PACKAGE (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Default `values_equal` (never equal): `PackagePieceValue.ForBuildFile` is a
    // `PackageoidValue`, a `NotComparableSkyValue`. Same function name as `PackageKey`, same
    // hermeticity (Java interns one `SkyFunctionName` per name).
    // Java: com/google/devtools/build/lib/skyframe/PackagePieceValue.java:57 PackagePieceValue.ForBuildFile
}

/// The EVAL_MACRO key (Java uses `PackagePieceIdentifier.ForMacro` itself as the key).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct EvalMacroKey(pub PackagePieceIdentifierForMacro);

impl fmt::Debug for EvalMacroKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl Key for EvalMacroKey {
    type Value = PackagePieceValueForMacro;
    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:131 PackagePieceIdentifier.ForMacro.functionName
    const FUNCTION: SkyFunctionName = sky_functions::EVAL_MACRO;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:62 SkyFunctions.EVAL_MACRO (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Default `values_equal` (never equal): `PackagePieceValue.ForMacro` is a `PackageoidValue`,
    // a `NotComparableSkyValue`.
    // Java: com/google/devtools/build/lib/skyframe/PackagePieceValue.java:83 PackagePieceValue.ForMacro
}

/// The key of a package piece, whichever function computes it.
fn piece_sky_key(id: &PackagePieceIdentifier) -> SkyKey {
    match id {
        PackagePieceIdentifier::ForBuildFile(id) => {
            SkyKey::new(PackagePieceForBuildFileKey(id.clone()))
        }
        PackagePieceIdentifier::ForMacro(id) => SkyKey::new(EvalMacroKey(id.clone())),
    }
}

/// Java `PackagePieceValue.ForBuildFile`.
// Java: com/google/devtools/build/lib/skyframe/PackagePieceValue.java:42 PackagePieceValue.ForBuildFile
pub struct PackagePieceValueForBuildFile {
    /// Java `getPackagePiece()` / `forBuildFile()` (and `getPackageoid()`).
    // Java: com/google/devtools/build/lib/skyframe/PackagePieceValue.java:69 PackagePieceValue.ForBuildFile.getPackagePiece
    pub for_build_file: Arc<PackagePiece>,
    pub starlark_semantics: Arc<StarlarkSemantics>,
    pub main_repository_mapping: RepositoryMapping,
}

impl fmt::Debug for PackagePieceValueForBuildFile {
    // Java: com/google/devtools/build/lib/skyframe/PackagePieceValue.java:63 PackagePieceValue.ForBuildFile.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<PackagePieceValue.ForBuildFile name={}>",
            self.for_build_file
                .get_identifier()
                .get_canonical_form_name()
        )
    }
}

/// Java `PackagePieceValue.ForMacro`.
// Java: com/google/devtools/build/lib/skyframe/PackagePieceValue.java:77 PackagePieceValue.ForMacro
pub struct PackagePieceValueForMacro {
    /// Java `getPackagePiece()` / `forMacro()` (and `getPackageoid()`).
    // Java: com/google/devtools/build/lib/skyframe/PackagePieceValue.java:89 PackagePieceValue.ForMacro.getPackagePiece
    // Java: com/google/devtools/build/lib/skyframe/PackagePieceValue.java:42 PackagePieceValue.getPackageoid
    pub for_macro: Arc<PackagePiece>,
}

impl fmt::Debug for PackagePieceValueForMacro {
    // Java: com/google/devtools/build/lib/skyframe/PackagePieceValue.java:91 PackagePieceValue.ForMacro.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<PackagePieceValue.ForMacro name={} defined_by={}>",
            self.for_macro.get_identifier().get_canonical_form_name(),
            self.for_macro.get_canonical_form_defined_by()
        )
    }
}

/// Java `PackageDeclarationsValue.Key`.
// Java: com/google/devtools/build/lib/skyframe/PackageDeclarationsValue.java:52 PackageDeclarationsValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PackageDeclarationsKey {
    pub package_id: PackageIdentifier,
}

impl fmt::Debug for PackageDeclarationsKey {
    // Java: the record's toString.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key[packageId={}]", self.package_id)
    }
}

impl Key for PackageDeclarationsKey {
    type Value = PackageDeclarationsValue;
    const FUNCTION: SkyFunctionName = sky_functions::PACKAGE_DECLARATIONS;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:58 SkyFunctions.PACKAGE_DECLARATIONS (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: a record of the metadata (a record), the declarations (`Declarations.equals`: package
    /// args, make env, transitive loads), the semantics and the main repository mapping. Its
    /// purpose is to change-prune between a macro's piece and the BUILD file piece.
    // Java: com/google/devtools/build/lib/skyframe/PackageDeclarationsValue.java:39 PackageDeclarationsValue
    fn values_equal(old: &PackageDeclarationsValue, new: &PackageDeclarationsValue) -> bool {
        *old.metadata == *new.metadata
            && (Arc::ptr_eq(&old.declarations, &new.declarations)
                || *old.declarations == *new.declarations)
            && *old.starlark_semantics == *new.starlark_semantics
            && old.main_repository_mapping == new.main_repository_mapping
    }
}

/// Java `PackageDeclarationsValue`: what the pieces of a package share, from its BUILD file
/// piece.
// Java: com/google/devtools/build/lib/skyframe/PackageDeclarationsValue.java:31 PackageDeclarationsValue
pub struct PackageDeclarationsValue {
    pub metadata: Arc<PackageMetadata>,
    pub declarations: Arc<PackageDeclarations>,
    pub starlark_semantics: Arc<StarlarkSemantics>,
    pub main_repository_mapping: RepositoryMapping,
}

/// Java `MacroInstanceValue.Key`.
// Java: com/google/devtools/build/lib/skyframe/MacroInstanceValue.java:41 MacroInstanceValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MacroInstanceKey {
    pub package_piece_id: PackagePieceIdentifier,
    pub macro_instance_name: String,
}

impl fmt::Debug for MacroInstanceKey {
    // Java: the record's toString.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Key[packagePieceId={:?}, macroInstanceName={}]",
            self.package_piece_id, self.macro_instance_name
        )
    }
}

impl Key for MacroInstanceKey {
    type Value = MacroInstanceValue;
    const FUNCTION: SkyFunctionName = sky_functions::MACRO_INSTANCE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:64 SkyFunctions.MACRO_INSTANCE (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Default `values_equal` (never equal). PORT: Java's record compares the `MacroInstance`
    // (`MacroInstance.equals`: attribute values, metadata, declarations, parent, call stack, macro
    // class by identity, ...); `MacroInstance` has no `PartialEq` yet (its attribute values hold
    // Starlark values compared with Java's `equals`). Never equal prunes less than Java, never
    // more.
    // Java: com/google/devtools/build/lib/skyframe/MacroInstanceValue.java:37 MacroInstanceValue
}

/// Java `MacroInstanceValue`.
// Java: com/google/devtools/build/lib/skyframe/MacroInstanceValue.java:30 MacroInstanceValue
pub struct MacroInstanceValue {
    pub macro_instance: Arc<MacroInstance>,
}

/// Java `NonFinalizerPackagePiecesValue.Key`.
// Java: com/google/devtools/build/lib/skyframe/NonFinalizerPackagePiecesValue.java:103 NonFinalizerPackagePiecesValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct NonFinalizerPackagePiecesKey {
    pub pkg_id: PackageIdentifier,
}

impl fmt::Debug for NonFinalizerPackagePiecesKey {
    // Java: the record's toString.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key[pkgId={}]", self.pkg_id)
    }
}

impl Key for NonFinalizerPackagePiecesKey {
    type Value = NonFinalizerPackagePiecesValue;
    const FUNCTION: SkyFunctionName = sky_functions::NON_FINALIZER_PACKAGE_PIECES;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:66 SkyFunctions.NON_FINALIZER_PACKAGE_PIECES (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Default `values_equal` (never equal). PORT: Java's record compares its components; the
    // pieces, targets and exception by identity (equal only when every piece node stayed the same),
    // the macro instances by `MacroInstance.equals` (not ported, see `MacroInstanceKey`). Never
    // equal prunes less than Java, never more.
    // Java: com/google/devtools/build/lib/skyframe/NonFinalizerPackagePiecesValue.java:45 NonFinalizerPackagePiecesValue
}

/// Java `NonFinalizerPackagePiecesValue`: the BUILD file piece and the pieces of all
/// non-finalizer macros, recursively.
// Java: com/google/devtools/build/lib/skyframe/NonFinalizerPackagePiecesValue.java:45 NonFinalizerPackagePiecesValue
pub struct NonFinalizerPackagePiecesValue {
    pub package_pieces: IndexMap<PackagePieceIdentifier, Arc<PackagePiece>>,
    pub error_keys: Vec<PackagePieceIdentifier>,
    /// Java `nameConflictBetweenPackagePiecesException`.
    pub name_conflict_between_package_pieces_exception: Option<EvalError>,
    pub targets: BTreeMap<String, Target>,
    pub macro_instances: BTreeMap<String, Arc<MacroInstance>>,
    pub starlark_semantics: Arc<StarlarkSemantics>,
    pub main_repository_mapping: RepositoryMapping,
}

impl NonFinalizerPackagePiecesValue {
    // Java: com/google/devtools/build/lib/skyframe/NonFinalizerPackagePiecesValue.java:94 NonFinalizerPackagePiecesValue.containsErrors
    pub fn contains_errors(&self) -> bool {
        !self.error_keys.is_empty()
            || self
                .name_conflict_between_package_pieces_exception
                .is_some()
    }
}

/// Java `PackagePieces`: a set of package pieces, the BUILD file piece first. Java's accessors
/// `getPackagePieces()` / `getErrorKeys()` (of the interface, `NonFinalizerPackagePiecesValue` and
/// `RecursiveExpander`) are the public fields.
// Java: com/google/devtools/build/lib/skyframe/NonFinalizerPackagePiecesValue.java:86 NonFinalizerPackagePiecesValue.getPackagePieces
// Java: com/google/devtools/build/lib/skyframe/NonFinalizerPackagePiecesValue.java:96 NonFinalizerPackagePiecesValue.getErrorKeys
// Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:248 EvalMacroFunction.RecursiveExpander.getPackagePieces
// Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:258 EvalMacroFunction.RecursiveExpander.getErrorKeys
// Java: com/google/devtools/build/lib/skyframe/PackagePieces.java:31 PackagePieces
#[derive(Clone, Default)]
pub struct PackagePieces {
    pub package_pieces: IndexMap<PackagePieceIdentifier, Arc<PackagePiece>>,
    pub error_keys: Vec<PackagePieceIdentifier>,
}

impl PackagePieces {
    // Java: com/google/devtools/build/lib/skyframe/PackagePieces.java:37 PackagePieces.getPackagePieceForBuildFile
    // Java: com/google/devtools/build/lib/skyframe/NonFinalizerPackagePiecesValue.java:91 NonFinalizerPackagePiecesValue.getPackagePieceForBuildFile
    // Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:253 EvalMacroFunction.RecursiveExpander.getPackagePieceForBuildFile
    pub fn get_package_piece_for_build_file(&self) -> &Arc<PackagePiece> {
        self.package_pieces
            .values()
            .next()
            .expect("the BUILD file piece")
    }

    // Java: com/google/devtools/build/lib/skyframe/PackagePieces.java:46 PackagePieces.getFirstPieceContainingErrors
    pub fn get_first_piece_containing_errors(&self) -> Option<&Arc<PackagePiece>> {
        let key = self.error_keys.first()?;
        let piece = &self.package_pieces[key];
        blaze_util::check_state!(piece.contains_errors());
        Some(piece)
    }

    /// Records the targets and macros of all pieces into `recorder`.
    // Java: com/google/devtools/build/lib/skyframe/PackagePieces.java:62 PackagePieces.recordTargetsAndMacros
    pub fn record_targets_and_macros_into_recorder(
        &self,
        recorder: &mut TargetRecorder,
    ) -> Result<(), EvalError> {
        for package_piece in self.package_pieces.values() {
            recorder
                .add_all_from_package_piece(package_piece, /* skip_build_file= */ false)
                .map_err(wrap_name_conflict_exception)?;
        }
        Ok(())
    }

    /// Records the targets and macros of all pieces into the package builder.
    // Java: com/google/devtools/build/lib/skyframe/PackagePieces.java:73 PackagePieces.recordTargetsAndMacros
    pub fn record_targets_and_macros(
        &self,
        pkg_builder: &mut TargetDefinitionContext,
    ) -> Result<(), EvalError> {
        for package_piece in self.package_pieces.values() {
            pkg_builder
                .add_all_from_package_piece(package_piece)
                .map_err(wrap_name_conflict_exception)?;
        }
        Ok(())
    }
}

// Java: com/google/devtools/build/lib/skyframe/PackagePieces.java:83 PackagePieces.wrapNameConflictException
fn wrap_name_conflict_exception(e: NameConflictException) -> EvalError {
    let callstack = match e.get_macro() {
        Some(m) => m.reconstruct_parent_call_stack(),
        None => reconstruct_call_stack(e.get_target().expect("a target or a macro")),
    };
    let mut ex = EvalError {
        kind: EvalErrorKind::Eval,
        message: e.get_message().to_owned(),
        callstack,
        include_stack: true,
    };
    ex.read_missing_source_lines();
    ex
}

// Java: com/google/devtools/build/lib/skyframe/PackagePieces.java:91 PackagePieces.reconstructCallStack
fn reconstruct_call_stack(target: &Target) -> Vec<StackEntry> {
    if let Some(rule) = target.get_associated_rule() {
        return rule.reconstruct_call_stack();
    }
    if let Some(declaring_macro) = target.get_declaring_macro() {
        return declaring_macro.reconstruct_parent_call_stack();
    }
    vec![StackEntry {
        name: blaze_packages::package::call_stack::TOP_LEVEL.to_owned(),
        location: Some(target.get_location()),
        source_line: None,
    }]
}

// ---------------------------------------------------------------------------------------------
// Exceptions.
// ---------------------------------------------------------------------------------------------

/// Java `MacroInstanceFunction.NoSuchMacroInstanceException`.
// Java: com/google/devtools/build/lib/skyframe/MacroInstanceFunction.java:84 MacroInstanceFunction.NoSuchMacroInstanceException
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct NoSuchMacroInstanceException {
    message: String,
}

impl NoSuchMacroInstanceException {
    // Java: com/google/devtools/build/lib/skyframe/MacroInstanceFunction.java:85 MacroInstanceFunction.NoSuchMacroInstanceException.<init>
    fn new(key: &MacroInstanceKey, package_piece: &PackagePiece) -> Self {
        NoSuchMacroInstanceException {
            message: format!(
                "Macro instance '{}' not found in {}",
                key.macro_instance_name,
                package_piece.get_short_description()
            ),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// Java `NoSuchThingException.getDetailedExitCode()` without an explicit code.
    pub fn get_detailed_exit_code(&self) -> blaze_packages::exceptions::DetailedExitCode {
        blaze_packages::exceptions::DetailedExitCode::package_loading(
            self.message.clone(),
            PackageLoadingCode::PackageLoadingUnknown,
        )
    }
}

/// The exceptions the piece functions catch and rethrow (Java's `getValueOrThrow(key,
/// NoSuchPackageException.class, NoSuchPackagePieceException.class,
/// NoSuchMacroInstanceException.class)`).
#[derive(Debug, Clone)]
pub enum PieceException {
    Package(NoSuchPackageException),
    Piece(NoSuchPackagePieceException),
    MacroInstance(NoSuchMacroInstanceException),
}

impl PieceException {
    fn catch(e: &SkyError) -> Option<PieceException> {
        if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
            return Some(PieceException::Package(e.clone()));
        }
        if let Some(e) = e.downcast_ref::<NoSuchPackagePieceException>() {
            return Some(PieceException::Piece(e.clone()));
        }
        e.downcast_ref::<NoSuchMacroInstanceException>()
            .map(|e| PieceException::MacroInstance(e.clone()))
    }

    /// Java `throw new XFunctionException(cause)` (Transience.PERSISTENT): the exception classes
    /// of the piece functions, each a SkyFunctionException wrapping one of these causes.
    // Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:403 EvalMacroFunction.EvalMacroFunctionException.<init>
    // Java: com/google/devtools/build/lib/skyframe/MacroInstanceFunction.java:69 MacroInstanceFunction.MacroInstanceFunctionException.<init>
    // Java: com/google/devtools/build/lib/skyframe/NonFinalizerPackagePiecesFunction.java:107 NonFinalizerPackagePiecesFunction.NonFinalizerPackagePiecesFunctionException.<init>
    // Java: com/google/devtools/build/lib/skyframe/PackageDeclarationsFunction.java:65 PackageDeclarationsFunction.PackageDeclarationsFunctionException.<init>
    fn rethrow(self) -> ComputeError {
        match self {
            PieceException::Package(e) => SkyFunctionException::new(e, Transience::Persistent),
            PieceException::Piece(e) => SkyFunctionException::new(e, Transience::Persistent),
            PieceException::MacroInstance(e) => {
                SkyFunctionException::new(e, Transience::Persistent)
            }
        }
        .into()
    }
}

// ---------------------------------------------------------------------------------------------
// Functions.
// ---------------------------------------------------------------------------------------------

/// PACKAGE_DECLARATIONS: the metadata and declarations of the BUILD file piece.
// Java: com/google/devtools/build/lib/skyframe/PackageDeclarationsFunction.java:36 PackageDeclarationsFunction.compute
pub fn package_declarations_function(
    key: &PackageDeclarationsKey,
    env: &mut Environment<'_>,
) -> Result<Option<PackageDeclarationsValue>, ComputeError> {
    let package_piece_value = match env.get_value_or_throw_with(
        PackagePieceForBuildFileKey(PackagePieceIdentifierForBuildFile::new(
            key.package_id.clone(),
        )),
        |e| match PieceException::catch(e) {
            Some(PieceException::MacroInstance(_)) | None => None,
            other => other,
        },
    )? {
        Err(e) => return Err(e.rethrow()),
        Ok(None) => return Ok(None),
        Ok(Some(v)) => v,
    };
    Ok(Some(PackageDeclarationsValue {
        metadata: package_piece_value.for_build_file.get_metadata().clone(),
        declarations: package_piece_value
            .for_build_file
            .get_declarations()
            .clone(),
        starlark_semantics: package_piece_value.starlark_semantics.clone(),
        main_repository_mapping: package_piece_value.main_repository_mapping.clone(),
    }))
}

/// MACRO_INSTANCE: a macro instance declared in a package piece.
// Java: com/google/devtools/build/lib/skyframe/MacroInstanceFunction.java:36 MacroInstanceFunction.compute
pub fn macro_instance_function(
    key: &MacroInstanceKey,
    env: &mut Environment<'_>,
) -> Result<Option<MacroInstanceValue>, ComputeError> {
    let catch = |e: &SkyError| match PieceException::catch(e) {
        Some(PieceException::MacroInstance(_)) | None => None,
        other => other,
    };
    let package_piece = match &key.package_piece_id {
        PackagePieceIdentifier::ForBuildFile(id) => {
            match env.get_value_or_throw_with(PackagePieceForBuildFileKey(id.clone()), catch)? {
                Err(e) => return Err(e.rethrow()),
                Ok(None) => return Ok(None),
                Ok(Some(v)) => v.for_build_file.clone(),
            }
        }
        PackagePieceIdentifier::ForMacro(id) => {
            match env.get_value_or_throw_with(EvalMacroKey(id.clone()), catch)? {
                Err(e) => return Err(e.rethrow()),
                Ok(None) => return Ok(None),
                Ok(Some(v)) => v.for_macro.clone(),
            }
        }
    };
    let Some(macro_instance) = package_piece.get_macro_by_name(&key.macro_instance_name) else {
        return Err(
            PieceException::MacroInstance(NoSuchMacroInstanceException::new(key, &package_piece))
                .rethrow(),
        );
    };
    Ok(Some(MacroInstanceValue {
        macro_instance: macro_instance.clone(),
    }))
}

/// EVAL_MACRO: evaluates one symbolic macro into its package piece.
// Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:63 EvalMacroFunction
pub struct EvalMacroFunction {
    pub package_factory: Arc<PackageFactory>,
}

impl EvalMacroFunction {
    // Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:75 EvalMacroFunction.compute
    pub fn compute(
        &self,
        key: &EvalMacroKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<PackagePieceValueForMacro>, ComputeError> {
        let key = &key.0;
        let package_declarations_value = match env.get_value_or_throw_with(
            PackageDeclarationsKey {
                package_id: key.get_package_identifier().clone(),
            },
            |e| match PieceException::catch(e) {
                Some(PieceException::MacroInstance(_)) | None => None,
                other => other,
            },
        )? {
            Err(e) => return Err(e.rethrow()),
            Ok(None) => return Ok(None),
            Ok(Some(v)) => v,
        };

        let macro_instance_value = match env.get_value_or_throw_with(
            MacroInstanceKey {
                package_piece_id: key.get_parent_identifier().clone(),
                macro_instance_name: key.get_instance_name().to_owned(),
            },
            PieceException::catch,
        )? {
            Err(e) => return Err(e.rethrow()),
            Ok(None) => return Ok(None),
            Ok(Some(v)) => v,
        };
        let macro_instance = macro_instance_value.macro_instance.clone();

        let mut non_finalizer_package_pieces_value = None;
        let mut existing_rules_map_for_finalizer: Option<IndexMap<String, Arc<Rule>>> = None;
        if macro_instance.get_macro_class().is_finalizer() {
            match env.get_value_or_throw_with(
                NonFinalizerPackagePiecesKey {
                    pkg_id: key.get_package_identifier().clone(),
                },
                PieceException::catch,
            )? {
                Err(e) => return Err(e.rethrow()),
                Ok(None) => return Ok(None),
                Ok(Some(v)) => {
                    if !v.contains_errors() {
                        existing_rules_map_for_finalizer = Some(
                            v.targets
                                .iter()
                                .filter_map(|(name, t)| {
                                    t.as_rule().map(|r| (name.clone(), r.clone()))
                                })
                                .collect(),
                        );
                    }
                    non_finalizer_package_pieces_value = Some(v);
                }
            }
        }

        let semantics = package_declarations_value.starlark_semantics.clone();
        let mut package_piece_builder = self.package_factory.new_package_piece_for_macro_builder(
            package_declarations_value.metadata.clone(),
            package_declarations_value.declarations.clone(),
            macro_instance.clone(),
            key.get_parent_identifier().clone(),
            &semantics,
            package_declarations_value.main_repository_mapping.clone(),
            existing_rules_map_for_finalizer,
        );
        match non_finalizer_package_pieces_value {
            Some(v) if v.contains_errors() => {
                if !v.error_keys.is_empty() {
                    let error_key = &v.error_keys[0];
                    let error_piece = &v.package_pieces[error_key];
                    handle_finalizer_dependency_error(
                        &mut package_piece_builder,
                        &macro_instance,
                        key,
                        &format!("error in {}", error_piece.get_short_description()),
                    );
                } else {
                    handle_finalizer_dependency_error(
                        &mut package_piece_builder,
                        &macro_instance,
                        key,
                        &v.name_conflict_between_package_pieces_exception
                            .as_ref()
                            .expect("a name conflict")
                            .message,
                    );
                }
                package_piece_builder.set_contains_errors();
            }
            _ => {
                let (builder, result) = MacroClass::execute_macro_implementation(
                    &macro_instance,
                    package_piece_builder,
                    &semantics,
                );
                package_piece_builder = builder;
                if let Err(e) = result {
                    if e.kind == EvalErrorKind::Cancelled {
                        return Err(Interrupted.into());
                    }
                    package_piece_builder
                        .get_local_event_handler()
                        .handle(&Package::error(
                            e.location(),
                            &e.message_with_stack(),
                            PackageLoadingCode::StarlarkEvalError,
                        ));
                    package_piece_builder.set_contains_errors();
                }
            }
        }

        // PORT: the load time metric is not ported (profiling only).
        if let Err(e) = package_piece_builder.build_partial() {
            return Err(PieceException::Package(e).rethrow());
        }
        let package_piece = package_piece_builder.finish_build_piece();
        package_piece_builder
            .get_local_event_handler()
            .replay_on(env.listener());
        self.package_factory
            .after_done_loading_package_piece(&package_piece, &semantics);
        Ok(Some(PackagePieceValueForMacro {
            for_macro: package_piece,
        }))
    }
}

// Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:205 EvalMacroFunction.handleFinalizerDependencyError
fn handle_finalizer_dependency_error(
    package_piece_builder: &mut TargetDefinitionContext,
    evaluated_macro: &MacroInstance,
    identifier: &PackagePieceIdentifierForMacro,
    message: &str,
) {
    // Java: getPackagePiece().getShortDescription() of the piece under construction.
    let short_description = format!(
        "package piece for {}macro {} defined by {}%{}",
        if evaluated_macro.get_macro_class().is_finalizer() {
            "finalizer "
        } else {
            ""
        },
        identifier.get_canonical_form_name(),
        evaluated_macro
            .get_macro_class()
            .get_defining_bzl_label()
            .get_canonical_form(),
        evaluated_macro.get_macro_class().get_name()
    );
    package_piece_builder
        .get_local_event_handler()
        .handle(&Package::error(
            Some(evaluated_macro.get_build_file_location()),
            &format!("cannot compute {short_description}: {message}"),
            PackageLoadingCode::StarlarkEvalError,
        ));
}

/// Java `EvalMacroFunction.RecursiveExpander`: expands package pieces recursively, starting from
/// a BUILD file piece.
// Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:226 EvalMacroFunction.RecursiveExpander
#[derive(Default)]
pub struct RecursiveExpander {
    pieces: PackagePieces,
    // Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:263 EvalMacroFunction.RecursiveExpander.getStarlarkSemantics
    starlark_semantics: Option<Arc<StarlarkSemantics>>,
    // Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:268 EvalMacroFunction.RecursiveExpander.getMainRepositoryMapping
    main_repository_mapping: Option<RepositoryMapping>,
}

/// A piece value of either function.
enum AnyPieceValue {
    ForBuildFile(Arc<PackagePieceValueForBuildFile>),
    ForMacro(Arc<PackagePieceValueForMacro>),
}

impl RecursiveExpander {
    /// Expands from the BUILD file piece of `pkg_id`. `Ok(None)` when values are missing.
    // Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:272 EvalMacroFunction.RecursiveExpander.expand
    pub fn expand_package(
        mut self,
        pkg_id: &PackageIdentifier,
        env: &mut Environment<'_>,
        expand_finalizers: bool,
    ) -> Result<Result<Option<RecursiveExpander>, PieceException>, Interrupted> {
        let keys = vec![PackagePieceIdentifier::ForBuildFile(
            PackagePieceIdentifierForBuildFile::new(pkg_id.clone()),
        )];
        Ok(match self.expand(keys, env, expand_finalizers)? {
            Err(e) => Err(e),
            Ok(false) => Ok(None),
            Ok(true) => Ok(Some(self)),
        })
    }

    /// Returns `Ok(true)` when all values were present.
    // Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:289 EvalMacroFunction.RecursiveExpander.expand
    // PORT: request order. Java requests each level's keys in one batch; they are requested in
    // order here.
    fn expand(
        &mut self,
        keys: Vec<PackagePieceIdentifier>,
        env: &mut Environment<'_>,
        expand_finalizers: bool,
    ) -> Result<Result<bool, PieceException>, Interrupted> {
        if keys.is_empty() {
            return Ok(Ok(true));
        }
        if self.pieces.package_pieces.is_empty() {
            blaze_util::check_argument!(
                keys.len() == 1 && matches!(keys[0], PackagePieceIdentifier::ForBuildFile(_)),
                "expansion must start from a PackagePieceIdentifier.ForBuildFile"
            );
        }
        let mut values_missing = false;
        let sky_keys: Vec<SkyKey> = keys.iter().map(piece_sky_key).collect();
        let lookup_result = env.get_values_and_exceptions(&sky_keys)?;
        let mut child_keys: Vec<PackagePieceIdentifier> = Vec::new();
        for key in &keys {
            let value = match key {
                PackagePieceIdentifier::ForBuildFile(id) => match lookup_result
                    .get_or_throw_with(&PackagePieceForBuildFileKey(id.clone()), |e| {
                        PieceException::catch(e)
                    }) {
                    Err(e) => return Ok(Err(e)),
                    Ok(v) => v.map(AnyPieceValue::ForBuildFile),
                },
                PackagePieceIdentifier::ForMacro(id) => match lookup_result
                    .get_or_throw_with(&EvalMacroKey(id.clone()), |e| PieceException::catch(e))
                {
                    Err(e) => return Ok(Err(e)),
                    Ok(v) => v.map(AnyPieceValue::ForMacro),
                },
            };
            let Some(value) = value else {
                values_missing = true;
                continue;
            };
            let piece = match value {
                AnyPieceValue::ForBuildFile(v) => {
                    self.starlark_semantics = Some(v.starlark_semantics.clone());
                    self.main_repository_mapping = Some(v.main_repository_mapping.clone());
                    v.for_build_file.clone()
                }
                AnyPieceValue::ForMacro(v) => v.for_macro.clone(),
            };
            self.pieces
                .package_pieces
                .insert(key.clone(), piece.clone());
            if piece.contains_errors() {
                if !self.pieces.error_keys.contains(key) {
                    self.pieces.error_keys.push(key.clone());
                }
            } else {
                for child_macro_instance in piece.get_macros() {
                    let child_key =
                        PackagePieceIdentifier::ForMacro(PackagePieceIdentifierForMacro::new(
                            key.get_package_identifier().clone(),
                            key.clone(),
                            child_macro_instance.get_name().to_owned(),
                        ));
                    if self.pieces.package_pieces.contains_key(&child_key) {
                        continue;
                    }
                    if expand_finalizers || !child_macro_instance.get_macro_class().is_finalizer() {
                        child_keys.push(child_key);
                    }
                }
            }
        }
        match self.expand(child_keys, env, expand_finalizers)? {
            Err(e) => return Ok(Err(e)),
            Ok(false) => values_missing = true,
            Ok(true) => {}
        }
        Ok(Ok(!values_missing))
    }

    /// Expands the finalizers of `non_finalizer_package_pieces` (and the macros they declare).
    /// `Ok(None)` when values are missing.
    // Java: com/google/devtools/build/lib/skyframe/EvalMacroFunction.java:360 EvalMacroFunction.RecursiveExpander.expandFinalizers
    pub fn expand_finalizers(
        non_finalizer_package_pieces: &NonFinalizerPackagePiecesValue,
        env: &mut Environment<'_>,
    ) -> Result<Result<Option<PackagePieces>, PieceException>, Interrupted> {
        let mut unexpanded_keys: Vec<PackagePieceIdentifier> = Vec::new();
        for package_piece in non_finalizer_package_pieces.package_pieces.values() {
            for macro_ in package_piece.get_macros() {
                let key = PackagePieceIdentifier::ForMacro(PackagePieceIdentifierForMacro::new(
                    package_piece.get_package_identifier().clone(),
                    package_piece.get_identifier(),
                    macro_.get_name().to_owned(),
                ));
                if !non_finalizer_package_pieces
                    .package_pieces
                    .contains_key(&key)
                {
                    unexpanded_keys.push(key);
                }
            }
        }
        let current = PackagePieces {
            package_pieces: non_finalizer_package_pieces.package_pieces.clone(),
            error_keys: non_finalizer_package_pieces.error_keys.clone(),
        };
        if unexpanded_keys.is_empty() {
            return Ok(Ok(Some(current)));
        }
        let mut expander = RecursiveExpander {
            pieces: current,
            starlark_semantics: Some(non_finalizer_package_pieces.starlark_semantics.clone()),
            main_repository_mapping: Some(
                non_finalizer_package_pieces.main_repository_mapping.clone(),
            ),
        };
        Ok(
            match expander.expand(unexpanded_keys, env, /* expand_finalizers= */ true)? {
                Err(e) => Err(e),
                Ok(false) => Ok(None),
                Ok(true) => Ok(Some(expander.pieces)),
            },
        )
    }
}

/// NON_FINALIZER_PACKAGE_PIECES: the BUILD file piece and all non-finalizer macro pieces, with
/// their targets and macros checked for name conflicts.
// Java: com/google/devtools/build/lib/skyframe/NonFinalizerPackagePiecesFunction.java:42 NonFinalizerPackagePiecesFunction.compute
pub fn non_finalizer_package_pieces_function(
    key: &NonFinalizerPackagePiecesKey,
    env: &mut Environment<'_>,
) -> Result<Option<NonFinalizerPackagePiecesValue>, ComputeError> {
    let expander = match RecursiveExpander::default().expand_package(
        &key.pkg_id,
        env,
        /* expand_finalizers= */ false,
    )? {
        Err(e) => return Err(e.rethrow()),
        Ok(None) => return Ok(None),
        Ok(Some(expander)) => expander,
    };
    blaze_util::check_state!(!expander.pieces.package_pieces.is_empty());
    let starlark_semantics = expander
        .starlark_semantics
        .clone()
        .expect("semantics of the BUILD file piece");
    let main_repository_mapping = expander
        .main_repository_mapping
        .clone()
        .expect("main repository mapping of the BUILD file piece");
    if expander.pieces.package_pieces.len() == 1 {
        return Ok(Some(NonFinalizerPackagePiecesValue {
            targets: expander
                .pieces
                .get_package_piece_for_build_file()
                .get_targets()
                .clone(),
            package_pieces: expander.pieces.package_pieces,
            error_keys: expander.pieces.error_keys,
            name_conflict_between_package_pieces_exception: None,
            macro_instances: BTreeMap::new(),
            starlark_semantics,
            main_repository_mapping,
        }));
    }
    let mut target_recorder = TargetRecorder::new(
        /* enable_name_conflict_checking= */ true,
        /* track_full_macro_information= */ false,
        /* enable_target_map_snapshotting= */ false,
    );
    let mut name_conflict_exception = None;
    if let Err(e) = expander
        .pieces
        .record_targets_and_macros_into_recorder(&mut target_recorder)
    {
        env.listener().handle(&Package::error(
            e.location(),
            &e.message_with_stack(),
            PackageLoadingCode::StarlarkEvalError,
        ));
        name_conflict_exception = Some(e);
    }
    Ok(Some(NonFinalizerPackagePiecesValue {
        package_pieces: expander.pieces.package_pieces,
        error_keys: expander.pieces.error_keys,
        name_conflict_between_package_pieces_exception: name_conflict_exception,
        targets: target_recorder
            .get_target_map()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        macro_instances: target_recorder
            .get_macro_map()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        starlark_semantics,
        main_repository_mapping,
    }))
}
