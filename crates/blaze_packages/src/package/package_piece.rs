//! Ports of `packages/PackagePieceIdentifier.java`, `packages/PackagePiece.java` (with its
//! `ForBuildFile.Builder` and `ForMacro.Builder`), `packages/NoSuchPackagePieceException.java`,
//! `Package.newPackageFromPackagePiecesBuilder` / `Package.Builder.addAllFromPackagePiece` and
//! `TargetRecorder.addAllFromPackagePiece`: the pieces of a package under lazy symbolic macro
//! expansion (`--experimental_lazy_macro_expansion_packages`).
//!
//! PORT: Java's piece builders are subclasses of `TargetDefinitionContext`; here the one builder
//! type [`TargetDefinitionContext`] carries a [`PieceBuilderKind`] (none for `Package.Builder`),
//! and the methods those subclasses override check it.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_events::Event;
use blaze_events::Postable;
use blaze_starlark::Location;
use blaze_vfs::RootedPath;
use indexmap::IndexMap;

use crate::exceptions::DetailedExitCode;
use crate::exceptions::NoSuchTargetException;
use crate::macros::MacroInstance;
use crate::package::ConfigSettingVisibilityPolicy;
use crate::package::Globber;
use crate::package::MacroFrame;
use crate::package::NameConflictException;
use crate::package::Package;
use crate::package::PackageDeclarations;
use crate::package::PackageLimits;
use crate::package::PackageMetadata;
use crate::package::PackageSettings;
use crate::package::TargetDefinitionContext;
use crate::package::TargetRecorder;
use crate::target::InputFile;
use crate::target::Rule;
use crate::target::RuleOrMacroInstance;
use crate::target::Target;

/// Java `PackagePieceIdentifier.ForBuildFile`: the piece of a package obtained by evaluating its
/// BUILD file without expanding symbolic macros. Its SkyFunction is PACKAGE.
// Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:44 PackagePieceIdentifier.ForBuildFile
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PackagePieceIdentifierForBuildFile {
    package_identifier: PackageIdentifier,
}

impl PackagePieceIdentifierForBuildFile {
    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:88 PackagePieceIdentifier.ForBuildFile.<init>
    pub fn new(package_identifier: PackageIdentifier) -> Self {
        PackagePieceIdentifierForBuildFile { package_identifier }
    }

    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:50 PackagePieceIdentifier.ForBuildFile.getPackageIdentifier
    pub fn get_package_identifier(&self) -> &PackageIdentifier {
        &self.package_identifier
    }

    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:55 PackagePieceIdentifier.ForBuildFile.getCanonicalFormName
    pub fn get_canonical_form_name(&self) -> String {
        self.package_identifier.get_canonical_form()
    }
}

impl fmt::Debug for PackagePieceIdentifierForBuildFile {
    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:60 PackagePieceIdentifier.ForBuildFile.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<PackagePieceIdentifier.ForBuildFile pkg={}>",
            self.get_canonical_form_name()
        )
    }
}

impl fmt::Display for PackagePieceIdentifierForBuildFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Java `PackagePieceIdentifier.ForMacro`: the piece obtained by evaluating one symbolic macro
/// instance (declared in the parent piece). Its SkyFunction is EVAL_MACRO.
// Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:96 PackagePieceIdentifier.ForMacro
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PackagePieceIdentifierForMacro {
    package_identifier: PackageIdentifier,
    parent_identifier: Arc<PackagePieceIdentifier>,
    instance_name: String,
}

