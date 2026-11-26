//! Ports of `IndexRegistry`, `RegistryFactory` and `RegistryFactoryImpl`.

use std::sync::{Arc, OnceLock};

use blaze_events::{ExtendedEventHandler, StoredEventHandler};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use crate::checksum::Checksum;
use crate::gson::{JsonValue, parse_lenient};
use crate::java_uri::{JavaUri, UriSyntaxException};
use crate::module_key::ModuleKey;
use crate::registry::{
    ModuleFile, Registry, RegistryException, RegistryFetcher, RegistryFileDownloadEvent,
    YankedVersionsValue,
};
use crate::repo_spec::{
    ArchiveRepoSpecBuilder, GitRepoSpecBuilder, RemoteFile, RepoSpec, local_path_repo_spec,
};
use crate::resolution::LockfileMode;
use crate::version::Version;

/// The default list of registries (`BazelRepositoryModule.DEFAULT_REGISTRIES`).
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:132 BazelRepositoryModule.DEFAULT_REGISTRIES
pub const DEFAULT_REGISTRIES: &[&str] = &["https://bcr.bazel.build/"];

/// How to handle the list of file hashes known from the lockfile when downloading files from
/// the registry.
// Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:68 IndexRegistry.KnownFileHashesMode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownFileHashesMode {
    /// Neither use nor update any file hashes.
    Ignore,
    /// Use file hashes from the lockfile if available and add hashes for new files; avoid
    /// revalidating mutable registry information.
    UseAndUpdate,
    /// Like `UseAndUpdate`, but always revalidate mutable registry information.
    UseImmutableAndUpdate,
    /// Require file hashes for all registry downloads.
    Enforce,
}

/// Fields of `bazel_registry.json`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:263 IndexRegistry.BazelRegistryJson
#[derive(Debug, Clone, Default)]
struct BazelRegistryJson {
    mirrors: Option<Vec<String>>,
    module_base_path: Option<String>,
}

/// A registry that serves module metadata from a static HTTP server or a local path. Port of
/// `IndexRegistry`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:63 IndexRegistry
pub struct IndexRegistry {
    uri: JavaUri,
    client_env: IndexMap<String, String>,
    known_file_hashes: IndexMap<String, Option<Checksum>>,
    previously_selected_yanked_versions: IndexMap<ModuleKey, String>,
    known_file_hashes_mode: KnownFileHashesMode,
    module_mirrors: IndexSet<JavaUri>,
    /// `null` when vendor mode is off.
    vendor_manager: Option<crate::vendor_manager::VendorManager>,
    bazel_registry_json: OnceLock<(
        Result<Option<BazelRegistryJson>, RegistryException>,
        StoredEventHandler,
    )>,
}

impl std::fmt::Debug for IndexRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexRegistry")
            .field("uri", &self.uri.to_string())
            .finish()
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:109 IndexRegistry.SOURCE_JSON_FILENAME
const SOURCE_JSON_FILENAME: &str = "source.json";

/// `constructUrl`: joins segments with '/' unless one is already there.
// Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:136 IndexRegistry.constructUrl
fn construct_url(base: &str, segments: &[&str]) -> String {
    let mut url = String::from(base);
    for segment in segments {
        if !url.ends_with('/') && !segment.starts_with('/') {
            url.push('/');
        }
        url.push_str(segment);
    }
    url
}

/// A field of a Gson-reflected JSON class, read leniently like Gson's `TypeAdapters`.
fn json_field<'a>(obj: &'a JsonValue, name: &str) -> Option<&'a JsonValue> {
    match obj.get(name) {
        Some(JsonValue::Null) | None => None,
        Some(v) => Some(v),
    }
}

fn type_error(url: &str, expected: &str, got: &JsonValue, field: &str) -> RegistryException {
    RegistryException::Io(format!(
        "Unable to parse json at url {url}: java.lang.IllegalStateException: Expected {expected} \
         but was {} at path $.{field}",
        got.token_name()
    ))
}

/// Gson `TypeAdapters.STRING.read`: strings, numbers and booleans become strings.
fn json_string(
    url: &str,
    obj: &JsonValue,
    field: &str,
) -> Result<Option<String>, RegistryException> {
    match json_field(obj, field) {
        None => Ok(None),
        Some(JsonValue::String(s)) | Some(JsonValue::Number(s)) => Ok(Some(s.clone())),
        Some(JsonValue::Bool(b)) => Ok(Some(b.to_string())),
        Some(v) => Err(type_error(url, "a string", v, field)),
    }
}

