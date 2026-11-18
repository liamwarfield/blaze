//! Port of `packages/LabelConverter.java`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::LabelPackageContext;
use blaze_cmdline::LabelRepoMappingRecorder;
use blaze_cmdline::LabelSyntaxException;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use parking_lot::Mutex;
use starlark::eval::Evaluator;

use crate::bzl_context::BazelModuleContext;

/// A shared repo-mapping recorder (Java `Label.RepoMappingRecorder` held by a converter).
pub type SharedRepoMappingRecorder = Arc<Mutex<dyn LabelRepoMappingRecorder + Send>>;

/// Converts label literals to [`Label`]s relative to a base package and repo mapping, caching
/// results (Java `LabelConverter`).
// Java: com/google/devtools/build/lib/packages/LabelConverter.java:33 LabelConverter
pub struct LabelConverter {
    package_context: LabelPackageContext,
    // PORT: Java's HashMap cache is not thread-safe; a Mutex keeps the converter Send + Sync.
    label_cache: Mutex<HashMap<String, Label>>,
    repo_mapping_recorder: Option<SharedRepoMappingRecorder>,
}

impl LabelConverter {
    /// Java `LabelConverter.forBzlEvaluatingThread`: the package and repo mapping of the
    /// innermost .bzl on the call stack.
    // Java: com/google/devtools/build/lib/packages/LabelConverter.java:41 LabelConverter.forBzlEvaluatingThread
    pub fn for_bzl_evaluating_thread(
        eval: &Evaluator<'_, '_, '_>,
    ) -> Result<LabelConverter, blaze_cmdline::EvalException> {
        // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:163 BazelModuleContext.ofInnermostBzlOrThrow
        let module_context = BazelModuleContext::of_innermost_bzl(eval)
            .expect("NullPointerException: no BazelModuleContext for the innermost .bzl");
        Ok(LabelConverter::from_package_context(
            module_context.package_context(),
        ))
    }

    // Java: com/google/devtools/build/lib/packages/LabelConverter.java:50 LabelConverter.<init>
    fn new_internal(
        package_context: LabelPackageContext,
        repo_mapping_recorder: Option<SharedRepoMappingRecorder>,
    ) -> Self {
        LabelConverter {
            package_context,
            label_cache: Mutex::new(HashMap::new()),
            repo_mapping_recorder,
        }
    }

    // Java: com/google/devtools/build/lib/packages/LabelConverter.java:58 LabelConverter.<init>
    pub fn from_package_context(package_context: LabelPackageContext) -> Self {
        Self::new_internal(package_context, None)
    }

    /// Creates a label converter using the given base package and repo mapping.
    // Java: com/google/devtools/build/lib/packages/LabelConverter.java:63 LabelConverter.<init>
    pub fn new(base: &PackageIdentifier, repository_mapping: RepositoryMapping) -> Self {
        Self::from_package_context(LabelPackageContext::of(base, repository_mapping))
    }

    /// Like [`LabelConverter::new`], recording all repo mapping lookups in `recorder`.
    // Java: com/google/devtools/build/lib/packages/LabelConverter.java:71 LabelConverter.<init>
    pub fn with_recorder(
        base: &PackageIdentifier,
        repository_mapping: RepositoryMapping,
        repo_mapping_recorder: SharedRepoMappingRecorder,
    ) -> Self {
        Self::new_internal(
            LabelPackageContext::of(base, repository_mapping),
            Some(repo_mapping_recorder),
        )
    }

    /// The base package identifier relative labels are resolved against.
    // Java: com/google/devtools/build/lib/packages/LabelConverter.java:79 LabelConverter.getBasePackage
    pub fn get_base_package(&self) -> PackageIdentifier {
        self.package_context.package_identifier()
    }

    /// Returns the Label corresponding to the input, using the current conversion context.
    // Java: com/google/devtools/build/lib/packages/LabelConverter.java:84 LabelConverter.convert
    pub fn convert(&self, input: &str) -> Result<Label, LabelSyntaxException> {
        // Optimization: First check the package-local map, avoiding Label validation, Label
        // construction, and global Interner lookup.
        if let Some(converted) = self.label_cache.lock().get(input) {
            return Ok(converted.clone());
        }
        let converted = match &self.repo_mapping_recorder {
            Some(recorder) => {
                let mut recorder = recorder.lock();
                Label::parse_with_package_context_recording(
                    input,
                    &self.package_context,
                    Some(&mut *recorder),
                )?
            }
            None => Label::parse_with_package_context(input, &self.package_context)?,
        };
        self.label_cache
            .lock()
            .insert(input.to_owned(), converted.clone());
        Ok(converted)
    }
}

impl fmt::Display for LabelConverter {
    // Java: com/google/devtools/build/lib/packages/LabelConverter.java:97 LabelConverter.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get_base_package())
    }
}

impl fmt::Debug for LabelConverter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LabelConverter({})", self.get_base_package())
    }
}
