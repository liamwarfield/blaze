//! Port of `src/test/java/com/google/devtools/build/lib/analysis/ShellConfigurationTest.java`.

use std::sync::Arc;

use blaze_optdefs::fragment::shell_configuration::ShellConfigurationOptions;
use blaze_options::Foreign;
use blaze_vfs::PathFragment;

use crate::testing::BuildViewTestCase;
use crate::testing::build_view_test_case::exec_options;

// Java: ShellConfigurationTest.optionsAlsoApplyToHost
#[test]
fn options_also_apply_to_host() {
    let mut t = BuildViewTestCase::new();
    let mut options = (**t.get_target_configuration().get_options()).clone();
    options
        .get_mut::<ShellConfigurationOptions>()
        .expect("ShellConfiguration.Options")
        .shell_executable = Some(Foreign(PathFragment::create("/my/shell/binary")));

    let exec = exec_options(&Arc::new(options), t.event_handler());
    let exec_options = exec
        .get::<ShellConfigurationOptions>()
        .expect("ShellConfiguration.Options");
    assert_eq!(
        exec_options.shell_executable.as_ref().map(|p| &p.0),
        Some(&PathFragment::create("/my/shell/binary"))
    );
}