/// Gson `TypeAdapters.INTEGER.read` for an `int` field (missing or null is 0).
fn json_int(url: &str, obj: &JsonValue, field: &str) -> Result<i32, RegistryException> {
    match json_field(obj, field) {
        None => Ok(0),
        Some(JsonValue::Number(n)) | Some(JsonValue::String(n)) => n.parse::<i32>().map_err(|_| {
            RegistryException::Io(format!(
                "Unable to parse json at url {url}: com.google.gson.JsonSyntaxException: \
                 java.lang.NumberFormatException: For input string: \"{n}\" at path $.{field}"
            ))
        }),
        Some(v) => Err(type_error(url, "an int", v, field)),
    }
}

/// Gson `TypeAdapters.BOOLEAN.read` for a `boolean` field.
fn json_bool(url: &str, obj: &JsonValue, field: &str) -> Result<bool, RegistryException> {
    match json_field(obj, field) {
        None => Ok(false),
        Some(JsonValue::Bool(b)) => Ok(*b),
        // A string is parsed with Boolean.parseBoolean.
        Some(JsonValue::String(s)) => Ok(s.eq_ignore_ascii_case("true")),
        Some(v) => Err(type_error(url, "a boolean", v, field)),
    }
}

fn json_string_map(
    url: &str,
    obj: &JsonValue,
    field: &str,
) -> Result<Option<IndexMap<String, String>>, RegistryException> {
    match json_field(obj, field) {
        None => Ok(None),
        Some(JsonValue::Object(members)) => {
            let mut m = IndexMap::new();
            for (k, v) in members {
                let s = match v {
                    JsonValue::String(s) | JsonValue::Number(s) => s.clone(),
                    JsonValue::Bool(b) => b.to_string(),
                    JsonValue::Null => {
                        blaze_util::unported!("IndexRegistry.parseJson", "null map value in {url}")
                    }
                    other => return Err(type_error(url, "a string", other, field)),
                };
                m.insert(k.clone(), s);
            }
            Ok(Some(m))
        }
        Some(v) => Err(type_error(url, "BEGIN_OBJECT", v, field)),
    }
}

fn json_string_list(
    url: &str,
    obj: &JsonValue,
    field: &str,
) -> Result<Option<Vec<String>>, RegistryException> {
    match json_field(obj, field) {
        None => Ok(None),
        Some(JsonValue::Array(items)) => {
            let mut out = Vec::new();
            for v in items {
                match v {
                    JsonValue::String(s) | JsonValue::Number(s) => out.push(s.clone()),
                    JsonValue::Bool(b) => out.push(b.to_string()),
                    JsonValue::Null => {
                        blaze_util::unported!("IndexRegistry.parseJson", "null list item in {url}")
                    }
                    other => return Err(type_error(url, "a string", other, field)),
                }
            }
            Ok(Some(out))
        }
        Some(v) => Err(type_error(url, "BEGIN_ARRAY", v, field)),
    }
}

