//! Port of `packages/TargetDefinitionContext.java`, merged with the builder classes that
//! extend it, `Package.AbstractBuilder` and `Package.Builder` (the only subclasses ported; the
//! `Package.Builder`-specific methods are in [`crate::package::package`]).
//!
//! It is the phase payload of BUILD-file and macro evaluation threads
//! (`blaze_starlark::ThreadContext::with_payload`); natives get it with
//! [`TargetDefinitionContext::from_or_fail`] (Java `TargetDefinitionContext.fromOrFail`).
//! A native must drop the returned `RefMut` before calling back into Starlark.

use std::cell::RefMut;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;

use blaze_cmdline::EvalException;
use blaze_cmdline::Label;
use blaze_cmdline::LabelSyntaxException;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_events::EventHandler;
use blaze_events::EventKind;
use blaze_events::StoredEventHandler;
use blaze_starlark::Location;
use blaze_starlark::ThreadContext;
use blaze_starlark::error::StackEntry;
use blaze_vfs::RootedPath;
use indexmap::IndexMap;
use indexmap::IndexSet;
use starlark::eval::Evaluator;

use crate::exceptions::DetailedExitCode;
use crate::exceptions::NoSuchPackageException;
use crate::macros::MacroClass;
use crate::macros::MacroInstance;
use crate::package::CallStackNode;
use crate::package::Globber;
use crate::package::MacroFrame;
use crate::package::NameConflictException;
use crate::package::Package;
use crate::package::PackageArgs;
use crate::package::PackageDeclarations;
use crate::package::PackageLimits;
use crate::package::PackageMetadata;
use crate::package::PackageSettings;
use crate::package::RuleVisibility;
use crate::package::TargetRecorder;
use crate::package::call_stack;
use crate::package::test_suite_implicit_tests_accumulator::TestSuiteImplicitTestsAccumulator;
use crate::rule_class::RuleClass;
use crate::target::EnvironmentGroup;
use crate::target::ImplicitTestsRef;
use crate::target::InputFile;
use crate::target::InputFileKind;
use crate::target::PackageGroup;
use crate::target::Rule;
use crate::target::RuleOrMacroInstance;
use crate::target::Target;
use crate::types::LabelConverter;
use crate::types::License;

/// Java `TargetDefinitionContext.FromOrFailMode`.
// Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:214 TargetDefinitionContext.FromOrFailMode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FromOrFailMode {
    NoMacros,
    OnlyFinalizerMacros,
}

/// The state of a package under construction (Java `TargetDefinitionContext` +
/// `Package.AbstractBuilder` + `Package.Builder`).
// Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:60 TargetDefinitionContext
pub struct TargetDefinitionContext {
    // ---- Java TargetDefinitionContext fields
    /// Same as the package's metadata.
    pub(crate) metadata: Arc<PackageMetadata>,
    /// Java `pkg.getDeclarations()` (mutable until `finishBuild`).
    pub(crate) declarations: Arc<PackageDeclarations>,
    /// The container on which targets and macro instances are added and conflicts detected.
    pub(crate) recorder: TargetRecorder,
    pub(crate) simplify_unconditional_selects_in_rule_attrs: bool,
    // PORT: an `Arc` so that `StarlarkRuleFunction.call` can install this same converter as the
    // thread's `LabelConverter` thread-local while the builder is off the thread.
    pub(crate) label_converter: Arc<LabelConverter>,
    /// Sorted so that the iteration order of variables is deterministic.
    pub(crate) make_env: BTreeMap<String, String>,
    pub(crate) local_event_handler: Arc<StoredEventHandler>,
    pub(crate) io_exception_message: Option<String>,
    /// The message of Java's `ioException`.
    pub(crate) io_exception: Option<String>,
    pub(crate) io_exception_detailed_exit_code: Option<DetailedExitCode>,
    /// Used by glob(); `None` where glob() is disallowed.
    pub(crate) globber: Option<Arc<dyn Globber>>,
    /// Environment to its group.
    pub(crate) environment_groups: HashMap<Label, Arc<EnvironmentGroup>>,
    pub(crate) generator_map: IndexMap<Location, String>,
    pub(crate) package_limits: PackageLimits,
    /// Java `testSuiteImplicitTestsAccumulator`.
    // PORT: a Mutex because RuleClass.createRule populates attributes through a shared reference.
    pub(crate) test_suite_implicit_tests_accumulator:
        parking_lot::Mutex<TestSuiteImplicitTestsAccumulator>,
    pub(crate) failure_detail_override: Option<DetailedExitCode>,
    pub(crate) already_built: bool,
    /// Java `pkg.targets != null`: finishBuild() has propagated the builder's targets and error
    /// status to the packageoid.
    pub(crate) packageoid_initialized: bool,
    pub(crate) computation_steps: i64,
    /// Java `StarlarkThreadContext`'s main repository mapping supplier.
    pub(crate) main_repository_mapping: RepositoryMapping,
    // ---- Java Package.AbstractBuilder fields
    pub(crate) precompute_transitive_loads: bool,
    pub(crate) package_function_used: bool,
    pub(crate) transitive_bzl_digest: Vec<u8>,
    pub(crate) no_implicit_file_export: bool,
    // ---- Java Package fields set by the builder
    pub(crate) build_file: Option<Arc<InputFile>>,
    // ---- Java Package.Builder fields
    pub(crate) package_settings: PackageSettings,
    /// Snapshot of non-finalizer-instantiated rules for rule finalizers.
    pub(crate) rules_snapshot_view_for_finalizers: Option<IndexMap<String, Arc<Rule>>>,
    pub(crate) unexpanded_macros: IndexSet<String>,
    /// Java: the target map is no longer a `SnapshottableBiMap` (after `beforeBuild`).
    pub(crate) target_map_unwrapped: bool,
    /// The package returned by the first `finishBuild` (Java `pkg`).
    pub(crate) built_package: Option<Arc<Package>>,
    /// The label converter of a running rule initializer (Java sets it as a thread-local of the
    /// thread while the initializer runs).
    pub(crate) initializer_label_converter: Option<Arc<LabelConverter>>,
}

