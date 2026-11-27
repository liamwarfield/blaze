//! Port of `lib/remote/downloader/GrpcRemoteDownloader.java`.

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use blaze_events::{Event, ExtendedEventHandler};
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_repo_fetch::authandtls::{Credentials, Headers};
use blaze_repo_fetch::downloader::download_progress_event::FetchEvent;
use blaze_repo_fetch::downloader::streams::HashOutputStream;
use blaze_repo_fetch::downloader::{Checksum, Downloader};
use blaze_repo_fetch::errors::{IoException, IoExceptionKind};
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::Path;

use crate::common::errors::{self, StatusRuntimeException};
use crate::common::output_digest_mismatch_exception::OutputDigestMismatchException;
use crate::common::{RemoteActionExecutionContext, RemoteCacheClient};
use crate::grpc::channel::ClientInterceptor;
use crate::grpc::{CallOptions, ReferenceCountedChannel, methods};
use crate::reapi::asset::{FetchBlobRequest, FetchBlobResponse, Qualifier};
use crate::reapi::digest_function;
use crate::remote_retrier::RemoteRetrier;
use crate::runtime::get_from_future;
use crate::util::{tracing_metadata_utils, utils};

/// The `Qualifier::name` field uses well-known string keys to attach arbitrary key-value metadata
/// to download requests. These are the qualifier names supported by Bazel.
// Java: com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloader.java:85 GrpcRemoteDownloader.QUALIFIER_CHECKSUM_SRI
const QUALIFIER_CHECKSUM_SRI: &str = "checksum.sri";
// Java: com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloader.java:86 GrpcRemoteDownloader.QUALIFIER_CANONICAL_ID
const QUALIFIER_CANONICAL_ID: &str = "bazel.canonical_id";
/// The `:` character is not permitted in an HTTP header name. So, we use it to delimit the
/// qualifier prefix which denotes an HTTP header qualifer from the header name itself.
// Java: com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloader.java:91 GrpcRemoteDownloader.QUALIFIER_HTTP_HEADER_PREFIX
const QUALIFIER_HTTP_HEADER_PREFIX: &str = "http_header:";
/// Same as HTTP_HEADER_PREFIX, but only apply for a specific URL. The index starts from 0 and
/// corresponds to the URL index in the request. Server should prefer using the URL-specific
/// header value over the generic header value when both are present.
// Java: com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloader.java:96 GrpcRemoteDownloader.QUALIFIER_HTTP_HEADER_URL_PREFIX
const QUALIFIER_HTTP_HEADER_URL_PREFIX: &str = "http_header_url:";

/// A Downloader implementation that uses Bazel's Remote Execution APIs to delegate downloads of
/// external files to a remote service.
///
/// See <https://github.com/bazelbuild/remote-apis> for more details on the exact capabilities and
/// semantics of the Remote Execution API.
// Java: com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloader.java:67 GrpcRemoteDownloader
pub struct GrpcRemoteDownloader {
    build_request_id: String,
    command_id: String,
    channel: ReferenceCountedChannel,
    credentials: Option<Arc<dyn crate::grpc::channel::CallCredentials>>,
    retrier: Arc<RemoteRetrier>,
    cache_client: Arc<dyn RemoteCacheClient>,
    digest_function: digest_function::Value,
    options: Arc<RemoteOptions>,
    verbose_failures: bool,
    http_downloader: Arc<dyn Downloader>,
    remote_downloader_local_fallback: bool,
    /// `TracingMetadataUtils.newDownloaderHeadersInterceptor(options)` of the stub.
    downloader_headers: Arc<dyn ClientInterceptor>,
    closed: AtomicBool,
}

