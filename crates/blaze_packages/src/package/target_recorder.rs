//! Port of `packages/TargetRecorder.java`: the container on which targets and macro instances
//! are added during package construction, with name-conflict detection.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::macros::MacroInstance;
use crate::target::EnvironmentGroup;
use crate::target::InputFile;
use crate::target::OutputFile;
use crate::target::Rule;
use crate::target::Target;

/// Java `TargetRecorder.MACRO_NAMING_RULES`.
// Java: com/google/devtools/build/lib/packages/TargetRecorder.java:48 TargetRecorder.MACRO_NAMING_RULES
pub const MACRO_NAMING_RULES: &str = "Name must be the same as the macro's name, or the macro's name followed by '_' (recommended), '-', or '.', and a non-empty string.";

/// The frame of a symbolic macro being evaluated (Java `TargetRecorder.MacroFrame`).
// Java: com/google/devtools/build/lib/packages/TargetRecorder.java:83 TargetRecorder.MacroFrame
#[derive(Debug, Clone)]
pub struct MacroFrame {
    pub macro_instance: Arc<MacroInstance>,
    pub main_submacro_has_been_defined: bool,
}

impl MacroFrame {
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:92 TargetRecorder.MacroFrame.<init>
    pub fn new(macro_instance: Arc<MacroInstance>) -> Self {
        MacroFrame {
            macro_instance,
            main_submacro_has_been_defined: false,
        }
    }
}

/// A target or macro that conflicts by name (Java `TargetRecorder.NameConflictException`).
// Java: com/google/devtools/build/lib/packages/TargetRecorder.java:849 TargetRecorder.NameConflictException
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct NameConflictException {
    pub message: String,
    pub target: Option<Target>,
    pub macro_: Option<Arc<MacroInstance>>,
    /// True for Java's subclass `MacroNamespaceViolationException`.
    pub is_macro_namespace_violation: bool,
}

