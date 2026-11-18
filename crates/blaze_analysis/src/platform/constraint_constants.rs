//! Port of `analysis/constraints/ConstraintConstants.java`: constants needed for use of the
//! constraints system.
//!
//! PORT: lives with the platform providers (agent P) since it is made of them.

use blaze_cmdline::Label;
use blaze_shell::util::os::OS;

use crate::platform::constraint_setting_info::ConstraintSettingInfo;
use crate::platform::constraint_value_info::ConstraintValueInfo;
use crate::platform::platform_info::PlatformInfo;

// Java: com/google/devtools/build/lib/analysis/constraints/ConstraintConstants.java:28 ConstraintConstants.ENVIRONMENT_RULE
pub const ENVIRONMENT_RULE: &str = "environment";

// Java: com/google/devtools/build/lib/analysis/constraints/ConstraintConstants.java:30 ConstraintConstants.OS_CONSTRAINT_SETTING
fn os_constraint_setting() -> ConstraintSettingInfo {
    ConstraintSettingInfo::create(Label::parse_canonical_unchecked("@platforms//os:os"))
}

// Java: com/google/devtools/build/lib/analysis/constraints/ConstraintConstants.java:34 ConstraintConstants.CPU_CONSTRAINT_SETTING
pub fn cpu_constraint_setting() -> ConstraintSettingInfo {
    ConstraintSettingInfo::create(Label::parse_canonical_unchecked("@platforms//cpu:cpu"))
}

/// Standard mapping between OS and the corresponding platform constraints, in Java's order.
// Java: com/google/devtools/build/lib/analysis/constraints/ConstraintConstants.java:39 ConstraintConstants.CONSTRAINT_VALUE_TO_OS
fn constraint_value_to_os() -> Vec<(ConstraintValueInfo, OS)> {
    let v = |l: &str| {
        ConstraintValueInfo::create(os_constraint_setting(), Label::parse_canonical_unchecked(l))
    };
    vec![
        (v("@platforms//os:linux"), OS::Linux),
        (v("@platforms//os:osx"), OS::Darwin),
        (v("@platforms//os:macos"), OS::Darwin),
        (v("@platforms//os:windows"), OS::Windows),
        (v("@platforms//os:freebsd"), OS::Freebsd),
        (v("@platforms//os:openbsd"), OS::Openbsd),
        (v("@platforms//os:none"), OS::Unknown),
    ]
}

/// Returns the OS corresponding to the given platform's constraint collection based on the
/// contained platform constraint, falling back to the host platform if none is found.
// PORT: Java takes a non-null PlatformInfo; `None` fails like Java's NullPointerException.
// Java: com/google/devtools/build/lib/analysis/constraints/ConstraintConstants.java:82 ConstraintConstants.getOsFromConstraintsOrHost
pub fn get_os_from_constraints_or_host(platform_info: Option<&PlatformInfo>) -> OS {
    let platform_info = platform_info.expect("NullPointerException: platformInfo");
    let Some(os_constraint_value) = platform_info.constraints().get(&os_constraint_setting())
    else {
        // The platform doesn't specify any OS constraint, which makes it difficult to say how
        // the parts of Bazel that are OS-specific should behave. Purely for backwards
        // compatibility and to avoid unexpected breakages, we fall back to the host OS in this
        // case.
        return OS::get_current();
    };
    // If the constraint value isn't known to Bazel, it is certainly distinct from all the values
    // Bazel specifically cares about (e.g. for Windows- or macOS-specific behavior). This is
    // best modeled by returning UNKNOWN, which is distinct from all the specific OS values in
    // the enum.
    constraint_value_to_os()
        .into_iter()
        .find(|(v, _)| *v == os_constraint_value)
        .map(|(_, os)| os)
        .unwrap_or(OS::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_from_constraints() {
        let linux = PlatformInfo::builder()
            .set_label(Label::parse_canonical_unchecked("//p:linux"))
            .add_constraint(ConstraintValueInfo::create(
                os_constraint_setting(),
                Label::parse_canonical_unchecked("@platforms//os:linux"),
            ))
            .build()
            .unwrap();
        assert_eq!(get_os_from_constraints_or_host(Some(&linux)), OS::Linux);
        let other = PlatformInfo::builder()
            .set_label(Label::parse_canonical_unchecked("//p:other"))
            .add_constraint(ConstraintValueInfo::create(
                os_constraint_setting(),
                Label::parse_canonical_unchecked("@platforms//os:haiku"),
            ))
            .build()
            .unwrap();
        assert_eq!(get_os_from_constraints_or_host(Some(&other)), OS::Unknown);
        let none = PlatformInfo::builder()
            .set_label(Label::parse_canonical_unchecked("//p:none"))
            .build()
            .unwrap();
        assert_eq!(
            get_os_from_constraints_or_host(Some(&none)),
            OS::get_current()
        );
    }
}
