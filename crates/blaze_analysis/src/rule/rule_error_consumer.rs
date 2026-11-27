//! Ports of `analysis/RuleErrorConsumer.java`, `packages/RuleClass.RuleErrorException`,
//! `analysis/EventHandlingErrorReporter.java`, `RuleContext.ErrorReporter` and
//! `RuleContext.SuppressingErrorReporter`.

use std::fmt;
use std::sync::Arc;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler, ExtendedEventHandler, Postable};
use blaze_packages::attribute::Attribute;
use blaze_packages::target::Rule;
use parking_lot::Mutex;

use crate::rule::analysis_environment::CachingAnalysisEnvironment;
use crate::rule::to_event_location;

/// An exception to indicate that rule analysis failed (Java `RuleClass.RuleErrorException`).
/// The error has already been reported on the rule context.
// Java: com/google/devtools/build/lib/packages/RuleClass.java:227 RuleClass.RuleErrorException
#[derive(Debug, Clone, Default, thiserror::Error)]
pub struct RuleErrorException {
    /// Java `getMessage()` (`null` for the no-argument constructor).
    pub message: Option<String>,
}

impl RuleErrorException {
    pub fn new(message: impl Into<String>) -> RuleErrorException {
        RuleErrorException {
            message: Some(message.into()),
        }
    }

    /// Java `new RuleErrorException()`.
    pub fn empty() -> RuleErrorException {
        RuleErrorException { message: None }
    }
}

impl fmt::Display for RuleErrorException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.message {
            Some(m) => f.write_str(m),
            None => f.write_str("null"),
        }
    }
}

/// A thin interface exposing only the warning and error reporting functionality of a rule.
///
/// When a class only needs access to reporting functionality, it is preferable to use this
/// interface rather than passing `RuleContext`.
// Java: com/google/devtools/build/lib/analysis/RuleErrorConsumer.java:27 RuleErrorConsumer
pub trait RuleErrorConsumer: Send + Sync {
    /// Consume a non-attribute-specific warning in a rule.
    fn rule_warning(&self, message: &str);

    /// Consume a non-attribute-specific error in a rule.
    fn rule_error(&self, message: &str);

    /// Consume an attribute-specific warning in a rule.
    fn attribute_warning(&self, attr_name: &str, message: &str);

    /// Consume an attribute-specific error in a rule.
    fn attribute_error(&self, attr_name: &str, message: &str);

    /// Returns whether this instance is known to have errors at this point during analysis. Do
    /// not call this method after the analysis phase is complete.
    fn has_errors(&self) -> bool;

    /// Convenience function to report non-attribute-specific errors in the current rule and then
    /// throw a [`RuleErrorException`], immediately exiting the build invocation. Alternatively,
    /// invoke `rule_error` instead to collect additional error information before ending the
    /// invocation.
    ///
    /// PORT: Java throws; the Rust caller returns `Err(<this value>)`.
    // Java: com/google/devtools/build/lib/analysis/RuleErrorConsumer.java:56 RuleErrorConsumer.throwWithRuleError
    fn throw_with_rule_error(&self, message: &str) -> RuleErrorException {
        self.rule_error(message);
        RuleErrorException::new(message)
    }

    /// Convenience function to report attribute-specific errors in the current rule, and then
    /// throw a [`RuleErrorException`].
    // Java: com/google/devtools/build/lib/analysis/RuleErrorConsumer.java:88 RuleErrorConsumer.throwWithAttributeError
    fn throw_with_attribute_error(&self, attr_name: &str, message: &str) -> RuleErrorException {
        self.attribute_error(attr_name, message);
        RuleErrorException::new(message)
    }

    /// Convenience function that throws if there are errors.
    // Java: com/google/devtools/build/lib/analysis/RuleErrorConsumer.java:103 RuleErrorConsumer.assertNoErrors
    fn assert_no_errors(&self) -> Result<(), RuleErrorException> {
        if self.has_errors() {
            return Err(RuleErrorException::empty());
        }
        Ok(())
    }
}

