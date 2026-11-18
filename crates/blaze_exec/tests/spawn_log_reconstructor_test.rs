//! Port of `src/test/java/com/google/devtools/build/lib/exec/SpawnLogReconstructorTest.java`.

use blaze_exec::exec::spawn_log_reconstructor::{RunfilesPathMatch, SpawnLogReconstructor};

const PRODUCT_NAME: &str = "bazel";

fn result(repo: Option<&str>, path: &str) -> RunfilesPathMatch {
    RunfilesPathMatch {
        repo: repo.map(str::to_string),
        path: path.to_string(),
    }
}

fn match_default(path: &str) -> RunfilesPathMatch {
    SpawnLogReconstructor::extract_runfiles_path(path, /* sibling_repository_layout= */ false)
}

fn match_sibling(path: &str) -> RunfilesPathMatch {
    SpawnLogReconstructor::extract_runfiles_path(path, /* sibling_repository_layout= */ true)
}

#[test]
fn extract_runfiles_path_default() {
    assert_eq!(match_default("file.txt"), result(None, "file.txt"));
    assert_eq!(match_default("pkg/file.txt"), result(None, "pkg/file.txt"));
    assert_eq!(
        match_default("pkg/external/file.txt"),
        result(None, "pkg/external/file.txt")
    );
    assert_eq!(
        match_default("external/some_repo/pkg/file.txt"),
        result(Some("some_repo"), "pkg/file.txt")
    );
    assert_eq!(
        match_default("external/some-repo+/pkg/file.txt"),
        result(Some("some-repo+"), "pkg/file.txt")
    );
    assert_eq!(
        match_default(&format!("{PRODUCT_NAME}-out/k8-fastbuild/bin/pkg/file.txt")),
        result(None, "pkg/file.txt")
    );
    assert_eq!(
        match_default(&format!(
            "{PRODUCT_NAME}-out/k8-fastbuild/bin/pkg/external/file.txt"
        )),
        result(None, "pkg/external/file.txt")
    );
    assert_eq!(
        match_default(&format!(
            "{PRODUCT_NAME}-out/k8-fastbuild/bin/external/some_repo/pkg/file.txt"
        )),
        result(Some("some_repo"), "pkg/file.txt")
    );
    assert_eq!(
        match_default(&format!(
            "{PRODUCT_NAME}-out/k8-fastbuild/bin/external/some-repo+/pkg/file.txt"
        )),
        result(Some("some-repo+"), "pkg/file.txt")
    );
}

#[test]
fn extract_runfiles_path_sibling() {
    assert_eq!(match_sibling("file.txt"), result(None, "file.txt"));
    assert_eq!(match_sibling("pkg/file.txt"), result(None, "pkg/file.txt"));
    assert_eq!(
        match_sibling("pkg/external/file.txt"),
        result(None, "pkg/external/file.txt")
    );
    assert_eq!(
        match_sibling("external/pkg/file.txt"),
        result(None, "external/pkg/file.txt")
    );
    assert_eq!(
        match_sibling("../some_repo/pkg/file.txt"),
        result(Some("some_repo"), "pkg/file.txt")
    );
    assert_eq!(
        match_sibling("../some-repo+/pkg/file.txt"),
        result(Some("some-repo+"), "pkg/file.txt")
    );
    assert_eq!(
        match_sibling(&format!("{PRODUCT_NAME}-out/k8-fastbuild/bin/pkg/file.txt")),
        result(None, "pkg/file.txt")
    );
    assert_eq!(
        match_sibling(&format!(
            "{PRODUCT_NAME}-out/k8-fastbuild/bin/pkg/external/file.txt"
        )),
        result(None, "pkg/external/file.txt")
    );
    assert_eq!(
        match_sibling(&format!(
            "{PRODUCT_NAME}-out/k8-fastbuild/bin/external/pkg/file.txt"
        )),
        result(None, "external/pkg/file.txt")
    );
    assert_eq!(
        match_sibling(&format!(
            "{PRODUCT_NAME}-out/some_repo/k8-fastbuild/bin/pkg/file.txt"
        )),
        result(Some("some_repo"), "pkg/file.txt")
    );
    assert_eq!(
        match_sibling(&format!(
            "{PRODUCT_NAME}-out/some-repo+/k8-fastbuild/bin/pkg/file.txt"
        )),
        result(Some("some-repo+"), "pkg/file.txt")
    );
    assert_eq!(
        match_sibling(&format!(
            "{PRODUCT_NAME}-out/k8-fastbuild/coverage-metadata/bin/other/pkg/gen.txt"
        )),
        result(None, "bin/other/pkg/gen.txt")
    );
    assert_eq!(
        match_sibling(&format!(
            "{PRODUCT_NAME}-out/some_repo/k8-fastbuild/coverage-metadata/bin/other/pkg/gen.txt"
        )),
        result(Some("some_repo"), "bin/other/pkg/gen.txt")
    );
}
