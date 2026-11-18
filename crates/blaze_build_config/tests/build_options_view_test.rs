//! Port of `src/test/java/com/google/devtools/build/lib/analysis/config/BuildOptionsViewTest.java`.

use blaze_build_config::build_options::{BuildOptions, BuildOptionsView};
use blaze_build_config::fragment_options_class::{CORE_OPTIONS, CPP_OPTIONS, FragmentOptionsClass};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::cpp_options::CppOptions;

fn build_config_options() -> Vec<FragmentOptionsClass> {
    vec![&CORE_OPTIONS, &CPP_OPTIONS]
}

// Java: BuildOptionsViewTest.constructBuildOptions (@Before)
fn construct_build_options() -> BuildOptions {
    BuildOptions::of_args(&build_config_options(), &[])
        .unwrap_or_else(|e| panic!("{}", e.get_message()))
}

#[test]
fn allowed_get() {
    let options = construct_build_options();
    let restricted_options = BuildOptionsView::new(&options, vec![&CORE_OPTIONS]);
    assert!(std::ptr::eq(
        restricted_options.get::<CoreOptions>().unwrap(),
        options.get::<CoreOptions>().unwrap()
    ));
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn prohibited_get() {
    let options = construct_build_options();
    let restricted_options = BuildOptionsView::new(&options, vec![&CORE_OPTIONS]);
    restricted_options.get::<CppOptions>();
}

#[test]
fn allowed_contains() {
    let options = construct_build_options();
    let restricted_options = BuildOptionsView::new(&options, vec![&CORE_OPTIONS]);
    assert!(restricted_options.contains(&CORE_OPTIONS));
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn prohibited_contains() {
    let options = construct_build_options();
    let restricted_options = BuildOptionsView::new(&options, vec![&CORE_OPTIONS]);
    restricted_options.contains(&CPP_OPTIONS);
}

#[test]
#[ignore = "needs BuildOptionsView.clone (not ported)"]
fn clone_test() {}
