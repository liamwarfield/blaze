//! `OptionsParser`, the public face of the options parser.

use std::collections::BTreeMap;
use std::sync::Arc;

use indexmap::{IndexMap, IndexSet};

use crate::converter::ConversionContext;
use crate::converters::HelpVerbosity;
use crate::data::OptionsData;
use crate::definition::{OptionDefinitionRef, OptionsBase, OptionsClass, OptionsClassRef};
use crate::exception::{ConstructionException, OptionsParsingException};
use crate::global_rc::GlobalRcUtils;
use crate::option_filter_descriptions::OptionFilterDescriptions;
use crate::options_usage::OptionsUsage;
use crate::parsed::{OptionInstanceOrigin, ParsedOptionRef};
use crate::parser_impl::{OptionsParserImpl, OptionsParserImplResult};
use crate::priority::PriorityCategory;
use crate::tags::OptionDocumentationCategory;
use crate::value::Value;
use crate::value_description::OptionValueDescription;

/// Java's `String.trim()`: strips the characters `<= ' '` from both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

// Java: com/google/devtools/common/options/OptionsParser.java:111 OptionsParser.STARLARK_SKIPPED_PREFIXES
/// Skipped prefixes for starlark options.
pub const STARLARK_SKIPPED_PREFIXES: [&str; 4] = ["--//", "--no//", "--@", "--no@"];

// Java: com/google/devtools/common/options/ArgsPreProcessor.java:21 ArgsPreProcessor
/// A preprocessing step for the args list, executed before parsing.
pub trait ArgsPreProcessor: Send + Sync {
    fn pre_process(
        &self,
        args: Vec<ArgAndFallbackData>,
    ) -> Result<Vec<ArgAndFallbackData>, OptionsParsingException>;
}

// Java: com/google/devtools/common/options/OptionsParser.java:928 OptionsParser.ArgAndFallbackData
/// An arg plus the options data of other commands whose options should be silently ignored.
#[derive(Clone)]
pub struct ArgAndFallbackData {
    pub arg: String,
    pub fallback_data: Option<Arc<OptionsData>>,
}

impl ArgAndFallbackData {
    pub fn new(arg: impl Into<String>, fallback_data: Option<Arc<OptionsData>>) -> Self {
        ArgAndFallbackData {
            arg: arg.into(),
            fallback_data,
        }
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:937 OptionsParser.ArgAndFallbackData.wrapWithFallbackData
    pub fn wrap_with_fallback_data(
        args: impl IntoIterator<Item = impl Into<String>>,
        fallback_data: Option<Arc<OptionsData>>,
    ) -> Vec<ArgAndFallbackData> {
        args.into_iter()
            .map(|a| ArgAndFallbackData::new(a, fallback_data.clone()))
            .collect()
    }
}

impl std::fmt::Debug for ArgAndFallbackData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.arg)
    }
}

// Java: com/google/devtools/common/options/OptionsParser.java:389 OptionsParser.OptionDescription
/// The metadata about an option, in the context of a parser.
#[derive(Clone, Debug, PartialEq)]
pub struct OptionDescription {
    option_definition: OptionDefinitionRef,
    evaluated_expansion: Vec<String>,
}

impl OptionDescription {
    pub(crate) fn new(definition: OptionDefinitionRef, options_data: &OptionsData) -> Self {
        OptionDescription {
            option_definition: definition,
            evaluated_expansion: options_data.get_evaluated_expansion(definition),
        }
    }

    pub fn get_option_definition(&self) -> OptionDefinitionRef {
        self.option_definition
    }

    pub fn is_expansion(&self) -> bool {
        self.option_definition.is_expansion_option()
    }
}

// Java: com/google/devtools/common/options/OptionsParser.java:159 OptionsParser.Builder
/// Builder of [`OptionsParser`].
pub struct Builder {
    options_data: Option<Arc<OptionsData>>,
    error: Option<ConstructionException>,
    args_pre_processor: Option<Arc<dyn ArgsPreProcessor>>,
    skipped_prefixes: Vec<String>,
    ignore_internal_options: bool,
    is_first_round_of_parsing: bool,
    alias_flag: Option<String>,
    conversion_context: Option<Arc<ConversionContext>>,
    aliases: IndexMap<String, String>,
    allow_residue: bool,
    ignore_user_options: bool,
}

