//! Ports of `analysis/config/Fragment.java`, `FragmentClassSet`, `FragmentRegistry`,
//! `FragmentFactory` and `InvalidConfigurationException`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_options::OptionsClass;

use crate::build_options::BuildOptions;
use crate::fragment_options_class::{FragmentOptionsClass, fragment_options_class};

// Java: com/google/devtools/build/lib/analysis/config/InvalidConfigurationException.java:28 InvalidConfigurationException
/// Thrown if the configuration options lead to an invalid configuration, or if any of the
/// configuration labels cannot be loaded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct InvalidConfigurationException {
    pub message: String,
    /// `FailureDetails.BuildConfiguration.Code` name (Java's `DetailedExitCode`).
    pub code: &'static str,
    /// The `PackageLoading.Code` number when the exception wraps a `NoSuchThingException` and
    /// carries its detailed exit code (Java `new InvalidConfigurationException(
    /// noSuchThingException.getDetailedExitCode(), e)`); `code` is then unused.
    pub package_loading_code: Option<i32>,
    /// The same for a wrapped `NoSuchThingException` whose detailed exit code has another
    /// category: `<Category>[.<CODE>]` as blaze_packages' `FailureCode::Other` names it.
    pub wrapped_failure_code: Option<String>,
}

impl InvalidConfigurationException {
    pub fn new(message: impl Into<String>) -> Self {
        InvalidConfigurationException {
            message: message.into(),
            code: "BUILD_CONFIGURATION_UNKNOWN",
            package_loading_code: None,
            wrapped_failure_code: None,
        }
    }

    pub fn with_code(message: impl Into<String>, code: &'static str) -> Self {
        InvalidConfigurationException {
            message: message.into(),
            code,
            package_loading_code: None,
            wrapped_failure_code: None,
        }
    }

    /// `new InvalidConfigurationException(detailedExitCode, cause)` for a `NoSuchThingException`'s
    /// `PackageLoading` code.
    // Java: com/google/devtools/build/lib/analysis/config/InvalidConfigurationException.java:62 InvalidConfigurationException.<init>
    pub fn with_package_loading_code(message: impl Into<String>, code: i32) -> Self {
        InvalidConfigurationException {
            message: message.into(),
            code: "BUILD_CONFIGURATION_UNKNOWN",
            package_loading_code: Some(code),
            wrapped_failure_code: None,
        }
    }

    /// `new InvalidConfigurationException(noSuchThingException.getDetailedExitCode(), e)` for a
    /// detailed exit code of a category other than `PackageLoading` (see
    /// [`Self::wrapped_failure_code`]).
    pub fn with_wrapped_failure_code(message: impl Into<String>, code: impl Into<String>) -> Self {
        InvalidConfigurationException {
            message: message.into(),
            code: "BUILD_CONFIGURATION_UNKNOWN",
            package_loading_code: None,
            wrapped_failure_code: Some(code.into()),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}

// Java: com/google/devtools/build/lib/analysis/config/Fragment.java:105 OutputDirectoriesContext.AddToMnemonicException
/// Wraps an exception thrown when a value can't be added to the mnemonic.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Invalid option value {bad_value}")]
pub struct AddToMnemonicException {
    pub tunneled_exception: String,
    pub bad_value: String,
}

// Java: com/google/devtools/build/lib/analysis/config/Fragment.java:67 Fragment.OutputDirectoriesContext
/// Context needed by implementations of `Fragment.processForOutputPathMnemonic`.
pub trait OutputDirectoriesContext {
    /// If available, get the baseline version of some FragmentOptions.
    fn get_baseline(
        &self,
        options_class: FragmentOptionsClass,
    ) -> Option<&dyn blaze_options::definition::OptionsBase>;

    /// Adds given String to the explicit part of the output path.
    fn add_to_mnemonic(&mut self, value: Option<&str>) -> Result<(), AddToMnemonicException>;

    /// Marks an option as explicitly included in the output path (excluded from the ST hash).
    fn mark_as_explicit_in_output_path_for(&mut self, option_name: &str);
}

// Java: com/google/devtools/build/lib/analysis/config/Fragment.java:38 Fragment
/// An interface for language-specific configurations.
///
/// All implementations must be immutable and communicate this as clearly as possible (e.g.
/// declare `ImmutableList` signatures on their interfaces vs. `List`). This is because fragment
/// instances may be shared across configurations.
pub trait Fragment: Any + Send + Sync + fmt::Debug {
    /// The fragment's class.
    fn fragment_class(&self) -> FragmentClass;

    // Java: com/google/devtools/build/lib/analysis/config/Fragment.java:45 Fragment.shouldInclude
    /// When a fragment doesn't want to be included in the configuration, this returns false.
    fn should_include(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/analysis/config/Fragment.java:59 Fragment.reportInvalidOptions
    /// Validates the options for this Fragment. Issues warnings for the use of deprecated options,
    /// and warnings or errors for any option settings that conflict.
    fn report_invalid_options(&self, _reporter: &dyn EventHandler, _build_options: &BuildOptions) {}

    // Java: com/google/devtools/build/lib/analysis/config/Fragment.java:121 Fragment.processForOutputPathMnemonic
    /// Add items to the output path mnemonic. If nothing is added, then that is fine.
    fn process_for_output_path_mnemonic(
        &self,
        _ctx: &mut dyn OutputDirectoriesContext,
    ) -> Result<(), AddToMnemonicException> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any;
}

impl dyn Fragment {
    pub fn downcast_ref<T: Fragment>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }
}

/// Constructor of a fragment class (Java: `fragmentClass.getConstructor(BuildOptions.class)`).
pub type FragmentConstructor =
    fn(&BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException>;

/// A `Class<? extends Fragment>`: its Java name, `@RequiresOptions` and constructor.
pub struct FragmentClassInfo {
    /// `Class.getName()`.
    pub java_name: &'static str,
    /// `@StarlarkBuiltin(name = ...)` of the class, if any (`ctx.fragments.<name>`).
    pub starlark_name: Option<&'static str>,
    /// `@RequiresOptions(options = {...})`, in declaration order.
    // Java: com/google/devtools/build/lib/analysis/config/RequiresOptions.java:32 RequiresOptions.options
    pub required_options: fn() -> Vec<FragmentOptionsClass>,
    /// `@RequiresOptions(starlark = true)`.
    // Java: com/google/devtools/build/lib/analysis/config/RequiresOptions.java:35 RequiresOptions.starlark
    pub requires_starlark_options: bool,
    pub constructor: FragmentConstructor,
}

/// A reference to a fragment class. Equality is identity.
pub type FragmentClass = &'static FragmentClassInfo;

impl FragmentClassInfo {
    pub fn get_name(&self) -> &'static str {
        self.java_name
    }

    pub fn get_simple_name(&self) -> &'static str {
        let n = self.java_name;
        let i = n.rfind(['.', '$']).map(|i| i + 1).unwrap_or(0);
        &n[i..]
    }
}

impl PartialEq for FragmentClassInfo {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}
impl Eq for FragmentClassInfo {}
impl std::hash::Hash for FragmentClassInfo {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.java_name.hash(state)
    }
}
impl fmt::Debug for FragmentClassInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "class {}", self.java_name)
    }
}

