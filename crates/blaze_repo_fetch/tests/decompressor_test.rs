//! Ports of the decompressor tests: `StripPrefixedPathTest`, `DecompressorValueTest`,
//! `CompressedTarFunctionTest`, `ZipDecompressorTest`, `CompressedFunctionTest`,
//! `GzFunctionTest`, `ArFunctionTest`, `SevenZDecompressorTest` (and `TestArchiveDescriptor`).
// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/decompressor/

use std::io::Write as _;

use blaze_repo_fetch::decompressor::{
    self, Decompressor, DecompressorDescriptor, DecompressorDescriptorBuilder, StripPrefixedPath,
};
use blaze_repo_fetch::errors::DecompressorError;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, Symlinks, UnixFileSystem};
use indexmap::IndexMap;

fn fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

fn path_of(fs: &FileSystemRef, p: &std::path::Path) -> Path {
    Path::create_from_str(
        p.canonicalize()
            .unwrap_or(p.to_path_buf())
            .to_str()
            .unwrap(),
        fs,
    )
}

fn data_file(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn readdir_names(p: &Path) -> Vec<String> {
    let mut v: Vec<String> = p
        .readdir(Symlinks::Nofollow)
        .unwrap()
        .iter()
        .map(|d| d.get_name().to_string())
        .collect();
    v.sort();
    v
}

const ROOT_FOLDER_NAME: &str = "root_folder";
const INNER_FOLDER_NAME: &str = "another_folder";

/// Port of `TestArchiveDescriptor`.
struct TestArchiveDescriptor {
    tmp: tempfile::TempDir,
    fs: FileSystemRef,
    archive_name: &'static str,
    out_dir_name: &'static str,
    with_hard_links: bool,
}

impl TestArchiveDescriptor {
    fn new(archive_name: &'static str, out_dir_name: &'static str, with_hard_links: bool) -> Self {
        TestArchiveDescriptor {
            tmp: tempfile::tempdir().unwrap(),
            fs: fs(),
            archive_name,
            out_dir_name,
            with_hard_links,
        }
    }

    fn create_descriptor_builder(&self) -> DecompressorDescriptorBuilder {
        let tarball_path = path_of(&self.fs, &data_file(self.archive_name));
        let working_dir = path_of(&self.fs, self.tmp.path());
        let out_dir = working_dir.get_relative_str(self.out_dir_name);
        DecompressorDescriptor::builder()
            .set_destination_path(out_dir)
            .set_archive_path(tarball_path)
    }

    /// PORT: Java's TestArchiveDescriptor extracts every test of a class into the same shared
    /// `TestUtils.tmpDir()/out`, so the strip-components tests run on top of an earlier
    /// unstripped extraction: Bazel does not strip components from link targets, and the hard
    /// link and absolute symlink targets (`root_folder/another_folder/regularFile`) only exist
    /// because of that leftover tree. Blaze uses a fresh directory per test, so it recreates
    /// that state explicitly.
    fn prepopulate_shared_out_dir(&self, decompressor: Decompressor) {
        decompressor
            .decompress(&self.create_descriptor_builder().build())
            .unwrap();
    }

    /// Validate the content of the output directory.
    fn assert_output_files(&self, root_output_dir: &Path, relative_path: &[&str]) {
        assert!(
            root_output_dir
                .as_fragment()
                .ends_with(&PathFragment::create(self.out_dir_name))
        );
        let mut output_dir = root_output_dir.clone();
        for part in relative_path {
            output_dir = output_dir.get_relative_str(part);
        }
        assert!(output_dir.exists());
        let regular = output_dir.get_relative_str("regularFile");
        assert!(regular.exists());
        assert_ne!(regular.get_file_size().unwrap(), 0);
        assert!(!regular.is_symbolic_link());
        for link in ["relativeSymbolicLinkFile", "absoluteSymbolicLinkFile"] {
            let l = output_dir.get_relative_str(link);
            assert!(l.exists(), "{}", link);
            assert_ne!(l.get_file_size().unwrap(), 0);
            assert!(l.is_symbolic_link());
            assert!(same_file(&regular, &l));
        }
        if self.with_hard_links {
            let h = output_dir.get_relative_str("hardLinkFile");
            assert!(h.exists());
            assert_ne!(h.get_file_size().unwrap(), 0);
            assert!(!h.is_symbolic_link());
            assert!(same_file(&regular, &h));
        }
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let ma = std::fs::metadata(a.get_path_string()).unwrap();
    let mb = std::fs::metadata(b.get_path_string()).unwrap();
    ma.dev() == mb.dev() && ma.ino() == mb.ino()
}

fn rename_map(from: &str, to: &str) -> IndexMap<String, String> {
    let mut m = IndexMap::new();
    m.insert(from.to_string(), to.to_string());
    m
}

fn inner_dir_name() -> String {
    format!("{}/{}", ROOT_FOLDER_NAME, INNER_FOLDER_NAME)
}

mod strip_prefixed_path_test {
    use super::*;

    #[test]
    fn test_strip() {
        let result = StripPrefixedPath::maybe_deprefix(b"foo/bar", Some("foo"));
        assert_eq!(&PathFragment::create("bar"), result.get_path_fragment());
        assert!(result.found_prefix());
        assert!(!result.skip());
        let result = StripPrefixedPath::maybe_deprefix(b"foo", Some("foo"));
        assert!(result.skip());
        let result = StripPrefixedPath::maybe_deprefix(b"bar/baz", Some("foo"));
        assert!(!result.found_prefix());
        let result = StripPrefixedPath::maybe_deprefix(b"foof/bar", Some("foo"));
        assert!(!result.found_prefix());
    }

    #[test]
    fn test_absolute() {
        let result = StripPrefixedPath::maybe_deprefix(b"/foo/bar", None);
        assert_eq!(result.get_path_fragment(), &PathFragment::create("foo/bar"));
        let result = StripPrefixedPath::maybe_deprefix(b"///foo/bar/baz", None);
        assert_eq!(
            result.get_path_fragment(),
            &PathFragment::create("foo/bar/baz")
        );
        let result = StripPrefixedPath::maybe_deprefix(b"/foo/bar/baz", Some("/foo"));
        assert_eq!(result.get_path_fragment(), &PathFragment::create("bar/baz"));
    }

    #[test]
    fn test_windows_absolute() {
        // PORT: skipped, Windows only (the Java test returns early on other OSes).
    }

    #[test]
    fn test_normalize() {
        let result = StripPrefixedPath::maybe_deprefix(b"../bar", None);
        assert_eq!(result.get_path_fragment(), &PathFragment::create("../bar"));
        let result = StripPrefixedPath::maybe_deprefix(b"foo/../baz", None);
        assert_eq!(result.get_path_fragment(), &PathFragment::create("baz"));
        let result = StripPrefixedPath::maybe_deprefix(b"foo/../baz", Some("foo"));
        assert_eq!(result.get_path_fragment(), &PathFragment::create("baz"));
    }

    #[test]
    fn test_deprefix_symlink() {
        let fs = fs();
        let usr = Path::create_from_str("/usr", &fs);
        // there is no attempt to get absolute path for the relative symlinks target path
        assert_eq!(
            StripPrefixedPath::maybe_deprefix_symlink(b"a/b", None, &usr),
            PathFragment::create("a/b")
        );
        assert_eq!(
            StripPrefixedPath::maybe_deprefix_symlink(b"/a/b", None, &usr),
            PathFragment::create("/usr/a/b")
        );
        assert_eq!(
            StripPrefixedPath::maybe_deprefix_symlink(b"/root/a/b", Some("root"), &usr),
            PathFragment::create("/usr/a/b")
        );
        assert_eq!(
            StripPrefixedPath::maybe_deprefix_symlink(b"root/a/b", Some("root"), &usr),
            PathFragment::create("a/b")
        );
    }
}

mod decompressor_value_test {
    use super::*;
    use blaze_repo_fetch::decompressor::{
        all_supported_extensions, get_decompressor_for_path, get_decompressor_for_type,
    };

    #[test]
    fn test_known_file_extensions_do_not_throw() {
        let fs = fs();
        let cases = [
            ("bar.zip", Decompressor::ZipDecompressor),
            ("bar.jar", Decompressor::ZipDecompressor),
            ("bar.baz.zip", Decompressor::ZipDecompressor),
            ("bar.baz.nupkg", Decompressor::ZipDecompressor),
            ("bar.baz.whl", Decompressor::ZipDecompressor),
            ("bar.baz.tar.gz", Decompressor::TarGzFunction),
            ("bar.baz.tgz", Decompressor::TarGzFunction),
            ("bar.baz.gz", Decompressor::GzFunction),
            ("bar.baz.tar.xz", Decompressor::TarXzFunction),
            ("bar.baz.txz", Decompressor::TarXzFunction),
            ("bar.baz.xz", Decompressor::XzFunction),
            ("bar.baz.tar.zst", Decompressor::TarZstFunction),
            ("bar.baz.tzst", Decompressor::TarZstFunction),
            ("bar.baz.zst", Decompressor::ZstFunction),
            ("bar.baz.tar.bz2", Decompressor::TarBz2Function),
            ("bar.baz.tbz", Decompressor::TarBz2Function),
            ("bar.baz.bz2", Decompressor::Bz2Function),
            ("bar.baz.ar", Decompressor::ArFunction),
            ("bar.baz.deb", Decompressor::ArFunction),
            ("bar.baz.7z", Decompressor::SevenZDecompressor),
        ];
        for (name, expected) in cases {
            let path = Path::create_from_str(
                &format!("/foo/.external-repositories/some-repo/{}", name),
                &fs,
            );
            assert_eq!(
                get_decompressor_for_path(&path).unwrap(),
                expected,
                "{}",
                name
            );
        }
    }

    #[test]
    fn test_unknown_file_extensions_throw() {
        let fs = fs();
        let zip_path = Path::create_from_str("/foo/.external-repositories/some-repo/bar.baz", &fs);
        let expected = get_decompressor_for_path(&zip_path).unwrap_err();
        assert!(
            expected
                .get_message()
                .contains("Expected a file with a .zip, .jar,")
        );
    }

    #[test]
    fn http_bzl_documentation() {
        // The SUPPORTED_ARCHIVE_FORMATS list in the pinned tools/build_defs/repo/http.bzl.
        let http_bzl = std::path::Path::new(&std::env::var("HOME").unwrap())
            .join(".cache/blaze/src/bazel-9.3.2/tools/build_defs/repo/http.bzl");
        let Ok(contents) = std::fs::read_to_string(&http_bzl) else {
            eprintln!("skipping: {} not found", http_bzl.display());
            return;
        };
        let start = contents.find("SUPPORTED_ARCHIVE_FORMATS =").unwrap();
        let start_bracket = start + contents[start..].find('[').unwrap();
        let end_bracket = start_bracket + contents[start_bracket..].find(']').unwrap();
        let formats = &contents[start_bracket + 1..end_bracket];
        let observed: Vec<String> = formats
            .split(',')
            .map(str::trim)
            .filter(|s| s.contains('"'))
            .map(|s| s[1..s.len() - 1].to_string())
            .collect();
        assert_eq!(all_supported_extensions("", ""), observed);
    }

    #[test]
    fn get_decompressor_by_type() {
        assert_eq!(
            get_decompressor_for_type("zip").unwrap(),
            Decompressor::ZipDecompressor
        );
        assert_eq!(
            get_decompressor_for_type("deb").unwrap(),
            Decompressor::ArFunction
        );
        let expected = get_decompressor_for_type("baz").unwrap_err();
        assert!(
            expected
                .get_message()
                .contains("No decompressor found for type baz")
        );
        assert!(
            expected
                .get_message()
                .contains("Available types are: zip, jar")
        );
    }
}

/// The tar and zip tests share their bodies.
macro_rules! archive_tests {
    ($modname:ident, $archive:expr, $with_hard_links:expr, $decompressor:expr, $inner_out:expr, $ignore_strip:meta) => {
        mod $modname {
            use super::*;

            fn decompress(descriptor: DecompressorDescriptor) -> Path {
                $decompressor.decompress(&descriptor).unwrap()
            }

            #[test]
            fn test_decompress_without_prefix() {
                let d = TestArchiveDescriptor::new($archive, $inner_out, $with_hard_links);
                let output_dir = decompress(d.create_descriptor_builder().build());
                d.assert_output_files(&output_dir, &[ROOT_FOLDER_NAME, INNER_FOLDER_NAME]);
            }

            #[test]
            fn test_decompress_with_prefix() {
                let d = TestArchiveDescriptor::new($archive, "out", $with_hard_links);
                let output_dir =
                    decompress(d.create_descriptor_builder().set_prefix(ROOT_FOLDER_NAME).build());
                d.assert_output_files(&output_dir, &[INNER_FOLDER_NAME]);
            }

            #[test]
            #[cfg_attr(
                $ignore_strip,
                ignore = "Java passes only through state left in JUnit's shared tmp dir: Bazel does not strip components from symlink targets, so absoluteSymbolicLinkFile points at out/root_folder/..., a different file than out/another_folder/regularFile"
            )]
            fn test_decompress_with_strip_components() {
                let d = TestArchiveDescriptor::new($archive, "out", $with_hard_links);
                d.prepopulate_shared_out_dir($decompressor);
                let output_dir =
                    decompress(d.create_descriptor_builder().set_strip_components(1).build());
                d.assert_output_files(&output_dir, &[INNER_FOLDER_NAME]);
            }

            #[test]
            fn test_decompress_with_renamed_files() {
                let d = TestArchiveDescriptor::new($archive, "out", $with_hard_links);
                let inner = inner_dir_name();
                let output_dir = decompress(
                    d.create_descriptor_builder()
                        .set_rename_files(rename_map(
                            &format!("{}/hardLinkFile", inner),
                            &format!("{}/renamedFile", inner),
                        ))
                        .build(),
                );
                let inner_dir = output_dir
                    .get_relative_str(ROOT_FOLDER_NAME)
                    .get_relative_str(INNER_FOLDER_NAME);
                assert!(inner_dir.get_relative_str("renamedFile").exists());
            }

            #[test]
            fn test_decompress_with_renamed_files_and_prefix() {
                let d = TestArchiveDescriptor::new($archive, "out", $with_hard_links);
                let inner = inner_dir_name();
                let output_dir = decompress(
                    d.create_descriptor_builder()
                        .set_prefix(ROOT_FOLDER_NAME)
                        .set_rename_files(rename_map(
                            &format!("{}/hardLinkFile", inner),
                            &format!("{}/renamedFile", inner),
                        ))
                        .build(),
                );
                let inner_dir = output_dir.get_relative_str(INNER_FOLDER_NAME);
                assert!(inner_dir.get_relative_str("renamedFile").exists());
            }

            #[test]
            fn test_decompress_with_renamed_files_and_strip_components() {
                let d = TestArchiveDescriptor::new($archive, "out", $with_hard_links);
                d.prepopulate_shared_out_dir($decompressor);
                let inner = inner_dir_name();
                let output_dir = decompress(
                    d.create_descriptor_builder()
                        .set_strip_components(1)
                        .set_rename_files(rename_map(
                            &format!("{}/hardLinkFile", inner),
                            &format!("{}/renamedFile", inner),
                        ))
                        .build(),
                );
                let inner_dir = output_dir.get_relative_str(INNER_FOLDER_NAME);
                assert!(inner_dir.get_relative_str("renamedFile").exists());
            }
        }
    };
}