impl TargetDefinitionContext {
    /// Java `new Package.Builder(...)` (with the `AbstractBuilder` and `TargetDefinitionContext`
    /// constructors).
    // Java: com/google/devtools/build/lib/packages/Package.java:1120 Package.Builder.<init>
    // Java: com/google/devtools/build/lib/packages/Package.java:976 Package.AbstractBuilder.<init>
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:388 TargetDefinitionContext.<init>
    pub(crate) fn new_package_builder_internal(
        metadata: Arc<PackageMetadata>,
        declarations: Arc<PackageDeclarations>,
        package_settings: PackageSettings,
        no_implicit_file_export: bool,
        simplify_unconditional_selects_in_rule_attrs: bool,
        main_repository_mapping: RepositoryMapping,
        generator_map: Option<IndexMap<Location, String>>,
        globber: Option<Arc<dyn Globber>>,
        enable_name_conflict_checking: bool,
        track_full_macro_information: bool,
        package_limits: PackageLimits,
    ) -> TargetDefinitionContext {
        let label_converter = Arc::new(LabelConverter::new(
            &metadata.package_identifier,
            metadata.repository_mapping.clone(),
        ));
        let mut context = TargetDefinitionContext {
            metadata: metadata.clone(),
            declarations,
            recorder: TargetRecorder::new(
                enable_name_conflict_checking,
                track_full_macro_information,
                /* enable_target_map_snapshotting= */ true,
            ),
            simplify_unconditional_selects_in_rule_attrs,
            label_converter,
            make_env: BTreeMap::new(),
            local_event_handler: Arc::new(StoredEventHandler::new()),
            io_exception_message: None,
            io_exception: None,
            io_exception_detailed_exit_code: None,
            globber,
            environment_groups: HashMap::new(),
            generator_map: generator_map.unwrap_or_default(),
            package_limits,
            test_suite_implicit_tests_accumulator: parking_lot::Mutex::new(
                TestSuiteImplicitTestsAccumulator::default(),
            ),
            failure_detail_override: None,
            already_built: false,
            packageoid_initialized: false,
            computation_steps: 0,
            main_repository_mapping,
            precompute_transitive_loads: package_settings.precompute_transitive_loads,
            package_function_used: false,
            transitive_bzl_digest: Vec::new(),
            no_implicit_file_export,
            build_file: None,
            package_settings,
            rules_snapshot_view_for_finalizers: None,
            unexpanded_macros: IndexSet::new(),
            target_map_unwrapped: false,
            built_package: None,
            initializer_label_converter: None,
        };
        // Java: Package.AbstractBuilder.<init>
        if metadata.get_name().starts_with("javatests/") {
            let mut args = PackageArgs::builder();
            args.set_default_test_only(true);
            context.merge_package_args_from(&args.build());
        }
        // Add target for the BUILD file itself.
        // (This may be overridden by an exports_file declaration.)
        context
            .recorder
            .add_input_file_unchecked(Arc::new(InputFile::new(
                metadata.clone(),
                context.declarations.clone(),
                metadata.build_file_label.clone(),
                metadata.get_build_file_location(),
                InputFileKind::Plain,
            )));
        context
    }