impl Builder {
    fn new() -> Self {
        Builder {
            options_data: None,
            error: None,
            args_pre_processor: None,
            skipped_prefixes: Vec::new(),
            ignore_internal_options: true,
            is_first_round_of_parsing: false,
            alias_flag: None,
            conversion_context: None,
            aliases: IndexMap::new(),
            allow_residue: true,
            ignore_user_options: false,
        }
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:170 OptionsParser.Builder.optionsData
    pub fn options_data(mut self, options_data: Arc<OptionsData>) -> Self {
        assert!(
            !options_data.created_with_allow_duplicates_parsing_equivalently(),
            "IllegalArgumentException"
        );
        self.options_data = Some(options_data);
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:185 OptionsParser.Builder.optionsClasses
    /// Sets the options classes. A `ConstructionException` is reported by [`Builder::build`].
    pub fn options_classes(mut self, options_classes: &[OptionsClassRef]) -> Self {
        match OptionsData::get(options_classes, false) {
            Ok(d) => self.options_data = Some(d),
            Err(e) => self.error = Some(e),
        }
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:202 OptionsParser.Builder.argsPreProcessor
    pub fn args_pre_processor(mut self, pre_processor: Arc<dyn ArgsPreProcessor>) -> Self {
        self.args_pre_processor = Some(pre_processor);
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:209 OptionsParser.Builder.skipStarlarkOptionPrefixes
    pub fn skip_starlark_option_prefixes(mut self) -> Self {
        for prefix in STARLARK_SKIPPED_PREFIXES {
            self.skipped_prefixes.push(prefix.to_string());
        }
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:223 OptionsParser.Builder.allowResidue
    pub fn allow_residue(mut self, allow_residue: bool) -> Self {
        self.allow_residue = allow_residue;
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:230 OptionsParser.Builder.ignoreInternalOptions
    pub fn ignore_internal_options(mut self, ignore_internal_options: bool) -> Self {
        self.ignore_internal_options = ignore_internal_options;
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:237 OptionsParser.Builder.ignoreUserOptions
    pub fn ignore_user_options(mut self) -> Self {
        self.ignore_user_options = true;
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:244 OptionsParser.Builder.withAliasFlag
    pub fn with_alias_flag(mut self, alias_flag: Option<&str>) -> Self {
        if let Some(a) = alias_flag {
            self.alias_flag = Some(a.to_string());
        }
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:254 OptionsParser.Builder.withAliases
    pub fn with_aliases(mut self, aliases: &IndexMap<String, String>) -> Self {
        for (k, v) in aliases {
            self.aliases.insert(k.clone(), v.clone());
        }
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:259 OptionsParser.Builder.withConversionContext
    pub fn with_conversion_context(mut self, conversion_context: Arc<ConversionContext>) -> Self {
        self.conversion_context = Some(conversion_context);
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:265 OptionsParser.Builder.isFirstRoundOfParsing
    pub fn is_first_round_of_parsing(mut self, is_first_round_of_parsing: bool) -> Self {
        self.is_first_round_of_parsing = is_first_round_of_parsing;
        self
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:271 OptionsParser.Builder.build
    pub fn build(self) -> Result<OptionsParser, ConstructionException> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let options_data = match self.options_data {
            Some(d) => d,
            None => OptionsData::get(&[], false)?,
        };
        Ok(OptionsParser {
            impl_: OptionsParserImpl::new(
                options_data,
                self.args_pre_processor,
                self.skipped_prefixes,
                self.ignore_internal_options,
                self.alias_flag,
                self.conversion_context,
                self.aliases,
                self.is_first_round_of_parsing,
            ),
            residue: Vec::new(),
            post_double_dash_residue: Vec::new(),
            allow_residue: self.allow_residue,
            ignore_user_options: self.ignore_user_options,
            starlark_options: BTreeMap::new(),
            scopes_attributes: BTreeMap::new(),
            aliases: IndexMap::new(),
            success: true,
        })
    }
}

// Java: com/google/devtools/common/options/OptionsParser.java:75 OptionsParser
/// A parser for options. Implements Java's `OptionsParsingResult` as inherent methods.
pub struct OptionsParser {
    impl_: OptionsParserImpl,
    residue: Vec<String>,
    post_double_dash_residue: Vec<String>,
    allow_residue: bool,
    ignore_user_options: bool,
    starlark_options: BTreeMap<String, Value>,
    scopes_attributes: BTreeMap<String, String>,
    aliases: IndexMap<String, String>,
    success: bool,
}

impl OptionsParser {
    // Java: com/google/devtools/common/options/OptionsParser.java:277 OptionsParser.builder
    pub fn builder() -> Builder {
        Builder::new()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:281 OptionsParser.toBuilder
    pub fn to_builder(&self) -> Builder {
        let mut b = Builder::new();
        b.options_data = Some(self.impl_.options_data.clone());
        b.args_pre_processor = self.impl_.args_pre_processor.clone();
        if let Some(a) = &self.impl_.alias_flag {
            b.alias_flag = Some(a.clone());
        }
        b.aliases = self.impl_.flag_alias_mappings().clone();
        b.conversion_context = self.impl_.conversion_context.clone();
        b.ignore_internal_options = self.impl_.ignore_internal_options;
        b.skipped_prefixes = self.impl_.skipped_prefixes.clone();
        b.allow_residue = self.allow_residue;
        b
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:122 OptionsParser.getOptionsData
    pub fn get_options_data(
        options_classes: &[OptionsClassRef],
    ) -> Result<Arc<OptionsData>, ConstructionException> {
        OptionsData::get(options_classes, false)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:127 OptionsParser.getFallbackOptionsData
    pub fn get_fallback_options_data(
        options_classes: &[OptionsClassRef],
    ) -> Result<Arc<OptionsData>, ConstructionException> {
        OptionsData::get(options_classes, true)
    }

    /// The options data used by this parser.
    pub fn options_data(&self) -> &Arc<OptionsData> {
        &self.impl_.options_data
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:303 OptionsParser.getConversionContext
    pub fn get_conversion_context(&self) -> Option<&Arc<ConversionContext>> {
        self.impl_.conversion_context.as_ref()
    }

    pub fn get_starlark_options(&self) -> &BTreeMap<String, Value> {
        &self.starlark_options
    }

    pub fn get_scopes_attributes(&self) -> &BTreeMap<String, String> {
        &self.scopes_attributes
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:318 OptionsParser.getExplicitStarlarkOptions
    pub fn get_explicit_starlark_options(
        &self,
        filter: impl Fn(&ParsedOptionRef) -> bool,
    ) -> BTreeMap<String, Value> {
        let explicit_options: IndexSet<String> = self
            .impl_
            .get_skipped_options()
            .iter()
            .filter(|d| d.is_explicit())
            .filter(|d| filter(d))
            .map(|d| {
                let form = d.get_command_line_form().unwrap_or("");
                form[2..].split('=').next().unwrap_or("").to_string()
            })
            .collect();
        self.starlark_options
            .iter()
            .filter(|(k, _)| explicit_options.contains(*k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn set_starlark_options(&mut self, starlark_options: BTreeMap<String, Value>) {
        self.starlark_options = starlark_options;
    }

    pub fn set_scopes_attributes(&mut self, scopes_attributes: BTreeMap<String, String>) {
        self.scopes_attributes = scopes_attributes;
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:369 OptionsParser.parseAndExitUponError
    pub fn parse_and_exit_upon_error(
        &mut self,
        priority: PriorityCategory,
        source: &str,
        args: &[&str],
    ) {
        if args.contains(&"--help") {
            // System.out: a PrintStream with the default charset (ISO-8859-1).
            let text = format!(
                "{}\n",
                self.describe_options_with_deprecated_categories(
                    &IndexMap::new(),
                    HelpVerbosity::LONG
                )
            );
            let _ = std::io::Write::write_all(
                &mut std::io::stdout(),
                &blaze_util::string_encoding::latin1_encode_lossy(&text),
            );
            std::process::exit(0);
        }
        if let Err(e) = self.parse_with_priority(
            priority,
            Some(source),
            args.iter().map(|s| s.to_string()).collect(),
        ) {
            // System.err: a PrintStream with the default charset (ISO-8859-1).
            let message = format!(
                "Error parsing command line: {}\nTry --help.\n",
                e.get_message()
            );
            let _ = std::io::Write::write_all(
                &mut std::io::stderr(),
                &blaze_util::string_encoding::latin1_encode_lossy(&message),
            );
            std::process::exit(2);
        }
    }

    /// Returns a description of all the options this parser can digest. In addition to `Option`
    /// annotations, this method also interprets `OptionsUsage` annotations which give an
    /// intuitive short description for the options. Options of the same category (see
    /// `OptionDocumentationCategory`) will be grouped together.
    ///
    /// `help_verbosity`: if `LONG`, the options will be described verbosely, including their
    /// types, defaults and descriptions. If `MEDIUM`, the descriptions are omitted, and if
    /// `SHORT`, the options are just enumerated.
    // Java: com/google/devtools/common/options/OptionsParser.java:443 OptionsParser.describeOptions
    pub fn describe_options(&self, help_verbosity: HelpVerbosity) -> String {
        let mut desc = String::new();
        let options_by_category = self.get_options_sorted_by_category();
        let option_category_descriptions =
            OptionFilterDescriptions::get_option_categories_enum_description();
        for (category, categorized_option_list) in &options_by_category {
            let category_description = &option_category_descriptions[category];

            // Describe the category if we're going to end up using it at all.
            if !categorized_option_list.is_empty() {
                desc.push('\n');
                desc.push_str(category_description);
                desc.push_str(":\n");
            }
            // Describe the options in this category.
            for option_def in categorized_option_list {
                OptionsUsage::get_usage(
                    option_def,
                    &mut desc,
                    help_verbosity,
                    &self.impl_.options_data,
                    true,
                );
            }
        }

        java_trim(&desc).to_string()
    }

    /// Returns a description of all the options this parser can digest, grouped by the
    /// deprecated `category` strings.
    ///
    /// `category_descriptions`: a mapping from category names to category descriptions.
    /// Descriptions are optional; if omitted, a string based on the category name will be used.
    // Java: com/google/devtools/common/options/OptionsParser.java:517 OptionsParser.describeOptionsWithDeprecatedCategories
    pub fn describe_options_with_deprecated_categories(
        &self,
        category_descriptions: &IndexMap<String, String>,
        help_verbosity: HelpVerbosity,
    ) -> String {
        let data = &self.impl_.options_data;
        let mut desc = String::new();
        if !data.get_options_classes().is_empty() {
            let mut all_fields: Vec<OptionDefinitionRef> = Vec::new();
            for options_class in data.get_options_classes() {
                all_fields.extend(options_class.definitions());
            }
            // OptionDefinition.BY_CATEGORY (a stable sort).
            all_fields.sort_by(|a, b| {
                a.get_option_category()
                    .cmp(b.get_option_category())
                    .then_with(|| a.get_option_name().cmp(b.get_option_name()))
            });
            let mut prev_category: Option<&str> = None;

            for option_definition in all_fields {
                let category = option_definition.get_option_category();
                if Some(category) != prev_category
                    && option_definition.get_documentation_category()
                        != OptionDocumentationCategory::UNDOCUMENTED
                {
                    let description = match category_descriptions.get(category) {
                        Some(d) => d.clone(),
                        None => format!("Options category '{}'", category),
                    };
                    desc.push('\n');
                    desc.push_str(&description);
                    desc.push_str(":\n");
                    prev_category = Some(category);
                }

                if option_definition.get_documentation_category()
                    != OptionDocumentationCategory::UNDOCUMENTED
                {
                    OptionsUsage::get_usage(
                        option_definition,
                        &mut desc,
                        help_verbosity,
                        data,
                        false,
                    );
                }
            }
        }
        java_trim(&desc).to_string()
    }

    /// Returns a string listing the possible flag completion for this command along with the
    /// command completion if any. See `OptionsUsage.getCompletion` for more details on the
    /// format for the flag completion.
    // Java: com/google/devtools/common/options/OptionsParser.java:557 OptionsParser.getOptionsCompletion
    pub fn get_options_completion(&self) -> String {
        let mut desc = String::new();

        self.visit_options(
            |option_definition| {
                option_definition.get_documentation_category()
                    != OptionDocumentationCategory::UNDOCUMENTED
            },
            |option_definition| OptionsUsage::get_completion(option_definition, &mut desc),
        );

        desc
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:568 OptionsParser.visitOptions
    pub fn visit_options(
        &self,
        predicate: impl Fn(OptionDefinitionRef) -> bool,
        mut visitor: impl FnMut(OptionDefinitionRef),
    ) {
        let mut all: Vec<OptionDefinitionRef> = self
            .impl_
            .options_data
            .get_options_classes()
            .iter()
            .flat_map(|c| c.definitions())
            .collect();
        all.sort_by(|a, b| a.get_option_name().cmp(b.get_option_name()));
        for d in all.into_iter().filter(|d| predicate(d)) {
            visitor(d);
        }
    }

    /// Returns all documented options loaded in this parser, grouped by categories in display
    /// order.
    // Java: com/google/devtools/common/options/OptionsParser.java:470 OptionsParser.getOptionsSortedByCategory
    pub fn get_options_sorted_by_category(
        &self,
    ) -> IndexMap<OptionDocumentationCategory, Vec<OptionDefinitionRef>> {
        let data = &self.impl_.options_data;
        if data.get_options_classes().is_empty() {
            return IndexMap::new();
        }

        // Get the documented options grouped by category.
        let mut options_by_categories: IndexMap<
            OptionDocumentationCategory,
            Vec<OptionDefinitionRef>,
        > = IndexMap::new();
        for options_class in data.get_options_classes() {
            for option_definition in options_class.definitions() {
                // Only track documented options.
                if option_definition.get_documentation_category()
                    != OptionDocumentationCategory::UNDOCUMENTED
                {
                    options_by_categories
                        .entry(option_definition.get_documentation_category())
                        .or_default()
                        .push(option_definition);
                }
            }
        }

        // Put the categories into display order and sort the options in each category.
        let mut sorted_categories_to_options = IndexMap::new();
        for category in OptionFilterDescriptions::DOCUMENTATION_ORDER {
            // PORT: Java's ListMultimap.get returns an empty (non-null) list for a missing key,
            // so every category in the display order is present.
            let mut option_list = options_by_categories
                .get(category)
                .cloned()
                .unwrap_or_default();
            option_list.sort_by(|a, b| a.get_option_name().cmp(b.get_option_name()));
            sorted_categories_to_options.insert(*category, option_list);
        }
        sorted_categories_to_options
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:591 OptionsParser.getOptionDescription
    pub fn get_option_description(&self, name: &str) -> Option<OptionDescription> {
        self.impl_.get_option_description(name)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:604 OptionsParser.getExpansionValueDescriptions
    pub fn get_expansion_value_descriptions(
        &mut self,
        expansion_option: OptionDefinitionRef,
        origin_of_expansion_option: &OptionInstanceOrigin,
    ) -> Result<Vec<ParsedOptionRef>, OptionsParsingException> {
        self.impl_
            .get_expansion_value_descriptions(expansion_option, origin_of_expansion_option)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:619 OptionsParser.getOptionValueDescription
    /// The value of the named option, or `None` if it was never set. Panics for unknown names.
    pub fn get_option_value_description(&self, name: &str) -> Option<OptionValueDescription> {
        self.impl_.get_option_value_description(name)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:627 OptionsParser.parse
    /// `parse(String... args)`: command-line priority, null source.
    pub fn parse(&mut self, args: &[&str]) -> Result<(), OptionsParsingException> {
        self.parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            None,
            args.iter().map(|s| s.to_string()).collect(),
        )
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:634 OptionsParser.parse
    pub fn parse_list(&mut self, args: Vec<String>) -> Result<(), OptionsParsingException> {
        self.parse_with_priority(PriorityCategory::COMMAND_LINE, None, args)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:654 OptionsParser.parse
    pub fn parse_with_priority(
        &mut self,
        priority: PriorityCategory,
        source: Option<&str>,
        args: Vec<String>,
    ) -> Result<(), OptionsParsingException> {
        let source = source.map(str::to_string);
        self.parse_with_source_function(priority, &move |_| source.clone(), args, None)?;
        Ok(())
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:678 OptionsParser.parseWithSourceFunction
    /// Returns the args that were ignored because they only resolved against `fallback_data`.
    pub fn parse_with_source_function(
        &mut self,
        priority: PriorityCategory,
        source_function: &dyn Fn(OptionDefinitionRef) -> Option<String>,
        args: Vec<String>,
        fallback_data: Option<Arc<OptionsData>>,
    ) -> Result<Vec<String>, OptionsParsingException> {
        assert!(
            priority != PriorityCategory::DEFAULT,
            "IllegalArgumentException"
        );
        let result = self.impl_.parse_category(
            priority,
            source_function,
            ArgAndFallbackData::wrap_with_fallback_data(args, fallback_data),
        )?;
        self.add_residue_from_result(&result)?;
        for (k, v) in &result.aliases {
            self.aliases.insert(k.clone(), v.clone());
        }
        Ok(result.ignored_args)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:709 OptionsParser.parseArgsAsExpansionOfOption
    pub fn parse_args_as_expansion_of_option(
        &mut self,
        option_to_expand: &ParsedOptionRef,
        source: &str,
        args: Vec<ArgAndFallbackData>,
    ) -> Result<Vec<String>, OptionsParsingException> {
        assert!(
            option_to_expand.get_priority().get_priority_category() != PriorityCategory::DEFAULT,
            "IllegalArgumentException: Priority cannot be default, which was specified for arglist {:?}",
            args
        );
        let source = source.to_string();
        let result = self.impl_.parse_args_as_expansion_of_option(
            option_to_expand,
            &move |_| Some(source.clone()),
            args,
        )?;
        self.add_residue_from_result(&result)?;
        Ok(result.ignored_args)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:725 OptionsParser.addResidueFromResult
    fn add_residue_from_result(
        &mut self,
        result: &OptionsParserImplResult,
    ) -> Result<(), OptionsParsingException> {
        self.residue.extend(result.get_residue());
        self.post_double_dash_residue
            .extend(result.post_double_dash_residue.iter().cloned());
        if !self.allow_residue && !self.residue.is_empty() {
            return Err(OptionsParsingException::new(format!(
                "Unrecognized arguments: {}",
                self.residue.join(" ")
            )));
        }
        Ok(())
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:745 OptionsParser.setOptionValueAtSpecificPriorityWithoutExpansion
    pub fn set_option_value_at_specific_priority_without_expansion(
        &mut self,
        origin: OptionInstanceOrigin,
        option: OptionDefinitionRef,
        value: &str,
    ) -> Result<(), OptionsParsingException> {
        self.impl_
            .set_option_value_at_specific_priority_without_expansion(origin, option, value)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:761 OptionsParser.clearValue
    pub fn clear_value(&mut self, option: OptionDefinitionRef) -> Option<OptionValueDescription> {
        self.impl_.clear_value(option)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:766 OptionsParser.getAliases
    pub fn get_aliases(&self) -> IndexMap<String, String> {
        self.aliases.clone()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:771 OptionsParser.setError
    pub fn set_error(&mut self) {
        self.success = false;
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:776 OptionsParser.success
    pub fn success(&self) -> bool {
        self.success
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:781 OptionsParser.getSkippedArgs
    pub fn get_skipped_args(&self) -> Vec<String> {
        self.impl_.get_skipped_args()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:786 OptionsParser.getResidue
    pub fn get_residue(&self) -> Vec<String> {
        self.residue.clone()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:791 OptionsParser.getPreDoubleDashResidue
    pub fn get_pre_double_dash_residue(&self) -> Vec<String> {
        if self.post_double_dash_residue.is_empty() {
            self.residue.clone()
        } else {
            self.residue
                .iter()
                .filter(|r| !self.post_double_dash_residue.contains(r))
                .cloned()
                .collect()
        }
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:804 OptionsParser.setResidue
    pub fn set_residue(&mut self, residue: Vec<String>, post_double_dash_residue: Vec<String>) {
        assert!(
            post_double_dash_residue.iter().all(|r| residue.contains(r)),
            "IllegalArgumentException"
        );
        self.residue = residue;
        self.post_double_dash_residue = post_double_dash_residue;
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:813 OptionsParser.getWarnings
    pub fn get_warnings(&self) -> Vec<String> {
        self.impl_.get_warnings()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:818 OptionsParser.getOptions
    /// A fresh instance of `O` with the parsed values, or `None` if `O` is not registered.
    pub fn get_options<O: OptionsClass>(&self) -> Option<O> {
        self.impl_.get_parsed_options::<O>()
    }

    /// Dynamic form of [`OptionsParser::get_options`].
    pub fn get_options_dyn(&self, class: OptionsClassRef) -> Option<Box<dyn OptionsBase>> {
        self.impl_.get_parsed_options_dyn(class)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:823 OptionsParser.containsExplicitOption
    pub fn contains_explicit_option(&self, name: &str) -> bool {
        self.impl_.contains_explicit_option(name)
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:828 OptionsParser.asCompleteListOfParsedOptions
    pub fn as_complete_list_of_parsed_options(&self) -> Vec<ParsedOptionRef> {
        self.impl_.as_complete_list_of_parsed_options()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:833 OptionsParser.asListOfExplicitOptions
    pub fn as_list_of_explicit_options(&self) -> Vec<ParsedOptionRef> {
        self.impl_.as_list_of_explicit_options()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:838 OptionsParser.asListOfCanonicalOptions
    pub fn as_list_of_canonical_options(&self) -> Vec<ParsedOptionRef> {
        self.impl_.as_canonicalized_list_of_parsed_options()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:843 OptionsParser.asListOfOptionValues
    pub fn as_list_of_option_values(&self) -> Vec<OptionValueDescription> {
        self.impl_.as_list_of_effective_options()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:848 OptionsParser.allOptionValues
    pub fn all_option_values(&self) -> Vec<OptionValueDescription> {
        self.impl_.all_option_values()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:853 OptionsParser.canonicalize
    /// The canonical form of the parsed options (`canonicalize-flags`).
    pub fn canonicalize(&self) -> Vec<String> {
        self.impl_.as_canonicalized_list()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:857 OptionsParser.getFinalExpansion
    fn get_final_expansion(option: &ParsedOptionRef) -> String {
        let Some(mut option) = option.get_expanded_from().cloned() else {
            return String::new();
        };
        while let Some(parent) = option.get_expanded_from().cloned() {
            option = parent;
        }
        option.get_canonical_form()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:868 OptionsParser.getUserOptions
    /// Options from user rc files or the command line, mapped to their top-level expansion.
    /// PORT: java-hash-order — Java builds a HashMap; this returns insertion order.
    pub fn get_user_options(&self) -> IndexMap<String, String> {
        if self.ignore_user_options {
            return IndexMap::new();
        }
        let mut user_options = IndexMap::new();
        for option in self.as_complete_list_of_parsed_options() {
            if GlobalRcUtils::is_global_rc_option(&option)
                || option.get_canonical_form().contains("default_override")
            {
                continue;
            }
            user_options.insert(
                option.get_canonical_form(),
                Self::get_final_expansion(&option),
            );
        }
        for option in self.impl_.get_skipped_options() {
            if GlobalRcUtils::is_global_rc_option(option) {
                continue;
            }
            let o = option.get_unconverted_value().unwrap_or("").to_string();
            let key = o.replace("--", "");
            let key = key.split('=').next().unwrap_or("");
            if self.starlark_options.contains_key(key) {
                user_options.insert(o, String::new());
            }
        }
        user_options
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:893 OptionsParser.getOptionDefinitions
    /// All options fields of the given options class, in alphabetic order.
    pub fn get_option_definitions(options_class: OptionsClassRef) -> Vec<OptionDefinitionRef> {
        options_class.definitions().collect()
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:908 OptionsParser.getOptionDefinitionByName
    pub fn get_option_definition_by_name(
        options_class: OptionsClassRef,
        option_name: &str,
    ) -> OptionDefinitionRef {
        let found: Vec<OptionDefinitionRef> = options_class
            .definitions()
            .filter(|d| d.get_option_name() == option_name)
            .collect();
        match found.len() {
            0 => panic!("NoSuchElementException"),
            1 => found[0],
            _ => panic!("IllegalArgumentException: expected one element"),
        }
    }

    // Java: com/google/devtools/common/options/OptionsParser.java:919 OptionsParser.getUsesOnlyCoreTypes
    pub fn get_uses_only_core_types(
        options_class: OptionsClassRef,
    ) -> Result<bool, ConstructionException> {
        Ok(OptionsData::get(&[options_class], false)?.get_uses_only_core_types(options_class))
    }
}
