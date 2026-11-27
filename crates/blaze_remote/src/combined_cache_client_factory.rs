//! Port of `lib/remote/CombinedCacheClientFactory.java`.

use std::io;
use std::sync::Arc;

use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_vfs::Path;

use crate::authandtls::Credentials;
use crate::common::remote_cache_client::RemoteCacheClient;
use crate::disk::disk_cache_client::DiskCacheClient;
use crate::remote_retrier::RemoteRetrier;
use crate::util::digest_util::DigestUtil;

/// A record holding a [`DiskCacheClient`] and [`RemoteCacheClient`] pair. Either may be absent.
// Java: com/google/devtools/build/lib/remote/CombinedCacheClientFactory.java:43 CombinedCacheClientFactory.CombinedCacheClient
#[derive(Clone)]
pub struct CombinedCacheClient {
    remote_cache_client: Option<Arc<dyn RemoteCacheClient>>,
    disk_cache_client: Option<Arc<DiskCacheClient>>,
}

impl CombinedCacheClient {
    /// `new CombinedCacheClient(remoteCacheClient, diskCacheClient)`.
    pub fn new(
        remote_cache_client: Option<Arc<dyn RemoteCacheClient>>,
        disk_cache_client: Option<Arc<DiskCacheClient>>,
    ) -> CombinedCacheClient {
        CombinedCacheClient {
            remote_cache_client,
            disk_cache_client,
        }
    }

    /// `remoteCacheClient()`.
    pub fn remote_cache_client(&self) -> Option<&Arc<dyn RemoteCacheClient>> {
        self.remote_cache_client.as_ref()
    }

    /// `diskCacheClient()`.
    pub fn disk_cache_client(&self) -> Option<&Arc<DiskCacheClient>> {
        self.disk_cache_client.as_ref()
    }
}

/// `CombinedCacheClientFactory.create(options, creds, authAndTlsOptions, workingDirectory,
/// digestUtil, retrier, checkActionResultIntegrity)`.
///
/// PORT: `workingDirectory` is an `Option` so that a null working directory fails as in Java
/// (`Preconditions.checkNotNull`).
// Java: com/google/devtools/build/lib/remote/CombinedCacheClientFactory.java:46 CombinedCacheClientFactory.create
pub fn create(
    options: &RemoteOptions,
    creds: Option<Arc<dyn Credentials>>,
    auth_and_tls_options: &AuthAndTLSOptions,
    working_directory: Option<&Path>,
    digest_util: &DigestUtil,
    retrier: Arc<RemoteRetrier>,
    check_action_result_integrity: bool,
) -> io::Result<CombinedCacheClient> {
    let Some(working_directory) = working_directory else {
        panic!("NullPointerException: workingDirectory");
    };
    let mut http_cache_client: Option<Arc<dyn RemoteCacheClient>> = None;
    let mut disk_cache_client: Option<Arc<DiskCacheClient>> = None;
    if is_http_cache(options) {
        http_cache_client = Some(create_http(
            options,
            creds,
            auth_and_tls_options,
            digest_util,
            retrier,
        ));
    }
    if is_disk_cache(options) {
        disk_cache_client = Some(Arc::new(create_disk_cache(
            working_directory,
            options,
            digest_util,
            check_action_result_integrity,
        )?));
    }
    if http_cache_client.is_none() && disk_cache_client.is_none() {
        panic!(
            "IllegalArgumentException: Unrecognized RemoteOptions configuration: remote Http cache URL and/or local disk cache options expected."
        );
    }
    Ok(CombinedCacheClient::new(
        http_cache_client,
        disk_cache_client,
    ))
}

/// `isRemoteCacheOptions(options)`.
// Java: com/google/devtools/build/lib/remote/CombinedCacheClientFactory.java:73 CombinedCacheClientFactory.isRemoteCacheOptions
pub fn is_remote_cache_options(options: &RemoteOptions) -> bool {
    is_http_cache(options) || is_disk_cache(options)
}

