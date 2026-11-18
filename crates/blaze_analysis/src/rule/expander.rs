//! Port of `analysis/Expander.java`.

use std::collections::BTreeSet;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_cmdline::Label;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::types::Type;
use blaze_shell::shell_utils::ShellUtils;
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::rule::location_expander::LocationTemplateContext;
use crate::rule::rule_context::RuleContext;
use crate::rule::rule_error_consumer::RuleErrorConsumer;
use crate::rule::stringtemplate::{TemplateContext, TemplateExpander};

/// Expansion of strings and string lists by replacing make variables and `$(location)`
/// functions.
// Java: com/google/devtools/build/lib/analysis/Expander.java:37 Expander
pub struct Expander<'a> {
    rule_context: &'a RuleContext,
    template_context: Arc<dyn TemplateContext + Send + Sync>,
    label_map: Option<IndexMap<Label, Vec<Artifact>>>,
    /// Which variables were looked up over this instance's lifetime? (shared with the rule
    /// context for `getRequiredConfigFragments`).
    looked_up_variables: Arc<Mutex<BTreeSet<String>>>,
}

impl<'a> Expander<'a> {
    // Java: com/google/devtools/build/lib/analysis/Expander.java:46 Expander.<init>
    pub fn new(
        rule_context: &'a RuleContext,
        template_context: Arc<dyn TemplateContext + Send + Sync>,
        label_map: Option<IndexMap<Label, Vec<Artifact>>>,
    ) -> Expander<'a> {
        Self::with_looked_up(rule_context, template_context, label_map, None)
    }

    // Java: com/google/devtools/build/lib/analysis/Expander.java:57 Expander.<init>
    fn with_looked_up(
        rule_context: &'a RuleContext,
        template_context: Arc<dyn TemplateContext + Send + Sync>,
        label_map: Option<IndexMap<Label, Vec<Artifact>>>,
        looked_up_variables: Option<Arc<Mutex<BTreeSet<String>>>>,
    ) -> Expander<'a> {
        Expander {
            rule_context,
            template_context,
            label_map,
            looked_up_variables: looked_up_variables
                .unwrap_or_else(|| Arc::new(Mutex::new(BTreeSet::new()))),
        }
    }

    /// The shared set of looked-up variables (registered with the rule context).
    pub fn looked_up_variables_handle(&self) -> Arc<Mutex<BTreeSet<String>>> {
        self.looked_up_variables.clone()
    }

    /// Returns a new instance that also expands locations using the default configuration of
    /// `LocationTemplateContext`.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:72 Expander.withLocations
    fn with_locations(&self, exec_paths: bool, allow_data: bool) -> Expander<'a> {
        let new_template_context = LocationTemplateContext::new(
            self.template_context.clone(),
            self.rule_context,
            self.label_map.clone(),
            exec_paths,
            allow_data,
            true,
            false,
        );
        Expander::with_looked_up(
            self.rule_context,
            Arc::new(new_template_context),
            self.label_map.clone(),
            Some(self.looked_up_variables.clone()),
        )
    }

    /// Returns a new instance that also expands locations, passing `allow_data` to the
    /// underlying `LocationTemplateContext`.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:84 Expander.withDataLocations
    pub fn with_data_locations(&self) -> Expander<'a> {
        self.with_locations(false, true)
    }

    /// Returns a new instance that also expands locations, passing `allow_data` and
    /// `exec_paths` to the underlying `LocationTemplateContext`.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:92 Expander.withDataExecLocations
    pub fn with_data_exec_locations(&self) -> Expander<'a> {
        self.with_locations(true, true)
    }

    /// Returns a new instance that also expands locations, passing the given location map, as
    /// well as `exec_paths` to the underlying `LocationTemplateContext`.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:101 Expander.withExecLocationsNoSrcs
    pub fn with_exec_locations_no_srcs(
        &self,
        locations: IndexMap<Label, Vec<Artifact>>,
        windows_path: bool,
    ) -> Expander<'a> {
        let new_template_context = LocationTemplateContext::new(
            self.template_context.clone(),
            self.rule_context,
            Some(locations),
            true,
            false,
            false,
            windows_path,
        );
        Expander::with_looked_up(
            self.rule_context,
            Arc::new(new_template_context),
            self.label_map.clone(),
            Some(self.looked_up_variables.clone()),
        )
    }

    /// Returns a new instance that also expands locations, passing the given location map, as
    /// well as `exec_paths` to the underlying `LocationTemplateContext`.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:115 Expander.withExecLocations
    pub fn with_exec_locations(&self, locations: IndexMap<Label, Vec<Artifact>>) -> Expander<'a> {
        let new_template_context = LocationTemplateContext::new(
            self.template_context.clone(),
            self.rule_context,
            Some(locations),
            true,
            false,
            true,
            false,
        );
        Expander::with_looked_up(
            self.rule_context,
            Arc::new(new_template_context),
            self.label_map.clone(),
            Some(self.looked_up_variables.clone()),
        )
    }

    /// Expands the given value string, tokenizes it, and then adds it to the given list. The
    /// attribute name is only used for error reporting.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:128 Expander.tokenizeAndExpandMakeVars
    pub fn tokenize_and_expand_make_vars(
        &self,
        result: &mut Vec<String>,
        attribute_name: &str,
        value: &str,
    ) {
        self.expand_value(result, attribute_name, value, true);
    }

    /// Expands make variables and tokenizes the result if `should_tokenize`, adding to `tokens`.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:137 Expander.expandValue
    fn expand_value(
        &self,
        tokens: &mut Vec<String>,
        attribute_name: &str,
        value: &str,
        should_tokenize: bool,
    ) {
        let value = self.expand(Some(attribute_name), value);
        if should_tokenize {
            if let Err(e) = ShellUtils::tokenize(tokens, &value) {
                self.rule_context
                    .attribute_error(attribute_name, &e.to_string());
            }
        } else {
            tokens.push(value);
        }
    }

    /// Returns the string "expression" after expanding all embedded references to "Make"
    /// variables. If any errors are encountered, they are reported, and "expression" is returned
    /// unchanged.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:156 Expander.expand
    pub fn expand_attribute(&self, attribute_name: &str) -> String {
        let value = self
            .rule_context
            .attributes()
            .get(attribute_name, Type::String)
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        self.expand(Some(attribute_name), &value)
    }

    /// Expands all references to "Make" variables embedded within string `expression`. If any
    /// errors are encountered, they are reported, and `expression` is returned unchanged.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:170 Expander.expand
    pub fn expand(&self, attribute_name: Option<&str>, expression: &str) -> String {
        match TemplateExpander::expand(expression, self.template_context.as_ref()) {
            Ok(expansion) => {
                self.looked_up_variables
                    .lock()
                    .extend(expansion.looked_up_variables);
                expansion.expansion
            }
            Err(e) => {
                match attribute_name {
                    None => self.rule_context.rule_error(&e.0),
                    Some(a) => self.rule_context.attribute_error(a, &e.0),
                }
                expression.to_owned()
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/Expander.java:186 Expander.expandAndTokenizeList
    fn expand_and_tokenize_list(
        &self,
        attr_name: &str,
        values: &[String],
        should_tokenize: bool,
    ) -> Vec<String> {
        let mut variables = Vec::new();
        for variable in values {
            self.expand_value(&mut variables, attr_name, variable, should_tokenize);
        }
        variables
    }

    /// Obtains the value of the attribute, expands all values, and returns the resulting list.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:198 Expander.list
    pub fn list_attribute(&self, attr_name: &str) -> Vec<String> {
        let values = self.string_list_attribute(attr_name);
        self.list(attr_name, &values)
    }

    /// Expands all the strings in the given list.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:206 Expander.list
    pub fn list(&self, attr_name: &str, values: &[String]) -> Vec<String> {
        self.expand_and_tokenize_list(attr_name, values, false)
    }

    /// Obtains the value of the attribute, expands, and tokenizes all values.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:214 Expander.tokenized
    pub fn tokenized_attribute(&self, attr_name: &str) -> Vec<String> {
        let values = self.string_list_attribute(attr_name);
        self.tokenized(attr_name, &values)
    }

    /// Expands all the strings in the given list, and tokenizes them after expansion.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:222 Expander.tokenized
    pub fn tokenized(&self, attr_name: &str, values: &[String]) -> Vec<String> {
        self.expand_and_tokenize_list(attr_name, values, true)
    }

    /// If the string consists of a single variable, returns the expansion of that variable.
    /// Otherwise, returns `None`. Syntax errors are reported.
    // Java: com/google/devtools/build/lib/analysis/Expander.java:233 Expander.expandSingleMakeVariable
    pub fn expand_single_make_variable(&self, attr_name: &str, expression: &str) -> Option<String> {
        match TemplateExpander::expand_single_variable(expression, self.template_context.as_ref()) {
            Ok(v) => v,
            Err(e) => {
                self.rule_context.attribute_error(attr_name, &e.0);
                Some(expression.to_owned())
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/Expander.java:243 Expander.lookedUpVariables
    pub fn looked_up_variables(&self) -> BTreeSet<String> {
        self.looked_up_variables.lock().clone()
    }

    fn string_list_attribute(&self, attr_name: &str) -> Vec<String> {
        self.rule_context
            .attributes()
            .get(attr_name, Type::StringList)
            .and_then(|v| v.as_string_list().map(|s| s.to_vec()))
            .unwrap_or_default()
    }
}