// PORT: the Java test subclasses CompressedTarFunction with a GZIPInputStream; TarGzFunction is
// the same decompressor.
archive_tests!(
    compressed_tar_function_test,
    "test_decompress_archive.tar.gz",
    true,
    Decompressor::TarGzFunction,
    "out",
    any()
);

archive_tests!(
    zip_decompressor_test,
    "test_decompress_archive.zip",
    false,
    Decompressor::ZipDecompressor,
    "out/inner",
    all()
);

mod zip_permissions_test {
    use blaze_repo_fetch::decompressor::zip::{
        WINDOWS_FILE_ATTRIBUTE_ARCHIVE, WINDOWS_FILE_ATTRIBUTE_DIRECTORY,
        WINDOWS_FILE_ATTRIBUTE_NORMAL, get_permissions,
    };

    const FILE: u32 = 0o100644;
    const EXECUTABLE: u32 = 0o100755;
    const DIRECTORY: u32 = 0o40755;

    #[test]
    fn test_get_permissions() {
        assert_eq!(get_permissions(FILE << 16, "foo/bar").unwrap(), FILE);
        assert_eq!(
            get_permissions(EXECUTABLE << 16, "foo/bar").unwrap(),
            EXECUTABLE
        );
        assert_eq!(
            get_permissions(DIRECTORY << 16, "foo/bar").unwrap(),
            DIRECTORY
        );
    }