impl NameConflictException {
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:855 TargetRecorder.NameConflictException.<init>
    pub fn for_target(message: impl Into<String>, subject: Target) -> Self {
        NameConflictException {
            message: message.into(),
            target: Some(subject),
            macro_: None,
            is_macro_namespace_violation: false,
        }
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:865 TargetRecorder.NameConflictException.<init>
    pub fn for_macro(message: impl Into<String>, subject: Arc<MacroInstance>) -> Self {
        NameConflictException {
            message: message.into(),
            target: None,
            macro_: Some(subject),
            is_macro_namespace_violation: false,
        }
    }

    /// Java `new MacroNamespaceViolationException(message, macro)`.
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:897 TargetRecorder.MacroNamespaceViolationException.<init>
    pub fn macro_namespace_violation(
        message: impl Into<String>,
        subject: Arc<MacroInstance>,
    ) -> Self {
        NameConflictException {
            is_macro_namespace_violation: true,
            ..Self::for_macro(message, subject)
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:874 TargetRecorder.NameConflictException.getTarget
    pub fn get_target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:883 TargetRecorder.NameConflictException.getMacro
    pub fn get_macro(&self) -> Option<&Arc<MacroInstance>> {
        self.macro_.as_ref()
    }
}

/// Java `TargetRecorder.TargetOrMacro`.
// Java: com/google/devtools/build/lib/packages/TargetRecorder.java:809 TargetRecorder.TargetOrMacro
enum TargetOrMacro<'a> {
    Target(&'a Target),
    Macro(&'a Arc<MacroInstance>),
}

impl TargetOrMacro<'_> {
    fn get_name(&self) -> &str {
        match self {
            TargetOrMacro::Target(t) => t.get_name(),
            TargetOrMacro::Macro(m) => m.get_name(),
        }
    }

    fn get_kind(&self) -> &'static str {
        match self {
            TargetOrMacro::Target(_) => "target",
            TargetOrMacro::Macro(_) => "macro",
        }
    }

    fn conflict(&self, message: String) -> NameConflictException {
        match self {
            TargetOrMacro::Target(t) => NameConflictException::for_target(message, (*t).clone()),
            TargetOrMacro::Macro(m) => NameConflictException::for_macro(message, (*m).clone()),
        }
    }
}

/// Java `TargetRecorder.MacroNamespaceViolationException`.
// Java: com/google/devtools/build/lib/packages/TargetRecorder.java:892 TargetRecorder.MacroNamespaceViolationException
pub type MacroNamespaceViolationException = NameConflictException;

/// Java `TargetRecorder`.
// Java: com/google/devtools/build/lib/packages/TargetRecorder.java:46 TargetRecorder
#[derive(Debug)]
pub struct TargetRecorder {
    contains_errors: bool,
    /// All targets, in insertion order (Java `SnapshottableBiMap`, which tracks rule insertion
    /// order for `native.existing_rules()`).
    target_map: IndexMap<String, Target>,
    /// Symbolic macro instances by id (not name).
    macro_map: IndexMap<String, Arc<MacroInstance>>,
    current_macro_frame: Option<MacroFrame>,
    /// `None` unless name conflict checking is enabled. Keyed by rule label.
    rule_labels: Option<HashMap<Label, Vec<Label>>>,
    rules_created_in_macros: Option<std::collections::HashSet<Label>>,
    macro_namespace_violating_targets: IndexMap<String, String>,
    targets_to_declaring_macro: Option<IndexMap<String, Arc<MacroInstance>>>,
    targets_to_declaring_package: Option<IndexMap<String, PackageIdentifier>>,
    output_file_prefixes: Option<HashMap<String, Arc<OutputFile>>>,
}

impl TargetRecorder {
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:184 TargetRecorder.<init>
    pub fn new(
        enable_name_conflict_checking: bool,
        track_full_macro_information: bool,
        enable_target_map_snapshotting: bool,
    ) -> Self {
        let _ = enable_target_map_snapshotting;
        TargetRecorder {
            contains_errors: false,
            target_map: IndexMap::new(),
            macro_map: IndexMap::new(),
            current_macro_frame: None,
            rule_labels: enable_name_conflict_checking.then(HashMap::new),
            rules_created_in_macros: enable_name_conflict_checking
                .then(std::collections::HashSet::new),
            macro_namespace_violating_targets: IndexMap::new(),
            targets_to_declaring_macro: track_full_macro_information.then(IndexMap::new),
            targets_to_declaring_package: (!track_full_macro_information).then(IndexMap::new),
            output_file_prefixes: enable_name_conflict_checking.then(HashMap::new),
        }
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:210 TargetRecorder.getTargetMap
    pub fn get_target_map(&self) -> &IndexMap<String, Target> {
        &self.target_map
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:214 TargetRecorder.getMacroMap
    pub fn get_macro_map(&self) -> &IndexMap<String, Arc<MacroInstance>> {
        &self.macro_map
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:223 TargetRecorder.hasMacroWithName
    pub fn has_macro_with_name(&self, name: &str) -> bool {
        // Macros are indexed by id, not name, so we can't just use macroMap.get() directly.
        // Instead, we reason that if at least one macro by the given name exists, then there is one
        // with an id suffix of ":1".
        self.macro_map.contains_key(&format!("{name}:1"))
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:230 TargetRecorder.getRuleLabels
    pub fn get_rule_labels(&self, rule: &Rule) -> Vec<Label> {
        match &self.rule_labels {
            Some(rule_labels) => rule_labels
                .get(rule.get_label())
                .cloned()
                .expect("NullPointerException: no labels recorded for rule"),
            None => rule.get_labels(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:234 TargetRecorder.isRuleCreatedInMacro
    pub fn is_rule_created_in_macro(&self, rule: &Rule) -> bool {
        self.rules_created_in_macros
            .as_ref()
            .expect("NullPointerException: name conflict checking is disabled")
            .contains(rule.get_label())
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:243 TargetRecorder.getMacroNamespaceViolatingTargets
    pub fn get_macro_namespace_violating_targets(&self) -> &IndexMap<String, String> {
        &self.macro_namespace_violating_targets
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:257 TargetRecorder.getTargetsToDeclaringMacro
    pub fn get_targets_to_declaring_macro(&self) -> Option<&IndexMap<String, Arc<MacroInstance>>> {
        self.targets_to_declaring_macro.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:269 TargetRecorder.getTargetsToDeclaringPackage
    pub fn get_targets_to_declaring_package(&self) -> Option<&IndexMap<String, PackageIdentifier>> {
        self.targets_to_declaring_package.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:291 TargetRecorder.setContainsErrors
    pub fn set_contains_errors(&mut self) {
        self.contains_errors = true;
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:295 TargetRecorder.containsErrors
    pub fn contains_errors(&self) -> bool {
        self.contains_errors
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:300 TargetRecorder.isNameConflictCheckingEnabled
    fn is_name_conflict_checking_enabled(&self) -> bool {
        self.rule_labels.is_some()
    }

    /// Inserts a target into `target_map`; returns the previous target, if any. No validation is
    /// done on the target's name.
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:310 TargetRecorder.putTargetInternal
    fn put_target_internal(&mut self, target: Target) -> Option<Target> {
        let name = target.get_name().to_owned();
        let existing = self.target_map.insert(name.clone(), target);
        if let (Some(targets_to_declaring_macro), Some(frame)) = (
            self.targets_to_declaring_macro.as_mut(),
            self.current_macro_frame.as_ref(),
        ) {
            targets_to_declaring_macro.insert(name, frame.macro_instance.clone());
        }
        existing
    }

    /// Adds a target (rules go through [`TargetRecorder::add_rule`]).
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:325 TargetRecorder.addTarget
    pub fn add_target(&mut self, target: Target) -> Result<(), NameConflictException> {
        if let Target::Rule(rule) = target {
            // Use addRule() to ensure all rule-related maps and caches are consulted.
            // checkTargetName() and putTargetInternal() are both reached through addRule().
            self.add_rule(rule)
        } else {
            self.check_target_name(&target)?;
            self.put_target_internal(target);
            Ok(())
        }
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:344 TargetRecorder.addInputFileUnchecked
    pub fn add_input_file_unchecked(&mut self, file: Arc<InputFile>) {
        let prev = self.put_target_internal(Target::InputFile(file));
        blaze_util::check_state!(prev.is_none());
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:354 TargetRecorder.replaceInputFileUnchecked
    pub fn replace_input_file_unchecked(&mut self, file: Arc<InputFile>) {
        let prev = self.put_target_internal(Target::InputFile(file));
        blaze_util::check_state!(
            matches!(prev, Some(Target::InputFile(_))),
            "{:?}",
            prev.as_ref().map(|t| t.to_string())
        );
    }

    /// Replaces an environment group by a processed copy (Java mutates the group in place in
    /// `finalBuilderValidationHook`).
    pub(crate) fn replace_environment_group(&mut self, group: Arc<EnvironmentGroup>) {
        let name = group.get_name().to_owned();
        let prev = self
            .target_map
            .insert(name, Target::EnvironmentGroup(group));
        blaze_util::check_state!(matches!(prev, Some(Target::EnvironmentGroup(_))));
    }

    /// Removes a target (Java: removal through the unwrapped target map in
    /// `Package.Builder.beforeBuild`).
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:374 TargetRecorder.unwrapSnapshottableBiMap
    pub(crate) fn remove_target(&mut self, name: &str) -> Option<Target> {
        self.target_map.shift_remove(name)
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:360 TargetRecorder.getTarget
    pub fn get_target(&self, name: &str) -> Option<&Target> {
        self.target_map.get(name)
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:383 TargetRecorder.getTargets
    pub fn get_targets(&self) -> impl Iterator<Item = &Target> {
        self.target_map.values()
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:393 TargetRecorder.getRules
    pub fn get_rules(&self) -> impl Iterator<Item = &Arc<Rule>> {
        self.target_map.values().filter_map(Target::as_rule)
    }

    /// Adds a rule and its output files without conflict checks.
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:415 TargetRecorder.addRuleUnchecked
    pub fn add_rule_unchecked(&mut self, rule: Arc<Rule>) {
        blaze_util::check_state!(
            !self.is_name_conflict_checking_enabled(),
            "Expected name conflict checking to be disabled"
        );
        self.add_rule_internal(rule);
    }

    /// Adds a rule and its outputs to the targets map, and propagates the error bit from the rule
    /// to the package.
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:401 TargetRecorder.addRuleInternal
    fn add_rule_internal(&mut self, rule: Arc<Rule>) {
        for output_file in rule.get_output_files() {
            self.put_target_internal(Target::OutputFile(output_file));
        }
        let contains_errors = rule.contains_errors();
        self.put_target_internal(Target::Rule(rule));
        if contains_errors {
            self.set_contains_errors();
        }
    }

    /// Adds a rule and its output files, checking for name conflicts.
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:424 TargetRecorder.addRule
    pub fn add_rule(&mut self, rule: Arc<Rule>) -> Result<(), NameConflictException> {
        blaze_util::check_state!(
            self.is_name_conflict_checking_enabled(),
            "Expected name conflict checking to be enabled"
        );

        let labels = rule.get_labels();
        self.check_rule_and_outputs(&rule, &labels)?;
        let label = rule.get_label().clone();
        self.add_rule_internal(rule);
        self.rule_labels
            .as_mut()
            .unwrap()
            .insert(label.clone(), labels);
        if self.current_macro_frame.is_some() {
            self.rules_created_in_macros.as_mut().unwrap().insert(label);
        }
        Ok(())
    }

    /// Adds a symbolic macro instance to the package.
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:437 TargetRecorder.addMacro
    pub fn add_macro(&mut self, macro_: Arc<MacroInstance>) -> Result<(), NameConflictException> {
        self.check_macro_name(&macro_)?;
        let prev = self.macro_map.insert(macro_.get_id(), macro_.clone());
        blaze_util::check_state!(prev.is_none());

        // Track whether a main submacro has been seen yet. Conflict checking for this is done in
        // checkMacroName().
        if let Some(frame) = self.current_macro_frame.as_mut()
            && macro_.get_name() == frame.macro_instance.get_name()
        {
            frame.main_submacro_has_been_defined = true;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:488 TargetRecorder.getCurrentMacroFrame
    pub fn get_current_macro_frame(&self) -> Option<&MacroFrame> {
        self.current_macro_frame.as_ref()
    }

    pub fn get_current_macro_frame_mut(&mut self) -> Option<&mut MacroFrame> {
        self.current_macro_frame.as_mut()
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:499 TargetRecorder.currentlyInNonFinalizerMacro
    pub fn currently_in_non_finalizer_macro(&self) -> bool {
        self.current_macro_frame
            .as_ref()
            .is_some_and(|f| !f.macro_instance.get_macro_class().is_finalizer())
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:507 TargetRecorder.currentlyInFinalizer
    pub fn currently_in_finalizer(&self) -> bool {
        self.current_macro_frame
            .as_ref()
            .is_some_and(|f| f.macro_instance.get_macro_class().is_finalizer())
    }

    /// Sets the current macro frame, returning the previous one.
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:518 TargetRecorder.setCurrentMacroFrame
    pub fn set_current_macro_frame(&mut self, frame: Option<MacroFrame>) -> Option<MacroFrame> {
        std::mem::replace(&mut self.current_macro_frame, frame)
    }

    /// True if `name` is `macro_name` or `macro_name` followed by `_`, `-` or `.` and more.
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:619 TargetRecorder.nameIsWithinMacroNamespace
    pub fn name_is_within_macro_namespace(name: &str, macro_name: &str) -> bool {
        if name == macro_name {
            return true;
        } else if let Some(suffix) = name.strip_prefix(macro_name) {
            // 0-length suffix handled above.
            // PORT: Java's String.length counts UTF-16 units; suffix has a 1-byte first char here.
            if suffix.chars().count() >= 2
                && (suffix.starts_with('_') || suffix.starts_with('.') || suffix.starts_with('-'))
            {
                return true;
            }
        }
        false
    }

    /// Precondition check for [`TargetRecorder::add_rule`] (Java `checkRuleAndOutputs`).
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:541 TargetRecorder.checkRuleAndOutputs
    fn check_rule_and_outputs(
        &mut self,
        rule: &Arc<Rule>,
        labels: &[Label],
    ) -> Result<(), NameConflictException> {
        blaze_util::check_state!(self.output_file_prefixes.is_some());

        // Check the name of the new rule itself.
        let rule_name = rule.get_name();
        self.check_target_name(&Target::Rule(rule.clone()))?;

        let output_files = rule.get_output_files();
        let mut output_files_by_name: HashMap<String, Arc<OutputFile>> = HashMap::new();

        // Check the new rule's output files, both for direct conflicts and prefix conflicts.
        for output_file in &output_files {
            let output_file_name = output_file.get_name().to_owned();
            // Check for duplicate within a single rule. (Can't use checkTargetName since this
            // rule's outputs aren't in the target map yet.)
            if output_files_by_name
                .insert(output_file_name.clone(), output_file.clone())
                .is_some()
            {
                return Err(NameConflictException::for_target(
                    format!(
                        "rule '{rule_name}' has more than one generated file named \
                         '{output_file_name}'"
                    ),
                    Target::OutputFile(output_file.clone()),
                ));
            }
            // Check for conflict with any other already added target.
            self.check_target_name(&Target::OutputFile(output_file.clone()))?;
            // TODO(bazel-team): We also need to check for a conflict between an output file and
            // its own rule, which is not yet in the targets map.

            // Check if this output file is the prefix of an already existing one.
            if let Some(existing) = self
                .output_file_prefixes
                .as_ref()
                .unwrap()
                .get(&output_file_name)
            {
                return Err(overlapping_output_file_prefixes(output_file, existing));
            }

            // Check if a prefix of this output file matches an already existing one.
            let output_file_fragment = PathFragment::create(&output_file_name);
            let segment_count = output_file_fragment.segment_count();
            for i in 1..segment_count {
                let prefix = output_file_fragment
                    .sub_fragment(0, i)
                    .get_path_string()
                    .to_owned();
                if let Some(existing) = output_files_by_name.get(&prefix) {
                    return Err(overlapping_output_file_prefixes(output_file, existing));
                }
                if let Some(Target::OutputFile(existing)) = self.target_map.get(&prefix) {
                    return Err(overlapping_output_file_prefixes(output_file, existing));
                }

                // Store in persistent map, for checking when adding future rules.
                self.output_file_prefixes
                    .as_mut()
                    .unwrap()
                    .entry(prefix)
                    .or_insert_with(|| output_file.clone());
            }
        }

        // Check for the same file appearing as both an input and output of the new rule.
        let package_identifier = rule.get_label().get_package_identifier();
        for input_label in labels {
            if package_identifier == input_label.get_package_identifier()
                && let Some(output) = output_files_by_name.get(input_label.get_name())
            {
                return Err(NameConflictException::for_target(
                    format!(
                        "rule '{}' has file '{}' as both an input and an output",
                        rule_name,
                        input_label.get_name()
                    ),
                    Target::OutputFile(output.clone()),
                ));
            }
        }
        Ok(())
    }

    /// Fails if the target's name conflicts; records macro naming violations.
    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:647 TargetRecorder.checkTargetName
    fn check_target_name(&mut self, target: &Target) -> Result<(), NameConflictException> {
        // We only care about the target's name, but we accept the full Target object to produce
        // better error messages.
        self.check_for_existing_target_name(target)?;

        self.check_for_existing_macro_name(&TargetOrMacro::Target(target))?;

        if let Some(frame) = &self.current_macro_frame
            && !Self::name_is_within_macro_namespace(
                target.get_name(),
                frame.macro_instance.get_name(),
            )
        {
            let macro_name = frame.macro_instance.get_name().to_owned();
            self.macro_namespace_violating_targets
                .insert(target.get_name().to_owned(), macro_name);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:695 TargetRecorder.checkForExistingTargetName
    fn check_for_existing_target_name(&self, target: &Target) -> Result<(), NameConflictException> {
        let Some(existing) = self.target_map.get(target.get_name()) else {
            return Ok(());
        };

        let mut subject = format!("{} '{}'", target.get_target_kind(), target.get_name());
        if let Target::OutputFile(given_output) = target {
            subject += &format!(
                " in rule '{}'",
                given_output.get_generating_rule().get_name()
            );
        }

        let mut object = match existing {
            Target::OutputFile(existing_output) => format!(
                "generated file from rule '{}'",
                existing_output.get_generating_rule().get_name()
            ),
            _ => existing.get_target_kind(),
        };
        object += &format!(", defined at {}", existing.get_location());

        Err(NameConflictException::for_target(
            format!("{subject} conflicts with existing {object}"),
            target.clone(),
        ))
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:725 TargetRecorder.checkMacroName
    fn check_macro_name(&self, macro_: &Arc<MacroInstance>) -> Result<(), NameConflictException> {
        let name = macro_.get_name();

        // A macro can share names with its main target but no other target. Since the macro
        // hasn't even been added yet, it hasn't run, and its main target is not yet defined.
        // Therefore, any match in the targets map represents a real conflict.
        if self.target_map.contains_key(name) {
            return Err(NameConflictException::for_macro(
                format!("macro '{name}' conflicts with an existing target."),
                macro_.clone(),
            ));
        }

        self.check_for_existing_macro_name(&TargetOrMacro::Macro(macro_))?;

        if let Some(frame) = &self.current_macro_frame
            && !Self::name_is_within_macro_namespace(name, frame.macro_instance.get_name())
        {
            return Err(NameConflictException::macro_namespace_violation(
                format!(
                    "macro '{}' cannot declare submacro named '{}'. {}",
                    frame.macro_instance.get_name(),
                    name,
                    MACRO_NAMING_RULES
                ),
                macro_.clone(),
            ));
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:759 TargetRecorder.checkForExistingMacroName
    fn check_for_existing_macro_name(
        &self,
        target_or_macro: &TargetOrMacro<'_>,
    ) -> Result<(), NameConflictException> {
        let name = target_or_macro.get_name();
        if !self.has_macro_with_name(name) {
            return Ok(());
        }

        // A conflict is still ok if it's only with enclosing macros. It's enough to check that 1)
        // we have the same name as the immediately enclosing macro (relying inductively on the
        // check that was done when that macro was added), and 2) there is no sibling macro of the
        // same name already defined in the current frame.
        if let Some(frame) = &self.current_macro_frame
            && name == frame.macro_instance.get_name()
            && !frame.main_submacro_has_been_defined
        {
            return Ok(());
        }

        Err(target_or_macro.conflict(format!(
            "{} '{}' conflicts with an existing macro (and was not created by it)",
            target_or_macro.get_kind(),
            name
        )))
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:670 TargetRecorder.putAllMacroNamespaceViolatingTargets
    pub fn put_all_macro_namespace_violating_targets(&mut self, targets: IndexMap<String, String>) {
        self.macro_namespace_violating_targets.extend(targets);
    }

    // Java: com/google/devtools/build/lib/packages/TargetRecorder.java:681 TargetRecorder.putAllTargetsToDeclaringPackage
    pub fn put_all_targets_to_declaring_package(
        &mut self,
        targets_to_declaring_package: IndexMap<String, PackageIdentifier>,
    ) {
        blaze_util::check_state!(
            self.targets_to_declaring_package.is_some(),
            "can only be called if trackFullMacroInformation was set to false in constructor"
        );
        self.targets_to_declaring_package
            .as_mut()
            .unwrap()
            .extend(targets_to_declaring_package);
    }
}

/// A [`NameConflictException`] about two output files clashing (one is a prefix of the other).
// Java: com/google/devtools/build/lib/packages/TargetRecorder.java:787 TargetRecorder.overlappingOutputFilePrefixes
fn overlapping_output_file_prefixes(
    added: &Arc<OutputFile>,
    existing: &Arc<OutputFile>,
) -> NameConflictException {
    if Arc::ptr_eq(added.get_generating_rule(), existing.get_generating_rule()) {
        NameConflictException::for_target(
            format!(
                "rule '{}' has conflicting output files '{}' and '{}'",
                added.get_generating_rule().get_name(),
                added.get_name(),
                existing.get_name()
            ),
            Target::OutputFile(added.clone()),
        )
    } else {
        NameConflictException::for_target(
            format!(
                "output file '{}' of rule '{}' conflicts with output file '{}' of rule '{}'",
                added.get_name(),
                added.get_generating_rule().get_name(),
                existing.get_name(),
                existing.get_generating_rule().get_name()
            ),
            Target::OutputFile(added.clone()),
        )
    }
}
