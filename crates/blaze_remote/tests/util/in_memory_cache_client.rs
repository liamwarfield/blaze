//! Port of the test classes `lib/remote/util/InMemoryCacheClient.java` and
//! `lib/remote/InMemoryCombinedCache.java`.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use async_trait::async_trait;
use blaze_protos::JavaMessage;
use blaze_remote::combined_cache::CombinedCache;
use blaze_remote::common::errors;
use blaze_remote::common::{
    ActionKey, Blob, CacheNotFoundException, MissingDigestsFinder, RemoteActionExecutionContext,
    RemoteCacheClient,
};
use blaze_remote::reapi::{
    ActionCacheUpdateCapabilities, ActionResult, CacheCapabilities, Digest, ServerCapabilities,
    symlink_absolute_path_strategy,
};
use blaze_remote::runtime::get_from_future;
use blaze_remote::util::DigestUtil;
use blaze_remote::util::async_task_cache::{AsyncTaskCache, AsyncTaskCacheNoResult};
use indexmap::IndexSet;
use parking_lot::Mutex;

/// A `RemoteCacheClient` that stores its contents in memory.
// Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:42 InMemoryCacheClient
pub struct InMemoryCacheClient {
    /// `RemoteCacheClient.casUploadCache`.
    cas_upload_cache: AsyncTaskCacheNoResult<Digest>,
    // PORT: Java hands the same exception object to every download of the digest; an `io::Error`
    // cannot be cloned, so it is shared (`errors::shared`).
    download_failures: Mutex<HashMap<Digest, Arc<io::Error>>>,
    ac: Mutex<HashMap<ActionKey, ActionResult>>,
    cas: Mutex<HashMap<Digest, Vec<u8>>>,

    num_success: AtomicI32,
    num_failures: AtomicI32,
    num_find_missing_digests: Mutex<HashMap<Digest, i32>>,
}

impl Default for InMemoryCacheClient {
    fn default() -> Self {
        InMemoryCacheClient::new()
    }
}

impl InMemoryCacheClient {
    /// `new InMemoryCacheClient(casEntries)`.
    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:55 InMemoryCacheClient.<init>
    pub fn new_with_cas_entries(cas_entries: HashMap<Digest, Vec<u8>>) -> InMemoryCacheClient {
        let client = InMemoryCacheClient::new();
        {
            let mut cas = client.cas.lock();
            for (key, value) in cas_entries {
                cas.insert(key, value);
            }
        }
        client
    }

    /// `new InMemoryCacheClient()`.
    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:62 InMemoryCacheClient.<init>
    pub fn new() -> InMemoryCacheClient {
        InMemoryCacheClient {
            cas_upload_cache: AsyncTaskCache::create(),
            download_failures: Mutex::new(HashMap::new()),
            ac: Mutex::new(HashMap::new()),
            cas: Mutex::new(HashMap::new()),
            num_success: AtomicI32::new(0),
            num_failures: AtomicI32::new(0),
            num_find_missing_digests: Mutex::new(HashMap::new()),
        }
    }

    /// `addDownloadFailure(digest, e)`.
    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:66 InMemoryCacheClient.addDownloadFailure
    pub fn add_download_failure(&self, digest: Digest, e: io::Error) {
        self.download_failures.lock().insert(digest, Arc::new(e));
    }

    /// Removes a CAS entry, e.g. to simulate the remote cache losing a previously uploaded blob.
    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:71 InMemoryCacheClient.removeCasEntry
    pub fn remove_cas_entry(&self, digest: &Digest) {
        self.cas.lock().remove(digest);
    }

    /// `getNumSuccessfulDownloads()`.
    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:75 InMemoryCacheClient.getNumSuccessfulDownloads
    pub fn get_num_successful_downloads(&self) -> i32 {
        self.num_success.load(Ordering::SeqCst)
    }

    /// `getNumFailedDownloads()`.
    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:79 InMemoryCacheClient.getNumFailedDownloads
    pub fn get_num_failed_downloads(&self) -> i32 {
        self.num_failures.load(Ordering::SeqCst)
    }

    /// `getNumFindMissingDigests()`.
    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:83 InMemoryCacheClient.getNumFindMissingDigests
    pub fn get_num_find_missing_digests(&self) -> HashMap<Digest, i32> {
        self.num_find_missing_digests.lock().clone()
    }
}

#[async_trait]
impl MissingDigestsFinder for InMemoryCacheClient {
    // PORT: Java runs this on a 100-thread executor.
    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:157 InMemoryCacheClient.findMissingDigests
    async fn find_missing_digests(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        digests: &[Digest],
    ) -> io::Result<IndexSet<Digest>> {
        let mut missing_builder = IndexSet::new();
        for digest in digests {
            *self
                .num_find_missing_digests
                .lock()
                .entry(digest.clone())
                .or_insert(0) += 1;
            if !self.cas.lock().contains_key(digest) {
                missing_builder.insert(digest.clone());
            }
        }
        Ok(missing_builder)
    }
}