    #[test]
    fn test_windows_permissions() {
        assert_eq!(
            get_permissions(WINDOWS_FILE_ATTRIBUTE_DIRECTORY, "foo/bar").unwrap(),
            DIRECTORY
        );
        assert_eq!(
            get_permissions(WINDOWS_FILE_ATTRIBUTE_ARCHIVE, "foo/bar").unwrap(),
            EXECUTABLE
        );
        assert_eq!(
            get_permissions(WINDOWS_FILE_ATTRIBUTE_NORMAL, "foo/bar").unwrap(),
            EXECUTABLE
        );
    }

    #[test]
    fn test_directory_with_regular_file_permissions() {
        assert_eq!(get_permissions(FILE, "foo/bar/").unwrap(), 0o40755);
    }
}

mod compressed_function_test {
    use super::*;

    const EXTRACTED_FILE_NAME: &str = "archive.txt";

    struct Setup {
        _tmp: tempfile::TempDir,
        archive: Path,
        extraction_dir: Path,
        compressed_file_name: String,
    }

    fn set_up(which: Decompressor, ext: &str) -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let fs = fs();
        let archive_dir = tmp.path().join("archives");
        let extraction_dir = tmp.path().join("extracted");
        std::fs::create_dir_all(&archive_dir).unwrap();
        std::fs::create_dir_all(&extraction_dir).unwrap();
        let compressed_file_name = format!("{}.{}", EXTRACTED_FILE_NAME, ext);
        let content = format!("test compressed {} file contents\n", compressed_file_name);
        let out = std::fs::File::create(archive_dir.join(&compressed_file_name)).unwrap();
        match which {
            Decompressor::Bz2Function => {
                let mut e = bzip2::write::BzEncoder::new(out, bzip2::Compression::default());
                e.write_all(content.as_bytes()).unwrap();
                e.finish().unwrap();
            }
            Decompressor::GzFunction => {
                let mut e = flate2::write::GzEncoder::new(out, flate2::Compression::default());
                e.write_all(content.as_bytes()).unwrap();
                e.finish().unwrap();
            }
            Decompressor::XzFunction => {
                let mut e = xz2::write::XzEncoder::new(out, 6);
                e.write_all(content.as_bytes()).unwrap();
                e.finish().unwrap();
            }
            Decompressor::ZstFunction => {
                let mut e = zstd::stream::write::Encoder::new(out, 0).unwrap();
                e.write_all(content.as_bytes()).unwrap();
                e.finish().unwrap();
            }
            _ => unreachable!(),
        }
        Setup {
            archive: path_of(&fs, &archive_dir).get_relative_str(&compressed_file_name),
            extraction_dir: path_of(&fs, &extraction_dir),
            _tmp: tmp,
            compressed_file_name,
        }
    }

    fn check(file_dir: &Path, name: &str, s: &Setup) {
        assert_eq!(readdir_names(file_dir), vec![name.to_string()]);
        let content =
            std::fs::read_to_string(file_dir.get_relative_str(name).get_path_string()).unwrap();
        assert!(content.contains(&format!(
            "test compressed {} file contents\n",
            s.compressed_file_name
        )));
    }

    fn test_decompress(which: Decompressor, ext: &str) {
        let s = set_up(which, ext);
        let d = DecompressorDescriptor::builder()
            .set_destination_path(s.extraction_dir.clone())
            .set_archive_path(s.archive.clone())
            .build();
        let file_dir = which.decompress(&d).unwrap();
        check(&file_dir, EXTRACTED_FILE_NAME, &s);
    }

    fn test_decompress_with_prefix_is_ignored(which: Decompressor, ext: &str) {
        let s = set_up(which, ext);
        let d = DecompressorDescriptor::builder()
            .set_destination_path(s.extraction_dir.clone())
            .set_prefix("archive")
            .set_archive_path(s.archive.clone())
            .build();
        let file_dir = which.decompress(&d).unwrap();
        check(&file_dir, EXTRACTED_FILE_NAME, &s);
    }

    fn test_decompress_with_renamed_files(which: Decompressor, ext: &str) {
        let s = set_up(which, ext);
        let d = DecompressorDescriptor::builder()
            .set_destination_path(s.extraction_dir.clone())
            .set_rename_files(rename_map(EXTRACTED_FILE_NAME, "renamedFile"))
            .set_archive_path(s.archive.clone())
            .build();
        let file_dir = which.decompress(&d).unwrap();
        check(&file_dir, "renamedFile", &s);
    }

    macro_rules! params {
        ($($m:ident => $which:expr, $ext:expr;)*) => {
            $(
                mod $m {
                    use super::*;
                    #[test]
                    fn test_decompress() { super::test_decompress($which, $ext); }
                    #[test]
                    fn test_decompress_with_prefix_is_ignored() {
                        super::test_decompress_with_prefix_is_ignored($which, $ext);
                    }
                    #[test]
                    fn test_decompress_with_renamed_files() {
                        super::test_decompress_with_renamed_files($which, $ext);
                    }
                }
            )*
        };
    }

    params! {
        bz2 => Decompressor::Bz2Function, "bz2";
        gz => Decompressor::GzFunction, "gz";
        xz => Decompressor::XzFunction, "xz";
        zst => Decompressor::ZstFunction, "zst";
    }
}