    /// The builder of the running BUILD / macro evaluation, or `None`.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:163 TargetDefinitionContext.fromOrNull
    pub fn from_or_null<'a>(
        eval: &Evaluator<'_, 'a, '_>,
    ) -> Option<RefMut<'a, TargetDefinitionContext>> {
        ThreadContext::get(eval)
            .ok()?
            .payload::<TargetDefinitionContext>()
    }

    /// Like [`TargetDefinitionContext::from_or_null`], failing with "`<what>` can only be used
    /// while evaluating a BUILD file or a ... macro".
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:176 TargetDefinitionContext.fromOrFail
    pub fn from_or_fail<'a>(
        eval: &Evaluator<'_, 'a, '_>,
        what: &str,
    ) -> Result<RefMut<'a, TargetDefinitionContext>, EvalException> {
        Self::from_or_fail_participle(eval, what, "used")
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:187 TargetDefinitionContext.fromOrFail
    pub fn from_or_fail_participle<'a>(
        eval: &Evaluator<'_, 'a, '_>,
        what: &str,
        participle: &str,
    ) -> Result<RefMut<'a, TargetDefinitionContext>, EvalException> {
        if let Some(ctx) = Self::from_or_null(eval) {
            return Ok(ctx);
        }
        let symbolic_macros_enabled = match ThreadContext::get(eval) {
            Ok(cx) => cx.semantics.experimental_enable_first_class_macros,
            Err(_) => blaze_util::unported!("TargetDefinitionContext.fromOrFail without context"),
        };
        Err(Self::new_from_or_fail_exception(
            what,
            participle,
            symbolic_macros_enabled,
            &[],
        ))
    }

    /// Like [`TargetDefinitionContext::from_or_fail`], but fails inside non-finalizer macros.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:203 TargetDefinitionContext.fromOrFailDisallowNonFinalizerMacros
    pub fn from_or_fail_disallow_non_finalizer_macros<'a>(
        eval: &Evaluator<'_, 'a, '_>,
        what: &str,
    ) -> Result<RefMut<'a, TargetDefinitionContext>, EvalException> {
        if let Some(ctx) = Self::from_or_null(eval)
            && !ctx.recorder.currently_in_non_finalizer_macro()
        {
            return Ok(ctx);
        }
        let symbolic_macros_enabled = match ThreadContext::get(eval) {
            Ok(cx) => cx.semantics.experimental_enable_first_class_macros,
            Err(_) => blaze_util::unported!(
                "TargetDefinitionContext.fromOrFailDisallowNonFinalizerMacros without context"
            ),
        };
        Err(Self::new_from_or_fail_exception(
            what,
            "used",
            symbolic_macros_enabled,
            &[FromOrFailMode::OnlyFinalizerMacros],
        ))
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:224 TargetDefinitionContext.newFromOrFailException
    pub fn new_from_or_fail_exception(
        what: &str,
        participle: &str,
        symbolic_macros_enabled: bool,
        modes: &[FromOrFailMode],
    ) -> EvalException {
        let mut allowed_uses: Vec<String> = Vec::new();
        allowed_uses.push("a BUILD file".to_owned());
        allowed_uses.push(format!(
            "a {}{}macro",
            if symbolic_macros_enabled {
                "legacy "
            } else {
                ""
            },
            if symbolic_macros_enabled
                && !modes.contains(&FromOrFailMode::NoMacros)
                && !modes.contains(&FromOrFailMode::OnlyFinalizerMacros)
            {
                "or symbolic "
            } else {
                ""
            }
        ));
        if symbolic_macros_enabled && modes.contains(&FromOrFailMode::OnlyFinalizerMacros) {
            allowed_uses.push("a rule finalizer".to_owned());
        }
        EvalException::new(format!(
            "{} can only be {} while evaluating {}",
            what,
            participle,
            join_english_list(&allowed_uses)
        ))
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:350 TargetDefinitionContext.getGeneratorNameByLocation
    pub fn get_generator_name_by_location(&self, loc: &Location) -> Option<&str> {
        self.generator_map.get(loc).map(String::as_str)
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:358 TargetDefinitionContext.getGeneratorMap
    pub fn get_generator_map(&self) -> &IndexMap<Location, String> {
        &self.generator_map
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:372 TargetDefinitionContext.getTestSuiteImplicitTestsRef
    pub fn get_test_suite_implicit_tests_ref(&self, tags: &[String]) -> ImplicitTestsRef {
        self.test_suite_implicit_tests_accumulator
            .lock()
            .get_test_suite_implicit_tests_ref_for_tags(tags)
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:418 TargetDefinitionContext.getMetadata
    pub fn get_metadata(&self) -> &Arc<PackageMetadata> {
        &self.metadata
    }

    pub fn get_declarations(&self) -> &Arc<PackageDeclarations> {
        &self.declarations
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:426 TargetDefinitionContext.getPackageIdentifier
    pub fn get_package_identifier(&self) -> &PackageIdentifier {
        &self.metadata.package_identifier
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:434 TargetDefinitionContext.getShortDescription
    pub fn get_short_description(&self) -> String {
        // Java: com/google/devtools/build/lib/packages/Package.java:471 Package.getShortDescription
        format!(
            "package {}",
            self.metadata.package_identifier.get_canonical_form()
        )
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:443 TargetDefinitionContext.getAssociatedModuleName
    pub fn get_associated_module_name(&self) -> Option<&str> {
        self.metadata.associated_module_name.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:452 TargetDefinitionContext.getAssociatedModuleVersion
    pub fn get_associated_module_version(&self) -> Option<&str> {
        self.metadata.associated_module_version.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:456 TargetDefinitionContext.getLabelConverter
    pub fn get_label_converter(&self) -> &LabelConverter {
        &self.label_converter
    }

    /// [`TargetDefinitionContext::get_label_converter`] as a shared handle (Java passes the same
    /// object to `thread.setThreadLocal(LabelConverter.class, ...)` around rule initializers).
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:456 TargetDefinitionContext.getLabelConverter
    pub fn get_label_converter_arc(&self) -> &Arc<LabelConverter> {
        &self.label_converter
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:464 TargetDefinitionContext.getFilename
    pub fn get_filename(&self) -> &RootedPath {
        &self.metadata.build_filename
    }

    /// The handler on which package-loading events are recorded (and later replayed).
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:469 TargetDefinitionContext.getLocalEventHandler
    pub fn get_local_event_handler(&self) -> &StoredEventHandler {
        &self.local_event_handler
    }

    /// The shared local event handler, so that other event sources (e.g. BUILD `print()`) can
    /// record into it in order.
    pub fn get_local_event_handler_arc(&self) -> &Arc<StoredEventHandler> {
        &self.local_event_handler
    }

    /// Java `packageLimits` (computation step limits; enforced by the thread step updaters).
    pub fn get_package_limits(&self) -> PackageLimits {
        self.package_limits
    }

    /// Java `StarlarkThreadContext.getMainRepoMappingForStarlark` supplier value.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:401 TargetDefinitionContext.<init>
    pub fn get_main_repository_mapping(&self) -> &RepositoryMapping {
        &self.main_repository_mapping
    }

    /// Java `PackageSettings` of the builder (`precomputeTransitiveLoads`,
    /// `succinctTargetNotFoundErrors`).
    pub fn get_package_settings(&self) -> PackageSettings {
        blaze_util::check_state!(
            self.precompute_transitive_loads == self.package_settings.precompute_transitive_loads
        );
        self.package_settings
    }

    /// Sets the label converter of a running rule initializer, returning the previous one (Java:
    /// `thread.setThreadLocal(LabelConverter.class, ...)` around the initializer call).
    pub fn set_initializer_label_converter(
        &mut self,
        label_converter: Option<Arc<LabelConverter>>,
    ) -> Option<Arc<LabelConverter>> {
        std::mem::replace(&mut self.initializer_label_converter, label_converter)
    }

    /// The label converter of the running rule initializer, if any.
    pub fn get_initializer_label_converter(&self) -> Option<&Arc<LabelConverter>> {
        self.initializer_label_converter.as_ref()
    }

    /// All targets added so far (Java `AbstractBuilder.getTargets`, used by PackageFunction).
    // Java: com/google/devtools/build/lib/packages/Package.java:775 Package.AbstractBuilder.getTargets
    pub fn get_targets(&self) -> impl Iterator<Item = &Target> {
        self.recorder.get_targets()
    }

    /// Removes a target between `buildPartial` and `finishBuild` (Java: removal through the view
    /// `getTargets()` returns, in PackageFunction.handleLabelsCrossingSubpackages...).
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:383 TargetRecorder.getTargets
    pub fn remove_target(&mut self, name: &str) -> Option<Target> {
        self.recorder.remove_target(name)
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:477 TargetDefinitionContext.getPartialPackageArgs
    pub fn get_partial_package_args(&self) -> PackageArgs {
        self.declarations.get_package_args()
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:481 TargetDefinitionContext.containsErrors
    pub fn contains_errors(&self) -> bool {
        self.recorder.contains_errors()
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:491 TargetDefinitionContext.setContainsErrors
    pub fn set_contains_errors(&mut self) {
        // Java: `checkState(pkg.targets == null, ...)`: errors may still be set by
        // finalBuilderValidationHook(), which finishBuild() runs after `alreadyBuilt = true`.
        blaze_util::check_state!(
            !self.packageoid_initialized,
            "TargetDefinitionContext.setContainsErrors() can only be used before finishBuild() has \
             propagated the builder's error status to the packageoid"
        );
        self.recorder.set_contains_errors();
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:499 TargetDefinitionContext.setIOException
    pub fn set_io_exception(
        &mut self,
        e: String,
        message: String,
        detailed_exit_code: DetailedExitCode,
    ) {
        self.io_exception = Some(e);
        self.io_exception_message = Some(message);
        self.io_exception_detailed_exit_code = Some(detailed_exit_code);
        self.set_contains_errors();
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:511 TargetDefinitionContext.getGlobber
    pub fn get_globber(&self) -> Option<&Arc<dyn Globber>> {
        self.globber.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:519 TargetDefinitionContext.simplifyUnconditionalSelectsInRuleAttrs
    pub fn simplify_unconditional_selects_in_rule_attrs(&self) -> bool {
        self.simplify_unconditional_selects_in_rule_attrs
    }

    /// The innermost symbolic macro being evaluated, if any.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:527 TargetDefinitionContext.currentMacro
    pub fn current_macro(&self) -> Option<&Arc<MacroInstance>> {
        self.recorder
            .get_current_macro_frame()
            .map(|f| &f.macro_instance)
    }

    /// A new rule of `rule_class` in this package, positioned per the thread's call stack.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:543 TargetDefinitionContext.createRule
    pub fn create_rule(
        &self,
        label: Label,
        rule_class: &Arc<RuleClass>,
        thread_call_stack: &[StackEntry],
    ) -> Rule {
        let mut full_interior_call_stack;
        let location;
        if let Some(current_macro) = self.current_macro() {
            location = current_macro.get_build_file_location().clone();
            full_interior_call_stack = call_stack::compact(thread_call_stack, /* start= */ 0);
            let mut macro_ = Some(current_macro);
            while let Some(m) = macro_ {
                full_interior_call_stack =
                    call_stack::concatenate(m.get_parent_call_stack(), full_interior_call_stack);
                macro_ = m.get_parent();
            }
        } else {
            location = match thread_call_stack.first() {
                None => Location::builtin(),
                Some(entry) => entry.location.clone().unwrap_or_else(Location::builtin),
            };
            full_interior_call_stack = call_stack::compact(thread_call_stack, /* start= */ 1);
        }
        self.create_rule_at(label, rule_class, location, full_interior_call_stack)
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:561 TargetDefinitionContext.createRule
    pub fn create_rule_at(
        &self,
        label: Label,
        rule_class: &Arc<RuleClass>,
        location: Location,
        interior_call_stack: Option<Arc<CallStackNode>>,
    ) -> Rule {
        Rule::new(
            self.metadata.clone(),
            self.declarations.clone(),
            self.current_macro().cloned(),
            label,
            rule_class.clone(),
            location,
            interior_call_stack,
        )
    }

    /// A new symbolic macro instance in this package.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:570 TargetDefinitionContext.createMacro
    pub fn create_macro(
        &self,
        macro_class: &Arc<MacroClass>,
        name: &str,
        same_name_depth: i32,
        parent_call_stack: &[StackEntry],
    ) -> Result<MacroInstance, LabelSyntaxException> {
        let parent = self.current_macro().cloned();
        let location;
        let compact_parent_call_stack;
        if let Some(parent) = &parent {
            location = parent.get_build_file_location().clone();
            compact_parent_call_stack = call_stack::compact(parent_call_stack, /* start= */ 0);
        } else {
            location = match parent_call_stack.first() {
                None => Location::builtin(),
                Some(entry) => entry.location.clone().unwrap_or_else(Location::builtin),
            };
            compact_parent_call_stack = call_stack::compact(parent_call_stack, /* start= */ 1);
        }
        let generator_name = match &parent {
            Some(parent) => parent.get_generator_name().map(str::to_owned),
            None => self.generator_map.get(&location).cloned(),
        };
        Ok(MacroInstance::new(
            self.metadata.clone(),
            self.declarations.clone(),
            parent,
            generator_name,
            location,
            compact_parent_call_stack,
            macro_class.clone(),
            Label::create_in_package(&self.metadata.package_identifier, name)?,
            same_name_depth,
        ))
    }

    /// True if symbolic macros are expanded eagerly (always, for a package builder).
    // Java: com/google/devtools/build/lib/packages/Package.java:1268 Package.Builder.eagerlyExpandMacros
    pub fn eagerly_expand_macros(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:602 TargetDefinitionContext.getCurrentMacroFrame
    pub fn get_current_macro_frame(&self) -> Option<&MacroFrame> {
        self.recorder.get_current_macro_frame()
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:607 TargetDefinitionContext.setCurrentMacroFrame
    pub fn set_current_macro_frame(&mut self, frame: Option<MacroFrame>) -> Option<MacroFrame> {
        self.recorder.set_current_macro_frame(frame)
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:611 TargetDefinitionContext.currentlyInNonFinalizerMacro
    pub fn currently_in_non_finalizer_macro(&self) -> bool {
        self.recorder.currently_in_non_finalizer_macro()
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:616 TargetDefinitionContext.getTarget
    pub fn get_target(&self, name: &str) -> Option<&Target> {
        self.recorder.get_target(name)
    }

    /// Rules visible to `native.existing_rules()`, in insertion order.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:633 TargetDefinitionContext.getRulesSnapshotView
    // Java: com/google/devtools/build/lib/packages/Package.java:1164 Package.Builder.getRulesSnapshotView
    pub fn get_rules_snapshot_view(&self) -> IndexMap<String, Arc<Rule>> {
        if let Some(view) = &self.rules_snapshot_view_for_finalizers {
            return view.clone();
        }
        if !self.target_map_unwrapped {
            // Java: SnapshottableBiMap.getTrackedSnapshot (rules in insertion order).
            self.recorder
                .get_target_map()
                .iter()
                .filter_map(|(name, target)| target.as_rule().map(|r| (name.clone(), r.clone())))
                .collect()
        } else {
            panic!(
                "IllegalStateException: getRulesSnapshotView() cannot be used after beforeBuild() \
                 has been called"
            );
        }
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:655 TargetDefinitionContext.getNonFinalizerInstantiatedRule
    // Java: com/google/devtools/build/lib/packages/Package.java:1173 Package.Builder.getNonFinalizerInstantiatedRule
    pub fn get_non_finalizer_instantiated_rule(&self, name: &str) -> Option<Arc<Rule>> {
        if let Some(view) = &self.rules_snapshot_view_for_finalizers {
            return view.get(name).cloned();
        }
        self.recorder
            .get_target_map()
            .get(name)
            .and_then(Target::as_rule)
            .cloned()
    }

    /// Creates and adds an input file target.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:678 TargetDefinitionContext.createInputFile
    pub fn create_input_file(
        &mut self,
        target_name: &str,
        location: Location,
    ) -> Result<Arc<InputFile>, NameConflictException> {
        let existing = self.recorder.get_target_map().get(target_name);

        if let Some(Target::InputFile(existing)) = existing {
            return Ok(existing.clone()); // idempotent
        }

        let label = match self.create_label(target_name) {
            Ok(label) => label,
            Err(e) => panic!(
                "IllegalArgumentException: FileTarget in package {} has illegal name: {}: {}",
                self.metadata.get_name(),
                target_name,
                e
            ),
        };
        let input_file = Arc::new(
            InputFile::new(
                self.metadata.clone(),
                self.declarations.clone(),
                label,
                location,
                InputFileKind::Plain,
            )
            .with_declaring_macro(self.current_macro().cloned()),
        );

        self.recorder
            .add_target(Target::InputFile(input_file.clone()))?;
        Ok(input_file)
    }

    /// Replaces `input_file` with one with the given visibility and license (`exports_files`).
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:700 TargetDefinitionContext.setVisibilityAndLicense
    pub fn set_visibility_and_license(
        &mut self,
        input_file: &Arc<InputFile>,
        visibility: Option<RuleVisibility>,
        license: Option<License>,
    ) {
        let filename = input_file.get_name();
        let cache_instance = match self.recorder.get_target_map().get(filename) {
            Some(Target::InputFile(f)) => f.clone(),
            _ => panic!(
                "IllegalArgumentException: Can't set visibility for nonexistent FileTarget {} in \
                 package {}.",
                filename,
                self.metadata.get_name()
            ),
        };
        // PORT: Java compares the visibility by identity; equal values describe the same
        // visibility, so the replacement would be an equal target.
        let cache_target = Target::InputFile(cache_instance.clone());
        if !cache_instance.is_visibility_specified()
            || Some(cache_target.get_visibility()) != visibility
            || Some(cache_target.get_license()) != license
        {
            self.recorder.replace_input_file_unchecked(Arc::new(
                InputFile::new(
                    self.metadata.clone(),
                    self.declarations.clone(),
                    cache_instance.get_label().clone(),
                    cache_target.get_location(),
                    InputFileKind::VisibilityLicenseSpecified {
                        visibility,
                        license,
                    },
                )
                .with_declaring_macro(self.current_macro().cloned()),
            ));
        }
    }

    /// The label of `target_name` in this package.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:725 TargetDefinitionContext.createLabel
    pub fn create_label(&self, target_name: &str) -> Result<Label, LabelSyntaxException> {
        Label::create_in_package(&self.metadata.package_identifier, target_name)
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:730 TargetDefinitionContext.addPackageGroup
    pub fn add_package_group(
        &mut self,
        name: &str,
        packages: &[String],
        includes: Vec<Label>,
        allow_public_private: bool,
        repo_root_means_current_repo: bool,
        location: Location,
    ) -> Result<(), crate::package::PackageBuilderError> {
        let group = Arc::new(PackageGroup::new(
            self.create_label(name)?,
            self.metadata.clone(),
            self.declarations.clone(),
            packages,
            includes,
            allow_public_private,
            repo_root_means_current_repo,
            &*self.local_event_handler,
            location,
        ));
        let contains_errors = group.contains_errors();
        self.recorder.add_target(Target::PackageGroup(group))?;

        if contains_errors {
            self.set_contains_errors();
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:756 TargetDefinitionContext.addRule
    pub fn add_rule(&mut self, rule: Arc<Rule>) -> Result<(), NameConflictException> {
        self.recorder.add_rule(rule)
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:761 TargetDefinitionContext.addMacro
    // Java: com/google/devtools/build/lib/packages/Package.java:1205 Package.Builder.addMacro
    pub fn add_macro(&mut self, macro_: Arc<MacroInstance>) -> Result<(), NameConflictException> {
        let id = macro_.get_id();
        self.recorder.add_macro(macro_)?;
        self.unexpanded_macros.insert(id);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:770 TargetDefinitionContext.setFailureDetailOverride
    pub fn set_failure_detail_override(&mut self, failure_detail: DetailedExitCode) {
        self.failure_detail_override = Some(failure_detail);
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:775 TargetDefinitionContext.getFailureDetail
    pub fn get_failure_detail(&self) -> Option<DetailedExitCode> {
        if let Some(failure_detail_override) = &self.failure_detail_override {
            return Some(failure_detail_override.clone());
        }

        for event in self.local_event_handler.get_events() {
            if event.get_kind() != EventKind::Error {
                continue;
            }
            if let Some(detailed_exit_code) = event.get_property::<DetailedExitCode>() {
                return Some(detailed_exit_code.clone());
            }
            // PORT: Java collects undetailed error events for a non-fatal bug report.
        }
        None
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:812 TargetDefinitionContext.getComputationSteps
    pub fn get_computation_steps(&self) -> i64 {
        self.computation_steps
    }

    /// Fails with the recorded I/O error, if any, before the package is built.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:820 TargetDefinitionContext.beforeBuild
    // Java: com/google/devtools/build/lib/packages/Package.java:1287 Package.Builder.beforeBuild
    pub fn before_build(&mut self) -> Result<(), NoSuchPackageException> {
        // For correct semantics, we refuse to build a package that hasn't thrown any
        // EvalExceptions but has declared symbolic macros that have not yet been expanded.
        blaze_util::check_state!(
            self.unexpanded_macros.is_empty() || self.contains_errors(),
            "Cannot build a package with unexpanded symbolic macros; call \
             expandAllRemainingMacros()"
        );

        // SnapshottableBiMap does not allow removing targets; repoint the targets map to its
        // underlying bimap and thus stop tracking insertion order.
        if !self.target_map_unwrapped {
            self.target_map_unwrapped = true;
            self.rules_snapshot_view_for_finalizers = None;
        }

        // Java: com/google/devtools/build/lib/packages/Package.java:870 Package.AbstractBuilder.beforeBuild
        self.before_build_without_discovering_assumed_input_files()?;
        let new_input_files = self.create_assumed_input_files();
        for file in new_input_files.into_values() {
            self.recorder.add_input_file_unchecked(file);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:859 Package.AbstractBuilder.beforeBuildWithoutDiscoveringAssumedInputFiles
    fn before_build_without_discovering_assumed_input_files(
        &mut self,
    ) -> Result<(), NoSuchPackageException> {
        // We create an InputFile corresponding to the BUILD file in Builder's constructor.
        // However, the visibility of this target may be overridden with an exports_files
        // directive, so we wait until now to obtain the current instance from the targets map.
        self.build_file = Some(self.current_build_file());

        self.before_build_base()
    }

    /// The BUILD file target as currently in the targets map.
    fn current_build_file(&self) -> Arc<InputFile> {
        match self
            .recorder
            .get_target_map()
            .get(self.metadata.build_file_label.get_name())
        {
            Some(Target::InputFile(f)) => f.clone(),
            other => panic!("ClassCastException: BUILD file target {other:?}"),
        }
    }

    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:820 TargetDefinitionContext.beforeBuild
    fn before_build_base(&mut self) -> Result<(), NoSuchPackageException> {
        if let Some(io_exception) = &self.io_exception {
            return Err(NoSuchPackageException::of_kind(
                crate::exceptions::NoSuchPackageKind::NoSuchPackage,
                self.get_package_identifier().clone(),
                self.io_exception_message.clone().unwrap_or_default(),
                Some(io_exception.clone()),
                self.io_exception_detailed_exit_code.clone(),
            ));
        }

        // Clear tests before discovering them again in order to keep this method idempotent -
        // otherwise we may double-count tests if we're called twice due to a skyframe restart,
        // etc.
        let mut accumulator = self.test_suite_implicit_tests_accumulator.lock();
        accumulator.clear_accumulated_tests();
        for rule in self.recorder.get_rules() {
            accumulator.process_rule(rule);
        }
        // Make sure all accumulated values are sorted for determinism.
        accumulator.sort_tests();
        Ok(())
    }

    /// Input files for labels referenced but not declared in this package.
    // Java: com/google/devtools/build/lib/packages/Package.java:900 Package.AbstractBuilder.createAssumedInputFiles
    fn create_assumed_input_files(&self) -> IndexMap<String, Arc<InputFile>> {
        // PORT: Java uses a HashMap; the files go into the (sorted) targets map, so the order is
        // not observable.
        let mut implicitly_created_input_files: IndexMap<String, Arc<InputFile>> = IndexMap::new();

        for rule in self.recorder.get_rules() {
            if !self.recorder.is_rule_created_in_macro(rule) {
                for label in self.recorder.get_rule_labels(rule) {
                    self.maybe_create_assumed_input_file(
                        &mut implicitly_created_input_files,
                        &label,
                        rule.get_location().clone(),
                    );
                }
            }
        }

        for macro_ in self.recorder.get_macro_map().values() {
            if macro_.get_parent().is_none() {
                let mut labels = Vec::new();
                macro_.visit_explicit_attribute_labels(&mut |label| labels.push(label.clone()));
                for label in labels {
                    self.maybe_create_assumed_input_file(
                        &mut implicitly_created_input_files,
                        &label,
                        // TODO(bazel-team): We don't save a MacroInstance's location information
                        // yet, but when we do, use that here.
                        Location::builtin(),
                    );
                }
            }
        }

        implicitly_created_input_files
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:944 Package.AbstractBuilder.maybeCreateAssumedInputFile
    fn maybe_create_assumed_input_file(
        &self,
        implicitly_created_input_files: &mut IndexMap<String, Arc<InputFile>>,
        label: &Label,
        loc: Location,
    ) {
        let name = label.get_name();
        if label.get_package_identifier() != &self.metadata.package_identifier {
            return;
        }
        if self.recorder.get_target_map().contains_key(name)
            || self.recorder.has_macro_with_name(name)
            || implicitly_created_input_files.contains_key(name)
        {
            return;
        }

        implicitly_created_input_files.insert(
            name.to_owned(),
            Arc::new(InputFile::new(
                self.metadata.clone(),
                self.declarations.clone(),
                label.clone(),
                loc,
                if self.no_implicit_file_export {
                    InputFileKind::PrivateVisibility
                } else {
                    InputFileKind::Plain
                },
            )),
        );
    }

    /// Java `buildPartial()`: `beforeBuild()` unless already built.
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:850 TargetDefinitionContext.buildPartial
    pub fn build_partial(&mut self) -> Result<(), NoSuchPackageException> {
        if self.already_built {
            return Ok(());
        }
        self.before_build()
    }

    /// Freezes rules and macros and builds the package (Java `finishBuild`, with
    /// `Package.Builder`'s hooks).
    // Java: com/google/devtools/build/lib/packages/TargetDefinitionContext.java:866 TargetDefinitionContext.finishBuild
    pub fn finish_build(&mut self) -> Arc<Package> {
        if self.already_built {
            return self
                .built_package
                .clone()
                .expect("package of a built builder");
        }
        self.already_built = true;

        // Freeze rules, compacting their attributes' representations.
        for rule in self.recorder.get_rules() {
            rule.freeze();
        }

        // Freeze macros, compacting their attributes' representations.
        for macro_ in self.recorder.get_macro_map().values() {
            macro_.freeze();
        }

        // Last chance to set the builder's error status.
        self.final_builder_validation_hook();

        // Initialize packageoid.
        self.packageoid_initialized = true;
        let contains_errors = self.declarations.contains_errors() || self.contains_errors();
        let failure_detail = self.get_failure_detail();
        let targets: BTreeMap<String, Target> = self
            .recorder
            .get_target_map()
            .iter()
            .map(|(name, target)| (name.clone(), target.clone()))
            .collect();

        // Java: com/google/devtools/build/lib/packages/Package.java:944 Package.AbstractBuilder.packageoidInitializationHook
        // Finish Package.Declarations construction.
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

        // Java: com/google/devtools/build/lib/packages/Package.java:1310 Package.Builder.packageoidInitializationHook
        let macros: BTreeMap<String, Arc<MacroInstance>> = self
            .recorder
            .get_macro_map()
            .iter()
            .map(|(id, m)| (id.clone(), m.clone()))
            .collect();
        let sorted = |m: Option<&IndexMap<String, Arc<MacroInstance>>>| {
            m.map(|m| {
                let mut v: Vec<_> = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v.into_iter().collect::<IndexMap<_, _>>()
            })
        };
        let targets_to_declaring_macro = sorted(self.recorder.get_targets_to_declaring_macro());
        let targets_to_declaring_package =
            self.recorder.get_targets_to_declaring_package().map(|m| {
                let mut v: Vec<_> = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v.into_iter().collect::<IndexMap<_, _>>()
            });

        // PORT: Java's buildFile stays null if beforeBuild was not called; the current BUILD file
        // target is used then.
        let build_file = match &self.build_file {
            Some(f) => f.clone(),
            None => self.current_build_file(),
        };

        let pkg = Arc::new(Package::from_parts(
            self.metadata.clone(),
            self.declarations.clone(),
            contains_errors,
            failure_detail,
            self.computation_steps,
            targets,
            build_file,
            macros,
            self.recorder
                .get_macro_namespace_violating_targets()
                .clone(),
            targets_to_declaring_macro,
            targets_to_declaring_package,
            self.local_event_handler.get_events(),
            self.local_event_handler.get_posts(),
        ));

        // Verify that we haven't introduced new errors on the builder since the call to
        // finalBuilderValidationHook().
        if self.contains_errors() {
            blaze_util::check_state!(
                pkg.contains_errors(),
                "Builder error status not propagated to package or package piece"
            );
        }

        self.built_package = Some(pkg.clone());
        pkg
    }

    /// Validates the member environments of environment groups (all targets are loaded now).
    // Java: com/google/devtools/build/lib/packages/Package.java:960 Package.AbstractBuilder.finalBuilderValidationHook
    fn final_builder_validation_hook(&mut self) {
        // Java: ImmutableSet.copyOf(environmentGroups.values()) (HashMap order).
        // PORT: java-hash-order. The groups are processed in label order here; the order only
        // affects the order of their error events.
        let mut groups: Vec<Arc<EnvironmentGroup>> = Vec::new();
        for group in self.environment_groups.values() {
            if !groups.iter().any(|g| Arc::ptr_eq(g, group)) {
                groups.push(group.clone());
            }
        }
        groups.sort_by(|a, b| a.get_label().cmp(b.get_label()));
        for env_group in groups {
            // PORT: Java mutates the group in place; the processed copy replaces the shared one.
            let mut processed = (*env_group).clone();
            let errors = processed.process_member_environments(self.recorder.get_target_map());
            let processed = Arc::new(processed);
            self.recorder.replace_environment_group(processed.clone());
            for group in self.environment_groups.values_mut() {
                if Arc::ptr_eq(group, &env_group) {
                    *group = processed.clone();
                }
            }
            if !errors.is_empty() {
                for error in &errors {
                    self.local_event_handler.handle(error);
                }
                // TODO(bazel-team): Can't we automatically infer containsError from the presence
                // of ERRORs on our handler?
                self.set_contains_errors();
            }
        }
    }

    /// Java `Package.Builder.build()`: `beforeBuild()` then `finishBuild()` (idempotent).
    // Java: com/google/devtools/build/lib/packages/Package.java:1325 Package.Builder.build
    pub fn build(&mut self) -> Result<Arc<Package>, NoSuchPackageException> {
        self.build_with(/* discover_assumed_input_files= */ true)
    }

    // Java: com/google/devtools/build/lib/packages/Package.java:1336 Package.Builder.build
    pub fn build_with(
        &mut self,
        discover_assumed_input_files: bool,
    ) -> Result<Arc<Package>, NoSuchPackageException> {
        if self.already_built {
            return Ok(self
                .built_package
                .clone()
                .expect("package of a built builder"));
        }
        if discover_assumed_input_files {
            self.before_build()?;
        } else {
            self.before_build_without_discovering_assumed_input_files()?;
        }
        Ok(self.finish_build())
    }
}

/// Errors of builder operations that Java reports through several exception types.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PackageBuilderError {
    #[error("{0}")]
    NameConflict(#[from] NameConflictException),
    #[error("{0}")]
    LabelSyntax(#[from] LabelSyntaxException),
}

/// Java `StringUtil.joinEnglishList(items)` ("a", "a or b", "a, b, or c").
// Java: com/google/devtools/build/lib/util/StringUtil.java StringUtil.joinEnglishList
fn join_english_list(items: &[String]) -> String {
    crate::attribute::string_util::join_english_list(items)
}
