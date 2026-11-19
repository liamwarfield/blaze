//! `BlazeOptionHandler.SkyframeExecutorTargetLoader` (`runtime/BlazeOptionHandler.java`), the
//! loader used for top-level flag parsing, and a re-export of the Starlark options parser
//! (`runtime/StarlarkOptionsParser.java`), which is ported in
//! `blaze_sky_analysis::starlark_options_parser` (ParsedFlagsFunction uses it inside Skyframe).

use std::sync::Arc;

use blaze_cmdline::{TargetParsingException, TargetPatternsCode};
use blaze_events::{Event, EventHandler, ExtendedEventHandler, Postable};
use blaze_packages::target::Target;
use blaze_sky_loading::package_manager::NoSuchPackageOrInterrupted;
use blaze_sky_top::skyframe_executor::{
    DEFAULT_THREAD_COUNT, SkyframeExecutor, TargetPatternPhaseError,
};
use blaze_vfs::PathFragment;

pub use blaze_sky_analysis::starlark_options_parser::*;

/// Lets [`StarlarkOptionsParser`] convert flag names to `Target`s through
/// `TargetPatternPhaseValue`.
///
/// This is used for top-level flag parsing, outside any `SkyFunction`.
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:371 BlazeOptionHandler.SkyframeExecutorTargetLoader
pub struct SkyframeExecutorTargetLoader<'a> {
    skyframe_executor: &'a SkyframeExecutor,
    relative_working_directory: PathFragment,
    reporter: Arc<dyn ExtendedEventHandler>,
}

impl<'a> SkyframeExecutorTargetLoader<'a> {
    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:384 BlazeOptionHandler.SkyframeExecutorTargetLoader.<init>
    pub fn new(
        skyframe_executor: &'a SkyframeExecutor,
        relative_working_directory: PathFragment,
        reporter: Arc<dyn ExtendedEventHandler>,
    ) -> Self {
        SkyframeExecutorTargetLoader {
            skyframe_executor,
            relative_working_directory,
            reporter: Arc::new(NonPostingEventHandler { delegate: reporter }),
        }
    }
}

impl BuildSettingLoader for SkyframeExecutorTargetLoader<'_> {
    // Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:394 BlazeOptionHandler.SkyframeExecutorTargetLoader.loadBuildSetting
    fn load_build_setting(
        &self,
        target_label: &str,
    ) -> Result<Option<Target>, BuildSettingLoadError> {
        let tpv = match self.skyframe_executor.load_target_patterns_without_filters(
            self.reporter.clone(),
            &[target_label.to_string()],
            &self.relative_working_directory,
            DEFAULT_THREAD_COUNT,
            /* keep_going= */ false,
        ) {
            Ok(tpv) => tpv,
            Err(TargetPatternPhaseError::Interrupted) => {
                return Err(BuildSettingLoadError::Interrupted);
            }
            Err(TargetPatternPhaseError::TargetParsing(e)) => {
                return Err(BuildSettingLoadError::TargetParsing(e));
            }
        };
        // tpv.getTargets(reporter, skyframeExecutor.getPackageManager())
        // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:66 TargetPatternPhaseValue.getTargetsFromLabels
        let mut result: Vec<Target> = Vec::new();
        for label in tpv.get_target_labels() {
            let package = match self
                .skyframe_executor
                .get_package_manager()
                .get_package(self.reporter.clone(), label.get_package_identifier())
            {
                Ok(package) => package,
                Err(NoSuchPackageOrInterrupted::Interrupted) => {
                    return Err(BuildSettingLoadError::Interrupted);
                }
                Err(NoSuchPackageOrInterrupted::NoSuchPackage(e)) => panic!(
                    "IllegalStateException: Failed to get preloaded package from TargetPatternPhaseValue for {}: {}",
                    label, e
                ),
            };
            match package.get_target(label.get_name()) {
                Ok(target) => result.push(target.clone()),
                Err(e) => panic!(
                    "IllegalStateException: Failed to get preloaded package from TargetPatternPhaseValue for {}: {}",
                    label,
                    e.get_message()
                ),
            }
        }
        if result.len() != 1 {
            return Err(BuildSettingLoadError::TargetParsing(
                TargetParsingException::new(
                    "user-defined flags must reference exactly one target",
                    TargetPatternsCode::TargetFormatInvalid,
                ),
            ));
        }
        Ok(result.pop())
    }
}

/// `ExtendedEventHandler` override that passes through "normal" events but not events that would
/// go to the build event proto.
///
/// Starlark flags are conceptually options but still need target pattern evaluation in
/// `BlazeOptionHandler.parseStarlarkOptions` to translate their labels to actual targets. If we
/// pass the `post`able events from that evaluation, that would produce "target loaded" and
/// "target configured" events in the build event proto output that consumers can confuse with
/// actual targets requested by the build.
///
/// Hence this class. By dropping those events, we restrict all info and error reporting logic to
/// the options parsing pipeline.
// Java: com/google/devtools/build/lib/runtime/BlazeOptionHandler.java:343 BlazeOptionHandler.NonPostingEventHandler
struct NonPostingEventHandler {
    delegate: Arc<dyn ExtendedEventHandler>,
}

impl EventHandler for NonPostingEventHandler {
    fn handle(&self, e: &Event) {
        self.delegate.handle(e);
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for NonPostingEventHandler {
    fn post(&self, _e: Arc<dyn Postable>) {
        // Fetches of external repositories are not reported as BES events and important to
        // surface in the CLI due to their long-running nature.
        // PORT: Java passes `FetchProgress` posts on to the delegate. Whether a `dyn Postable`
        // is a `FetchProgress` cannot be asked in `blaze_events` (no `as_fetch_progress`), so
        // those progress posts are dropped too; they only feed the progress display.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values: java.util.HashMap iteration order (JDK): "Aa" and "BB" share
    // String.hashCode() 2112, so they share a bucket and keep their insertion order; the
    // single-character keys "a".."d" have hash codes 97..100 and land in buckets 1..4 of 16;
    // 2112 is bucket 0.
    #[test]
    fn java_hash_map_key_order_follows_buckets() {
        let keys: Vec<String> = ["d", "BB", "b", "Aa", "a", "c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let refs: Vec<&String> = keys.iter().collect();
        let order: Vec<&str> = java_hash_map_key_order(&refs)
            .into_iter()
            .map(|i| keys[i].as_str())
            .collect();
        assert_eq!(order, vec!["BB", "Aa", "a", "b", "c", "d"]);
    }
}