mod gz_function_test {
    use super::*;

    const ARCHIVE_NAME: &str = "archive.txt.gz";
    const BASE_NAME: &str = "archive.txt";

    fn create_test_gzip_file(
        dir: &std::path::Path,
        file_name: &str,
        metadata_file_name: Option<&str>,
        mtime_seconds: u32,
    ) -> std::path::PathBuf {
        let compressed_file = dir.join(file_name);
        let out = std::fs::File::create(&compressed_file).unwrap();
        let mut b = flate2::GzBuilder::new().mtime(mtime_seconds);
        if let Some(n) = metadata_file_name {
            b = b.filename(n);
        }
        let mut e = b.write(out, flate2::Compression::default());
        e.write_all(b"test file contents\n").unwrap();
        e.finish().unwrap();
        compressed_file
    }

    /// `FileNameTest.uncompressesOriginalFilename`, observed through `decompress`.
    fn uncompresses_original_filename(metadata_file_name: Option<&str>, expected: &str) {
        let tmp = tempfile::tempdir().unwrap();
        let fs = fs();
        let gz = create_test_gzip_file(tmp.path(), ARCHIVE_NAME, metadata_file_name, 0);
        let out = tmp.path().join("out");
        let d = DecompressorDescriptor::builder()
            .set_archive_path(path_of(&fs, &gz))
            .set_destination_path(path_of(&fs, tmp.path()).get_relative_str("out"))
            .build();
        Decompressor::GzFunction.decompress(&d).unwrap();
        let names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec![expected.to_string()]);
    }

    #[test]
    fn uncompresses_original_filename_original_filename() {
        uncompresses_original_filename(Some("originalFilename"), "originalFilename");
    }

    #[test]
    fn uncompresses_original_filename_null() {
        uncompresses_original_filename(None, BASE_NAME);
    }

    #[test]
    fn uncompresses_original_filename_blank() {
        uncompresses_original_filename(Some("   "), BASE_NAME);
    }

    #[test]
    fn uncompresses_original_filename_fake_path() {
        uncompresses_original_filename(Some("fake/path/to/originalFilename"), "originalFilename");
    }

    #[test]
    fn uncompresses_original_filename_uplevel_path() {
        uncompresses_original_filename(Some("../../path/to/originalFilename"), "originalFilename");
    }

    #[test]
    fn set_last_modified_time() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = fs();
        // new GregorianCalendar(2000, FEBRUARY, 14, 3, 7, 14) in the local zone; the value only
        // feeds the arithmetic below, so any fixed instant exercises the same path.
        let unix_time_seconds: i64 = 950_497_634;
        // Java: parameters.setModificationTime(unixTimeSeconds) is in milliseconds and the
        // header stores it divided by 1000.
        let header_seconds = (unix_time_seconds / 1000) as u32;
        let gz = create_test_gzip_file(tmp.path(), "test.txt.gz", None, header_seconds);
        let d = DecompressorDescriptor::builder()
            .set_archive_path(path_of(&fs, &gz))
            .set_destination_path(path_of(&fs, tmp.path()).get_relative_str("out"))
            .build();
        let out = Decompressor::GzFunction.decompress(&d).unwrap();
        let hack_expected_time_seconds = unix_time_seconds / 1000 * 1000;
        assert_eq!(
            out.get_relative_str("test.txt")
                .get_last_modified_time()
                .unwrap(),
            hack_expected_time_seconds * 1000
        );
    }
}

