//! Port of the channel-level header interceptors of `lib/remote/util/TracingMetadataUtils.java`
//! (`newCacheHeadersInterceptor`, `newDownloaderHeadersInterceptor`, `newExecHeadersInterceptor`).
//!
//! These attach the `--remote_header` family to every call of one channel. Each function reads
//! exactly the options Java reads: the common `--remote_header` list followed by the list of the
//! one service the channel talks to, so a `--remote_cache_header` never reaches the executor's
//! channel and vice versa.

use blaze_optdefs::misc::remote_options::RemoteOptions;

use crate::grpc::AttachHeadersInterceptor;

/// `newMetadataForHeaders(headers)` followed by `metadata.merge(newMetadataForHeaders(more))`,
/// wrapped in `MetadataUtils.newAttachHeadersInterceptor(metadata)`.
///
/// `Metadata.put` and `Metadata.merge` append, so the result holds the entries of `headers` and
/// then those of `more`, in order, duplicates included.
///
/// PORT: `Metadata.Key.of(name, ASCII_STRING_MARSHALLER)` throws an `IllegalArgumentException`
/// for an invalid header name (and `Metadata.put` for an invalid value when the call starts);
/// both are a panic here, when the interceptor is built.
// Java: com/google/devtools/build/lib/remote/util/TracingMetadataUtils.java:129 TracingMetadataUtils.newMetadataForHeaders
fn new_attach_headers_interceptor(
    headers: &[(String, String)],
    more: &[(String, String)],
) -> AttachHeadersInterceptor {
    let mut metadata: Vec<(String, String)> = Vec::with_capacity(headers.len() + more.len());
    metadata.extend(headers.iter().cloned());
    metadata.extend(more.iter().cloned());
    match AttachHeadersInterceptor::new(&metadata) {
        Ok(interceptor) => interceptor,
        Err(e) => panic!("IllegalArgumentException: {e}"),
    }
}

/// `--remote_header` and `--remote_cache_header`, for the channel of the remote cache.
// Java: com/google/devtools/build/lib/remote/util/TracingMetadataUtils.java:139 TracingMetadataUtils.newCacheHeadersInterceptor
pub fn new_cache_headers_interceptor(options: &RemoteOptions) -> AttachHeadersInterceptor {
    new_attach_headers_interceptor(&options.remote_headers, &options.remote_cache_headers)
}

/// `--remote_header` and `--remote_downloader_header`, for the channel of the remote downloader.
// Java: com/google/devtools/build/lib/remote/util/TracingMetadataUtils.java:145 TracingMetadataUtils.newDownloaderHeadersInterceptor
pub fn new_downloader_headers_interceptor(options: &RemoteOptions) -> AttachHeadersInterceptor {
    new_attach_headers_interceptor(&options.remote_headers, &options.remote_downloader_headers)
}

/// `--remote_header` and `--remote_exec_header`, for the channel of the remote executor.
// Java: com/google/devtools/build/lib/remote/util/TracingMetadataUtils.java:151 TracingMetadataUtils.newExecHeadersInterceptor
pub fn new_exec_headers_interceptor(options: &RemoteOptions) -> AttachHeadersInterceptor {
    new_attach_headers_interceptor(&options.remote_headers, &options.remote_exec_headers)
}