impl PackagePieceIdentifierForMacro {
    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:146 PackagePieceIdentifier.ForMacro.<init>
    pub fn new(
        package_identifier: PackageIdentifier,
        parent_identifier: PackagePieceIdentifier,
        instance_name: String,
    ) -> Self {
        blaze_util::check_argument!(
            *parent_identifier.get_package_identifier() == package_identifier
        );
        PackagePieceIdentifierForMacro {
            package_identifier,
            parent_identifier: Arc::new(parent_identifier),
            instance_name,
        }
    }

    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:106 PackagePieceIdentifier.ForMacro.getInstanceName
    pub fn get_instance_name(&self) -> &str {
        &self.instance_name
    }

    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:111 PackagePieceIdentifier.ForMacro.getPackageIdentifier
    pub fn get_package_identifier(&self) -> &PackageIdentifier {
        &self.package_identifier
    }

    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:116 PackagePieceIdentifier.ForMacro.getParentIdentifier
    pub fn get_parent_identifier(&self) -> &PackagePieceIdentifier {
        &self.parent_identifier
    }

    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:121 PackagePieceIdentifier.ForMacro.getCanonicalFormName
    pub fn get_canonical_form_name(&self) -> String {
        format!(
            "{}:{}",
            self.package_identifier.get_canonical_form(),
            self.instance_name
        )
    }
}

impl fmt::Debug for PackagePieceIdentifierForMacro {
    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:126 PackagePieceIdentifier.ForMacro.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<PackagePieceIdentifier.ForMacro name={} declared_in={:?}>",
            self.get_canonical_form_name(),
            self.parent_identifier
        )
    }
}