mod ar_function_test {
    use super::*;

    const FIRST_FILE_NAME: &str = "archived_first.txt";
    const SECOND_FILE_NAME: &str = "archived_second.md";

    fn builder(tmp: &tempfile::TempDir) -> DecompressorDescriptorBuilder {
        let fs = fs();
        DecompressorDescriptor::builder()
            .set_destination_path(path_of(&fs, tmp.path()).get_relative_str("out"))
            .set_archive_path(path_of(&fs, &data_file("test_files.ar")))
    }

    #[test]
    fn test_decompress() {
        let tmp = tempfile::tempdir().unwrap();
        let output_dir = Decompressor::ArFunction
            .decompress(&builder(&tmp).build())
            .unwrap();
        assert!(output_dir.exists());
        let first = output_dir.get_relative_str(FIRST_FILE_NAME);
        assert!(first.exists());
        assert_eq!(first.get_file_size().unwrap(), 20);
        assert!(!first.is_symbolic_link());
        let second = output_dir.get_relative_str(SECOND_FILE_NAME);
        assert!(second.exists());
        assert_eq!(second.get_file_size().unwrap(), 29);
        assert!(!second.is_symbolic_link());
    }

    #[test]
    fn test_decompress_with_renamed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let output_dir = Decompressor::ArFunction
            .decompress(
                &builder(&tmp)
                    .set_rename_files(rename_map("archived_first.txt", "renamed_file.txt"))
                    .build(),
            )
            .unwrap();
        assert!(output_dir.exists());
        assert!(output_dir.get_relative_str("renamed_file.txt").exists());
    }
}