// Java: com/google/devtools/build/lib/analysis/config/Fragment.java:125 Fragment.requiredOptions
pub fn required_options(fragment_class: FragmentClass) -> Vec<FragmentOptionsClass> {
    (fragment_class.required_options)()
}

// Java: com/google/devtools/build/lib/analysis/config/Fragment.java:132 Fragment.requiresStarlarkOptions
pub fn requires_starlark_options(fragment_class: FragmentClass) -> bool {
    fragment_class.requires_starlark_options
}

/// Helper for fragment constructors: the options class `T` of `build_options`, which
/// `FragmentFactory` guarantees is present when it is a required option.
pub fn required<T: OptionsClass>(build_options: &BuildOptions) -> &T {
    build_options
        .get::<T>()
        .unwrap_or_else(|| panic!("NullPointerException: missing {:?}", T::class()))
}

/// `fragment_options_class::<T>()`, re-exported for fragment declarations.
pub fn options_class<T: OptionsClass>() -> FragmentOptionsClass {
    fragment_options_class::<T>()
}

// Java: com/google/devtools/build/lib/analysis/config/FragmentClassSet.java:38 FragmentClassSet
/// Sorted set of fragment classes (`LEXICAL_FRAGMENT_SORTER`: by class name).
// Java: com/google/devtools/build/lib/analysis/config/FragmentClassSet.java:88 FragmentClassSet.equals
// Java: com/google/devtools/build/lib/analysis/config/FragmentClassSet.java:99 FragmentClassSet.hashCode
// PORT: Java compares the cached hash code, then the sorted sets: the derived comparison of the
// sorted list is the same.
// PORT: skipped, no caller at the pin (the debug form is the derived Debug): FragmentClassSet.trim, FragmentClassSet.toString
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FragmentClassSet(Vec<FragmentClass>);

impl FragmentClassSet {
    // Java: com/google/devtools/build/lib/analysis/config/FragmentClassSet.java:50 FragmentClassSet.of
    pub fn of(fragments: impl IntoIterator<Item = FragmentClass>) -> FragmentClassSet {
        let mut v: Vec<FragmentClass> = Vec::new();
        for f in fragments {
            if !v.contains(&f) {
                v.push(f);
            }
        }
        v.sort_by(|a, b| crate::build_options::java_compare(a.get_name(), b.get_name()));
        FragmentClassSet(v)
    }