impl GrpcRemoteDownloader {
    // Java: com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloader.java:98 GrpcRemoteDownloader.<init>
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        build_request_id: String,
        command_id: String,
        channel: ReferenceCountedChannel,
        credentials: Option<Arc<dyn crate::grpc::channel::CallCredentials>>,
        retrier: Arc<RemoteRetrier>,
        cache_client: Arc<dyn RemoteCacheClient>,
        digest_function: digest_function::Value,
        options: Arc<RemoteOptions>,
        verbose_failures: bool,
        http_downloader: Arc<dyn Downloader>,
        remote_downloader_local_fallback: bool,
    ) -> GrpcRemoteDownloader {
        let downloader_headers: Arc<dyn ClientInterceptor> =
            Arc::new(crate::authandtls::headers::new_downloader_headers_interceptor(&options));
        GrpcRemoteDownloader {
            build_request_id,
            command_id,
            channel,
            credentials,
            retrier,
            cache_client,
            digest_function,
            options,
            verbose_failures,
            http_downloader,
            remote_downloader_local_fallback,
            downloader_headers,
            closed: AtomicBool::new(false),
        }
    }

    /// Releases the reference to the channel held by this downloader.
    ///
    /// The [`RemoteCacheClient`] is owned by the caller and thus not closed here.
    // Java: com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloader.java:129 GrpcRemoteDownloader.close
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.channel.release();
    }

    // Java: com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloader.java:303 GrpcRemoteDownloader.fetchBlockingStub
    fn fetch_blocking_stub(&self, context: &RemoteActionExecutionContext<'_>) -> CallOptions {
        CallOptions::new()
            .with_request_metadata(context.get_request_metadata().clone())
            .with_interceptor(self.downloader_headers.clone())
            .with_call_credentials(self.credentials.clone())
    }

    /// The download through the remote service; `Err` is the `StatusRuntimeException |
    /// IOException` of Java's `try`.
    fn download_remotely(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        request: &FetchBlobRequest,
        checksum: Option<&Checksum>,
        destination: &Path,
        event_uri: &mut String,
        event_handler: &dyn ExtendedEventHandler,
    ) -> io::Result<()> {
        let response: FetchBlobResponse = self.retrier.execute(|| {
            // channel.withChannelBlocking(channel -> fetchBlockingStub(...).fetchBlob(request))
            get_from_future(async {
                self.channel
                    .unary::<FetchBlobRequest, FetchBlobResponse>(
                        methods::FETCH_BLOB,
                        request.clone(),
                        self.fetch_blocking_stub(context),
                    )
                    .await
                    .map_err(errors::status_to_io_error)
            })
        })?;
        if !response.uri.is_empty() {
            *event_uri = response.uri.clone();
        }
        let status = response.status.clone().unwrap_or_default();
        if status.code == tonic::Code::Ok as i32 {
            event_handler.post(Arc::new(FetchEvent {
                url: event_uri.clone(),
                downloader: "GRPC",
                success: true,
            }));
        } else {
            // StatusProto.toStatusRuntimeException(response.getStatus())
            return Err(StatusRuntimeException::new(tonic::Status::new(
                tonic::Code::from_i32(status.code),
                status.message,
            ))
            .into());
        }
        let blob_digest = response.blob_digest.unwrap_or_default();

        self.retrier.execute(|| {
            let out = destination.get_output_stream()?;
            let result = match checksum {
                Some(checksum) => {
                    let mut out = HashOutputStream::new(out, checksum.clone());
                    get_from_future(self.cache_client.download_blob(
                        context,
                        &blob_digest,
                        &mut out,
                    ))
                    .and_then(|()| {
                        // try-with-resources: close() verifies the checksum.
                        out.close().map_err(io_from_repo_fetch)
                    })
                }
                None => {
                    let mut out = out;
                    get_from_future(self.cache_client.download_blob(
                        context,
                        &blob_digest,
                        &mut out,
                    ))
                    .and_then(|()| out.flush())
                }
            };
            result.map_err(
                |e| match errors::downcast::<OutputDigestMismatchException>(e) {
                    Ok(mut mismatch) => {
                        mismatch.set_output_path(destination.get_path_string());
                        mismatch.into()
                    }
                    Err(e) => e,
                },
            )
        })
    }

    // Java: com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloader.java:241 GrpcRemoteDownloader.newFetchBlobRequest
    #[allow(clippy::too_many_arguments)]
    pub fn new_fetch_blob_request(
        instance_name: &str,
        remote_downloader_propagate_credentials: bool,
        urls: &[JavaUri],
        checksum: Option<&Checksum>,
        canonical_id: &str,
        digest_function: digest_function::Value,
        headers: &Headers,
        credentials: &dyn Credentials,
        now: SystemTime,
    ) -> Result<FetchBlobRequest, IoException> {
        let mut request_builder = FetchBlobRequest {
            instance_name: instance_name.to_string(),
            digest_function: digest_function as i32,
            ..Default::default()
        };
        for (i, url) in urls.iter().enumerate() {
            request_builder.uris.push(url.as_str().to_string());

            if !remote_downloader_propagate_credentials {
                continue;
            }

            let metadata = credentials.get_request_metadata(url)?;
            for (name, values) in metadata {
                for value in values {
                    request_builder.qualifiers.push(Qualifier {
                        name: format!("{QUALIFIER_HTTP_HEADER_URL_PREFIX}{i}:{name}"),
                        value,
                    });
                }
            }
        }

        match checksum {
            Some(checksum) => request_builder.qualifiers.push(Qualifier {
                name: QUALIFIER_CHECKSUM_SRI.to_string(),
                value: checksum.to_subresource_integrity(),
            }),
            None => {
                // If no checksum is provided, never accept cached content.
                // Timestamp is offset by an hour to account for clock skew.
                let millis = (now + Duration::from_secs(3600))
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                // Timestamps.fromMillis
                request_builder.oldest_content_accepted =
                    Some(blaze_protos::google::protobuf::Timestamp {
                        seconds: millis.div_euclid(1000),
                        nanos: (millis.rem_euclid(1000) * 1_000_000) as i32,
                    });
            }
        }

        if !canonical_id.is_empty() {
            request_builder.qualifiers.push(Qualifier {
                name: QUALIFIER_CANONICAL_ID.to_string(),
                value: canonical_id.to_string(),
            });
        }

        for (name, values) in headers {
            // https://www.rfc-editor.org/rfc/rfc9110.html#name-field-order permits merging the
            // field-values with a comma.
            request_builder.qualifiers.push(Qualifier {
                name: format!("{QUALIFIER_HTTP_HEADER_PREFIX}{name}"),
                value: values.join(","),
            });
        }

        Ok(request_builder)
    }
}