/// Error message of an analysis root cause (Java `AnalysisRootCauseEvent`, posted on the first
/// error of a configured target). The build event stream turns it into its build event
/// (`blaze_bep`'s `AnalysisRootCauseBuildEvent`).
// Java: com/google/devtools/build/lib/analysis/AnalysisRootCauseEvent.java:40 AnalysisRootCauseEvent
#[derive(Debug, Clone)]
pub struct AnalysisRootCauseEvent {
    /// A tri-state representation of the configuration to capture two different notions of
    /// nullness: `Some(Some(value))` a configuration value; `Some(None)` the *null
    /// configuration*, used for unconfigurable targets, for example source files; `None` an
    /// *unavailable configuration*: sometimes errors may occur for a transient
    /// `BuildConfigurationKey` for which a `BuildConfigurationValue` is never computed, for
    /// example, the intermediate configuration after the attribute transition occurs but before
    /// the rule transition.
    pub configuration: Option<Option<Arc<BuildConfigurationValue>>>,
    /// `configurationId`: the checksum of the configuration id (`None` is the null
    /// configuration's id, `BuildEventIdUtil.nullConfigurationIdMessage()`).
    pub configuration_id: Option<String>,
    pub label: Label,
    pub error_message: String,
}

impl AnalysisRootCauseEvent {
    // Java: com/google/devtools/build/lib/analysis/AnalysisRootCauseEvent.java:60 AnalysisRootCauseEvent.withConfigurationValue
    pub fn with_configuration_value(
        configuration: Option<Arc<BuildConfigurationValue>>,
        label: Label,
        error_message: String,
    ) -> AnalysisRootCauseEvent {
        AnalysisRootCauseEvent {
            // BuildConfigurationValue.configurationIdMessage(configuration)
            configuration_id: configuration.as_ref().map(|c| c.checksum().to_owned()),
            configuration: Some(configuration),
            label,
            error_message,
        }
    }