    // Java: com/google/devtools/build/lib/analysis/config/FragmentClassSet.java:83 FragmentClassSet.iterator
    pub fn iter(&self) -> impl Iterator<Item = FragmentClass> + '_ {
        self.0.iter().copied()
    }

    // Java: com/google/devtools/build/lib/analysis/config/FragmentClassSet.java:70 FragmentClassSet.contains
    pub fn contains(&self, f: FragmentClass) -> bool {
        self.0.contains(&f)
    }

    pub fn contains_all(&self, other: &FragmentClassSet) -> bool {
        other.0.iter().all(|f| self.contains(f))
    }

    // Java: com/google/devtools/build/lib/analysis/config/FragmentClassSet.java:65 FragmentClassSet.size
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

// Java: com/google/devtools/build/lib/analysis/config/FragmentRegistry.java:22 FragmentRegistry
/// Registry of fragment classes and the options classes they require.
#[derive(Debug)]
pub struct FragmentRegistry {
    all_fragments: FragmentClassSet,
    universal_fragments: FragmentClassSet,
    options_classes: Vec<FragmentOptionsClass>,
}

impl FragmentRegistry {
    // Java: com/google/devtools/build/lib/analysis/config/FragmentRegistry.java:36 FragmentRegistry.create
    pub fn create(
        all_fragments: Vec<FragmentClass>,
        universal_fragments: Vec<FragmentClass>,
        additional_options: Vec<FragmentOptionsClass>,
    ) -> FragmentRegistry {
        let all_fragments_set = FragmentClassSet::of(all_fragments);
        let universal_fragments_set = FragmentClassSet::of(universal_fragments);
        if !all_fragments_set.contains_all(&universal_fragments_set) {
            let missing: Vec<_> = universal_fragments_set
                .iter()
                .filter(|f| !all_fragments_set.contains(f))
                .collect();
            panic!(
                "IllegalArgumentException: Missing universally required fragments: {:?}",
                missing
            );
        }
        let mut options_classes: Vec<FragmentOptionsClass> = Vec::new();
        for fragment in all_fragments_set.iter() {
            for o in required_options(fragment) {
                if !options_classes.contains(&o) {
                    options_classes.push(o);
                }
            }
        }
        for o in additional_options {
            if !options_classes.contains(&o) {
                options_classes.push(o);
            }
        }
        options_classes
            .sort_by(|a, b| crate::build_options::java_compare(a.get_name(), b.get_name()));
        FragmentRegistry {
            all_fragments: all_fragments_set,
            universal_fragments: universal_fragments_set,
            options_classes,
        }
    }

    pub fn get_all_fragments(&self) -> &FragmentClassSet {
        &self.all_fragments
    }

    pub fn get_universal_fragments(&self) -> &FragmentClassSet {
        &self.universal_fragments
    }

    pub fn get_options_classes(&self) -> &[FragmentOptionsClass] {
        &self.options_classes
    }
}

// Java: com/google/devtools/build/lib/analysis/config/FragmentFactory.java:28 FragmentFactory
/// Creates fragments from `BuildOptions`.
///
/// PORT: Java caches fragments by (trimmed options, class) with weak values; Blaze builds each
/// configuration once per command, so the cache is skipped (incremental).
#[derive(Default)]
pub struct FragmentFactory;

impl FragmentFactory {
    // Java: com/google/devtools/build/lib/analysis/config/FragmentFactory.java:37 FragmentFactory.createFragment
    /// Creates the requested `Fragment` using a given `BuildOptions`. Returns `None` if the
    /// fragment could not be built (e.g. the supplied BuildOptions does not contain the required
    /// `FragmentOption`s).
    pub fn create_fragment(
        &self,
        build_options: &BuildOptions,
        fragment_class: FragmentClass,
    ) -> Result<Option<Arc<dyn Fragment>>, InvalidConfigurationException> {
        let trimmed_options = Self::trim_to_required_options(build_options, fragment_class);
        Self::make_fragment(&trimmed_options, fragment_class)
    }

    // Java: com/google/devtools/build/lib/analysis/config/FragmentFactory.java:70 FragmentFactory.trimToRequiredOptions
    fn trim_to_required_options(original: &BuildOptions, fragment: FragmentClass) -> BuildOptions {
        let mut trimmed = BuildOptions::builder();
        let required = required_options(fragment);
        for (class, options) in original.get_native_options() {
            // CoreOptions is implicitly required by all fragments.
            if class.get_simple_name() == "CoreOptions" || required.contains(&class) {
                trimmed.add_fragment_options_dyn(class, options);
            }
        }
        if requires_starlark_options(fragment) {
            trimmed.add_starlark_options(original.get_starlark_options().clone());
        }
        trimmed.build()
    }

    // Java: com/google/devtools/build/lib/analysis/config/FragmentFactory.java:104 FragmentFactory.makeFragment
    fn make_fragment(
        build_options: &BuildOptions,
        fragment_class: FragmentClass,
    ) -> Result<Option<Arc<dyn Fragment>>, InvalidConfigurationException> {
        let fragment = (fragment_class.constructor)(build_options)?;
        Ok(if fragment.should_include() {
            Some(fragment)
        } else {
            None
        })
    }
}