mod seven_z_decompressor_test {
    use super::*;

    const ARCHIVE_NAME: &str = "test_decompress_archive.7z";
    const REGULAR_FILENAME: &str = "regularFile";

    fn archive_descriptor(out: &'static str) -> TestArchiveDescriptor {
        TestArchiveDescriptor::new(ARCHIVE_NAME, out, false)
    }

    fn decompress(d: DecompressorDescriptor) -> Result<Path, DecompressorError> {
        Decompressor::SevenZDecompressor.decompress(&d)
    }

    #[test]
    fn test_decompress_without_prefix() {
        let d = archive_descriptor("SevenZDecompressorTest_testDecompressWithoutPrefix");
        let output_dir = decompress(d.create_descriptor_builder().build()).unwrap();
        let file_dir = output_dir
            .get_relative_str(ROOT_FOLDER_NAME)
            .get_relative_str(INNER_FOLDER_NAME);
        assert!(readdir_names(&file_dir).contains(&REGULAR_FILENAME.to_string()));
        assert_ne!(
            file_dir
                .get_relative_str(REGULAR_FILENAME)
                .get_file_size()
                .unwrap(),
            0
        );
    }

    #[test]
    fn test_decompress_with_prefix() {
        let d = archive_descriptor("SevenZDecompressorTest_testDecompressWithPrefix");
        let output_dir = decompress(
            d.create_descriptor_builder()
                .set_prefix(ROOT_FOLDER_NAME)
                .build(),
        )
        .unwrap();
        let file_dir = output_dir.get_relative_str(INNER_FOLDER_NAME);
        assert!(readdir_names(&file_dir).contains(&REGULAR_FILENAME.to_string()));
    }

    #[test]
    fn test_decompress_with_strip_components() {
        let d = archive_descriptor("SevenZDecompressorTest_testDecompressWithStripComponents");
        let output_dir = decompress(
            d.create_descriptor_builder()
                .set_strip_components(1)
                .build(),
        )
        .unwrap();
        let file_dir = output_dir.get_relative_str(INNER_FOLDER_NAME);
        assert!(readdir_names(&file_dir).contains(&REGULAR_FILENAME.to_string()));
    }

