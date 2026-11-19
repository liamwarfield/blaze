//! `ActionExecutionFunction.PackageRootResolverWithEnvironment.findPackageRootsForFiles`
//! ([`SkyframeContainingPackageRootLookup`]) against a fake `CONTAINING_PACKAGE_LOOKUP` function.
//!
//! Bazel 9.3.2 has no JUnit test of this class (it is covered by integration tests); these cases
//! follow the branches of the Java method: a file in a package, a file in no package, an external
//! repository path (plain and `--experimental_sibling_repository_layout`), an exec path with no
//! discoverable package, and the two exceptions it converts (`BuildFileNotFoundException`, with and
//! without an explicit detailed exit code, and `InconsistentFilesystemException`).

use std::collections::BTreeMap;

use blaze_cmdline::{PackageIdentifier, RepositoryName};
use blaze_exec::actions::action_cache_checker::PackageRootExceptionOrInterrupted;
use blaze_packages::exceptions::{DetailedExitCode, NoSuchPackageException, PackageLoadingCode};
use blaze_protos::failure_details::{include_scanning, package_loading};
use blaze_sky_exec::action_execution_function::SkyframeContainingPackageRootLookup;
use blaze_sky_exec::skyframe_action_executor::ContainingPackageRootLookup;
use blaze_sky_loading::containing_package_lookup::{
    ContainingPackageLookupKey, ContainingPackageLookupValue,
};
use blaze_sky_loading::io::IOException;
use blaze_sky_loading::precomputed::STARLARK_SEMANTICS;
use blaze_skyframe::{
    ComputeError, EvaluationContext, EvaluatorBuilder, Key, SkyFunctionException, SkyFunctionName,
    SkyKey,
};
use blaze_starlark::StarlarkSemantics;

use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, Root, UnixFileSystem};