    /// `configuration_id`: the checksum of `configurationIdMessage(configurationKey)` (`None`
    /// for a null key).
    // Java: com/google/devtools/build/lib/analysis/AnalysisRootCauseEvent.java:69 AnalysisRootCauseEvent.withUnavailableConfiguration
    pub fn with_unavailable_configuration(
        configuration_id: Option<String>,
        label: Label,
        error_message: String,
    ) -> AnalysisRootCauseEvent {
        AnalysisRootCauseEvent {
            configuration: None,
            configuration_id,
            label,
            error_message,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisRootCauseEvent.java:84 AnalysisRootCauseEvent.getLabel
    pub fn get_label(&self) -> &Label {
        &self.label
    }
}

impl Postable for AnalysisRootCauseEvent {
    // Java: com/google/devtools/build/lib/analysis/AnalysisRootCauseEvent.java:84 AnalysisRootCauseEvent.storeForReplay
    fn store_for_replay(&self) -> bool {
        true
    }
}

/// Base class for implementations of [`RuleErrorConsumer`] that report errors and warnings to the
/// analysis environment's event handler (Java `EventHandlingErrorReporter`), specialized as
/// `RuleContext.ErrorReporter` (the only subclass this crate needs).
// Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:30 EventHandlingErrorReporter
// Java: com/google/devtools/build/lib/analysis/RuleContext.java:2176 RuleContext.ErrorReporter
pub struct ErrorReporter {
    rule_class_name_for_logging: String,
    env: Arc<CachingAnalysisEnvironment>,
    rule: Arc<Rule>,
    configuration: Arc<BuildConfigurationValue>,
}

impl ErrorReporter {
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2181 RuleContext.ErrorReporter.<init>
    pub fn new(
        env: Arc<CachingAnalysisEnvironment>,
        rule: Arc<Rule>,
        configuration: Arc<BuildConfigurationValue>,
        rule_class_name_for_logging: String,
    ) -> ErrorReporter {
        ErrorReporter {
            rule_class_name_for_logging,
            env,
            rule,
            configuration,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:40 EventHandlingErrorReporter.reportError
    fn report_error(&self, message: String) {
        if !self.has_errors() {
            self.env.get_event_handler().post(Arc::new(
                AnalysisRootCauseEvent::with_configuration_value(
                    Some(self.configuration.clone()),
                    self.get_label().clone(),
                    message.clone(),
                ),
            ));
        }
        self.env.get_event_handler().handle(&Event::error_at(
            Some(to_event_location(self.rule.get_location())),
            message,
        ));
    }

    // Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:63 EventHandlingErrorReporter.reportWarning
    pub fn report_warning(&self, message: String) {
        self.env.get_event_handler().handle(&Event::warn_at(
            Some(to_event_location(self.rule.get_location())),
            message,
        ));
    }

    // Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:78 EventHandlingErrorReporter.prefixRuleMessage
    fn prefix_rule_message(&self, message: &str) -> String {
        format!(
            "in {} rule {}: {}",
            self.rule_class_name_for_logging,
            self.get_label(),
            message
        )
    }

    // Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:82 EventHandlingErrorReporter.maskInternalAttributeNames
    fn mask_internal_attribute_names(name: &str) -> &str {
        if Attribute::is_implicit_name(name) {
            "(an implicit dependency)"
        } else {
            name
        }
    }

    // Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:91 EventHandlingErrorReporter.completeAttributeMessage
    fn complete_attribute_message(&self, attr_name: &str, message: &str) -> String {
        format!(
            "in {} attribute of {} rule {}: {}{}",
            Self::mask_internal_attribute_names(attr_name),
            self.rule_class_name_for_logging,
            self.get_label(),
            message,
            self.get_macro_message_appendix(attr_name)
        )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2192 RuleContext.ErrorReporter.getMacroMessageAppendix
    fn get_macro_message_appendix(&self, _unused_attr_name: &str) -> String {
        // TODO(b/141234726):  Historically this reported the location
        // of the rule attribute in the macro call (assuming no **kwargs),
        // but we no longer locations for individual attributes.
        // We should record the instantiation call stack in each rule
        // and report the position of its topmost frame here.
        use blaze_packages::target::RuleOrMacroInstance;
        if self.rule.is_rule_created_in_macro() {
            format!(
                ". Since this rule was created by the macro '{}', the error might have been \
                 caused by the macro implementation",
                self.get_generator_function()
            )
        } else {
            String::new()
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2206 RuleContext.ErrorReporter.getGeneratorFunction
    fn get_generator_function(&self) -> String {
        use blaze_packages::target::RuleOrMacroInstance;
        match self.rule.get_attr("generator_function") {
            Some(v) => v.as_str().unwrap_or("null").to_owned(),
            None => "null".to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2211 RuleContext.ErrorReporter.getLabel
    fn get_label(&self) -> &Label {
        self.rule.get_label()
    }
}

impl RuleErrorConsumer for ErrorReporter {
    // Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:68 EventHandlingErrorReporter.ruleWarning
    fn rule_warning(&self, message: &str) {
        self.env.get_event_handler().handle(&Event::warn_at(
            Some(to_event_location(self.rule.get_location())),
            self.prefix_rule_message(message),
        ));
    }

    // Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:49 EventHandlingErrorReporter.ruleError
    fn rule_error(&self, message: &str) {
        self.report_error(self.prefix_rule_message(message));
    }

    // Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:73 EventHandlingErrorReporter.attributeWarning
    fn attribute_warning(&self, attr_name: &str, message: &str) {
        self.report_warning(self.complete_attribute_message(attr_name, message));
    }

    // Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:54 EventHandlingErrorReporter.attributeError
    fn attribute_error(&self, attr_name: &str, message: &str) {
        self.report_error(self.complete_attribute_message(attr_name, message));
    }

    // Java: com/google/devtools/build/lib/analysis/EventHandlingErrorReporter.java:59 EventHandlingErrorReporter.hasErrors
    fn has_errors(&self) -> bool {
        self.env.has_errors()
    }
}

/// Implementation of an error consumer which does not post any events, saves rule and attribute
/// errors for future consumption, and drops warnings.
// Java: com/google/devtools/build/lib/analysis/RuleContext.java:2230 RuleContext.SuppressingErrorReporter
#[derive(Default)]
pub struct SuppressingErrorReporter {
    error_messages: Mutex<Vec<String>>,
}

impl SuppressingErrorReporter {
    pub fn new() -> SuppressingErrorReporter {
        SuppressingErrorReporter::default()
    }

    /// Returns the error message strings reported to this error consumer.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2255 RuleContext.SuppressingErrorReporter.getErrorMessages
    pub fn get_error_messages(&self) -> Vec<String> {
        self.error_messages.lock().clone()
    }
}

impl RuleErrorConsumer for SuppressingErrorReporter {
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2233 RuleContext.SuppressingErrorReporter.ruleWarning
    fn rule_warning(&self, _message: &str) {}

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2236 RuleContext.SuppressingErrorReporter.ruleError
    fn rule_error(&self, message: &str) {
        self.error_messages.lock().push(message.to_owned());
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2241 RuleContext.SuppressingErrorReporter.attributeWarning
    fn attribute_warning(&self, _attr_name: &str, _message: &str) {}

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2244 RuleContext.SuppressingErrorReporter.attributeError
    fn attribute_error(&self, _attr_name: &str, message: &str) {
        self.error_messages.lock().push(message.to_owned());
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2249 RuleContext.SuppressingErrorReporter.hasErrors
    fn has_errors(&self) -> bool {
        !self.error_messages.lock().is_empty()
    }
}

/// The reporter of a rule context (Java `RuleErrorConsumer reporter`, one of the two classes
/// above; `getSuppressedErrorMessages` downcasts).
pub enum RuleReporter {
    Events(ErrorReporter),
    Suppressing(SuppressingErrorReporter),
}

impl RuleErrorConsumer for RuleReporter {
    fn rule_warning(&self, message: &str) {
        match self {
            RuleReporter::Events(r) => r.rule_warning(message),
            RuleReporter::Suppressing(r) => r.rule_warning(message),
        }
    }

    fn rule_error(&self, message: &str) {
        match self {
            RuleReporter::Events(r) => r.rule_error(message),
            RuleReporter::Suppressing(r) => r.rule_error(message),
        }
    }

    fn attribute_warning(&self, attr_name: &str, message: &str) {
        match self {
            RuleReporter::Events(r) => r.attribute_warning(attr_name, message),
            RuleReporter::Suppressing(r) => r.attribute_warning(attr_name, message),
        }
    }

    fn attribute_error(&self, attr_name: &str, message: &str) {
        match self {
            RuleReporter::Events(r) => r.attribute_error(attr_name, message),
            RuleReporter::Suppressing(r) => r.attribute_error(attr_name, message),
        }
    }

    fn has_errors(&self) -> bool {
        match self {
            RuleReporter::Events(r) => r.has_errors(),
            RuleReporter::Suppressing(r) => r.has_errors(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: com/google/devtools/build/lib/analysis/RuleContextTest (SuppressingErrorReporter
    // behaviour is exercised through allow_analysis_failures tests; these check the class).
    #[test]
    fn suppressing_error_reporter_collects_errors_and_drops_warnings() {
        let r = SuppressingErrorReporter::new();
        assert!(!r.has_errors());
        r.rule_warning("w");
        r.attribute_warning("srcs", "w");
        assert!(!r.has_errors());
        r.rule_error("e1");
        r.attribute_error("srcs", "e2");
        assert!(r.has_errors());
        assert_eq!(
            r.get_error_messages(),
            vec!["e1".to_owned(), "e2".to_owned()]
        );
    }

    #[test]
    fn throw_with_rule_error_reports_and_returns_exception() {
        let r = SuppressingErrorReporter::new();
        let e = r.throw_with_rule_error("bad");
        assert_eq!(e.to_string(), "bad");
        assert_eq!(r.get_error_messages(), vec!["bad".to_owned()]);
        assert!(r.assert_no_errors().is_err());
    }
}