    fn renames() -> IndexMap<String, String> {
        let inner = inner_dir_name();
        rename_map(
            &format!("{}/{}", inner, REGULAR_FILENAME),
            &format!("{}/renamedFile", inner),
        )
    }

    #[test]
    fn test_decompress_with_renamed_files() {
        let d = archive_descriptor("SevenZDecompressorTest_testDecompressWithRenamedFiles");
        let output_dir = decompress(
            d.create_descriptor_builder()
                .set_rename_files(renames())
                .build(),
        )
        .unwrap();
        let file_dir = output_dir
            .get_relative_str(ROOT_FOLDER_NAME)
            .get_relative_str(INNER_FOLDER_NAME);
        assert!(readdir_names(&file_dir).contains(&"renamedFile".to_string()));
        assert_ne!(
            file_dir
                .get_relative_str("renamedFile")
                .get_file_size()
                .unwrap(),
            0
        );
    }

    #[test]
    fn test_decompress_with_renamed_files_and_prefix() {
        let d =
            archive_descriptor("SevenZDecompressorTest_testDecompressWithRenamedFilesAndPrefix");
        let output_dir = decompress(
            d.create_descriptor_builder()
                .set_prefix(ROOT_FOLDER_NAME)
                .set_rename_files(renames())
                .build(),
        )
        .unwrap();
        let file_dir = output_dir.get_relative_str(INNER_FOLDER_NAME);
        assert!(readdir_names(&file_dir).contains(&"renamedFile".to_string()));
        assert_ne!(
            file_dir
                .get_relative_str("renamedFile")
                .get_file_size()
                .unwrap(),
            0
        );
    }

    #[test]
    fn test_decompress_with_renamed_files_and_strip_components() {
        let d = archive_descriptor(
            "SevenZDecompressorTest_testDecompressWithRenamedFilesAndStripComponents",
        );
        let output_dir = decompress(
            d.create_descriptor_builder()
                .set_strip_components(1)
                .set_rename_files(renames())
                .build(),
        )
        .unwrap();
        let file_dir = output_dir.get_relative_str(INNER_FOLDER_NAME);
        assert!(readdir_names(&file_dir).contains(&"renamedFile".to_string()));
        assert_ne!(
            file_dir
                .get_relative_str("renamedFile")
                .get_file_size()
                .unwrap(),
            0
        );
    }

    #[test]
    fn test_decompress_strip_all_components() {
        let d = archive_descriptor("SevenZDecompressorTest_testDecompressStripAllComponents");
        let output_dir = decompress(
            d.create_descriptor_builder()
                .set_strip_components(1_000)
                .build(),
        )
        .unwrap();
        assert!(!output_dir.exists());
    }

    fn write_7z(path: &std::path::Path, entries: &[(&str, &[u8], Option<std::time::SystemTime>)]) {
        let mut w = sevenz_rust2::ArchiveWriter::create(path).unwrap();
        for (name, content, mtime) in entries {
            let mut e = sevenz_rust2::ArchiveEntry::new_file(name);
            if let Some(t) = mtime {
                e.has_last_modified_date = true;
                e.last_modified_date = sevenz_rust2::NtTime::try_from(*t).unwrap();
            }
            w.push_archive_entry(e, Some(*content)).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn test7z_file_modification_date() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = fs();
        let archive_dir = tmp.path().join("archives");
        let extraction_dir = tmp.path().join("extracted");
        std::fs::create_dir_all(&archive_dir).unwrap();
        std::fs::create_dir_all(&extraction_dir).unwrap();
        // new GregorianCalendar(2000, FEBRUARY, 14, 3, 7, 14) (local zone in Java; any fixed
        // instant exercises the same code).
        let test_date_millis: i64 = 950_497_634_000;
        let test_date =
            std::time::UNIX_EPOCH + std::time::Duration::from_millis(test_date_millis as u64);
        write_7z(
            &archive_dir.join(ARCHIVE_NAME),
            &[
                (
                    "root_folder/another_folder/regularFile",
                    b"regular test file contents with modification date 2000/02/14\n",
                    Some(test_date),
                ),
                (
                    "root_folder/another_folder/fileNoModificationDate",
                    b"entry has no modification date\n",
                    None,
                ),
            ],
        );
        let d = DecompressorDescriptor::builder()
            .set_destination_path(path_of(&fs, &extraction_dir))
            .set_archive_path(path_of(&fs, &archive_dir).get_relative_str(ARCHIVE_NAME))
            .build();
        let output_dir = decompress(d).unwrap();
        let file_dir = output_dir
            .get_relative_str(ROOT_FOLDER_NAME)
            .get_relative_str(INNER_FOLDER_NAME);
        assert_eq!(
            readdir_names(&file_dir),
            vec![
                "fileNoModificationDate".to_string(),
                "regularFile".to_string()
            ]
        );
        assert_eq!(
            file_dir
                .get_relative_str("regularFile")
                .get_last_modified_time()
                .unwrap(),
            test_date_millis
        );
    }

    #[test]
    fn test7z_entries_with_no_name_throws() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = fs();
        let archive_dir = tmp.path().join("archives");
        let extraction_dir = tmp.path().join("extracted");
        std::fs::create_dir_all(&archive_dir).unwrap();
        std::fs::create_dir_all(&extraction_dir).unwrap();
        write_7z(
            &archive_dir.join(ARCHIVE_NAME),
            &[
                ("", b"entry without a name\n", None),
                ("", b"entry without a name2\n", None),
            ],
        );
        let d = DecompressorDescriptor::builder()
            .set_destination_path(path_of(&fs, &extraction_dir))
            .set_archive_path(path_of(&fs, &archive_dir).get_relative_str(ARCHIVE_NAME))
            .build();
        let e = decompress(d).unwrap_err();
        assert_eq!(e.to_string(), "7z archive contains unnamed entry");
    }
}