/// Exec path to its root (`None`: no containing package), or the `PackageRootException`'s
/// message, `IncludeScanning.code` and `IncludeScanning.package_loading_code`.
type Outcome = Result<BTreeMap<String, Option<String>>, (String, i32, i32)>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ResolveKey(Vec<&'static str>);

impl Key for ResolveKey {
    type Value = Outcome;
    const FUNCTION: SkyFunctionName = SkyFunctionName("TEST_RESOLVE");
}

fn fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

fn root(path: &str) -> Root {
    Root::from_path(Path::create(PathFragment::create(path), &fs()))
}

fn main_pkg(name: &str) -> PackageIdentifier {
    PackageIdentifier::create_in_main_repo(PathFragment::create(name))
}

/// The fake `ContainingPackageLookupFunction`.
fn containing_package_lookup(
    key: &ContainingPackageLookupKey,
) -> Result<Option<ContainingPackageLookupValue>, ComputeError> {
    let id = &key.0;
    let repo = id.get_repository();
    let pkg = id.get_package_fragment().get_path_string().to_owned();
    if repo.is_main() {
        match pkg.as_str() {
            "inc" | "inc/sub" => Ok(Some(ContainingPackageLookupValue::with_containing_package(
                main_pkg("inc"),
                root("/ws"),
            ))),
            "nopkg" => Ok(Some(ContainingPackageLookupValue::NONE)),
            "broken" => Err(SkyFunctionException::persistent(
                NoSuchPackageException::build_file_not_found_with_detailed_exit_code(
                    id.clone(),
                    "BUILD file not found",
                    DetailedExitCode::package_loading(
                        "BUILD file not found",
                        PackageLoadingCode::BuildFileMissing,
                    ),
                ),
            )
            .into()),
            "undetailed" => Err(SkyFunctionException::persistent(
                NoSuchPackageException::build_file_not_found(id.clone(), "no BUILD"),
            )
            .into()),
            "inconsistent" => Err(SkyFunctionException::persistent(
                IOException::inconsistent_filesystem("stat failed."),
            )
            .into()),
            other => panic!("unexpected package {other}"),
        }
    } else {
        assert_eq!(repo.get_name(), "ext");
        Ok(Some(ContainingPackageLookupValue::with_containing_package(
            PackageIdentifier::create_root_package(repo.clone()),
            root("/ob/external/ext"),
        )))
    }
}

fn resolve(paths: &[&'static str], sibling_repository_layout: bool) -> Outcome {
    let semantics = StarlarkSemantics {
        experimental_sibling_repository_layout: sibling_repository_layout,
        ..StarlarkSemantics::default()
    };
    let builder = STARLARK_SEMANTICS.set(EvaluatorBuilder::new(), semantics);
    let evaluator = builder
        .typed_function::<ContainingPackageLookupKey, _>(|k, _env| containing_package_lookup(k))
        .typed_function::<ResolveKey, _>(|k, env| {
            let exec_paths: Vec<PathFragment> =
                k.0.iter().map(|p| PathFragment::create(p)).collect();
            match SkyframeContainingPackageRootLookup.find_package_roots_for_files(&exec_paths, env)
            {
                Ok(Some(m)) => Ok(Some(Ok(m
                    .into_iter()
                    .map(|(p, r)| {
                        (
                            p.get_path_string().to_owned(),
                            r.map(|r| r.as_path().unwrap().get_path_string().to_owned()),
                        )
                    })
                    .collect()))),
                Ok(None) => Ok(None),
                Err(PackageRootExceptionOrInterrupted::PackageRoot(e)) => Ok(Some(Err((
                    e.message.clone(),
                    e.get_error().code,
                    e.get_error().package_loading_code,
                )))),
                Err(PackageRootExceptionOrInterrupted::Interrupted(_)) => {
                    Err(ComputeError::Interrupted(blaze_skyframe::Interrupted))
                }
            }
        })
        .build();
    let key = ResolveKey(paths.to_vec());
    let result = evaluator
        .evaluate(
            &[SkyKey::new(key.clone())],
            &EvaluationContext {
                keep_going: true,
                ..EvaluationContext::default()
            },
        )
        .unwrap();
    (*result.get(&key).expect("resolve value")).clone()
}

fn roots(pairs: &[(&str, Option<&str>)]) -> Outcome {
    Ok(pairs
        .iter()
        .map(|(p, r)| (p.to_string(), r.map(str::to_owned)))
        .collect())
}

#[test]
fn file_in_a_package() {
    assert_eq!(
        resolve(&["inc/x.h", "inc/sub/y.h"], false),
        roots(&[("inc/x.h", Some("/ws")), ("inc/sub/y.h", Some("/ws"))])
    );
}

#[test]
fn file_in_no_package() {
    assert_eq!(
        resolve(&["nopkg/x.h", "inc/x.h"], false),
        roots(&[("nopkg/x.h", None), ("inc/x.h", Some("/ws"))])
    );
}

#[test]
fn external_repository_path() {
    assert_eq!(
        resolve(&["external/ext/include/e.h"], false),
        roots(&[("external/ext/include/e.h", Some("/ob/external/ext"))])
    );
}

#[test]
fn external_repository_path_sibling_repository_layout() {
    assert_eq!(
        resolve(&["../ext/include/e.h"], true),
        roots(&[("../ext/include/e.h", Some("/ob/external/ext"))])
    );
}

#[test]
fn exec_path_without_package_is_left_out() {
    // PackageIdentifier.discoverFromExecPath: an invalid repository name gives no package, so the
    // path gets no entry at all (not a null root).
    assert!(RepositoryName::create("@bad").is_err());
    assert_eq!(
        resolve(&["external/@bad/x.h", "inc/x.h"], false),
        roots(&[("inc/x.h", Some("/ws"))])
    );
}

#[test]
fn build_file_not_found_with_explicit_exit_code() {
    assert_eq!(
        resolve(&["broken/x.h"], false),
        Err((
            "Unable to resolve broken/x.h as an artifact: no such package 'broken': BUILD file \
             not found"
                .to_owned(),
            include_scanning::Code::UserPackageLoadFailure as i32,
            package_loading::Code::BuildFileMissing as i32,
        ))
    );
}

#[test]
fn build_file_not_found_without_explicit_exit_code() {
    assert_eq!(
        resolve(&["undetailed/x.h"], false),
        Err((
            "Unable to resolve undetailed/x.h as an artifact: no such package 'undetailed': no \
             BUILD"
                .to_owned(),
            include_scanning::Code::UndifferentiatedPackageLoadFailure as i32,
            package_loading::Code::PackageMissing as i32,
        ))
    );
}

#[test]
fn inconsistent_filesystem() {
    assert_eq!(
        resolve(&["inconsistent/x.h"], false),
        Err((
            "Unable to resolve inconsistent/x.h as an artifact: Inconsistent filesystem \
             operations. stat failed. The results of the build are not guaranteed to be \
             correct. You should probably run 'bazel clean' and investigate the filesystem \
             inconsistency (likely due to filesystem updates concurrent with the build)"
                .to_owned(),
            include_scanning::Code::SystemPackageLoadFailure as i32,
            package_loading::Code::PersistentInconsistentFilesystemError as i32,
        ))
    );
}

#[test]
fn first_exception_in_exec_path_order() {
    // Java iterates execPaths and throws for the first path whose lookup failed.
    let Err((message, _, _)) = resolve(&["inc/x.h", "inconsistent/x.h", "broken/x.h"], false)
    else {
        panic!("expected an exception");
    };
    assert!(
        message.starts_with("Unable to resolve inconsistent/x.h as an artifact"),
        "{message}"
    );
}