impl fmt::Display for PackagePieceIdentifierForMacro {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Java `PackagePieceIdentifier` (sealed: ForBuildFile, ForMacro).
// Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:30 PackagePieceIdentifier
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum PackagePieceIdentifier {
    ForBuildFile(PackagePieceIdentifierForBuildFile),
    ForMacro(PackagePieceIdentifierForMacro),
}

impl PackagePieceIdentifier {
    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:36 PackagePieceIdentifier.getCanonicalFormName
    pub fn get_canonical_form_name(&self) -> String {
        match self {
            PackagePieceIdentifier::ForBuildFile(id) => id.get_canonical_form_name(),
            PackagePieceIdentifier::ForMacro(id) => id.get_canonical_form_name(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/PackagePieceIdentifier.java:39 PackagePieceIdentifier.getPackageIdentifier
    pub fn get_package_identifier(&self) -> &PackageIdentifier {
        match self {
            PackagePieceIdentifier::ForBuildFile(id) => id.get_package_identifier(),
            PackagePieceIdentifier::ForMacro(id) => id.get_package_identifier(),
        }
    }
}

impl fmt::Debug for PackagePieceIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackagePieceIdentifier::ForBuildFile(id) => fmt::Debug::fmt(id, f),
            PackagePieceIdentifier::ForMacro(id) => fmt::Debug::fmt(id, f),
        }
    }
}

impl fmt::Display for PackagePieceIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Java `NoSuchPackagePieceException`.
// Java: com/google/devtools/build/lib/packages/NoSuchPackagePieceException.java:27 NoSuchPackagePieceException
#[derive(Debug, Clone, thiserror::Error)]
#[error("no such package piece {package_piece_id}: {message}")]
pub struct NoSuchPackagePieceException {
    package_piece_id: PackagePieceIdentifier,
    message: String,
    detailed_exit_code: Option<DetailedExitCode>,
}

impl NoSuchPackagePieceException {
    // Java: com/google/devtools/build/lib/packages/NoSuchPackagePieceException.java:31 NoSuchPackagePieceException.<init>
    pub fn new(
        package_piece_id: PackagePieceIdentifier,
        message: impl Into<String>,
        detailed_exit_code: Option<DetailedExitCode>,
    ) -> Self {
        NoSuchPackagePieceException {
            package_piece_id,
            message: message.into(),
            detailed_exit_code,
        }
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchPackagePieceException.java:58 NoSuchPackagePieceException.getPackagePieceIdentifier
    pub fn get_package_piece_identifier(&self) -> &PackagePieceIdentifier {
        &self.package_piece_id
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchPackagePieceException.java:62 NoSuchPackagePieceException.getRawMessage
    pub fn get_raw_message(&self) -> &str {
        &self.message
    }

    /// Java `getMessage()`: "no such package piece <id>: <raw message>".
    // Java: com/google/devtools/build/lib/packages/NoSuchPackagePieceException.java:67 NoSuchPackagePieceException.getMessage
    pub fn get_message(&self) -> String {
        self.to_string()
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchPackagePieceException.java:76 NoSuchPackagePieceException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> DetailedExitCode {
        match &self.detailed_exit_code {
            Some(d) => d.clone(),
            None => DetailedExitCode::package_loading(
                self.get_message(),
                crate::exceptions::PackageLoadingCode::PackageMissing,
            ),
        }
    }
}

/// What kind of piece a [`PackagePiece`] is (Java's subclasses).
#[derive(Debug, Clone)]
pub enum PackagePieceKind {
    /// Java `PackagePiece.ForBuildFile`.
    ForBuildFile {
        identifier: PackagePieceIdentifierForBuildFile,
        build_file: Arc<InputFile>,
    },
    /// Java `PackagePiece.ForMacro`.
    ForMacro {
        identifier: PackagePieceIdentifierForMacro,
        evaluated_macro: Arc<MacroInstance>,
        macro_namespace_violations: BTreeSet<String>,
    },
}

/// A piece of a package (Java `PackagePiece`): the targets and macros declared by the BUILD file
/// or by one symbolic macro, with the macros it declares not expanded.
// Java: com/google/devtools/build/lib/packages/PackagePiece.java:47 PackagePiece
#[derive(Debug)]
pub struct PackagePiece {
    kind: PackagePieceKind,
    metadata: Arc<PackageMetadata>,
    declarations: Arc<PackageDeclarations>,
    contains_errors: bool,
    failure_detail: Option<DetailedExitCode>,
    computation_steps: i64,
    targets: BTreeMap<String, Target>,
    macros_by_name: BTreeMap<String, Arc<MacroInstance>>,
    events: Vec<Event>,
    posts: Vec<Arc<dyn Postable>>,
}

impl PackagePiece {
    pub fn get_kind(&self) -> &PackagePieceKind {
        &self.kind
    }

    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:54 PackagePiece.getIdentifier
    pub fn get_identifier(&self) -> PackagePieceIdentifier {
        match &self.kind {
            PackagePieceKind::ForBuildFile { identifier, .. } => {
                PackagePieceIdentifier::ForBuildFile(identifier.clone())
            }
            PackagePieceKind::ForMacro { identifier, .. } => {
                PackagePieceIdentifier::ForMacro(identifier.clone())
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:109 Packageoid.getMetadata
    pub fn get_metadata(&self) -> &Arc<PackageMetadata> {
        &self.metadata
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:117 Packageoid.getDeclarations
    pub fn get_declarations(&self) -> &Arc<PackageDeclarations> {
        &self.declarations
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:125 Packageoid.getPackageIdentifier
    pub fn get_package_identifier(&self) -> &PackageIdentifier {
        &self.metadata.package_identifier
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:160 Packageoid.containsErrors
    pub fn contains_errors(&self) -> bool {
        self.contains_errors
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:168 Packageoid.getFailureDetail
    pub fn get_failure_detail(&self) -> Option<&DetailedExitCode> {
        self.failure_detail.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:177 Packageoid.getComputationSteps
    pub fn get_computation_steps(&self) -> i64 {
        self.computation_steps
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:143 Packageoid.getTargets
    pub fn get_targets(&self) -> &BTreeMap<String, Target> {
        &self.targets
    }

    pub fn get_events(&self) -> &[Event] {
        &self.events
    }

    pub fn get_posts(&self) -> &[Arc<dyn Postable>] {
        &self.posts
    }

    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:70 PackagePiece.getTarget
    pub fn get_target(&self, target_name: &str) -> Result<&Target, NoSuchTargetException> {
        if let Some(target) = self.targets.get(target_name) {
            return Ok(target);
        }
        Err(self.no_such_target_exception(target_name))
    }

    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:84 PackagePiece.getMacroByName
    pub fn get_macro_by_name(&self, name: &str) -> Option<&Arc<MacroInstance>> {
        self.macros_by_name.get(name)
    }

    /// The macros declared directly in this piece, by name.
    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:92 PackagePiece.getMacros
    pub fn get_macros(&self) -> Vec<Arc<MacroInstance>> {
        self.macros_by_name.values().cloned().collect()
    }

    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:96 PackagePiece.noSuchTargetException
    fn no_such_target_exception(&self, target_name: &str) -> NoSuchTargetException {
        let label = match Label::create_in_package(self.get_package_identifier(), target_name) {
            Ok(l) => l,
            Err(e) => panic!("IllegalArgumentException: {target_name}: {e}"),
        };
        if self.metadata.succinct_target_not_found_errors {
            NoSuchTargetException::with_label(
                label,
                &format!(
                    "target '{}' not declared in {}",
                    target_name,
                    self.get_short_description()
                ),
            )
        } else {
            let names: Vec<&str> = self.targets.keys().map(String::as_str).collect();
            let alternate_target_suggestion =
                Package::get_alternate_target_suggestion(&self.metadata, target_name, &names);
            NoSuchTargetException::with_label(
                label,
                &format!(
                    "target '{}' not declared in {}{}",
                    target_name,
                    self.get_short_description(),
                    alternate_target_suggestion
                ),
            )
        }
    }

    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:135 PackagePiece.getCanonicalFormDefinedBy
    pub fn get_canonical_form_defined_by(&self) -> String {
        match &self.kind {
            // Java: com/google/devtools/build/lib/packages/PackagePiece.java:180 PackagePiece.ForBuildFile.getCanonicalFormDefinedBy
            PackagePieceKind::ForBuildFile { .. } => {
                self.metadata.build_file_label.get_canonical_form()
            }
            // Java: com/google/devtools/build/lib/packages/PackagePiece.java:344 PackagePiece.ForMacro.getCanonicalFormDefinedBy
            PackagePieceKind::ForMacro {
                evaluated_macro, ..
            } => {
                let macro_class = evaluated_macro.get_macro_class();
                format!(
                    "{}%{}",
                    macro_class.get_defining_bzl_label().get_canonical_form(),
                    macro_class.get_name()
                )
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:194 Packageoid.getShortDescription
    pub fn get_short_description(&self) -> String {
        match &self.kind {
            // Java: com/google/devtools/build/lib/packages/PackagePiece.java:185 PackagePiece.ForBuildFile.getShortDescription
            PackagePieceKind::ForBuildFile { .. } => format!(
                "top-level package piece defined by {}",
                self.get_canonical_form_defined_by()
            ),
            // Java: com/google/devtools/build/lib/packages/PackagePiece.java:351 PackagePiece.ForMacro.getShortDescription
            PackagePieceKind::ForMacro {
                identifier,
                evaluated_macro,
                ..
            } => format!(
                "package piece for {}macro {} defined by {}",
                if evaluated_macro.get_macro_class().is_finalizer() {
                    "finalizer "
                } else {
                    ""
                },
                identifier.get_canonical_form_name(),
                self.get_canonical_form_defined_by()
            ),
        }
    }

    /// Java `PackagePiece.ForBuildFile.getBuildFile`.
    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:190 PackagePiece.ForBuildFile.getBuildFile
    pub fn get_build_file(&self) -> Option<&Arc<InputFile>> {
        match &self.kind {
            PackagePieceKind::ForBuildFile { build_file, .. } => Some(build_file),
            PackagePieceKind::ForMacro { .. } => None,
        }
    }

    /// Java `PackagePiece.ForMacro.getEvaluatedMacro`.
    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:360 PackagePiece.ForMacro.getEvaluatedMacro
    pub fn get_evaluated_macro(&self) -> Option<&Arc<MacroInstance>> {
        match &self.kind {
            PackagePieceKind::ForBuildFile { .. } => None,
            PackagePieceKind::ForMacro {
                evaluated_macro, ..
            } => Some(evaluated_macro),
        }
    }

    /// Java `PackagePiece.ForMacro.getDeclaringPackage`.
    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:369 PackagePiece.ForMacro.getDeclaringPackage
    pub fn get_declaring_package(&self) -> Option<&PackageIdentifier> {
        self.get_evaluated_macro().map(|m| {
            m.get_macro_class()
                .get_defining_bzl_label()
                .get_package_identifier()
        })
    }

    /// Java `PackagePiece.ForMacro.macroNamespaceViolations` (empty for the BUILD file piece).
    pub fn get_macro_namespace_violations(&self) -> Option<&BTreeSet<String>> {
        match &self.kind {
            PackagePieceKind::ForBuildFile { .. } => None,
            PackagePieceKind::ForMacro {
                macro_namespace_violations,
                ..
            } => Some(macro_namespace_violations),
        }
    }
}

impl fmt::Display for PackagePiece {
    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:123 PackagePiece.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rules: Vec<String> = self
            .targets
            .values()
            .filter_map(|t| t.as_rule().map(|r| r.to_string()))
            .collect();
        write!(
            f,
            "PackagePiece({} defined by {})=[{}]",
            self.get_identifier().get_canonical_form_name(),
            self.get_canonical_form_defined_by(),
            rules.join(", ")
        )
    }
}

/// The piece a [`TargetDefinitionContext`] builds, when it is a piece builder (Java
/// `PackagePiece.ForBuildFile.Builder` / `PackagePiece.ForMacro.Builder`).
#[derive(Debug, Clone)]
pub enum PieceBuilderKind {
    ForBuildFile {
        identifier: PackagePieceIdentifierForBuildFile,
    },
    ForMacro {
        identifier: PackagePieceIdentifierForMacro,
        evaluated_macro: Arc<MacroInstance>,
        /// Java `existingRulesMapForFinalizer` (`native.existing_rules()` of a finalizer).
        existing_rules_map_for_finalizer: Option<IndexMap<String, Arc<Rule>>>,
    },
}

impl TargetDefinitionContext {
    /// Java `PackagePiece.ForBuildFile.newBuilder`.
    // PORT: cpuBoundSemaphore, packageOverheadEstimator and the symbol generator are dropped
    // (single build per process; memory accounting).
    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:211 PackagePiece.ForBuildFile.newBuilder
    pub fn new_package_piece_for_build_file_builder(
        package_settings: PackageSettings,
        identifier: PackagePieceIdentifierForBuildFile,
        filename: RootedPath,
        workspace_name: String,
        associated_module_name: Option<String>,
        associated_module_version: Option<String>,
        no_implicit_file_export: bool,
        simplify_unconditional_selects_in_rule_attrs: bool,
        repository_mapping: RepositoryMapping,
        main_repository_mapping: RepositoryMapping,
        generator_map: Option<IndexMap<Location, String>>,
        config_setting_visibility_policy: Option<ConfigSettingVisibilityPolicy>,
        globber: Option<Arc<dyn Globber>>,
        enable_name_conflict_checking: bool,
        track_full_macro_information: bool,
        package_limits: PackageLimits,
    ) -> TargetDefinitionContext {
        let metadata = PackageMetadata::of(
            identifier.get_package_identifier().clone(),
            filename,
            workspace_name,
            repository_mapping,
            associated_module_name,
            associated_module_version,
            config_setting_visibility_policy,
            package_settings.succinct_target_not_found_errors,
        );
        let mut builder = TargetDefinitionContext::new_internal(
            Arc::new(metadata),
            Arc::new(PackageDeclarations::new_builder()),
            package_settings,
            no_implicit_file_export,
            simplify_unconditional_selects_in_rule_attrs,
            main_repository_mapping,
            generator_map,
            globber,
            enable_name_conflict_checking,
            track_full_macro_information,
            /* enable_target_map_snapshotting= */ false,
            package_limits,
            /* abstract_builder= */ true,
        );
        builder.piece = Some(PieceBuilderKind::ForBuildFile { identifier });
        builder
    }

    /// Java `PackagePiece.ForMacro.newBuilder`.
    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:420 PackagePiece.ForMacro.newBuilder
    pub fn new_package_piece_for_macro_builder(
        metadata: Arc<PackageMetadata>,
        declarations: Arc<PackageDeclarations>,
        evaluated_macro: Arc<MacroInstance>,
        parent_identifier: PackagePieceIdentifier,
        simplify_unconditional_selects_in_rule_attrs: bool,
        main_repository_mapping: RepositoryMapping,
        enable_name_conflict_checking: bool,
        track_full_macro_information: bool,
        package_limits: PackageLimits,
        existing_rules_map_for_finalizer: Option<IndexMap<String, Arc<Rule>>>,
    ) -> TargetDefinitionContext {
        // Java: PackagePiece.ForMacro.<init>
        blaze_util::check_argument!(
            metadata.package_identifier
                == evaluated_macro.get_package_metadata().package_identifier
        );
        blaze_util::check_argument!(
            metadata.package_identifier == *parent_identifier.get_package_identifier()
        );
        match evaluated_macro.get_parent() {
            Some(parent) => match &parent_identifier {
                PackagePieceIdentifier::ForMacro(id) => {
                    // Java: checkIdentifierMatchesMacro
                    blaze_util::check_argument!(
                        parent.get_package_metadata().package_identifier
                            == *id.get_package_identifier()
                    );
                    blaze_util::check_argument!(parent.get_name() == id.get_instance_name());
                }
                PackagePieceIdentifier::ForBuildFile(_) => {
                    panic!("IllegalArgumentException: the parent of a nested macro is a macro")
                }
            },
            None => blaze_util::check_argument!(matches!(
                parent_identifier,
                PackagePieceIdentifier::ForBuildFile(_)
            )),
        }
        let identifier = PackagePieceIdentifierForMacro::new(
            metadata.package_identifier.clone(),
            parent_identifier,
            evaluated_macro.get_name().to_owned(),
        );
        let declarations = Arc::new(declarations.copy_for_packageoid());
        let mut builder = TargetDefinitionContext::new_internal(
            metadata,
            declarations,
            PackageSettings::default(),
            /* no_implicit_file_export= */ false,
            simplify_unconditional_selects_in_rule_attrs,
            main_repository_mapping,
            /* generator_map= */ None,
            /* globber= */ None,
            enable_name_conflict_checking,
            track_full_macro_information,
            /* enable_target_map_snapshotting= */ false,
            package_limits,
            /* abstract_builder= */ false,
        );
        builder.piece = Some(PieceBuilderKind::ForMacro {
            identifier,
            evaluated_macro,
            existing_rules_map_for_finalizer,
        });
        builder
    }

    /// True for `Package.Builder` (not a piece builder).
    pub fn is_package_builder(&self) -> bool {
        self.piece.is_none()
    }

    /// The piece this builder builds, if it is a piece builder.
    pub fn get_piece_builder_kind(&self) -> Option<&PieceBuilderKind> {
        self.piece.as_ref()
    }

    /// Freezes rules and macros and builds the package piece (Java `finishBuild` of the piece
    /// builders, with their `packageoidInitializationHook`s).
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:866 TargetDefinitionContext.finishBuild
    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:300 PackagePiece.ForBuildFile.Builder.packageoidInitializationHook
    // Java: com/google/devtools/build/lib/packages/PackagePiece.java:512 PackagePiece.ForMacro.Builder.packageoidInitializationHook
    pub fn finish_build_piece(&mut self) -> Arc<PackagePiece> {
        if self.already_built {
            return self
                .built_piece
                .clone()
                .expect("package piece of a built builder");
        }
        self.already_built = true;

        for rule in self.recorder.get_rules() {
            rule.freeze();
        }
        for macro_ in self.recorder.get_macro_map().values() {
            macro_.freeze();
        }

        let piece_kind = self.piece.clone().expect("a package piece builder");
        let is_for_build_file = matches!(piece_kind, PieceBuilderKind::ForBuildFile { .. });
        if is_for_build_file {
            // Java: Package.AbstractBuilder.finalBuilderValidationHook
            self.final_builder_validation_hook();
        }

        self.packageoid_initialized = true;
        let contains_errors = self.contains_errors();
        let failure_detail = self.get_failure_detail();
        let targets: BTreeMap<String, Target> = self
            .recorder
            .get_target_map()
            .iter()
            .map(|(name, target)| (name.clone(), target.clone()))
            .collect();

        if is_for_build_file {
            // Java: com/google/devtools/build/lib/packages/Package.java:944 Package.AbstractBuilder.packageoidInitializationHook
            let contains_errors = self.declarations.contains_errors() || contains_errors;
            if !self.declarations.loads_set() {
                blaze_util::check_state!(contains_errors, "Loads not set for error-free package");
                let digest = blaze_util::Fingerprint::new().digest_and_reset();
                self.set_loads(Vec::new(), digest);
            }
            let make_env: IndexMap<String, String> = self
                .make_env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            self.declarations.set_make_environment(make_env).build();
            self.declarations.set_contains_errors(contains_errors);
            self.declarations.set_failure_detail(failure_detail.clone());
        }

        let macros_by_name: BTreeMap<String, Arc<MacroInstance>> = self
            .recorder
            .get_macro_map()
            .values()
            .map(|m| (m.get_name().to_owned(), m.clone()))
            .collect();
        let kind = match piece_kind {
            PieceBuilderKind::ForBuildFile { identifier } => PackagePieceKind::ForBuildFile {
                identifier,
                build_file: match &self.build_file {
                    Some(f) => f.clone(),
                    None => self.current_build_file_for_piece(),
                },
            },
            PieceBuilderKind::ForMacro {
                identifier,
                evaluated_macro,
                ..
            } => PackagePieceKind::ForMacro {
                identifier,
                evaluated_macro,
                macro_namespace_violations: self
                    .recorder
                    .get_macro_namespace_violating_targets()
                    .keys()
                    .cloned()
                    .collect(),
            },
        };
        let contains_errors = if is_for_build_file {
            self.declarations.contains_errors() || contains_errors
        } else {
            // PORT: the piece's error status, reachable from its targets (see
            // PackageDeclarations::copy_for_packageoid).
            self.declarations.set_contains_errors(contains_errors);
            self.declarations.set_failure_detail(failure_detail.clone());
            contains_errors
        };
        let piece = Arc::new(PackagePiece {
            kind,
            metadata: self.metadata.clone(),
            declarations: self.declarations.clone(),
            contains_errors,
            failure_detail,
            computation_steps: self.computation_steps,
            targets,
            macros_by_name,
            events: self.local_event_handler.get_events(),
            posts: self.local_event_handler.get_posts(),
        });
        if self.contains_errors() {
            blaze_util::check_state!(
                piece.contains_errors(),
                "Builder error status not propagated to package or package piece"
            );
        }
        self.built_piece = Some(piece.clone());
        piece
    }

    /// Java `Package.newPackageFromPackagePiecesBuilder`: a package builder for the package whose
    /// BUILD file piece has `metadata`, `declarations` and `build_file`.
    // Java: com/google/devtools/build/lib/packages/Package.java:617 Package.newPackageFromPackagePiecesBuilder
    pub fn new_package_from_package_pieces_builder(
        package_settings: PackageSettings,
        metadata: Arc<PackageMetadata>,
        declarations: Arc<PackageDeclarations>,
        no_implicit_file_export: bool,
        simplify_unconditional_selects_in_rule_attrs: bool,
        main_repository_mapping: RepositoryMapping,
        generator_map: Option<IndexMap<Location, String>>,
        globber: Option<Arc<dyn Globber>>,
        enable_name_conflict_checking: bool,
        track_full_macro_information: bool,
        package_limits: PackageLimits,
        build_file: Arc<InputFile>,
    ) -> TargetDefinitionContext {
        let declarations = Arc::new(declarations.copy_for_packageoid());
        let mut builder = TargetDefinitionContext::new_internal(
            metadata,
            declarations,
            package_settings,
            no_implicit_file_export,
            simplify_unconditional_selects_in_rule_attrs,
            main_repository_mapping,
            generator_map,
            globber,
            enable_name_conflict_checking,
            track_full_macro_information,
            /* enable_target_map_snapshotting= */ true,
            package_limits,
            /* abstract_builder= */ false,
        );
        // Java: the builder is a Package.Builder; its BUILD file target is the piece's.
        builder.build_file = Some(build_file.clone());
        builder.recorder.add_input_file_unchecked(build_file);
        builder
    }

    /// Adds the targets and macros of a piece to this package builder.
    // Java: com/google/devtools/build/lib/packages/Package.java:1197 Package.Builder.addAllFromPackagePiece
    pub fn add_all_from_package_piece(
        &mut self,
        package_piece: &PackagePiece,
    ) -> Result<(), NameConflictException> {
        // We add the BUILD file in newPackageFromPackagePiecesBuilder(), not here. (We want to
        // ensure that the package always has a BUILD file target, even if addAllFromPackagePiece
        // would throw a name conflict.)
        self.recorder
            .add_all_from_package_piece(package_piece, /* skip_build_file= */ true)?;
        self.computation_steps += package_piece.get_computation_steps();
        Ok(())
    }

    // PORT: Java's ForBuildFile builder records its BUILD file target in the AbstractBuilder
    // constructor; the current one is used if beforeBuild was not called.
    fn current_build_file_for_piece(&self) -> Arc<InputFile> {
        let name = self.metadata.build_file_label.get_name();
        match self.recorder.get_target_map().get(name) {
            Some(Target::InputFile(f)) => f.clone(),
            _ => panic!("IllegalStateException: no BUILD file target in the package piece"),
        }
    }
}

impl TargetRecorder {
    /// Adds the macros and targets of a piece (rule output files come with their rules).
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:457 TargetRecorder.addAllFromPackagePiece
    pub fn add_all_from_package_piece(
        &mut self,
        package_piece: &PackagePiece,
        skip_build_file: bool,
    ) -> Result<(), NameConflictException> {
        let prev = match package_piece.get_evaluated_macro() {
            Some(m) => self.set_current_macro_frame(Some(MacroFrame::new(m.clone()))),
            None => self.set_current_macro_frame(None),
        };
        let result = (|| {
            for macro_ in package_piece.get_macros() {
                self.add_macro(macro_)?;
            }
            let build_file = package_piece.get_build_file();
            for target in package_piece.get_targets().values() {
                if skip_build_file
                    && let (Target::InputFile(f), Some(b)) = (target, build_file)
                    && Arc::ptr_eq(f, b)
                {
                    continue;
                }
                if let Target::OutputFile(_) = target {
                    // Rule output files are recorded as a side effect of addTarget(rule)
                    continue;
                }
                self.add_target(target.clone())?;
            }
            Ok(())
        })();
        // PORT: Java leaves the frame set when a name conflict is thrown; the recorder is not used
        // again after that.
        let _unused = self.set_current_macro_frame(prev);
        result
    }
}