impl Downloader for GrpcRemoteDownloader {
    // Java: com/google/devtools/build/lib/remote/downloader/GrpcRemoteDownloader.java:137 GrpcRemoteDownloader.download
    fn download(
        &self,
        urls: &[JavaUri],
        headers: &Headers,
        credentials: Arc<dyn Credentials>,
        checksum: Option<&Checksum>,
        canonical_id: &str,
        destination: &Path,
        event_handler: Arc<dyn ExtendedEventHandler>,
        client_env: &HashMap<String, String>,
        type_: Option<&str>,
        context: &str,
    ) -> Result<(), IoException> {
        // file: URLs can't use the gRPC downloader.
        if urls.iter().any(|url| url.get_scheme() == Some("file")) {
            return self.http_downloader.download(
                urls,
                headers,
                credentials,
                checksum,
                canonical_id,
                destination,
                event_handler,
                client_env,
                type_,
                context,
            );
        }
        let metadata = tracing_metadata_utils::build_metadata_from_parts(
            &self.build_request_id,
            &self.command_id,
            "remote_downloader",
            /* mnemonic= */ None,
            /* label= */ Some(context),
            /* configuration_id= */ None,
        );
        let remote_action_execution_context = RemoteActionExecutionContext::create(metadata);

        let request = Self::new_fetch_blob_request(
            &self.options.remote_instance_name,
            self.options.remote_downloader_propagate_credentials,
            urls,
            checksum,
            canonical_id,
            self.digest_function,
            headers,
            &*credentials,
            SystemTime::now(),
        )?;
        let mut event_uri = urls
            .first()
            .map(|u| u.as_str().to_string())
            .unwrap_or_default();
        let result = self.download_remotely(
            &remote_action_execution_context,
            &request,
            checksum,
            destination,
            &mut event_uri,
            &*event_handler,
        );
        let Err(e) = result else {
            return Ok(());
        };
        if errors::is_interrupted(&e) {
            return Err(IoException::of(IoExceptionKind::InterruptedIOException, ""));
        }
        event_handler.post(Arc::new(FetchEvent {
            url: event_uri,
            downloader: "GRPC",
            success: false,
        }));
        if !self.remote_downloader_local_fallback {
            // `throw new IOException(e)` of a StatusRuntimeException, else `throw e`: both are
            // the message of `e` here.
            return Err(to_repo_fetch(&e));
        }
        event_handler.handle(&Event::warn(format!(
            "Remote Cache: {}",
            utils::grpc_aware_error_message(&e, self.verbose_failures)
        )));
        self.http_downloader.download(
            urls,
            headers,
            credentials,
            checksum,
            canonical_id,
            destination,
            event_handler,
            client_env,
            type_,
            context,
        )
    }
}

/// The downloader's `IOException` carried through the remote code's `io::Error` unchanged (its
/// class, e.g. `UnrecoverableHttpException`, is kept for the caller).
fn io_from_repo_fetch(e: IoException) -> io::Error {
    io::Error::other(e)
}

/// The `IOException` an `io::Error` stands for: the downloader's own when it carries one.
fn to_repo_fetch(e: &io::Error) -> IoException {
    match e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<IoException>())
    {
        Some(e) => e.clone(),
        None => IoException::new(e.to_string()),
    }
}