#[async_trait]
impl RemoteCacheClient for InMemoryCacheClient {
    fn cas_upload_cache(&self) -> &AsyncTaskCacheNoResult<Digest> {
        &self.cas_upload_cache
    }

    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:113 InMemoryCacheClient.getServerCapabilities
    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        let cache_capabilities = CacheCapabilities {
            action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            symlink_absolute_path_strategy: symlink_absolute_path_strategy::Value::Allowed as i32,
            ..Default::default()
        };
        Ok(ServerCapabilities {
            cache_capabilities: Some(cache_capabilities),
            ..Default::default()
        })
    }

    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:124 InMemoryCacheClient.getAuthority
    async fn get_authority(&self) -> io::Result<String> {
        Ok(String::new())
    }

    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:129 InMemoryCacheClient.downloadActionResult
    async fn download_action_result(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        _inline_out_err: bool,
        _inline_output_files: &HashSet<String>,
    ) -> io::Result<Option<ActionResult>> {
        let action_result = self.ac.lock().get(action_key).cloned();
        Ok(action_result)
    }

    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:139 InMemoryCacheClient.uploadActionResult
    async fn upload_action_result(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        action_result: &ActionResult,
    ) -> io::Result<()> {
        self.ac
            .lock()
            .insert(action_key.clone(), action_result.clone());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:90 InMemoryCacheClient.downloadBlob
    async fn download_blob(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: &mut (dyn Write + Send),
    ) -> io::Result<()> {
        let failure = self.download_failures.lock().get(digest).cloned();
        if let Some(failure) = failure {
            self.num_failures.fetch_add(1, Ordering::SeqCst);
            return Err(errors::shared(&failure));
        }

        let data = self.cas.lock().get(digest).cloned();
        let Some(data) = data else {
            return Err(CacheNotFoundException::new(digest.clone()).into());
        };

        if let Err(e) = out.write_all(&data).and_then(|()| out.flush()) {
            self.num_failures.fetch_add(1, Ordering::SeqCst);
            return Err(e);
        }
        self.num_success.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:146 InMemoryCacheClient.uploadBlobImpl
    async fn upload_blob_impl(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: &dyn Blob,
    ) -> io::Result<()> {
        let mut data = Vec::new();
        blob.get()?.read_to_end(&mut data)?;
        self.cas.lock().insert(digest.clone(), data);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/util/InMemoryCacheClient.java:175 InMemoryCacheClient.close
    fn close(&self) {
        self.cas.lock().clear();
        self.ac.lock().clear();
    }
}

/// `InMemoryCombinedCache`: a combined cache over an [`InMemoryCacheClient`].
///
/// PORT: the Java class extends `RemoteExecutionCache` (which extends `CombinedCache`); here it
/// wraps a [`CombinedCache`] and derefs to it. The tests of `RemoteExecutionCache` need their own
/// wrapper once that class is ported.
// Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:30 InMemoryCombinedCache
pub struct InMemoryCombinedCache {
    combined_cache: CombinedCache,
    /// `remoteCacheClient`.
    remote_cache_client: Arc<dyn RemoteCacheClient>,
    /// `(InMemoryCacheClient) remoteCacheClient`, when it is one.
    in_memory_cache_client: Option<Arc<InMemoryCacheClient>>,
    /// `digestUtil`.
    digest_util: DigestUtil,
}

impl Deref for InMemoryCombinedCache {
    type Target = CombinedCache;

    fn deref(&self) -> &CombinedCache {
        &self.combined_cache
    }
}

impl InMemoryCombinedCache {
    fn create(
        in_memory_cache_client: InMemoryCacheClient,
        digest_util: DigestUtil,
        symlink_template: Option<String>,
    ) -> InMemoryCombinedCache {
        let in_memory_cache_client = Arc::new(in_memory_cache_client);
        let remote_cache_client: Arc<dyn RemoteCacheClient> = in_memory_cache_client.clone();
        InMemoryCombinedCache {
            combined_cache: CombinedCache::new(
                Some(remote_cache_client.clone()),
                /* diskCacheClient= */ None,
                symlink_template,
                digest_util.clone(),
                /* chunkingFunction= */ None,
            ),
            remote_cache_client,
            in_memory_cache_client: Some(in_memory_cache_client),
            digest_util,
        }
    }

    /// `new InMemoryCombinedCache(casEntries, digestUtil)`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:32 InMemoryCombinedCache.<init>
    pub fn new_with_cas_entries(
        cas_entries: HashMap<Digest, Vec<u8>>,
        digest_util: DigestUtil,
    ) -> InMemoryCombinedCache {
        InMemoryCombinedCache::create(
            InMemoryCacheClient::new_with_cas_entries(cas_entries),
            digest_util,
            /* symlinkTemplate= */ None,
        )
    }

    /// `new InMemoryCombinedCache(casEntries, digestUtil, symlinkTemplate)`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:41 InMemoryCombinedCache.<init>
    pub fn new_with_symlink_template(
        cas_entries: HashMap<Digest, Vec<u8>>,
        digest_util: DigestUtil,
        symlink_template: Option<String>,
    ) -> InMemoryCombinedCache {
        InMemoryCombinedCache::create(
            InMemoryCacheClient::new_with_cas_entries(cas_entries),
            digest_util,
            symlink_template,
        )
    }

    /// `new InMemoryCombinedCache(digestUtil)`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:51 InMemoryCombinedCache.<init>
    pub fn new(digest_util: DigestUtil) -> InMemoryCombinedCache {
        InMemoryCombinedCache::create(
            InMemoryCacheClient::new(),
            digest_util,
            /* symlinkTemplate= */ None,
        )
    }

    /// `new InMemoryCombinedCache(remoteCacheClient, digestUtil)`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:60 InMemoryCombinedCache.<init>
    pub fn new_with_remote_cache_client(
        remote_cache_client: Arc<dyn RemoteCacheClient>,
        digest_util: DigestUtil,
    ) -> InMemoryCombinedCache {
        InMemoryCombinedCache {
            combined_cache: CombinedCache::new(
                Some(remote_cache_client.clone()),
                /* diskCacheClient= */ None,
                /* symlinkTemplate= */ None,
                digest_util.clone(),
                /* chunkingFunction= */ None,
            ),
            remote_cache_client,
            in_memory_cache_client: None,
            digest_util,
        }
    }

    /// `(InMemoryCacheClient) remoteCacheClient`.
    fn in_memory_cache_client(&self) -> &InMemoryCacheClient {
        match &self.in_memory_cache_client {
            Some(client) => client,
            None => panic!("ClassCastException: remoteCacheClient is not an InMemoryCacheClient"),
        }
    }

    /// `addContents(context, txt)`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:69 InMemoryCombinedCache.addContents
    pub fn add_contents_str(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        txt: &str,
    ) -> io::Result<Digest> {
        self.add_contents(context, txt.as_bytes())
    }

    /// `addContents(context, bytes)`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:74 InMemoryCombinedCache.addContents
    pub fn add_contents(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        bytes: &[u8],
    ) -> io::Result<Digest> {
        let digest = self.digest_util.compute(bytes);
        get_from_future(self.remote_cache_client.upload_bytes(
            context,
            &digest,
            bytes::Bytes::copy_from_slice(bytes),
            /* force= */ false,
        ))?;
        Ok(digest)
    }

    /// `addContents(context, m)`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:83 InMemoryCombinedCache.addContents
    pub fn add_contents_message<M: JavaMessage>(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        m: &M,
    ) -> io::Result<Digest> {
        self.add_contents(context, &m.to_byte_array())
    }

    /// `addException(txt, e)`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:88 InMemoryCombinedCache.addException
    pub fn add_exception_str(&self, txt: &str, e: io::Error) -> Digest {
        let digest = self.digest_util.compute(txt.as_bytes());
        self.in_memory_cache_client()
            .add_download_failure(digest.clone(), e);
        digest
    }

    /// `addException(m, e)`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:94 InMemoryCombinedCache.addException
    pub fn add_exception_message<M: JavaMessage>(&self, m: &M, e: io::Error) -> Digest {
        let digest = self.digest_util.compute_message(m);
        self.in_memory_cache_client()
            .add_download_failure(digest.clone(), e);
        digest
    }

    /// `getNumSuccessfulDownloads()`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:100 InMemoryCombinedCache.getNumSuccessfulDownloads
    pub fn get_num_successful_downloads(&self) -> i32 {
        self.in_memory_cache_client().get_num_successful_downloads()
    }

    /// `getNumFailedDownloads()`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:104 InMemoryCombinedCache.getNumFailedDownloads
    pub fn get_num_failed_downloads(&self) -> i32 {
        self.in_memory_cache_client().get_num_failed_downloads()
    }

    /// `getNumFindMissingDigests()`.
    // Java: com/google/devtools/build/lib/remote/InMemoryCombinedCache.java:108 InMemoryCombinedCache.getNumFindMissingDigests
    pub fn get_num_find_missing_digests(&self) -> HashMap<Digest, i32> {
        self.in_memory_cache_client().get_num_find_missing_digests()
    }
}