/// `DecompressorValue.decompress` wraps IO failures and reports missing prefixes.
mod decompressor_value_errors {
    use super::*;

    #[test]
    fn missing_prefix_lists_candidates() {
        let d = TestArchiveDescriptor::new("test_decompress_archive.tar.gz", "out", true);
        let e = decompressor::decompress(
            &d.create_descriptor_builder().set_prefix("nope").build(),
            None,
        )
        .unwrap_err();
        assert!(e.get_message().ends_with(
            ": Prefix \"nope\" was given, but not found in the archive. Here are possible prefixes for this archive: \"root_folder\"."
        ), "{}", e.get_message());
        assert!(e.get_message().starts_with("Error extracting "));
    }
}

/// Blaze-only: non-ASCII entry names. The archives are those of
/// crates/blaze_repo_rules/tests/data/bytes_b2; the expected names are the oracle's (Bazel 9.3.2,
/// `download_and_extract` / `extract` in that workspace), written as internal strings (one char
/// per byte of the file name).
mod non_ascii_entry_names {
    use super::*;

    /// `t\xf4p/é.txt` renamed to `t\xf4p/renamed-é.txt` (keys are internal strings).
    fn rename() -> IndexMap<String, String> {
        rename_map(
            "t\u{f4}p/\u{c3}\u{a9}.txt",
            "t\u{f4}p/renamed-\u{c3}\u{a9}.txt",
        )
    }

    fn extract(archive: &str, prefix: Option<&str>) -> (tempfile::TempDir, Path) {
        let tmp = tempfile::tempdir().unwrap();
        let fs = fs();
        let out = path_of(&fs, tmp.path()).get_child("out");
        let mut builder = DecompressorDescriptor::builder()
            .set_archive_path(path_of(&fs, &data_file(&format!("bytes_b2/{archive}"))))
            .set_destination_path(out.clone())
            .set_rename_files(rename());
        if let Some(prefix) = prefix {
            builder = builder.set_prefix(prefix);
        }
        decompressor::decompress(&builder.build(), None).unwrap();
        (tmp, out)
    }

    /// Tar names are the raw header bytes (`toRawBytesString`), the prefix is matched on its
    /// ISO-8859-1 bytes, and `rename_files` keys match the internal names.
    #[test]
    fn tar_keeps_raw_name_bytes() {
        let (_tmp, out) = extract("archive.tar", Some("t\u{f4}p"));
        assert_eq!(
            readdir_names(&out),
            vec!["l\u{c3}\u{a9}", "raw\u{ff}.txt", "renamed-\u{c3}\u{a9}.txt"]
        );
        assert_eq!(
            out.get_child("l\u{c3}\u{a9}").read_symbolic_link().unwrap(),
            PathFragment::create("\u{c3}\u{a9}.txt")
        );
    }

    /// Zip names are decoded as UTF-8 (a Unicode string that no internal `rename_files` key with
    /// a non-ASCII char matches) and passed on as their UTF-8 bytes.
    #[test]
    fn zip_names_are_utf8() {
        let (_tmp, out) = extract("archive.zip", None);
        assert_eq!(readdir_names(&out), vec!["t\u{c3}\u{b4}p"]);
        assert_eq!(
            readdir_names(&out.get_child("t\u{c3}\u{b4}p")),
            vec!["plain.txt", "\u{c3}\u{a9}.txt"]
        );
    }

    /// Ar names are decoded as US-ASCII (each byte >= 0x80 is U+FFFD), which the file system
    /// sees as `?`.
    #[test]
    fn ar_non_ascii_bytes_become_question_marks() {
        let (_tmp, out) = extract("archive.ar", None);
        assert_eq!(readdir_names(&out), vec!["??.txt", "plain.txt"]);
    }
}
