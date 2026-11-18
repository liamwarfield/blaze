//! Port of `packages/Package.java` (with `Package.Metadata`, `Package.Declarations`,
//! `ConfigSettingVisibilityPolicy`, `Package.Builder.PackageSettings` / `PackageLimits`) and
//! `packages/Packageoid.java` (Package is the only packageoid ported; package pieces of lazy
//! macro expansion are not).
//!
//! The builder (Java `Package.Builder` / `AbstractBuilder` / `TargetDefinitionContext`) is
//! [`crate::package::TargetDefinitionContext`]; its `Package.Builder`-specific methods are in
//! this file.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_events::Event;
use blaze_events::EventHandler;
use blaze_events::Postable;
use blaze_starlark::Location;
use blaze_vfs::Path;
use blaze_vfs::PathFragment;
use blaze_vfs::Root;
use blaze_vfs::RootedPath;
use indexmap::IndexMap;
use parking_lot::RwLock;

use crate::exceptions::DetailedExitCode;
use crate::exceptions::NoSuchTargetException;
use crate::exceptions::PackageLoadingCode;
use crate::macros::MacroInstance;
use crate::package::Globber;
use crate::package::NameConflictException;
use crate::package::PackageArgs;
use crate::package::TargetDefinitionContext;
use crate::target::EnvironmentGroup;
use crate::target::InputFile;
use crate::target::Rule;
use crate::target::Target;

/// How to enforce `config_setting` visibility (Java `Package.ConfigSettingVisibilityPolicy`).
// Java: com/google/devtools/build/lib/packages/Package.java:113 Package.ConfigSettingVisibilityPolicy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfigSettingVisibilityPolicy {
    /// Don't enforce visibility for any config_setting.
    LegacyOff,
    /// Honor explicit visibility settings on config_setting, else use //visibility:public.
    DefaultPublic,
    /// Enforce config_setting visibility exactly the same as all other rules.
    DefaultStandard,
}

/// Package-level metadata known before BUILD evaluation (Java `Package.Metadata` record).
/// Shared by the package and its targets.
// Java: com/google/devtools/build/lib/packages/Package.java:1389 Package.Metadata
#[derive(Debug, Clone)]
pub struct PackageMetadata {
    pub package_identifier: PackageIdentifier,
    pub build_filename: RootedPath,
    pub build_file_label: Label,
    pub workspace_name: String,
    pub repository_mapping: RepositoryMapping,
    pub associated_module_name: Option<String>,
    pub associated_module_version: Option<String>,
    pub config_setting_visibility_policy: Option<ConfigSettingVisibilityPolicy>,
    pub succinct_target_not_found_errors: bool,
    pub source_root: Root,
}

impl PartialEq for PackageMetadata {
    // Java: record equality of all components.
    fn eq(&self, other: &PackageMetadata) -> bool {
        self.package_identifier == other.package_identifier
            && self.build_filename == other.build_filename
            && self.build_file_label == other.build_file_label
            && self.workspace_name == other.workspace_name
            && self.repository_mapping == other.repository_mapping
            && self.associated_module_name == other.associated_module_name
            && self.associated_module_version == other.associated_module_version
            && self.config_setting_visibility_policy == other.config_setting_visibility_policy
            && self.succinct_target_not_found_errors == other.succinct_target_not_found_errors
            && self.source_root == other.source_root
    }
}