impl IndexRegistry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        uri: JavaUri,
        client_env: IndexMap<String, String>,
        known_file_hashes: IndexMap<String, Option<Checksum>>,
        known_file_hashes_mode: KnownFileHashesMode,
        previously_selected_yanked_versions: IndexMap<ModuleKey, String>,
        vendor_dir: Option<PathFragment>,
        module_mirrors: IndexSet<JavaUri>,
    ) -> IndexRegistry {
        // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:125 IndexRegistry.<init>
        let vendor_manager = vendor_dir.map(crate::vendor_manager::VendorManager::new);
        IndexRegistry {
            uri,
            client_env,
            known_file_hashes,
            previously_selected_yanked_versions,
            known_file_hashes_mode,
            module_mirrors,
            vendor_manager,
            bazel_registry_json: OnceLock::new(),
        }
    }

    /// Grabs a file from the given URL. `NotFound` if it doesn't exist.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:148 IndexRegistry.grabFile
    fn grab_file(
        &self,
        url: &str,
        event_handler: &dyn ExtendedEventHandler,
        download_manager: &dyn RegistryFetcher,
        use_checksum: bool,
    ) -> Result<Vec<u8>, RegistryException> {
        let result = self.do_grab_file(download_manager, url, use_checksum);
        // The checksums of module files are always needed for the remote_module_file_integrity
        // attributes of the http_archive backing the module repo.
        if use_checksum {
            // `finally`: posted on success and on failure (with empty content). Interruptions
            // propagate the same way in Java.
            let content = result.as_ref().ok().map(|c| c.as_slice());
            event_handler.post(Arc::new(RegistryFileDownloadEvent::create(url, content)));
        }
        result
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:167 IndexRegistry.doGrabFile
    fn do_grab_file(
        &self,
        download_manager: &dyn RegistryFetcher,
        raw_url: &str,
        use_checksum: bool,
    ) -> Result<Vec<u8>, RegistryException> {
        let checksum: Option<Checksum> =
            if self.known_file_hashes_mode != KnownFileHashesMode::Ignore && use_checksum {
                match self.known_file_hashes.get(raw_url) {
                    None => {
                        if self.known_file_hashes_mode == KnownFileHashesMode::Enforce {
                            // MissingChecksumException (an IOException).
                            return Err(RegistryException::Io(format!(
                                "Missing checksum for registry file {raw_url} not permitted with \
                                 --lockfile_mode=error. Please run `bazel mod deps \
                                 --lockfile_mode=update` to update your lockfile."
                            )));
                        }
                        // This is a new file, download without providing a checksum.
                        None
                    }
                    Some(None) => {
                        // The file didn't exist when the lockfile was created, but may now.
                        if self.known_file_hashes_mode == KnownFileHashesMode::UseImmutableAndUpdate
                        {
                            None
                        } else {
                            // Guarantee reproducibility by assuming the file still doesn't exist.
                            return Err(RegistryException::NotFound(format!(
                                "{raw_url}: previously not found (as recorded in \
                                 MODULE.bazel.lock, refresh with --lockfile_mode=refresh)"
                            )));
                        }
                    }
                    // Known file: download with a checksum to potentially get a repository cache
                    // hit and ensure the remote file hasn't changed.
                    Some(Some(c)) => Some(c.clone()),
                }
            } else {
                None
            };

        if self.known_file_hashes_mode == KnownFileHashesMode::Enforce {
            blaze_util::check_state!(
                checksum.is_some(),
                "Cannot fetch a file without a checksum in ENFORCE mode. This is a bug in Bazel, \
                 please report at https://github.com/bazelbuild/bazel/issues/new/choose."
            );
        }
        let url = JavaUri::new(raw_url)
            .unwrap_or_else(|e| panic!("IllegalArgumentException: {}", e.get_message()));
        // Don't read the registry URL from the vendor directory in the following cases:
        // 1. vendorUtil is null, which means vendor mode is disabled.
        // 2. The checksum is not present, which means the URL is not vendored or the vendored
        // content is out-dated.
        // 3. The URL starts with "file:", which means it's a local file and isn't vendored.
        // 4. The vendor path doesn't exist, which means the URL is not vendored.
        // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:217 IndexRegistry.doGrabFile
        if let Some(vendor_manager) = &self.vendor_manager
            && let Some(checksum) = &checksum
            && url.get_scheme() != Some("file")
            && vendor_manager
                .is_url_vendored(&url)
                .map_err(|e| RegistryException::Io(e.0))?
        {
            return vendor_manager
                .read_registry_url(&url, checksum)
                .map_err(|message| {
                    RegistryException::Io(format!(
                        "Failed to read vendored registry file {} at {}: {}. Please rerun the \
                         bazel vendor command.",
                        raw_url,
                        vendor_manager
                            .get_vendor_path_for_url(&url)
                            .map(|p| p.get_path_string().to_string())
                            .unwrap_or_default(),
                        message
                    ))
                });
        }
        match download_manager.download_and_read_one_url_for_bzlmod(
            &url,
            &self.client_env,
            checksum.as_ref(),
        ) {
            Ok(content) => Ok(content),
            Err(RegistryException::NotFound(_)) => {
                Err(RegistryException::NotFound(format!("{raw_url}: not found")))
            }
            Err(RegistryException::Io(m)) => Err(RegistryException::Io(format!(
                "Failed to fetch registry file {raw_url}: {m}"
            ))),
            Err(RegistryException::Interrupted) => Err(RegistryException::Interrupted),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:257 IndexRegistry.constructModuleFileUrl
    fn construct_module_file_url(&self, key: &ModuleKey) -> String {
        construct_url(
            &self.get_url(),
            &[
                "modules",
                key.name(),
                &key.version().to_string(),
                "MODULE.bazel",
            ],
        )
    }

    /// Grabs a JSON file; `None` if it doesn't exist.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:303 IndexRegistry.grabJsonFile
    fn grab_json_file(
        &self,
        url: &str,
        event_handler: &dyn ExtendedEventHandler,
        download_manager: &dyn RegistryFetcher,
        use_checksum: bool,
    ) -> Result<Option<String>, RegistryException> {
        match self.grab_file(url, event_handler, download_manager, use_checksum) {
            // `new String(bytes, UTF_8)`: registry JSON is Unicode text, and Java keeps its
            // strings Unicode (no `unicodeToInternal` on the parsed fields).
            Ok(bytes) => Ok(Some(blaze_util::string_encoding::java_new_string_utf8(
                &bytes,
            ))),
            Err(RegistryException::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Grabs and parses a JSON file; `None` if it doesn't exist or is blank.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:321 IndexRegistry.grabJson
    fn grab_json(
        &self,
        url: &str,
        event_handler: &dyn ExtendedEventHandler,
        download_manager: &dyn RegistryFetcher,
        use_checksum: bool,
    ) -> Result<Option<JsonValue>, RegistryException> {
        let json_string =
            self.grab_json_file(url, event_handler, download_manager, use_checksum)?;
        match json_string {
            None => Ok(None),
            Some(s) if java_is_blank(&s) => Ok(None),
            Some(s) => Ok(Some(Self::parse_json(&s, url)?)),
        }
    }

    /// Parses JSON as Gson does into a class (a JSON null document parses to null).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:354 IndexRegistry.parseJson
    fn parse_json(json_string: &str, url: &str) -> Result<JsonValue, RegistryException> {
        match parse_lenient(json_string) {
            Ok(Some(v @ JsonValue::Object(_))) => Ok(v),
            Ok(Some(JsonValue::Null)) | Ok(None) => Ok(JsonValue::Object(Vec::new())),
            Ok(Some(other)) => Err(RegistryException::Io(format!(
                "Unable to parse json at url {url}: java.lang.IllegalStateException: Expected \
                 BEGIN_OBJECT but was {} at line 1 column 1 path $",
                other.token_name()
            ))),
            Err(e) => Err(RegistryException::Io(format!(
                "Unable to parse json at url {url}: {e}"
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:422 IndexRegistry.getSourceJsonUrl
    fn get_source_json_url(&self, key: &ModuleKey) -> String {
        construct_url(
            &self.get_url(),
            &[
                "modules",
                key.name(),
                &key.version().to_string(),
                SOURCE_JSON_FILENAME,
            ],
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:427 IndexRegistry.getBazelRegistryJson
    fn get_bazel_registry_json(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        download_manager: &dyn RegistryFetcher,
    ) -> Result<Option<BazelRegistryJson>, RegistryException> {
        let (result, events) = self.bazel_registry_json.get_or_init(|| {
            let stored_event_handler = StoredEventHandler::new();
            let url = construct_url(&self.get_url(), &["bazel_registry.json"]);
            let json = self
                .grab_json(&url, &stored_event_handler, download_manager, true)
                .and_then(|json| match json {
                    None => Ok(None),
                    Some(obj) => Ok(Some(BazelRegistryJson {
                        mirrors: json_string_list(&url, &obj, "mirrors")?,
                        module_base_path: json_string(&url, &obj, "module_base_path")?,
                    })),
                });
            (json, stored_event_handler)
        });
        // PORT: Java does not memoize a failed fetch (the exception escapes the initializer);
        // here the failure is memoized and returned again, which only differs on retries.
        events.replay_on(event_handler);
        result.clone()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:448 IndexRegistry.createLocalPathRepoSpec
    fn create_local_path_repo_spec(
        &self,
        source_path: Option<String>,
        bazel_registry_json: Option<BazelRegistryJson>,
        key: &ModuleKey,
    ) -> Result<RepoSpec, RegistryException> {
        let Some(mut path) = source_path else {
            panic!("NullPointerException: LocalPathSourceJson.path");
        };
        if !PathFragment::is_absolute_str(&path) {
            let module_base = bazel_registry_json
                .expect("NoSuchElementException: No value present")
                .module_base_path
                .unwrap_or_else(|| "null".to_string());
            path = format!("{module_base}/{path}");
            if !PathFragment::is_absolute_str(&module_base) {
                if self.uri.get_scheme() == Some("file") {
                    let uri_path = self.uri.get_path().unwrap_or_default();
                    if uri_path.is_empty() || !uri_path.starts_with('/') {
                        return Err(RegistryException::Io(format!(
                            "Provided non absolute local registry path for module {key}: {uri_path}"
                        )));
                    }
                    // Unix: file:///tmp --> /tmp
                    path = format!("{uri_path}/{path}");
                } else {
                    return Err(RegistryException::Io(format!(
                        "Provided non local registry for module {key}"
                    )));
                }
            }
        }
        Ok(local_path_repo_spec(
            PathFragment::create(&path).get_path_string(),
        ))
    }

    /// Remote patches: "url" => "integrity".
    fn remote_patches(
        &self,
        key: &ModuleKey,
        patches: Option<IndexMap<String, String>>,
    ) -> IndexMap<String, String> {
        let mut remote_patches = IndexMap::new();
        if let Some(patches) = patches {
            for (name, integrity) in patches {
                // ImmutableMap.Builder.buildKeepingLast
                remote_patches.insert(
                    construct_url(
                        &self.get_url(),
                        &[
                            "modules",
                            key.name(),
                            &key.version().to_string(),
                            "patches",
                            &name,
                        ],
                    ),
                    integrity,
                );
            }
        }
        remote_patches
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:478 IndexRegistry.createArchiveRepoSpec
    fn create_archive_repo_spec(
        &self,
        url: &str,
        source_json: &JsonValue,
        module_file_url: &str,
        module_file_checksum: &Checksum,
        bazel_registry_json: Option<BazelRegistryJson>,
        key: &ModuleKey,
    ) -> Result<RepoSpec, RegistryException> {
        let source_url = match json_string(url, source_json, "url")? {
            None => {
                return Err(RegistryException::Io(format!(
                    "Missing source URL for module {key}"
                )));
            }
            Some(s) => JavaUri::new(&s).map_err(|e| {
                RegistryException::Io(format!(
                    "Unable to parse json at url {url}: com.google.gson.JsonSyntaxException: \
                     java.net.URISyntaxException: {}",
                    e.get_message()
                ))
            })?,
        };
        let mirror_urls = json_string_list(url, source_json, "mirror_urls")?;
        let Some(integrity) = json_string(url, source_json, "integrity")? else {
            return Err(RegistryException::Io(format!(
                "Missing integrity for module {key}"
            )));
        };
        let strip_prefix = json_string(url, source_json, "strip_prefix")?;
        let patches = json_string_map(url, source_json, "patches")?;
        let overlay = json_string_map(url, source_json, "overlay")?;
        let patch_strip = json_int(url, source_json, "patch_strip")?;
        let archive_type = json_string(url, source_json, "archive_type")?;

        // Give precedence to mirrors specified via the command-line flag.
        let mut all_mirrors: IndexSet<String> =
            self.module_mirrors.iter().map(|u| u.to_string()).collect();
        if let Some(mirrors) = bazel_registry_json.and_then(|j| j.mirrors) {
            all_mirrors.extend(mirrors);
        }
        let mut urls = Vec::new();
        // For each mirror, add a URL that's the mirror URL concatenated with the source URL.
        for mirror in &all_mirrors {
            if JavaUri::new(mirror).is_err() {
                return Err(RegistryException::Io(format!(
                    "Malformed mirror URL specified in bazel_registry.json of {}",
                    self.uri
                )));
            }
            let authority = source_url.get_raw_authority().unwrap_or("");
            let path = source_url.get_raw_path().unwrap_or("");
            let query = source_url.get_raw_query();
            urls.push(
                construct_url(mirror, &[authority, path])
                    + &query.map(|q| format!("?{q}")).unwrap_or_default(),
            );
        }
        // Add the original source URL itself.
        urls.push(source_url.to_string());
        // Add mirror_urls from source.json as backups after the primary url.
        if let Some(m) = mirror_urls {
            urls.extend(m);
        }

        let remote_patches = self.remote_patches(key, patches);
        let overlay: IndexMap<String, RemoteFile> = overlay
            .unwrap_or_default()
            .into_iter()
            .map(|(name, integrity)| {
                let remote_url = construct_url(
                    &self.get_url(),
                    &[
                        "modules",
                        key.name(),
                        &key.version().to_string(),
                        "overlay",
                        &name,
                    ],
                );
                // URLs in the registry itself are not mirrored.
                (name, RemoteFile::new(integrity, vec![remote_url]))
            })
            .collect();
        Ok(ArchiveRepoSpecBuilder::new()
            .set_urls(urls)
            .set_integrity(&integrity)
            .set_strip_prefix(strip_prefix.as_deref().unwrap_or(""))
            .set_remote_patches(remote_patches)
            .set_overlay(overlay)
            .set_remote_module_file(RemoteFile::new(
                module_file_checksum.to_subresource_integrity(),
                vec![module_file_url.to_string()],
            ))
            .set_remote_patch_strip(patch_strip)
            .set_archive_type(archive_type.as_deref())
            .build())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:568 IndexRegistry.createGitRepoSpec
    fn create_git_repo_spec(
        &self,
        url: &str,
        source_json: &JsonValue,
        module_file_url: &str,
        module_file_checksum: &Checksum,
        key: &ModuleKey,
    ) -> Result<RepoSpec, RegistryException> {
        let remote = json_string(url, source_json, "remote")?;
        let commit = json_string(url, source_json, "commit")?;
        let shallow_since = json_string(url, source_json, "shallow_since")?;
        let tag = json_string(url, source_json, "tag")?;
        let init_submodules = json_bool(url, source_json, "init_submodules")?;
        let verbose = json_bool(url, source_json, "verbose")?;
        let patches = json_string_map(url, source_json, "patches")?;
        let patch_strip = json_int(url, source_json, "patch_strip")?;
        let strip_prefix = json_string(url, source_json, "strip_prefix")?;
        let remote_patches = self.remote_patches(key, patches);
        Ok(GitRepoSpecBuilder::new()
            .set_remote(remote.as_deref())
            .set_commit(commit.as_deref())
            .set_shallow_since(shallow_since.as_deref())
            .set_tag(tag.as_deref())
            .set_init_submodules(init_submodules)
            .set_verbose(verbose)
            .set_strip_prefix(strip_prefix.as_deref())
            .set_remote_module_file(&RemoteFile::new(
                module_file_checksum.to_subresource_integrity(),
                vec![module_file_url.to_string()],
            ))
            .set_remote_patches(&remote_patches)
            .set_remote_patch_strip(patch_strip)
            .build())
    }

    fn module_file_checksum<'a>(
        module_file_hashes: &'a IndexMap<String, Option<Checksum>>,
        module_file_url: &str,
    ) -> &'a Checksum {
        module_file_hashes
            .get(module_file_url)
            .expect("NullPointerException: moduleFileHashes.get(moduleFileUrl)")
            .as_ref()
            .expect("NoSuchElementException: No value present")
    }
}

impl Registry for IndexRegistry {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:131 IndexRegistry.getUrl
    fn get_url(&self) -> String {
        self.uri.to_string()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:249 IndexRegistry.getModuleFile
    fn get_module_file(
        &self,
        key: &ModuleKey,
        event_handler: &dyn ExtendedEventHandler,
        download_manager: &dyn RegistryFetcher,
    ) -> Result<ModuleFile, RegistryException> {
        let url = self.construct_module_file_url(key);
        let content = self.grab_file(&url, event_handler, download_manager, true)?;
        Ok(ModuleFile::create(content, url))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:364 IndexRegistry.getRepoSpec
    fn get_repo_spec(
        &self,
        key: &ModuleKey,
        module_file_hashes: &IndexMap<String, Option<Checksum>>,
        event_handler: &dyn ExtendedEventHandler,
        download_manager: &dyn RegistryFetcher,
    ) -> Result<RepoSpec, RegistryException> {
        let json_url = self.get_source_json_url(key);
        let Some(source_text) =
            self.grab_json_file(&json_url, event_handler, download_manager, true)?
        else {
            // FileNotFoundException (an IOException).
            return Err(RegistryException::Io(format!(
                "Module {key}'s {SOURCE_JSON_FILENAME} not found in registry {}",
                self.get_url()
            )));
        };
        let source_json = Self::parse_json(&source_text, &json_url)?;
        let type_ =
            json_string(&json_url, &source_json, "type")?.unwrap_or_else(|| "archive".to_string());
        match type_.as_str() {
            "archive" => {
                let module_file_url = self.construct_module_file_url(key);
                let checksum = Self::module_file_checksum(module_file_hashes, &module_file_url);
                let bazel_registry_json =
                    self.get_bazel_registry_json(event_handler, download_manager)?;
                self.create_archive_repo_spec(
                    &json_url,
                    &source_json,
                    &module_file_url,
                    checksum,
                    bazel_registry_json,
                    key,
                )
            }
            "local_path" => {
                let path = json_string(&json_url, &source_json, "path")?;
                let bazel_registry_json =
                    self.get_bazel_registry_json(event_handler, download_manager)?;
                self.create_local_path_repo_spec(path, bazel_registry_json, key)
            }
            "git_repository" => {
                let module_file_url = self.construct_module_file_url(key);
                let checksum = Self::module_file_checksum(module_file_hashes, &module_file_url);
                self.create_git_repo_spec(&json_url, &source_json, &module_file_url, checksum, key)
            }
            other => Err(RegistryException::Io(format!(
                "Invalid source type \"{other}\" for module {key}"
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:594 IndexRegistry.getYankedVersions
    fn get_yanked_versions(
        &self,
        module_name: &str,
        event_handler: &dyn ExtendedEventHandler,
        download_manager: &dyn RegistryFetcher,
    ) -> Result<Option<IndexMap<Version, String>>, RegistryException> {
        let url = construct_url(&self.get_url(), &["modules", module_name, "metadata.json"]);
        // metadata.json is not immutable.
        let Some(metadata_json) = self.grab_json(&url, event_handler, download_manager, false)?
        else {
            return Ok(None);
        };
        let mut yanked_versions = IndexMap::new();
        if let Some(yanked) = json_string_map(&url, &metadata_json, "yanked_versions")? {
            for (version, info) in yanked {
                let v = Version::parse(&version).map_err(|e| {
                    RegistryException::Io(format!(
                        "Could not parse module {module_name}'s metadata file: {}",
                        e.get_message()
                    ))
                })?;
                // ImmutableMap.Builder.buildOrThrow: distinct strings may normalize equally.
                if yanked_versions.insert(v.clone(), info).is_some() {
                    panic!("IllegalArgumentException: Multiple entries with same key: {v}");
                }
            }
        }
        Ok(Some(yanked_versions))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/IndexRegistry.java:626 IndexRegistry.tryGetYankedVersionsFromLockfile
    fn try_get_yanked_versions_from_lockfile(
        &self,
        selected_module_key: &ModuleKey,
    ) -> Option<YankedVersionsValue> {
        if self.known_file_hashes_mode == KnownFileHashesMode::UseImmutableAndUpdate {
            // Yanked version information is inherently mutable, so always refresh it.
            return None;
        }
        if let Some(yanked_info) = self
            .previously_selected_yanked_versions
            .get(selected_module_key)
        {
            // The module version was selected when the lockfile was created, but known to be
            // yanked (and explicitly allowed). Reuse the yanked info from the lockfile.
            return Some(YankedVersionsValue::create(Some(IndexMap::from([(
                selected_module_key.version().clone(),
                yanked_info.clone(),
            )]))));
        }
        if self.known_file_hashes_mode == KnownFileHashesMode::Enforce {
            // metadata.json is mutable and can't be fetched in error mode.
            return Some(YankedVersionsValue::none_yanked());
        }
        if self
            .known_file_hashes
            .contains_key(&self.get_source_json_url(selected_module_key))
        {
            // The source.json hash is in the lockfile, so the module was selected (and not
            // yanked) when the lockfile was created. Don't refresh.
            return Some(YankedVersionsValue::none_yanked());
        }
        // Network access to the registry is required.
        None
    }
}

/// Creates registries from URLs. Port of `RegistryFactory`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryFactory.java:27 RegistryFactory
pub trait RegistryFactory: Send + Sync {
    fn create_registry(
        &self,
        url: &str,
        lockfile_mode: LockfileMode,
        file_hashes: &IndexMap<String, Option<Checksum>>,
        previously_selected_yanked_versions: &IndexMap<ModuleKey, String>,
        vendor_dir: Option<PathFragment>,
        module_mirrors: &IndexSet<String>,
    ) -> Result<Arc<dyn Registry>, UriSyntaxException>;
}

/// Port of `RegistryFactoryImpl`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryFactoryImpl.java:29 RegistryFactoryImpl
pub struct RegistryFactoryImpl {
    nonstrict_repo_env_supplier: Box<dyn Fn() -> IndexMap<String, String> + Send + Sync>,
}

impl RegistryFactoryImpl {
    pub fn new(
        nonstrict_repo_env_supplier: Box<dyn Fn() -> IndexMap<String, String> + Send + Sync>,
    ) -> Self {
        RegistryFactoryImpl {
            nonstrict_repo_env_supplier,
        }
    }
}

impl RegistryFactory for RegistryFactoryImpl {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryFactoryImpl.java:37 RegistryFactoryImpl.createRegistry
    fn create_registry(
        &self,
        url: &str,
        lockfile_mode: LockfileMode,
        known_file_hashes: &IndexMap<String, Option<Checksum>>,
        previously_selected_yanked_versions: &IndexMap<ModuleKey, String>,
        vendor_dir: Option<PathFragment>,
        module_mirrors: &IndexSet<String>,
    ) -> Result<Arc<dyn Registry>, UriSyntaxException> {
        let uri = JavaUri::new(url)?;
        let Some(scheme) = uri.get_scheme() else {
            return Err(UriSyntaxException::new(
                uri.to_string(),
                "Registry URL has no scheme -- supported schemes are: http://, https:// and \
                 file://",
            ));
        };
        if uri.get_path().is_none() {
            return Err(UriSyntaxException::new(
                uri.to_string(),
                "Registry URL path is not valid -- did you mean to use file:///foo/bar or \
                 file:///c:/foo/bar for Windows?",
            ));
        }
        let known_file_hashes_mode = match scheme {
            "http" | "https" => match lockfile_mode {
                LockfileMode::Error => KnownFileHashesMode::Enforce,
                LockfileMode::Refresh => KnownFileHashesMode::UseImmutableAndUpdate,
                LockfileMode::Update => KnownFileHashesMode::UseAndUpdate,
                LockfileMode::Off => KnownFileHashesMode::Ignore,
            },
            "file" => KnownFileHashesMode::Ignore,
            _ => {
                return Err(UriSyntaxException::new(
                    uri.to_string(),
                    "Unrecognized registry URL protocol",
                ));
            }
        };
        let mut module_mirror_uris = IndexSet::new();
        for module_mirror in module_mirrors {
            module_mirror_uris.insert(JavaUri::new(module_mirror)?);
        }
        Ok(Arc::new(IndexRegistry::new(
            uri,
            (self.nonstrict_repo_env_supplier)(),
            known_file_hashes.clone(),
            known_file_hashes_mode,
            previously_selected_yanked_versions.clone(),
            vendor_dir,
            module_mirror_uris,
        )))
    }
}

/// Java `String.isBlank()`: every code point is `Character.isWhitespace`.
// TODO(byte-strings lead): move to blaze_util (a `java_is_whitespace(char)` predicate).
fn java_is_blank(s: &str) -> bool {
    s.chars().all(java_is_whitespace)
}

/// Java `Character.isWhitespace(int)`: a Unicode space separator, line or paragraph separator
/// that is not a non-breaking space (U+00A0, U+2007, U+202F), or one of U+0009–U+000D and
/// U+001C–U+001F. Not Rust's `char::is_whitespace` (which accepts U+0085 and the non-breaking
/// spaces and rejects U+001C–U+001F).
// Java (JDK): java.base/java/lang/Character.java Character.isWhitespace
fn java_is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | '\u{1c}'..='\u{20}'
            | '\u{1680}'
            | '\u{2000}'..='\u{2006}'
            | '\u{2008}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values: JDK 21 `Character.isWhitespace` / `String.isBlank` (probe run on
    // 2026-11-18).
    #[test]
    fn java_is_blank_follows_character_is_whitespace() {
        assert!(java_is_blank("\u{1c} \u{2028}\t\n"));
        assert!(!java_is_blank("\u{85}"));
        assert!(!java_is_blank("\u{a0}"));
        assert!(java_is_blank(""));
    }
}
