//! Package builder tests that need no BUILD evaluation: a builder is driven directly, as
//! Java's `PackageTest` / `PackageFactoryApparatus` helpers do. Expected values follow the Java
//! source (messages are copied from `TargetRecorder.java` / `Package.java`).

use std::sync::Arc;

use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::repository_mapping::EMPTY;
use blaze_packages::package::Package;
use blaze_packages::package::PackageLimits;
use blaze_packages::package::PackageSettings;
use blaze_packages::package::TargetDefinitionContext;
use blaze_packages::target::Target;
use blaze_starlark::Location;
use blaze_vfs::DigestHashFunction;
use blaze_vfs::PathFragment;
use blaze_vfs::Root;
use blaze_vfs::RootedPath;
use blaze_vfs::unix_file_system::UnixFileSystem;

fn new_builder(pkg: &str) -> TargetDefinitionContext {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = Root::from_path(blaze_vfs::Path::create_from_str("/workspace", &fs));
    let build_file =
        RootedPath::to_rooted_path(root, &PathFragment::create(&format!("{pkg}/BUILD")));
    Package::new_package_builder(
        PackageSettings::default(),
        PackageIdentifier::create_in_main_repo_str(pkg),
        build_file,
        "_main".to_owned(),
        None,
        None,
        /* no_implicit_file_export= */ false,
        /* simplify_unconditional_selects_in_rule_attrs= */ true,
        EMPTY.clone(),
        EMPTY.clone(),
        None,
        None,
        None,
        /* enable_name_conflict_checking= */ true,
        /* track_full_macro_information= */ true,
        PackageLimits::default(),
    )
}

fn loc(line: usize) -> Location {
    Location {
        file: "/workspace/pkg/BUILD".to_owned(),
        line,
        column: 1,
    }
}

#[test]
fn empty_package_has_build_file_target() {
    let mut builder = new_builder("pkg");
    builder.set_loads(Vec::new(), Vec::new());
    let pkg = builder.build().unwrap();
    assert!(!pkg.contains_errors());
    assert_eq!(pkg.get_name(), "pkg");
    assert_eq!(pkg.get_targets().len(), 1);
    let build = pkg.get_target("BUILD").unwrap();
    assert!(matches!(build, Target::InputFile(_)));
    assert_eq!(build.get_label().to_string(), "//pkg:BUILD");
    assert_eq!(build.get_location().to_string(), "/workspace/pkg/BUILD");
    assert_eq!(
        pkg.get_source_root().to_string(),
        Root::from_path(blaze_vfs::Path::create_from_str(
            "/workspace",
            pkg.get_filename().get_root().get_file_system()
        ))
        .to_string()
    );
    // Idempotent.
    let again = builder.build().unwrap();
    assert!(Arc::ptr_eq(&pkg, &again));
}

#[test]
fn create_input_file_is_idempotent() {
    let mut builder = new_builder("pkg");
    let a = builder.create_input_file("a.txt", loc(1)).unwrap();
    let b = builder.create_input_file("a.txt", loc(2)).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    builder.set_loads(Vec::new(), Vec::new());
    let pkg = builder.build().unwrap();
    let mut names: Vec<&String> = pkg.get_targets().keys().collect();
    names.sort();
    assert_eq!(names, vec!["BUILD", "a.txt"]);
}

#[test]
fn missing_target_message() {
    let mut builder = new_builder("pkg");
    builder.create_input_file("foo.cc", loc(1)).unwrap();
    builder.set_loads(Vec::new(), Vec::new());
    let pkg = builder.build().unwrap();
    // Java: Package.getTarget; the file does not exist on disk, so TargetSuggester suggests.
    let e = pkg.get_target("foo.c").unwrap_err();
    assert_eq!(
        e.get_message(),
        "no such target '//pkg:foo.c': target 'foo.c' not declared in package 'pkg' defined by \
         /workspace/pkg/BUILD (did you mean foo.cc?)"
    );
}
