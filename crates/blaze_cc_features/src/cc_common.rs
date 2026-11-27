//! Port of the parts of `com.google.devtools.build.lib.rules.cpp.CcCommon` that do not need
//! analysis: `CoptsFilter`.

use blaze_util::java_regex::Pattern;

// Native C++ classes and methods with no caller at the pin: the C++ rules moved to Starlark
// (rules_cc, builtins_bzl), which configures features, reads the FDO context and the debug
// package provider in Starlark; the Java left behind is reached from nowhere (checked by
// searching the pinned src/main/java for each name).
// PORT: skipped, no caller at the pin (feature configuration is rules_cc's Starlark; CcToolchainFeatures.configureFeatures is ported): CcCommon.configureFeaturesOrThrowEvalException, CcCommon.getCoverageFeatures, CcCommon.Language.<init>, CcCommon.Language.getRepresentation
// PORT: skipped, starlark-rust values are immutable unless they are mutable containers, so CoptsFilter needs no override: CcCommon.CoptsFilter.isImmutable
// PORT: skipped, read only by CcCommon.configureFeaturesOrThrowEvalException (no caller at the pin): FdoContext.<init>, FdoContext.getBranchFdoProfile, FdoContext.getPrefetchHintsArtifact, FdoContext.getPropellerOptimizeInputFile, FdoContext.getMemProfProfileArtifact, FdoContext.getProtoProfileArtifact, FdoContext.hasArtifacts, FdoContext.BranchFdoProfile.<init>, FdoContext.BranchFdoProfile.isAutoFdo, FdoContext.BranchFdoProfile.isAutoXBinaryFdo, FdoContext.BranchFdoProfile.isLlvmFdo, FdoContext.BranchFdoProfile.isLlvmCSFdo, FdoContext.BranchFdoProfile.getProfileArtifact, FdoContext.BranchFdoProfile.getBranchFdoMode
// PORT: skipped, read only through FdoContext.getPropellerOptimizeInputFile (no caller at the pin): PropellerOptimizeInputFile.<init>, PropellerOptimizeInputFile.getCcArtifact, PropellerOptimizeInputFile.getLdArtifact
// PORT: skipped, no caller at the pin: CppHelper.<init>, CppHelper.getToolchain, CppHelper.usePicForBinaries
// PORT: skipped, an interface without an implementation at the pin, so `validate_layering_check_features` is on no Starlark value: CppSemantics.validateLayeringCheckFeaturesForStarlark, CppSemantics.validateLayeringCheckFeatures
// PORT: skipped, no caller at the pin (debug_package_info is read by rules_cc's Starlark): DebugPackageProvider.get, DebugPackageProvider.<init>, DebugPackageProvider.getTargetLabel, DebugPackageProvider.getStrippedArtifact, DebugPackageProvider.getUnstrippedArtifact, DebugPackageProvider.getDwpArtifact, DebugPackageProvider.RulesCcProvider.<init>, DebugPackageProvider.RulesCcProvider.wrap

/// A filter that removes copts from a c++ compile action according to a nocopts regex.
// Java: com/google/devtools/build/lib/rules/cpp/CcCommon.java:101 CcCommon.CoptsFilter
// `java.util.regex.Pattern` is `blaze_util::java_regex::Pattern` (Java's dialect on the `regex`
// crate; its module docs list what is `unported!`).
#[derive(Clone, Debug)]
pub struct CoptsFilter {
    no_copts_pattern: Option<String>,
    /// `Pattern.compile(noCoptsPattern)`.
    anchored: Option<Pattern>,
    all_passes: bool,
}

impl CoptsFilter {
    /// Creates a filter that filters all matches to a regex; `Err` with
    /// `PatternSyntaxException.getMessage()` where `Pattern.compile` throws.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCommon.java:111 CcCommon.CoptsFilter.fromRegex
    pub fn from_regex(no_copts_pattern: &str) -> Result<CoptsFilter, String> {
        let anchored = Pattern::compile(no_copts_pattern, 0)?;
        Ok(CoptsFilter {
            no_copts_pattern: Some(no_copts_pattern.to_string()),
            anchored: Some(anchored),
            all_passes: false,
        })
    }

    /// Creates a filter that passes on all inputs.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCommon.java:116 CcCommon.CoptsFilter.alwaysPasses
    pub fn always_passes() -> CoptsFilter {
        CoptsFilter {
            no_copts_pattern: None,
            anchored: None,
            all_passes: true,
        }
    }

    /// Returns true if the provided string passes through the filter, or false if it should be
    /// removed.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCommon.java:124 CcCommon.CoptsFilter.passesFilter
    pub fn passes_filter(&self, flag: &str) -> bool {
        if self.all_passes {
            true
        } else {
            !self.anchored.as_ref().unwrap().matches(flag)
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCommon.java:138 CcCommon.CoptsFilter.repr
    pub fn repr(&self) -> String {
        format!(
            "CoptsFilter(noCoptsPattern={}, allPasses={})",
            self.no_copts_pattern.as_deref().unwrap_or("null"),
            self.all_passes
        )
    }
}
