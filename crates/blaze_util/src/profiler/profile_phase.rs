//! Port of `com/google/devtools/build/lib/profiler/ProfilePhase.java`.

/// Build phase markers. Used as a separators between different build phases.
// Java: com/google/devtools/build/lib/profiler/ProfilePhase.java:19 ProfilePhase
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfilePhase {
    LAUNCH,
    INIT,
    TARGET_PATTERN_EVAL,
    ANALYZE,
    ANALYZE_AND_EXECUTE,
    LICENSE,
    PREPARE,
    EXECUTE,
    FINISH,
    UNKNOWN,
}

impl ProfilePhase {
    pub const VALUES: &'static [ProfilePhase] = &[
        ProfilePhase::LAUNCH,
        ProfilePhase::INIT,
        ProfilePhase::TARGET_PATTERN_EVAL,
        ProfilePhase::ANALYZE,
        ProfilePhase::ANALYZE_AND_EXECUTE,
        ProfilePhase::LICENSE,
        ProfilePhase::PREPARE,
        ProfilePhase::EXECUTE,
        ProfilePhase::FINISH,
        ProfilePhase::UNKNOWN,
    ];

    /// Short name for the phase (`nick`).
    pub fn nick(self) -> &'static str {
        match self {
            ProfilePhase::LAUNCH => "launch",
            ProfilePhase::INIT => "init",
            ProfilePhase::TARGET_PATTERN_EVAL => "target pattern evaluation",
            ProfilePhase::ANALYZE => "interleaved loading-and-analysis",
            ProfilePhase::ANALYZE_AND_EXECUTE => "interleaved loading, analysis and execution",
            ProfilePhase::LICENSE => "license checking",
            ProfilePhase::PREPARE => "preparation",
            ProfilePhase::EXECUTE => "execution",
            ProfilePhase::FINISH => "finish",
            ProfilePhase::UNKNOWN => "unknown",
        }
    }

    /// Human readable description for the phase (`description`).
    pub fn description(self) -> &'static str {
        match self {
            ProfilePhase::LAUNCH => "Launch Blaze",
            ProfilePhase::INIT => "Initialize command",
            ProfilePhase::TARGET_PATTERN_EVAL => "Evaluate target patterns",
            ProfilePhase::ANALYZE => "Load and analyze dependencies",
            ProfilePhase::ANALYZE_AND_EXECUTE => "Load, analyze dependencies and build artifacts",
            ProfilePhase::LICENSE => "Analyze licenses",
            ProfilePhase::PREPARE => "Prepare for build",
            ProfilePhase::EXECUTE => "Build artifacts",
            ProfilePhase::FINISH => "Complete build",
            ProfilePhase::UNKNOWN => "unknown",
        }
    }

    // Java: com/google/devtools/build/lib/profiler/ProfilePhase.java:43 ProfilePhase.getPhaseFromDescription
    pub fn get_phase_from_description(description: &str) -> ProfilePhase {
        for profile_phase in Self::VALUES {
            if profile_phase.description() == description {
                return *profile_phase;
            }
        }
        ProfilePhase::UNKNOWN
    }
}
