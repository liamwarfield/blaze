//! Port of `Options<O>`: parsing options from a single options class.

use crate::definition::OptionsClass;
use crate::exception::OptionsParsingException;
use crate::parser::OptionsParser;
use crate::priority::PriorityCategory;

// Java: com/google/devtools/common/options/Options.java:51 Options
/// The parse result for a single options class: the options plus the residue.
#[derive(Clone, Debug)]
pub struct Options<O: OptionsClass> {
    options: O,
    remaining_args: Vec<String>,
}

impl<O: OptionsClass> Options<O> {
    // Java: com/google/devtools/common/options/Options.java:57 Options.parse
    pub fn parse(args: &[&str]) -> Result<Options<O>, OptionsParsingException> {
        let mut parser = OptionsParser::builder()
            .options_classes(&[O::class()])
            .build()
            .unwrap_or_else(|e| e.throw());
        parser.parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            None,
            args.iter().map(|s| s.to_string()).collect(),
        )?;
        let remaining_args = parser.get_residue();
        Ok(Options {
            options: parser.get_options::<O>().expect("options class registered"),
            remaining_args,
        })
    }

    // Java: com/google/devtools/common/options/Options.java:89 Options.getDefaults
    /// An options object at its default values.
    pub fn get_defaults() -> O {
        match Self::parse(&[]) {
            Ok(o) => o.options,
            Err(e) => panic!(
                "AssertionError: Error while parsing defaults: {}",
                e.get_message()
            ),
        }
    }

    // Java: com/google/devtools/common/options/Options.java:102 Options.getUsage
    pub fn get_usage() -> String {
        crate::unported!("Options.getUsage (OptionsUsage)")
    }

    // Java: com/google/devtools/common/options/Options.java:119 Options.getOptions
    pub fn get_options(&self) -> &O {
        &self.options
    }

    pub fn into_options(self) -> O {
        self.options
    }

    // Java: com/google/devtools/common/options/Options.java:126 Options.getRemainingArgs
    pub fn get_remaining_args(&self) -> &[String] {
        &self.remaining_args
    }
}
