//! Port of `analysis/config/OutputPathMnemonicComputer.java`.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_options::definition::OptionsBase;
use blaze_options::value::java_string_of;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use crate::build_options::BuildOptions;
use crate::fragment::{
    AddToMnemonicException, Fragment, FragmentClass, InvalidConfigurationException,
    OutputDirectoriesContext,
};
use crate::fragment_options_class::{FragmentOptionsClass, TEST_OPTIONS};
use crate::options_diff::{OptionInfo, OptionsDiff};
use crate::transitions::test_trimming_trim;

// The length of the hash of the config tacked onto the end of the output path.
// Limited for ergonomics and MAX_PATH reasons.
const HASH_LENGTH: usize = 12;

/// `LabelConstants.COMMAND_LINE_OPTION_PREFIX`.
pub const COMMAND_LINE_OPTION_PREFIX: &str = "//command_line_option:";

// Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:55 OutputPathMnemonicComputer.InvalidMnemonicException
/// Indicates a failure to construct the mnemonic for an output directory.
fn invalid_mnemonic_exception(message: &str, e: &str) -> InvalidConfigurationException {
    InvalidConfigurationException::with_code(
        format!("{} is invalid as part of a path: {}", message, e),
        "INVALID_OUTPUT_DIRECTORY_MNEMONIC",
    )
}

// Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:73 OutputPathMnemonicComputer.MnemonicContext
/// A fresh context to pass to `Fragment.processForOutputPathMnemonic`.
struct MnemonicContext<'a> {
    baseline_options: Option<&'a BuildOptions>,
    mnemonic_builder: String,
    explicit_in_output_path_builder: IndexSet<String>,
}

