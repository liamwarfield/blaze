//! Port of `GlobalRcUtils`.

use crate::parsed::ParsedOptionDescription;

// Java: com/google/devtools/common/options/GlobalRcUtils.java:23 GlobalRcUtils
/// Utility functions for global RC files.
pub struct GlobalRcUtils;

impl GlobalRcUtils {
    /// No global RC files in Bazel, so no global configs.
    pub const ALLOWED_GLOBAL_CONFIGS: &'static [&'static str] = &[];

    const GLOBAL_RC_FILES: [&'static str; 2] = ["client", "Invocation policy"];

    // Java: com/google/devtools/common/options/GlobalRcUtils.java:41 GlobalRcUtils.IS_GLOBAL_RC_OPTION
    /// No global RC files in Bazel. Consider "client" options to be global.
    pub fn is_global_rc_option(option: &ParsedOptionDescription) -> bool {
        for global_rc in Self::GLOBAL_RC_FILES {
            if option
                .get_origin()
                .get_source()
                .is_some_and(|s| s.ends_with(global_rc))
            {
                return true;
            }
            if let Some(expanded_from) = option.get_expanded_from()
                && expanded_from
                    .get_origin()
                    .get_source()
                    .is_some_and(|s| s.ends_with(global_rc))
            {
                return true;
            }
        }
        false
    }

    // Java: com/google/devtools/common/options/GlobalRcUtils.java:64 GlobalRcUtils.isGlobalRcFile
    pub fn is_global_rc_file(rc_file_path: &str) -> bool {
        Self::GLOBAL_RC_FILES
            .iter()
            .any(|g| rc_file_path.ends_with(g))
    }
}