/// `createHttp(options, creds, authAndTlsOptions, digestUtil, retrier)`.
///
/// Java wraps every exception of the body into a `RuntimeException` (a crash); here those are
/// panics with the text of `new RuntimeException(e).getMessage()` (`e.toString()`).
// Java: com/google/devtools/build/lib/remote/CombinedCacheClientFactory.java:77 CombinedCacheClientFactory.createHttp
fn create_http(
    options: &RemoteOptions,
    creds: Option<Arc<dyn Credentials>>,
    auth_and_tls_options: &AuthAndTLSOptions,
    digest_util: &DigestUtil,
    retrier: Arc<RemoteRetrier>,
) -> Arc<dyn RemoteCacheClient> {
    let Some(remote_cache) = options.remote_cache.as_deref() else {
        panic!("NullPointerException: remoteCache");
    };

    // `URI uri = URI.create(options.remoteCache)`: an IllegalArgumentException (wrapping the
    // URISyntaxException) for a malformed URI.
    let uri = match blaze_repo_fetch::uri::JavaUri::create(remote_cache) {
        Ok(uri) => uri,
        Err(e) => panic!("java.lang.RuntimeException: java.lang.IllegalArgumentException: {e}"),
    };
    let scheme = uri.get_scheme().unwrap_or("");
    // Preconditions.checkArgument(Ascii.toLowerCase(uri.getScheme()).startsWith("http"), ...)
    if !scheme.to_ascii_lowercase().starts_with("http") {
        panic!(
            "java.lang.RuntimeException: java.lang.IllegalArgumentException: remoteCache should start with http"
        );
    }

    let domain_socket = match &options.remote_proxy {
        Some(remote_proxy) if remote_proxy.starts_with("unix:") => {
            // new DomainSocketAddress(options.remoteProxy.replaceFirst("^unix:", ""))
            Some(std::path::PathBuf::from(
                blaze_util::string_encoding::internal_to_platform(&remote_proxy["unix:".len()..])
                    .into_owned(),
            ))
        }
        Some(remote_proxy) => {
            panic!(
                "java.lang.RuntimeException: java.lang.Exception: Remote cache proxy unsupported: {remote_proxy}"
            );
        }
        None => None,
    };
    // Math.toIntExact(options.remoteTimeout.toSeconds())
    let timeout_seconds = match i32::try_from(options.remote_timeout.as_secs()) {
        Ok(seconds) => seconds,
        Err(_) => {
            panic!("java.lang.RuntimeException: java.lang.ArithmeticException: integer overflow")
        }
    };
    // HttpCacheClient.createSSLContext, when the scheme is https.
    let tls = if scheme == "https" {
        let host = uri.get_host().unwrap_or_default().to_string();
        match crate::authandtls::google_auth_utils::http_cache_tls_config(
            &host,
            auth_and_tls_options,
        ) {
            Ok(config) => Some(Arc::new(config)),
            Err(e) => panic!("java.lang.RuntimeException: java.io.IOException: {e}"),
        }
    } else {
        None
    };
    match crate::http::http_cache_client::HttpCacheClient::create(
        domain_socket,
        uri,
        timeout_seconds,
        options.remote_max_connections,
        options.remote_verify_downloads,
        effective_headers(options),
        digest_util.clone(),
        retrier,
        creds,
        tls,
        crate::util::tracing_metadata_utils::tool_version(),
    ) {
        Ok(client) => Arc::new(client),
        Err(e) => panic!("java.lang.RuntimeException: java.io.IOException: {e}"),
    }
}

/// `createDiskCache(workingDirectory, options, digestUtil, checkActionResultIntegrity)`.
// Java: com/google/devtools/build/lib/remote/CombinedCacheClientFactory.java:124 CombinedCacheClientFactory.createDiskCache
pub fn create_disk_cache(
    working_directory: &Path,
    options: &RemoteOptions,
    digest_util: &DigestUtil,
    check_action_result_integrity: bool,
) -> io::Result<DiskCacheClient> {
    let Some(disk_cache) = &options.disk_cache else {
        panic!("NullPointerException");
    };
    let cache_dir = working_directory.get_relative(disk_cache.get());
    DiskCacheClient::new(cache_dir, digest_util, check_action_result_integrity)
}

/// `isDiskCache(options)`.
// Java: com/google/devtools/build/lib/remote/CombinedCacheClientFactory.java:134 CombinedCacheClientFactory.isDiskCache
pub fn is_disk_cache(options: &RemoteOptions) -> bool {
    options
        .disk_cache
        .as_ref()
        .is_some_and(|disk_cache| !disk_cache.get().is_empty())
}

/// `isHttpCache(options)`.
// Java: com/google/devtools/build/lib/remote/CombinedCacheClientFactory.java:138 CombinedCacheClientFactory.isHttpCache
pub fn is_http_cache(options: &RemoteOptions) -> bool {
    options.remote_cache.as_ref().is_some_and(|remote_cache| {
        // `Ascii.toLowerCase`
        let remote_cache = remote_cache.to_ascii_lowercase();
        remote_cache.starts_with("http://") || remote_cache.starts_with("https://")
    })
}

/// `effectiveHeaders(options)`.
// Java: com/google/devtools/build/lib/remote/CombinedCacheClientFactory.java:144 CombinedCacheClientFactory.effectiveHeaders
pub fn effective_headers(options: &RemoteOptions) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    headers.extend(options.remote_headers.iter().cloned());
    headers.extend(options.remote_cache_headers.iter().cloned());
    headers
}