impl PackageMetadata {
    /// Builds the metadata, computing the build file label and the source root.
    // Java: com/google/devtools/build/lib/packages/Package.java:1440 Package.Metadata.of
    pub fn of(
        package_identifier: PackageIdentifier,
        build_filename: RootedPath,
        workspace_name: String,
        repository_mapping: RepositoryMapping,
        associated_module_name: Option<String>,
        associated_module_version: Option<String>,
        config_setting_visibility_policy: Option<ConfigSettingVisibilityPolicy>,
        succinct_target_not_found_errors: bool,
    ) -> PackageMetadata {
        let build_file_label = match Label::create_in_package(
            &package_identifier,
            build_filename.get_root_relative_path().get_base_name(),
        ) {
            Ok(l) => l,
            // This can't actually happen.
            Err(_) => {
                panic!("AssertionError: Package BUILD file has an illegal name: {build_filename}")
            }
        };
        let source_root =
            PackageMetadata::compute_source_root(&package_identifier, &build_filename);
        PackageMetadata {
            package_identifier,
            build_filename,
            build_file_label,
            workspace_name,
            repository_mapping,
            associated_module_name,
            associated_module_version,
            config_setting_visibility_policy,
            succinct_target_not_found_errors,
            source_root,
        }
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1495 Package.Metadata.getName
    pub fn get_name(&self) -> &str {
        self.package_identifier
            .get_package_fragment()
            .get_path_string()
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1500 Package.Metadata.getPackageDirectory
    pub fn get_package_directory(&self) -> Path {
        PackageMetadata::get_package_directory_of(&self.build_filename)
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1508 Package.Metadata.getPackageDirectory
    fn get_package_directory_of(build_filename: &RootedPath) -> Path {
        build_filename
            .as_path()
            .get_parent_directory()
            .expect("NullPointerException: BUILD file has no parent directory")
    }

    /// The location of the BUILD file (no line or column).
    // Java: com/google/devtools/build/lib/packages/Package.java:1504 Package.Metadata.getBuildFileLocation
    pub fn get_build_file_location(&self) -> Location {
        Location {
            file: self.build_filename.as_path().get_path_string().to_owned(),
            line: 0,
            column: 0,
        }
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1512 Package.Metadata.computeSourceRoot
    fn compute_source_root(
        package_identifier: &PackageIdentifier,
        build_filename: &RootedPath,
    ) -> Root {
        let build_file_rooted_path = build_filename;
        let build_file_root = build_file_rooted_path.get_root();
        let pkg_id_fragment = package_identifier.get_source_root();
        let pkg_dir_fragment = build_file_rooted_path
            .get_root_relative_path()
            .get_parent_directory();

        let source_root = if Some(pkg_id_fragment) == pkg_dir_fragment.as_ref() {
            // Fast path: BUILD file path and package name are the same, don't create an extra
            // root.
            build_file_root.clone()
        } else {
            // TODO(bazel-team): Can this expr be simplified to just pkgDirFragment?
            let mut current = build_file_rooted_path
                .as_path()
                .as_fragment()
                .get_parent_directory();
            let len = pkg_id_fragment.segment_count();
            let mut i = 0;
            while i < len && current.is_some() {
                current = current.unwrap().get_parent_directory();
                i += 1;
            }
            match current {
                Some(current) if !current.is_empty() => {
                    // Note that current is an absolute path.
                    Root::from_path(build_file_root.get_relative(&current))
                }
                // This is never really expected to work. The below check should fail.
                _ => build_file_root.clone(),
            }
        };

        blaze_util::check_argument!(
            source_root.as_path().is_some()
                && source_root.get_relative(pkg_id_fragment)
                    == PackageMetadata::get_package_directory_of(build_filename),
            "Invalid BUILD file name for package '{}': {} (in source {} with packageDirectory {} and \
             package identifier source root {})",
            package_identifier,
            build_filename,
            source_root,
            PackageMetadata::get_package_directory_of(build_filename),
            package_identifier.get_source_root()
        );

        source_root
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DeclarationsInner {
    package_args: PackageArgs,
    make_env: IndexMap<String, String>,
    // PORT: Java keeps either the direct loads (Starlark Modules) or the transitive loads; the
    // loading code computes the transitive load labels and sets them here
    // (`setTransitiveLoads`).
    transitive_loads: Option<Vec<Label>>,
}

/// Package-level declarations made during BUILD evaluation: `package()` args, make variables
/// and loads (Java `Package.Declarations`; mutable through its `Builder` subclass until built).
/// Shared by the package and its targets.
// Java: com/google/devtools/build/lib/packages/Package.java:1556 Package.Declarations
#[derive(Debug)]
pub struct PackageDeclarations {
    inner: RwLock<DeclarationsInner>,
    /// Java `this instanceof Declarations.Builder`.
    mutable: AtomicBool,
    /// PORT: Java `Packageoid.containsErrors`, reachable from targets (which cannot point to the
    /// Package) through their shared declarations.
    contains_errors: AtomicBool,
    /// PORT: Java `Packageoid.failureDetail`, reachable from targets (see `contains_errors`).
    failure_detail: RwLock<Option<DetailedExitCode>>,
}

impl PackageDeclarations {
    /// A mutable instance (Java `new Declarations.Builder()`).
    // Java: com/google/devtools/build/lib/packages/Package.java:1758 Package.Declarations.Builder.<init>
    pub fn new_builder() -> PackageDeclarations {
        PackageDeclarations {
            inner: RwLock::new(DeclarationsInner {
                package_args: PackageArgs::default_args(),
                make_env: IndexMap::new(),
                transitive_loads: None,
            }),
            mutable: AtomicBool::new(true),
            contains_errors: AtomicBool::new(false),
            failure_detail: RwLock::new(None),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1608 Package.Declarations.getPackageArgs
    pub fn get_package_args(&self) -> PackageArgs {
        self.inner.read().package_args.clone()
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1612 Package.Declarations.getMakeEnvironment
    pub fn get_make_environment(&self) -> IndexMap<String, String> {
        self.inner.read().make_env.clone()
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1616 Package.Declarations.getOrComputeTransitivelyLoadedStarlarkFiles
    pub fn get_or_compute_transitively_loaded_starlark_files(&self) -> Vec<Label> {
        self.inner
            .read()
            .transitive_loads
            .clone()
            .expect("NullPointerException: Declarations not fully initialized")
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1633 Package.Declarations.countTransitivelyLoadedStarlarkFiles
    pub fn count_transitively_loaded_starlark_files(&self) -> usize {
        self.get_or_compute_transitively_loaded_starlark_files()
            .len()
    }

    /// Panics unless still mutable (Java `checkMutable`).
    // Java: com/google/devtools/build/lib/packages/Package.java:1655 Package.Declarations.checkMutable
    pub fn check_mutable(&self) -> &Self {
        if !self.mutable.load(Ordering::Acquire) {
            panic!(
                "IllegalStateException: Package declarations has been finalized and is immutable."
            );
        }
        self
    }

    /// Panics if still mutable (Java `checkImmutable`).
    // Java: com/google/devtools/build/lib/packages/Package.java:1663 Package.Declarations.checkImmutable
    pub fn check_immutable(&self) -> &Self {
        if self.mutable.load(Ordering::Acquire) {
            panic!("IllegalStateException: Package declarations is in mutable state.");
        }
        self
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1717 Package.Declarations.Builder.setPackageArgs
    pub fn set_package_args(&self, package_args: PackageArgs) -> &Self {
        self.check_mutable();
        self.inner.write().package_args = package_args;
        self
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1723 Package.Declarations.Builder.mergePackageArgsFrom
    pub fn merge_package_args_from(&self, package_args: &PackageArgs) -> &Self {
        self.check_mutable();
        let mut inner = self.inner.write();
        inner.package_args = inner.package_args.merge_with(package_args);
        drop(inner);
        self
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1729 Package.Declarations.Builder.setMakeEnvironment
    pub fn set_make_environment(&self, make_env: IndexMap<String, String>) -> &Self {
        self.check_mutable();
        self.inner.write().make_env = make_env;
        self
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1741 Package.Declarations.Builder.setTransitiveLoads
    pub fn set_transitive_loads(&self, transitive_loads: Vec<Label>) -> &Self {
        self.check_mutable();
        let mut inner = self.inner.write();
        // Java: com/google/devtools/build/lib/packages/Package.java:1747 Package.Declarations.Builder.checkLoadsNotSet
        if let Some(t) = &inner.transitive_loads {
            panic!("IllegalStateException: Transitive loads already set: {t:?}");
        }
        inner.transitive_loads = Some(transitive_loads);
        drop(inner);
        self
    }

    /// Makes the declarations immutable (Java `Declarations.Builder.build()`).
    // Java: com/google/devtools/build/lib/packages/Package.java:1753 Package.Declarations.Builder.build
    pub fn build(&self) {
        self.mutable.store(false, Ordering::Release);
    }

    /// Visits the labels of the .bzl files transitively loaded by the package (Java
    /// `visitLoadGraph`). The loads are always precomputed here, so each file is passed to
    /// `visitor` once regardless of its return value.
    // PORT: Java's LoadGraphVisitor returns whether to recurse; it is ignored for precomputed
    // transitive loads, the only form stored here.
    // Java: com/google/devtools/build/lib/packages/Package.java:1665 Package.Declarations.visitLoadGraph
    pub fn visit_load_graph<E>(
        &self,
        visitor: &mut dyn FnMut(&Label) -> Result<bool, E>,
    ) -> Result<(), E> {
        let transitive_loads = self
            .inner
            .read()
            .transitive_loads
            .clone()
            .expect("NullPointerException: Declarations not fully initialized");
        for load in &transitive_loads {
            visitor(load)?;
        }
        Ok(())
    }

    /// Java `Packageoid.containsErrors()` (see the field).
    pub fn contains_errors(&self) -> bool {
        self.contains_errors.load(Ordering::Acquire)
    }

    pub(crate) fn set_contains_errors(&self, value: bool) {
        self.contains_errors.store(value, Ordering::Release);
    }

    /// Java `Packageoid.getFailureDetail()`, reachable from targets (see `contains_errors`).
    pub fn get_failure_detail(&self) -> Option<DetailedExitCode> {
        self.failure_detail.read().clone()
    }

    pub(crate) fn set_failure_detail(&self, failure_detail: Option<DetailedExitCode>) {
        *self.failure_detail.write() = failure_detail;
    }

    /// True if the transitive loads have been set (Java `transitiveLoads != null`).
    pub(crate) fn loads_set(&self) -> bool {
        self.inner.read().transitive_loads.is_some()
    }
}

impl PartialEq for PackageDeclarations {
    // Java: com/google/devtools/build/lib/packages/Package.java:1573 Package.Declarations.equals
    fn eq(&self, other: &PackageDeclarations) -> bool {
        *self.inner.read() == *other.inner.read()
    }
}

/// Java `Package.Builder.PackageSettings` (an interface with defaults).
// Java: com/google/devtools/build/lib/packages/Package.java:1039 Package.Builder.PackageSettings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PackageSettings {
    pub succinct_target_not_found_errors: bool,
    pub precompute_transitive_loads: bool,
}

/// Java `Package.Builder.PackageLimits`.
// Java: com/google/devtools/build/lib/packages/Package.java:1073 Package.Builder.PackageLimits
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageLimits {
    pub max_starlark_computation_steps_per_package: i64,
}

impl Default for PackageLimits {
    // Java: com/google/devtools/build/lib/packages/Package.java:1079 Package.Builder.PackageLimits.DEFAULTS
    fn default() -> Self {
        PackageLimits {
            max_starlark_computation_steps_per_package: i64::MAX,
        }
    }
}

/// A loaded package: its targets and symbolic macros (Java `Package`). Immutable; Skyframe
/// shares it as `Arc<Package>`.
// Java: com/google/devtools/build/lib/packages/Package.java:96 Package
#[derive(Debug)]
pub struct Package {
    // Java: Packageoid fields.
    metadata: Arc<PackageMetadata>,
    declarations: Arc<PackageDeclarations>,
    contains_errors: bool,
    failure_detail: Option<DetailedExitCode>,
    computation_steps: i64,
    /// Java `ImmutableSortedMap<String, Target>`.
    // PORT: Java orders by String.compareTo (UTF-16 units); BTreeMap orders by UTF-8 bytes, the
    // same order for the BMP (and for every corpus name).
    targets: BTreeMap<String, Target>,
    // Java: Package fields.
    build_file: Arc<InputFile>,
    /// By macro id (Java `ImmutableSortedMap`).
    macros: BTreeMap<String, Arc<MacroInstance>>,
    macro_namespace_violating_targets: IndexMap<String, String>,
    targets_to_declaring_macro: Option<IndexMap<String, Arc<MacroInstance>>>,
    targets_to_declaring_package: Option<IndexMap<String, PackageIdentifier>>,
    /// The events recorded while building the package, replayed by PACKAGE (Java: the
    /// builder's `localEventHandler`, which PackageFunction keeps with the value).
    events: Vec<Event>,
    posts: Vec<Arc<dyn Postable>>,
}

impl Package {
    // Java: com/google/devtools/build/lib/packages/Packageoid.java:90 Packageoid.getMetadata
    pub fn get_metadata(&self) -> &Arc<PackageMetadata> {
        &self.metadata
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:98 Packageoid.getPackageIdentifier
    pub fn get_package_identifier(&self) -> &PackageIdentifier {
        &self.metadata.package_identifier
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:106 Packageoid.getDeclarations
    pub fn get_declarations(&self) -> &Arc<PackageDeclarations> {
        &self.declarations
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:117 Packageoid.getBuildFileLabel
    pub fn get_build_file_label(&self) -> &Label {
        &self.metadata.build_file_label
    }

    /// All targets by name, sorted.
    // Java: com/google/devtools/build/lib/packages/Packageoid.java:132 Packageoid.getTargets
    pub fn get_targets(&self) -> &BTreeMap<String, Target> {
        &self.targets
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:143 Packageoid.containsErrors
    pub fn contains_errors(&self) -> bool {
        self.contains_errors
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:168 Packageoid.getFailureDetail
    pub fn get_failure_detail(&self) -> Option<&DetailedExitCode> {
        self.failure_detail.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/Packageoid.java:176 Packageoid.getComputationSteps
    pub fn get_computation_steps(&self) -> i64 {
        self.computation_steps
    }

    /// The events reported while building this package, in order.
    pub fn get_events(&self) -> &[Event] {
        &self.events
    }

    pub fn get_posts(&self) -> &[Arc<dyn Postable>] {
        &self.posts
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:205 Package.getName
    pub fn get_name(&self) -> &str {
        self.metadata.get_name()
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:210 Package.getNameFragment
    pub fn get_name_fragment(&self) -> &PathFragment {
        self.metadata.package_identifier.get_package_fragment()
    }

    /// The BUILD file's path.
    // Java: com/google/devtools/build/lib/packages/Package.java:218 Package.getFilename
    pub fn get_filename(&self) -> &RootedPath {
        &self.metadata.build_filename
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:223 Package.getPackageDirectory
    pub fn get_package_directory(&self) -> Path {
        self.metadata.get_package_directory()
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:232 Package.getConfigSettingVisibilityPolicy
    pub fn get_config_setting_visibility_policy(&self) -> Option<ConfigSettingVisibilityPolicy> {
        self.metadata.config_setting_visibility_policy
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:237 Package.getWorkspaceName
    pub fn get_workspace_name(&self) -> &str {
        &self.metadata.workspace_name
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:242 Package.getBuildFile
    pub fn get_build_file(&self) -> &Arc<InputFile> {
        &self.build_file
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:247 Package.getPackageArgs
    pub fn get_package_args(&self) -> PackageArgs {
        self.declarations.get_package_args()
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:252 Package.getMakeEnvironment
    pub fn get_make_environment(&self) -> IndexMap<String, String> {
        self.declarations.get_make_environment()
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:262 Package.getSourceRoot
    pub fn get_source_root(&self) -> &Root {
        &self.metadata.source_root
    }

    /// The rules of this package, in name order (Java `getTargets(Rule.class)`).
    // Java: com/google/devtools/build/lib/packages/Package.java:278 Package.getTargets
    pub fn get_rules(&self) -> impl Iterator<Item = &Arc<Rule>> {
        self.targets.values().filter_map(Target::as_rule)
    }

    /// The rule named `target_name`, or `None` if there is none (or it is not a rule).
    // Java: com/google/devtools/build/lib/packages/Package.java:286 Package.getRule
    pub fn get_rule(&self, target_name: &str) -> Option<&Arc<Rule>> {
        self.targets.get(target_name).and_then(Target::as_rule)
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:295 Package.getMacroNamespaceViolatingTargets
    pub fn get_macro_namespace_violating_targets(&self) -> &IndexMap<String, String> {
        &self.macro_namespace_violating_targets
    }

    /// Fails if `target` violates the naming rules of its declaring macro.
    // Java: com/google/devtools/build/lib/packages/Package.java:318 Package.checkMacroNamespaceCompliance
    pub fn check_macro_namespace_compliance(
        &self,
        target: &Target,
    ) -> Result<(), NameConflictException> {
        blaze_util::check_argument!(
            target.get_package_metadata().package_identifier == self.metadata.package_identifier,
            "Target must belong to this package"
        );
        if let Some(macro_namespace_violated) = self
            .macro_namespace_violating_targets
            .get(target.get_name())
        {
            return Err(NameConflictException {
                message: format!(
                    "Target {} declared in symbolic macro '{}' violates macro naming rules and \
                     cannot be built. {}",
                    target.get_label(),
                    macro_namespace_violated,
                    crate::package::MACRO_NAMING_RULES
                ),
                target: Some(target.clone()),
                macro_: None,
                is_macro_namespace_violation: true,
            });
        }
        Ok(())
    }

    /// The target named `target_name`, or Java's "no such target" error (with suggestions).
    // Java: com/google/devtools/build/lib/packages/Package.java:335 Package.getTarget
    pub fn get_target(&self, target_name: &str) -> Result<&Target, NoSuchTargetException> {
        if let Some(target) = self.targets.get(target_name) {
            return Ok(target);
        }

        let label = match Label::create_in_package(&self.metadata.package_identifier, target_name) {
            Ok(label) => label,
            Err(e) => panic!("IllegalArgumentException: {target_name}: {e}"),
        };

        if self.metadata.succinct_target_not_found_errors {
            Err(NoSuchTargetException::with_label(
                label,
                &format!(
                    "target '{}' not declared in package '{}'",
                    target_name,
                    self.get_name()
                ),
            ))
        } else {
            let other_targets: Vec<&str> = self.targets.keys().map(String::as_str).collect();
            let alternate_target_suggestion = Package::get_alternate_target_suggestion(
                &self.metadata,
                target_name,
                &other_targets,
            );
            Err(NoSuchTargetException::with_label(
                label,
                &format!(
                    "target '{}' not declared in package '{}' defined by {}{}",
                    target_name,
                    self.get_name(),
                    self.metadata.build_filename.as_path().get_path_string(),
                    alternate_target_suggestion
                ),
            ))
        }
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:365 Package.getAlternateTargetSuggestion
    pub fn get_alternate_target_suggestion(
        metadata: &PackageMetadata,
        target_name: &str,
        other_targets: &[&str],
    ) -> String {
        // If there's a file on the disk that's not mentioned in the BUILD file,
        // produce a more informative error.  NOTE! this code path is only executed
        // on failure, which is (relatively) very rare.  In the common case no
        // stat(2) is executed.
        if !PathFragment::is_normalized(target_name) || target_name == "*" {
            // Don't check for file existence if the target name is not normalized
            // because the error message would be confusing and wrong. If the
            // targetName is "foo/bar/.", and there is a directory "foo/bar", it
            // doesn't mean that "//pkg:foo/bar/." is a valid label.
            // Also don't check if the target name is a single * character since
            // it's invalid on Windows.
            return String::new();
        }
        let filename = metadata
            .get_package_directory()
            .get_relative_str(target_name);
        if filename.is_directory() {
            format!(
                "; however, a source directory of this name exists.  (Perhaps add \
                 'exports_files([\"{}\"])' to {}, or define a filegroup?)",
                target_name,
                Package::get_repo_relative_build_file_path_string(metadata)
            )
        } else if filename.exists() {
            format!(
                "; however, a source file of this name exists.  (Perhaps add \
                 'exports_files([\"{}\"])' to {}?)",
                target_name,
                Package::get_repo_relative_build_file_path_string(metadata)
            )
        } else {
            crate::package::target_suggester::suggest_targets(target_name, other_targets)
        }
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:400 Package.getRepoRelativeBuildFilePathString
    fn get_repo_relative_build_file_path_string(metadata: &PackageMetadata) -> String {
        metadata
            .package_identifier
            .get_package_fragment()
            .get_relative_str(metadata.build_filename.as_path().get_base_name())
            .get_path_string()
            .to_owned()
    }

    /// Symbolic macro instances by id.
    // Java: com/google/devtools/build/lib/packages/Package.java:415 Package.getMacrosById
    pub fn get_macros_by_id(&self) -> &BTreeMap<String, Arc<MacroInstance>> {
        &self.macros
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:429 Package.getDeclaringMacroForTarget
    pub fn get_declaring_macro_for_target(&self, target: &str) -> Option<&Arc<MacroInstance>> {
        let targets_to_declaring_macro = self.targets_to_declaring_macro.as_ref();
        blaze_util::check_state!(
            targets_to_declaring_macro.is_some(),
            "Cannot retrieve MacroInstance information from deserialized packages"
        );
        blaze_util::check_argument!(
            self.targets.contains_key(target),
            "unknown target '{}'",
            target
        );
        targets_to_declaring_macro.unwrap().get(target)
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:448 Package.getDeclaringPackageForTargetIfInMacro
    pub fn get_declaring_package_for_target_if_in_macro(
        &self,
        target: &str,
    ) -> Option<PackageIdentifier> {
        blaze_util::check_argument!(
            self.targets.contains_key(target),
            "unknown target '{}'",
            target
        );
        // Exactly one of targetsToDeclaringMacro and targetsToDeclaringPackage is non-null,
        // depending on whether this package was produced by deserialization.
        if let Some(targets_to_declaring_macro) = &self.targets_to_declaring_macro {
            targets_to_declaring_macro
                .get(target)
                .map(|m| m.get_definition_package().clone())
        } else {
            self.targets_to_declaring_package
                .as_ref()
                .expect("NullPointerException: targetsToDeclaringPackage")
                .get(target)
                .cloned()
        }
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:471 Package.getShortDescription
    pub fn get_short_description(&self) -> String {
        format!(
            "package {}",
            self.metadata.package_identifier.get_canonical_form()
        )
    }

    /// Prints a human-readable dump of the package (tests).
    // Java: com/google/devtools/build/lib/packages/Package.java:479 Package.dump
    pub fn dump(&self, out: &mut dyn std::io::Write) -> std::io::Result<()> {
        use crate::attribute::AggregatingAttributeMapper;
        use crate::target::RuleOrMacroInstance;
        writeln!(
            out,
            "  Package {} ({})",
            self.get_name(),
            self.metadata.build_filename.as_path().get_path_string()
        )?;

        // Rules:
        writeln!(out, "    Rules")?;
        for rule in self.get_rules() {
            writeln!(
                out,
                "      {} {}",
                crate::target::TargetData::get_target_kind(&**rule),
                rule.get_label()
            )?;
            for attr in rule.get_attributes() {
                for possible_value in AggregatingAttributeMapper::of(&**rule)
                    .visit_attribute(attr.get_name(), attr.get_type())
                {
                    // PORT: Java prints the value's toString; values print with Debug here.
                    writeln!(out, "        {} = {:?}", attr.get_name(), possible_value)?;
                }
            }
        }

        // Files:
        writeln!(out, "    Files")?;
        for target in self.targets.values() {
            match target {
                Target::InputFile(file) => {
                    writeln!(
                        out,
                        "      {} {}",
                        target.get_target_kind(),
                        file.get_label()
                    )?;
                }
                Target::OutputFile(file) => {
                    writeln!(
                        out,
                        "      {} {} (generated by {})",
                        target.get_target_kind(),
                        file.get_label(),
                        file.get_generating_rule().get_label()
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// An error event with a package loading code (Java `Package.error`).
    // Java: com/google/devtools/build/lib/packages/Package.java:511 Package.error
    pub fn error(location: Option<&Location>, message: &str, code: PackageLoadingCode) -> Event {
        Package::error_with_detailed_exit_code(
            location,
            message,
            DetailedExitCode::package_loading(message, code),
        )
    }

    /// Like [`Package::error`] with a custom detailed exit code.
    // Java: com/google/devtools/build/lib/packages/Package.java:523 Package.errorWithDetailedExitCode
    pub fn error_with_detailed_exit_code(
        location: Option<&Location>,
        message: &str,
        detailed_exit_code: DetailedExitCode,
    ) -> Event {
        let error = Event::error_at(location.map(to_event_location), message);
        error.with_property(Some(detailed_exit_code))
    }

    /// Reports "package contains errors: ..." if the package is in error.
    // Java: com/google/devtools/build/lib/packages/Package.java:533 Package.maybeAddPackageContainsErrorsEventToHandler
    pub fn maybe_add_package_contains_errors_event_to_handler(
        pkg: &Package,
        event_handler: &dyn blaze_events::EventHandler,
    ) {
        if pkg.contains_errors() {
            event_handler.handle(&Event::error(format!(
                "package contains errors: {}{}",
                pkg.get_name_fragment(),
                match pkg.get_failure_detail() {
                    Some(d) => format!(": {}", d.message),
                    None => String::new(),
                }
            )));
        }
    }

    /// The failure detail attributed to `target` (Java `contextualizeFailureDetailForTarget`).
    // Java: com/google/devtools/build/lib/packages/Package.java:553 Package.contextualizeFailureDetailForTarget
    pub fn contextualize_failure_detail_for_target(
        failure_detail: Option<&DetailedExitCode>,
        target: &Target,
    ) -> DetailedExitCode {
        let prefix = format!(
            "Target '{}' contains an error and its package is in error",
            target.get_label()
        );
        match failure_detail {
            None => DetailedExitCode::package_loading(prefix, PackageLoadingCode::TargetMissing),
            Some(d) => DetailedExitCode {
                message: format!("{}: {}", prefix, d.message),
                code: d.code.clone(),
            },
        }
    }

    /// Creates a builder for a package (Java `Package.newPackageBuilder`). The returned
    /// [`TargetDefinitionContext`] is the BUILD thread's payload.
    // PORT: cpuBoundSemaphore, packageOverheadEstimator and the symbol generator are dropped
    // (single build per process; memory accounting).
    // Java: com/google/devtools/build/lib/packages/Package.java:570 Package.newPackageBuilder
    pub fn new_package_builder(
        package_settings: PackageSettings,
        id: PackageIdentifier,
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
            id,
            filename,
            workspace_name,
            repository_mapping,
            associated_module_name,
            associated_module_version,
            config_setting_visibility_policy,
            package_settings.succinct_target_not_found_errors,
        );
        TargetDefinitionContext::new_package_builder_internal(
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
            package_limits,
        )
    }

    /// Assembles a package from a finished builder (used by `finish_build`).
    pub(crate) fn from_parts(
        metadata: Arc<PackageMetadata>,
        declarations: Arc<PackageDeclarations>,
        contains_errors: bool,
        failure_detail: Option<DetailedExitCode>,
        computation_steps: i64,
        targets: BTreeMap<String, Target>,
        build_file: Arc<InputFile>,
        macros: BTreeMap<String, Arc<MacroInstance>>,
        macro_namespace_violating_targets: IndexMap<String, String>,
        targets_to_declaring_macro: Option<IndexMap<String, Arc<MacroInstance>>>,
        targets_to_declaring_package: Option<IndexMap<String, PackageIdentifier>>,
        events: Vec<Event>,
        posts: Vec<Arc<dyn Postable>>,
    ) -> Package {
        Package {
            metadata,
            declarations,
            contains_errors,
            failure_detail,
            computation_steps,
            targets,
            build_file,
            macros,
            macro_namespace_violating_targets,
            targets_to_declaring_macro,
            targets_to_declaring_package,
            events,
            posts,
        }
    }

    pub fn get_targets_to_declaring_macro(&self) -> Option<&IndexMap<String, Arc<MacroInstance>>> {
        self.targets_to_declaring_macro.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:306 Package.getTargetsToDeclaringPackage
    pub fn get_targets_to_declaring_package(&self) -> Option<&IndexMap<String, PackageIdentifier>> {
        self.targets_to_declaring_package.as_ref()
    }
}

/// Reports labels that appear more than once (Java `AbstractBuilder.hasDuplicateLabels`).
// Java: com/google/devtools/build/lib/packages/Package.java:850 Package.AbstractBuilder.hasDuplicateLabels
fn has_duplicate_labels(
    labels: &[Label],
    owner: &str,
    attr_name: &str,
    location: &Location,
    event_handler: &dyn blaze_events::EventHandler,
) -> bool {
    // Java: com/google/devtools/build/lib/collect/CollectionUtils.java:37 CollectionUtils.duplicatedElementsOf
    // PORT: java-hash-order. Java collects the duplicates in a HashSet; they are reported here in
    // order of second occurrence.
    let mut seen = std::collections::HashSet::new();
    let mut dupes: indexmap::IndexSet<&Label> = indexmap::IndexSet::new();
    if labels.len() >= 2 {
        for label in labels {
            if !seen.insert(label) {
                dupes.insert(label);
            }
        }
    }
    for dupe in &dupes {
        event_handler.handle(&Package::error(
            Some(location),
            &format!("label '{dupe}' is duplicated in the '{attr_name}' list of '{owner}'"),
            PackageLoadingCode::DuplicateLabel,
        ));
    }
    !dupes.is_empty()
}

impl fmt::Display for Package {
    // Java: com/google/devtools/build/lib/packages/Package.java:463 Package.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Java prints the Iterables.filter view: "[rule1, rule2]".
        write!(f, "Package({})=[", self.get_name())?;
        for (i, rule) in self.get_rules().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{rule}")?;
        }
        f.write_str("]")
    }
}

/// A Starlark location as the plain-data location events carry.
pub(crate) fn to_event_location(location: &Location) -> blaze_events::Location {
    blaze_events::Location::new(
        location.file.clone(),
        location.line as i32,
        location.column as i32,
    )
}

// Java `Package.AbstractBuilder` and `Package.Builder` methods of the package builder.
impl TargetDefinitionContext {
    /// Java `AbstractBuilder.fromOrFailAllowBuildOnly(thread, what)`: the builder, failing
    /// inside macros.
    // Java: com/google/devtools/build/lib/packages/Package.java:696 Package.AbstractBuilder.fromOrFailAllowBuildOnly
    pub fn from_or_fail_allow_build_only<'a>(
        eval: &starlark::eval::Evaluator<'_, 'a, '_>,
        what: &str,
    ) -> Result<std::cell::RefMut<'a, TargetDefinitionContext>, blaze_cmdline::EvalException> {
        Self::from_or_fail_allow_build_only_participle(eval, what, "used")
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:680 Package.AbstractBuilder.fromOrFailAllowBuildOnly
    pub fn from_or_fail_allow_build_only_participle<'a>(
        eval: &starlark::eval::Evaluator<'_, 'a, '_>,
        what: &str,
        participle: &str,
    ) -> Result<std::cell::RefMut<'a, TargetDefinitionContext>, blaze_cmdline::EvalException> {
        if let Some(builder) = TargetDefinitionContext::from_or_null(eval)
            && builder.recorder.get_current_macro_frame().is_none()
        {
            return Ok(builder);
        }
        let symbolic_macros_enabled = match blaze_starlark::ThreadContext::get(eval) {
            Ok(cx) => cx.semantics.experimental_enable_first_class_macros,
            Err(_) => blaze_util::unported!("Package.AbstractBuilder.fromOrFailAllowBuildOnly"),
        };
        Err(TargetDefinitionContext::new_from_or_fail_exception(
            what,
            participle,
            symbolic_macros_enabled,
            &[crate::package::FromOrFailMode::NoMacros],
        ))
    }

    /// Records the transitive .bzl loads and their digest.
    // PORT: Java's `setLoads(Iterable<Module>)` walks the modules' BazelModuleContexts; the
    // loading code passes the transitive load labels (in Java's `visitLoadGraphRecursively`
    // order) and the digest Java computes (a Fingerprint over the direct loads'
    // `bzlTransitiveDigest`s) instead. Transitive loads are always "precomputed".
    // Java: com/google/devtools/build/lib/packages/Package.java:720 Package.AbstractBuilder.setLoads
    pub fn set_loads(&mut self, transitive_loads: Vec<Label>, transitive_bzl_digest: Vec<u8>) {
        self.declarations
            .check_mutable()
            .set_transitive_loads(transitive_loads);
        self.transitive_bzl_digest = transitive_bzl_digest;
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:736 Package.AbstractBuilder.getTransitiveBzlDigest
    pub fn get_transitive_bzl_digest(&self) -> &[u8] {
        blaze_util::check_state!(!self.transitive_bzl_digest.is_empty());
        &self.transitive_bzl_digest
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:753 Package.AbstractBuilder.mergePackageArgsFrom
    pub fn merge_package_args_from(&mut self, package_args: &PackageArgs) {
        self.declarations
            .check_mutable()
            .merge_package_args_from(package_args);
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:761 Package.AbstractBuilder.setMakeVariable
    pub fn set_make_variable(&mut self, name: &str, value: &str) {
        self.make_env.insert(name.to_owned(), value.to_owned());
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:766 Package.AbstractBuilder.isPackageFunctionUsed
    pub fn is_package_function_used(&self) -> bool {
        self.package_function_used
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:770 Package.AbstractBuilder.setPackageFunctionUsed
    pub fn set_package_function_used(&mut self) {
        self.package_function_used = true;
    }

    /// Creates an environment group target, reporting errors.
    // Java: com/google/devtools/build/lib/packages/Package.java:779 Package.AbstractBuilder.addEnvironmentGroup
    pub fn add_environment_group(
        &mut self,
        name: &str,
        environments: Vec<Label>,
        defaults: Vec<Label>,
        location: Location,
    ) -> Result<(), crate::package::PackageBuilderError> {
        blaze_util::check_state!(self.current_macro().is_none());
        let event_handler = &*self.local_event_handler;

        let has_dupes_envs = has_duplicate_labels(
            &environments,
            name,
            "environments",
            &location,
            event_handler,
        );
        if has_dupes_envs
            || has_duplicate_labels(&defaults, name, "defaults", &location, event_handler)
        {
            self.set_contains_errors();
            return Ok(());
        }

        let mut group = EnvironmentGroup::new(
            self.create_label(name)?,
            self.metadata.clone(),
            self.declarations.clone(),
            environments,
            defaults,
            location.clone(),
        );

        // Invariant: once group is inserted into targets, it must also:
        // (a) be inserted into environmentGroups, or
        // (b) have its group.processMemberEnvironments called.
        // Otherwise it will remain uninitialized,
        // causing crashes when it is later toString-ed.
        // PORT: Java adds the group before these checks and mutates it in place; the Arc is
        // shared with the targets map, so the orphan initialization happens before insertion.
        let errors = group.validate_membership();
        let mut other_groups: Vec<(Label, Option<Arc<EnvironmentGroup>>)> = Vec::new();
        for environment in group.get_environments() {
            other_groups.push((
                environment.clone(),
                self.environment_groups.get(environment).cloned(),
            ));
        }
        if other_groups.iter().any(|(_, other)| other.is_some()) {
            // Ensure the orphan gets (trivially) initialized.
            let _ = group.process_member_environments(&IndexMap::new());
        }
        let group = Arc::new(group);
        self.recorder
            .add_target(Target::EnvironmentGroup(group.clone()))?;

        for error in errors {
            self.local_event_handler.handle(&error);
            self.set_contains_errors();
        }

        // For each declared environment, make sure it doesn't also belong to some other group.
        for (environment, other_group) in other_groups {
            if let Some(other_group) = other_group {
                self.local_event_handler.handle(&Package::error(
                    Some(&location),
                    &format!(
                        "environment {} belongs to both {} and {}",
                        environment,
                        group.get_label(),
                        other_group.get_label()
                    ),
                    PackageLoadingCode::EnvironmentInMultipleGroups,
                ));
                self.set_contains_errors();
            } else {
                self.environment_groups.insert(environment, group.clone());
            }
        }
        Ok(())
    }

    /// Adds a rule without checks (deserialization, WORKSPACE).
    // Java: com/google/devtools/build/lib/packages/Package.java:1196 Package.Builder.addRuleUnchecked
    pub fn add_rule_unchecked(&mut self, rule: Arc<Rule>) {
        self.recorder.add_rule_unchecked(rule)
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1236 Package.Builder.markMacroComplete
    pub fn mark_macro_complete(&mut self, macro_: &MacroInstance) {
        let id = macro_.get_id();
        if !self.unexpanded_macros.shift_remove(&id) {
            panic!("IllegalArgumentException: Macro id '{id}' unknown or already marked complete");
        }
    }

    /// Expands the symbolic macros not yet expanded (finalizers last).
    // Java: com/google/devtools/build/lib/packages/Package.java:1251 Package.Builder.expandAllRemainingMacros
    pub fn expand_all_remaining_macros(
        &mut self,
        semantics: &blaze_starlark::StarlarkSemantics,
    ) -> Result<(), blaze_cmdline::EvalException> {
        // Finalizer expansion step. Requires that the package not be in error (no point in
        // finalizing a package that already threw an EvalException).
        if !self.contains_errors() && !self.unexpanded_macros.is_empty() {
            blaze_util::check_state!(
                self.unexpanded_macros.iter().all(|id| self
                    .recorder
                    .get_macro_map()
                    .get(id)
                    .expect("macro id")
                    .get_macro_class()
                    .is_finalizer()),
                "At the beginning of finalizer expansion, unexpandedMacros must contain only \
                 finalizers"
            );

            // Save a snapshot of rule targets for use by native.existing_rules() inside all
            // finalizers. We must take this snapshot before calling any finalizer because the
            // snapshot must not include any rule instantiated by a finalizer or macro called from
            // a finalizer.
            if self.rules_snapshot_view_for_finalizers.is_none() {
                blaze_util::check_state!(
                    !self.target_map_unwrapped,
                    "Cannot call expandAllRemainingMacros() after beforeBuild() has been called"
                );
                self.rules_snapshot_view_for_finalizers = Some(self.get_rules_snapshot_view());
            }

            while let Some(id) = self.unexpanded_macros.first().cloned() {
                // NB: collection mutated by body
                let macro_ = self
                    .recorder
                    .get_macro_map()
                    .get(&id)
                    .expect("macro id")
                    .clone();
                crate::macros::MacroClass::execute_macro_implementation(&macro_, self, semantics)?;
            }
        }
        Ok(())
    }
}