impl<'a> MnemonicContext<'a> {
    fn new(baseline_options: Option<&'a BuildOptions>) -> Self {
        MnemonicContext {
            baseline_options,
            mnemonic_builder: String::new(),
            explicit_in_output_path_builder: IndexSet::new(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:127 MnemonicContext.consume
    fn consume(
        &mut self,
        fragment_class: FragmentClass,
        fragment: &dyn Fragment,
    ) -> Result<(), InvalidConfigurationException> {
        fragment
            .process_for_output_path_mnemonic(self)
            .map_err(|e| {
                invalid_mnemonic_exception(
                    &format!(
                        "Output directory name '{}' specified by {}",
                        e.bad_value,
                        fragment_class.get_simple_name()
                    ),
                    &e.tunneled_exception,
                )
            })
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:140 MnemonicContext.checkedAddToMnemonic
    fn checked_add_to_mnemonic(
        &mut self,
        value: Option<&str>,
        value_ctx: &str,
    ) -> Result<(), InvalidConfigurationException> {
        self.add_to_mnemonic(value).map_err(|e| {
            invalid_mnemonic_exception(
                &format!("{} '{}'", value_ctx, e.bad_value),
                &e.tunneled_exception,
            )
        })
    }

    fn get_mnemonic(&self) -> String {
        self.mnemonic_builder.clone()
    }

    fn get_explicit_in_output_path_options(&self) -> IndexSet<String> {
        self.explicit_in_output_path_builder.clone()
    }
}

impl OutputDirectoriesContext for MnemonicContext<'_> {
    // Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:88 MnemonicContext.getBaseline
    fn get_baseline(&self, options_class: FragmentOptionsClass) -> Option<&dyn OptionsBase> {
        self.baseline_options?.get_dyn(options_class)
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:98 MnemonicContext.addToMnemonic
    fn add_to_mnemonic(&mut self, value: Option<&str>) -> Result<(), AddToMnemonicException> {
        let Some(value) = value.filter(|v| !v.is_empty()) else {
            return Ok(());
        };
        // Allowing for path separators (e.g. /) would be a disaster.
        match PathFragment::check_separators(value) {
            Ok(()) => {
                // Want dashes in-between additions.
                if !self.mnemonic_builder.is_empty() {
                    self.mnemonic_builder.push('-');
                }
                self.mnemonic_builder.push_str(value);
                Ok(())
            }
            Err(e) => Err(AddToMnemonicException {
                tunneled_exception: e.message().to_string(),
                bad_value: value.to_string(),
            }),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:121 MnemonicContext.markAsExplicitInOutputPathFor
    fn mark_as_explicit_in_output_path_for(&mut self, option_name: &str) {
        self.explicit_in_output_path_builder
            .insert(option_name.to_string());
    }
}

// Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:179 OutputPathMnemonicComputer.computeMnemonic
/// Compute and return the output path mnemonic.
///
/// The general form is `[cpu]-[compilation_mode]-[platform_suffix?]-...-[-ST-hash?]` where ... is
/// any additions requested by the `Fragment` via `addToMnemonic` during calls to
/// `processForOutputPathMnemonic`.
pub fn compute_mnemonic(
    build_options: &BuildOptions,
    baseline_options: Option<&BuildOptions>,
    fragments: &[(FragmentClass, Arc<dyn Fragment>)],
) -> Result<String, InvalidConfigurationException> {
    let core_options = build_options.get::<CoreOptions>().expect("CoreOptions");

    if build_options.has_no_config() {
        // Historically, the noconfig output path mnemonic had the compilation mode.
        return Ok(format!(
            "{}-noconfig",
            blaze_options::value::OptionScalar::java_to_string(&core_options.compilation_mode)
        )); // See NoConfigTransition.
    }

    let platform_options = build_options.get::<PlatformOptions>();

    let mut ctx = MnemonicContext::new(baseline_options);

    handle_platform_cpu_descriptor(&mut ctx, core_options, platform_options)?;

    ctx.checked_add_to_mnemonic(
        Some(&blaze_options::value::OptionScalar::java_to_string(
            &core_options.compilation_mode,
        )),
        "Compilation mode",
    )?;
    ctx.mark_as_explicit_in_output_path_for("compilation_mode");

    if let Some(platform_suffix) = core_options
        .platform_suffix
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        ctx.checked_add_to_mnemonic(Some(platform_suffix), "Platform suffix")?;
    }
    ctx.mark_as_explicit_in_output_path_for("platform_suffix");

    for (fragment_class, fragment) in fragments {
        ctx.consume(fragment_class, fragment.as_ref())?;
    }

    let explicit_in_output_path_options = ctx.get_explicit_in_output_path_options();

    // Sanity check that every listed option in explicitInOutputPathOptions actually exists.
    let option_info_map = OptionInfo::build_map_from(build_options);
    let missing_options: Vec<&String> = explicit_in_output_path_options
        .iter()
        .filter(|option_name| !option_info_map.contains_key(*option_name))
        .collect();
    if !missing_options.is_empty() {
        panic!(
            "IllegalStateException: Internal error: Options registered for special output handling that do not exist: [{}]",
            missing_options
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    match baseline_options {
        None => {
            let fragment =
                compute_name_fragment_with_affected_by_starlark_transition(build_options);
            ctx.checked_add_to_mnemonic(Some(&fragment), "Transition directory name fragment")?;
        }
        Some(baseline_options) => {
            let fragment = compute_name_fragment_with_diff(
                build_options,
                baseline_options,
                &explicit_in_output_path_options,
            );
            ctx.checked_add_to_mnemonic(Some(&fragment), "Transition directory name fragment")?;
        }
    }
    Ok(ctx.get_mnemonic())
}

// Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:196 OutputPathMnemonicComputer.handlePlatformCpuDescriptor
fn handle_platform_cpu_descriptor(
    ctx: &mut MnemonicContext,
    core_options: &CoreOptions,
    platform_options: Option<&PlatformOptions>,
) -> Result<(), InvalidConfigurationException> {
    let Some(platform_options) = platform_options
        .filter(|p| core_options.use_platform_in_output_dir(p.compute_target_platform()))
    else {
        ctx.checked_add_to_mnemonic(Some(&core_options.cpu), "CPU/Platform descriptor")?;
        ctx.mark_as_explicit_in_output_path_for("cpu");
        return Ok(());
    };

    if platform_options.platforms.len() > 1 {
        ctx.checked_add_to_mnemonic(Some("multi-platform"), "CPU/Platform descriptor")?;
        // Intentionally not marking anything as explicit in output path so ST-hash used if
        // needed.
        return Ok(());
    }

    ctx.checked_add_to_mnemonic(
        Some(&compute_platform_name(
            platform_options.compute_target_platform(),
            core_options,
        )),
        "CPU/Platform descriptor",
    )?;
    ctx.mark_as_explicit_in_output_path_for("platforms");
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:258 OutputPathMnemonicComputer.computePlatformName
fn compute_platform_name(platform: &Label, options: &CoreOptions) -> String {
    if let Some(override_platform_name) = options.get_platform_cpu_name_override(platform) {
        return override_platform_name;
    }

    // Handle legacy heuristic if enabled.
    // Note that it is known this heuristic is not necessarily complete.
    if options.use_platforms_in_output_dir_legacy_heuristic {
        // Only use non-default platforms.
        if !PlatformOptions::platform_is_default(platform) {
            return platform.get_name().to_string();
        }
        // Fall back to using the CPU.
        return options.cpu.clone();
    }
    // As a last resort use hashCode of the unambiguous form of the label.
    format!(
        "platform-{:X}",
        java_string_hash_code(&platform.get_unambiguous_canonical_form()) as u32
    )
}

/// Java `String.hashCode` (over UTF-16 code units).
pub fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for c in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as i32);
    }
    h
}

// Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:235 OutputPathMnemonicComputer.computeNameFragmentWithDiff
/// Compute the hash for the new BuildOptions based on the names and values of all options (both
/// native and Starlark) that are different from some supplied baseline configuration.
pub fn compute_name_fragment_with_diff(
    to_options: &BuildOptions,
    baseline_options: &BuildOptions,
    explicit_in_output_path_options: &IndexSet<String>,
) -> String {
    // Quick short-circuit for trivial case.
    if to_options == baseline_options {
        return String::new();
    }

    let trimmed;
    let mut baseline_options = baseline_options;
    if !to_options.contains(&TEST_OPTIONS) {
        trimmed = test_trimming_trim(baseline_options);
        baseline_options = &trimmed;
    }

    let diff = OptionsDiff::diff(to_options, baseline_options);
    // Note: getFirst only excludes options trimmed between baselineOptions to toOptions and this
    // is considered OK as a given Rule should not be being built with options of different
    // trimmings. See longform note in ConfiguredTargetKey for details.
    let chosen_native_options: Vec<String> = diff
        .get_first()
        .keys()
        .map(|d| d.get_option_name().to_string())
        .filter(|option_name| !explicit_in_output_path_options.contains(option_name))
        .collect();
    // Note: getChangedStarlarkOptions includes all changed options, added options and removed
    // options between baselineOptions and toOptions. This is necessary since there is no current
    // notion of trimming a Starlark option: 'null' or non-existent justs means set to default.
    let chosen_starlark_options: Vec<String> = diff
        .get_changed_starlark_options()
        .iter()
        .map(|l| l.to_string())
        .collect();
    hash_chosen_options(to_options, &chosen_native_options, &chosen_starlark_options)
}

// Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:231 OutputPathMnemonicComputer.computeNameFragmentWithAffectedByStarlarkTransition
/// Compute the output directory name fragment corresponding to the new BuildOptions based on the
/// names and values of all options (both native and Starlark) previously transitioned anywhere
/// in the build by Starlark transitions.
fn compute_name_fragment_with_affected_by_starlark_transition(to_options: &BuildOptions) -> String {
    let build_config_options = to_options.get::<CoreOptions>().expect("CoreOptions");
    if build_config_options
        .affected_by_starlark_transition
        .is_empty()
    {
        return String::new();
    }

    let mut affected_native_options = Vec::new();
    let mut affected_starlark_options = Vec::new();
    for option_name in &build_config_options.affected_by_starlark_transition {
        if let Some(native) = option_name.strip_prefix(COMMAND_LINE_OPTION_PREFIX) {
            affected_native_options.push(native.to_string());
        } else {
            affected_starlark_options.push(option_name.clone());
        }
    }
    hash_chosen_options(
        to_options,
        &affected_native_options,
        &affected_starlark_options,
    )
}

// Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:371 OutputPathMnemonicComputer.hashChosenOptions
/// Compute a hash of the given BuildOptions by hashing only the options referenced in both
/// chosenNative and chosenStarlark. The pre-hash for each option will be of the form
/// `(//command_line_option:[native option]|[Starlark option label])=[value]`.
fn hash_chosen_options(
    to_options: &BuildOptions,
    chosen_native: &[String],
    chosen_starlark: &[String],
) -> String {
    let option_info_map = OptionInfo::build_map_from(to_options);

    // Note that the TreeMap guarantees a stable ordering of keys and thus it is okay if
    // chosenNative or chosenStarlark do not have a stable iteration order.
    // The map holds the value's `String.valueOf` (None for Java null).
    let mut to_hash: BTreeMap<JavaString, Option<String>> = BTreeMap::new();
    for native_option_name in chosen_native {
        let Some(option_info) = option_info_map.get(native_option_name) else {
            // This can occur if toOptions has been trimmed but the supplied chosen native options
            // includes that trimmed options.
            continue;
        };
        let fragment_options = to_options
            .get_dyn(option_info.get_option_class())
            .expect("present");
        let value = option_info.get_definition().get_value(fragment_options);
        to_hash.insert(
            JavaString(format!("//command_line_option:{}", native_option_name)),
            value.map(|v| java_string_of(Some(&v))),
        );
    }
    for starlark_option_name in chosen_starlark {
        let value = to_options
            .get_starlark_options()
            .get(&Label::parse_canonical_unchecked(starlark_option_name));
        to_hash.insert(
            JavaString(starlark_option_name.clone()),
            value.map(|v| v.java_to_string()),
        );
    }

    if to_hash.is_empty() {
        String::new()
    } else {
        let mut hash_strs = Vec::with_capacity(to_hash.len());
        for (key, value) in to_hash {
            match value {
                Some(v) => hash_strs.push(format!("{}={}", key.0, v)),
                // Avoid using =null to different from value being the non-null String "null"
                None => hash_strs.push(format!("{}@null", key.0)),
            }
        }
        transition_directory_name_fragment(&hash_strs)
    }
}

/// A `String` key ordered like Java's `String.compareTo`.
#[derive(PartialEq, Eq)]
struct JavaString(String);

impl Ord for JavaString {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        crate::build_options::java_compare(&self.0, &other.0)
    }
}
impl PartialOrd for JavaString {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Java: com/google/devtools/build/lib/analysis/config/OutputPathMnemonicComputer.java:418 OutputPathMnemonicComputer.transitionDirectoryNameFragment
pub fn transition_directory_name_fragment(opts: &[String]) -> String {
    let mut fp = blaze_util::Fingerprint::new();
    for opt in opts {
        fp.add_string(opt);
    }
    // Shorten the hash to HASH_LENGTH characters. This should provide sufficient collision
    // avoidance (that is, we don't expect anyone to experience a collision ever).
    // Shortening the hash is important for Windows paths that tend to be short.
    let suffix = fp.hex_digest_and_reset()[..HASH_LENGTH].to_string();
    format!("ST-{}", suffix)
}
