//! Port of `lib/authandtls/GoogleAuthUtils.java`: utility methods for using
//! `AuthAndTLSOptions` with gRPC channels.
//!
//! PORT: Java builds a Netty `ManagedChannel`; the port builds a tonic `Channel` (HTTP/2 over
//! hyper, TLS by rustls). What Netty takes as a `NegotiationType` is the scheme of the endpoint
//! URI here: tonic encrypts a connection if and only if the URI is `https` (and refuses an
//! `https` URI without a TLS configuration), so [`new_channel`] writes `https://` exactly when
//! Java picks `NegotiationType.TLS` and `http://` exactly when it picks
//! `NegotiationType.PLAINTEXT`. Server certificates are always verified on a TLS channel: the
//! only `ClientTlsConfig` this module builds is rustls' default WebPKI verifier over either the
//! `--tls_certificate` file or the webpki roots; no custom verifier is ever installed.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use blaze_events::{Event, EventHandler};
use blaze_optdefs::misc::auth_and_tls_options::{AuthAndTLSOptions, CredentialHelperOption};
use blaze_repo_fetch::authandtls::{Headers, NetrcCredentials, netrc_parser};
use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{FileSystemRef, Path};
use indexmap::IndexMap;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

use super::Credentials;
use super::call_credentials_provider::{
    CallCredentialsProvider, GoogleAuthCallCredentialsProvider, NO_CREDENTIALS,
};
use crate::common::errors::io_exception;
use crate::grpc::{CallCredentials, ClientInterceptor};

/// What `GoogleAuthUtils.newChannel` hands to its caller (`io.grpc.ManagedChannel`), as the
/// pieces `ReferenceCountedChannel::new` takes.
///
/// PORT: a Java `ManagedChannel` carries its interceptors, its authority and its default service
/// config; the Rust transport channel does not, so they travel next to it.
pub struct ManagedChannel {
    /// The transport, connecting lazily on first use (as a gRPC channel does).
    pub channel: Channel,
    /// `ManagedChannel.authority()`: the `--tls_authority_override` on a TLS channel that has
    /// one, else the authority of the target.
    pub authority: String,
    /// Whether the transport is TLS (`NegotiationType.TLS`).
    pub tls: bool,
    /// `builder.intercept(interceptors)`, in execution order: gRPC runs the interceptors of a
    /// channel builder in the reverse of the order they were added in, so this is the reverse of
    /// the list given to [`new_channel`].
    pub interceptors: Vec<Arc<dyn ClientInterceptor>>,
    /// `maxInboundMessageSize(Integer.MAX_VALUE)`.
    pub max_inbound_message_size: usize,
    /// `builder.disableServiceConfigLookUp(); builder.defaultServiceConfig(serviceConfig)`.
    ///
    /// NEEDS(channel): at the pin no remote stub calls `withDeadlineAfter`; the only call
    /// deadline is the `methodConfig[].timeout` of this service config
    /// (`RemoteGrpcServiceConfig.create`: `--remote_timeout` for every method of ActionCache,
    /// Capabilities, ContentAddressableStorage, ByteStream and Fetch).
    /// `ReferenceCountedChannel::new` has no parameter for it; it has to take the config and
    /// apply the matching method timeout as the deadline of every call that sets none.
    pub service_config: Option<serde_json::Map<String, serde_json::Value>>,
}

impl std::fmt::Debug for ManagedChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedChannel")
            .field("authority", &self.authority)
            .field("tls", &self.tls)
            .field("max_inbound_message_size", &self.max_inbound_message_size)
            .field("service_config", &self.service_config)
            .finish_non_exhaustive()
    }
}

/// `KeepAliveManager.MIN_KEEPALIVE_TIME_NANOS`: `NettyChannelBuilder.keepAliveTime` bumps a
/// smaller keepalive time to 10 seconds.
// Java: io/grpc/internal/KeepAliveManager.java:35 KeepAliveManager.MIN_KEEPALIVE_TIME_NANOS
const MIN_KEEPALIVE_TIME: Duration = Duration::from_secs(10);
/// `KeepAliveManager.MIN_KEEPALIVE_TIMEOUT_NANOS`: `NettyChannelBuilder.keepAliveTimeout` bumps a
/// smaller keepalive timeout to 10 milliseconds.
// Java: io/grpc/internal/KeepAliveManager.java:36 KeepAliveManager.MIN_KEEPALIVE_TIMEOUT_NANOS
const MIN_KEEPALIVE_TIMEOUT: Duration = Duration::from_millis(10);
/// `NettyChannelBuilder.AS_LARGE_AS_INFINITE` (`TimeUnit.DAYS.toNanos(1000L)`): a keepalive time
/// at least this large disables keepalive.
// Java: io/grpc/netty/NettyChannelBuilder.java NettyChannelBuilder.AS_LARGE_AS_INFINITE
const AS_LARGE_AS_INFINITE: Duration = Duration::from_secs(1000 * 24 * 60 * 60);

/// The `IOException` of the `catch (RuntimeException e)` of `newChannel`.
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:148 GoogleAuthUtils.newChannel
fn failed_to_connect(target_url: &str, message: impl std::fmt::Display) -> io::Error {
    // gRPC might throw all kinds of RuntimeExceptions: StatusRuntimeException,
    // IllegalStateException, NullPointerException, ...
    io_exception(format!("Failed to connect to '{target_url}': {message}"))
}

/// Create a new gRPC channel.
///
/// Fails in case the channel can't be constructed.
///
/// * `interceptors`: the list Java passes to `builder.intercept(interceptors)`, in Java's order.
///
/// PORT: Java's `executor` parameter has no counterpart (the calls run on
/// `crate::runtime::runtime()`).
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:74 GoogleAuthUtils.newChannel
pub fn new_channel(
    target: &str,
    proxy: Option<&str>,
    options: &AuthAndTLSOptions,
    interceptors: Option<Vec<Arc<dyn ClientInterceptor>>>,
    service_config: Option<&serde_json::Map<String, serde_json::Value>>,
) -> io::Result<ManagedChannel> {
    let ssl_context = if is_tls_enabled(target) {
        Some(create_ssl_context(
            options.tls_certificate.as_deref(),
            options.tls_client_certificate.as_deref(),
            options.tls_client_key.as_deref(),
        )?)
    } else {
        None
    };

    let target_url = convert_target_scheme(target);
    // PORT: `.negotiationType(isTlsEnabled(target) ? TLS : PLAINTEXT)` is the scheme of the
    // endpoint URI (see the module comment), so it is decided where the builder is created.
    let mut builder = new_netty_channel_builder(&target_url, proxy, is_tls_enabled(target))?;
    // .executor(executor): see above.
    // The server is trusted, so the default limit of 4 MiB on inbound messages only
    // breaks legitimately large messages such as the ActionResult of an action with
    // many output files (https://github.com/bazelbuild/bazel/issues/29821).
    let max_inbound_message_size = i32::MAX as usize;
    if !options.grpc_keepalive_time.is_zero() {
        // builder.keepAliveTime(options.grpcKeepaliveTime.toNanos(), NANOSECONDS);
        let keep_alive_time = options.grpc_keepalive_time.max(MIN_KEEPALIVE_TIME);
        // builder.keepAliveTimeout(options.grpcKeepaliveTimeout.toNanos(), NANOSECONDS);
        if options.grpc_keepalive_timeout.is_zero() {
            // checkArgument(keepAliveTimeout > 0L, "keepalive timeout must be positive")
            return Err(failed_to_connect(
                &target_url,
                "keepalive timeout must be positive",
            ));
        }
        let keep_alive_timeout = options.grpc_keepalive_timeout.max(MIN_KEEPALIVE_TIMEOUT);
        // Bump keepalive time to infinite. This disables keepalive.
        if keep_alive_time < AS_LARGE_AS_INFINITE {
            // Java sends keepalive pings only while there is at least one pending call
            // (`keepAliveWithoutCalls` stays false), which is also hyper's default
            // (`keep_alive_while_idle(false)`).
            builder = builder
                .http2_keep_alive_interval(keep_alive_time)
                .keep_alive_timeout(keep_alive_timeout);
        }
    }
    let is_unix_socket_channel = target_url.starts_with("unix:") || !proxy.unwrap_or("").is_empty();
    if options.grpc_tcp_keepalive && !is_unix_socket_channel {
        // builder.withOption(ChannelOption.SO_KEEPALIVE, true);
        let idle_seconds = options.grpc_tcp_keepalive_time.as_secs();
        if idle_seconds > 0 {
            // TCP_KEEPIDLE
            builder = builder.tcp_keepalive(Some(Duration::from_secs(to_int_exact(
                idle_seconds,
                &target_url,
            )?)));
        } else {
            // PORT: Java turns SO_KEEPALIVE on and leaves TCP_KEEPIDLE at the system default;
            // tonic can only enable SO_KEEPALIVE together with an idle time.
            blaze_util::unported!(
                "GoogleAuthUtils.newChannel (SO_KEEPALIVE with the system default TCP_KEEPIDLE: --grpc_tcp_keepalive_time=0)"
            );
        }
        let interval_seconds = options.grpc_tcp_keepalive_interval.as_secs();
        if interval_seconds > 0 {
            // TCP_KEEPINTVL
            builder = builder.tcp_keepalive_interval(Some(Duration::from_secs(to_int_exact(
                interval_seconds,
                &target_url,
            )?)));
        }
        if options.grpc_tcp_keepalive_count > 0 {
            // TCP_KEEPCNT
            builder = builder.tcp_keepalive_retries(Some(options.grpc_tcp_keepalive_count as u32));
        }
    }
    // if (interceptors != null) builder.intercept(interceptors);
    // Interceptors run in the reverse order in which they are added
    // (`ManagedChannelBuilder.intercept`).
    let mut interceptors = interceptors.unwrap_or_default();
    interceptors.reverse();
    // if (serviceConfig != null) { builder.disableServiceConfigLookUp();
    // builder.defaultServiceConfig(serviceConfig); }
    // PORT: there is no service config lookup to disable (no DNS TXT resolution); the default
    // service config is handed to the caller (see `ManagedChannel::service_config`).
    let service_config = service_config.cloned();

    let tls = ssl_context.is_some();
    // PORT: the authority of `NettyChannelBuilder.forTarget(targetUrl)` is the authority of the
    // target; `Endpoint::uri` is `<scheme>://<targetUrl>`.
    let mut authority = match builder.uri().authority() {
        Some(authority) => authority.as_str().to_string(),
        None => {
            return Err(failed_to_connect(&target_url, "target has no authority"));
        }
    };
    // The client certificate rustls refused when the TLS configuration was built (see below).
    let mut rejected_client_certificate: Option<String> = None;
    if let Some(mut ssl_context) = ssl_context {
        // builder.sslContext(sslContext);
        if let Some(tls_authority_override) = &options.tls_authority_override {
            // builder.overrideAuthority(options.tlsAuthorityOverride): the authority is the
            // `:authority` of every call and the name the server certificate is verified against
            // (it does not weaken verification: the certificate must still chain to a trust root
            // and must be valid for this name).
            let parsed = check_authority(tls_authority_override)
                .map_err(|message| failed_to_connect(&target_url, message))?;
            let origin = http::Uri::builder()
                .scheme("https")
                .authority(parsed.clone())
                .path_and_query("/")
                .build()
                .map_err(|e| failed_to_connect(&target_url, e))?;
            let host = parsed.host();
            // `Authority::host` keeps the brackets of an IPv6 literal; the certificate name is
            // the address itself.
            let host = host
                .strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .unwrap_or(host);
            ssl_context = ssl_context.domain_name(host);
            builder = builder.origin(origin);
            authority = tls_authority_override.clone();
        }
        // PORT: Java builds the `SslContext` before the channel builder
        // (`sslContextBuilder.build()` in `createSSlContext`); rustls builds its configuration
        // here, so its failures carry the text of that `catch`.
        let has_client_certificate =
            options.tls_client_certificate.is_some() && options.tls_client_key.is_some();
        builder = match builder.clone().tls_config(ssl_context) {
            Ok(builder) => builder,
            Err(e)
                if has_client_certificate
                    && create_ssl_context(options.tls_certificate.as_deref(), None, None)
                        .and_then(|without_identity| {
                            builder
                                .clone()
                                .tls_config(without_identity)
                                .map_err(|e| io_exception(e.to_string()))
                        })
                        .is_ok() =>
            {
                // PORT: rustls checks the client certificate when the configuration is built
                // (`with_client_auth_cert` parses it; it refuses an X.509 v1 certificate, root
                // ruling 2026-11-19: no custom verifier, fail closed), where Java's
                // `keyManager` / `build()` accept it and certificates are only judged in the
                // handshake. The rejection is therefore reported as a handshake failure of
                // every connection (a transport error: the capabilities check fails with it),
                // not as a "Failed to init TLS infrastructure" error of the channel.
                rejected_client_certificate = Some(innermost_message(&e));
                builder
            }
            Err(e) => {
                return Err(io_exception(format!(
                    "Failed to init TLS infrastructure: {}",
                    error_chain(&e)
                )));
            }
        };
    }
    // return builder.build();
    // PORT: `connect_lazy` needs a runtime context for the channel's background task.
    let channel = {
        let _guard = crate::runtime::runtime().enter();
        match rejected_client_certificate {
            Some(message) => {
                builder.connect_with_connector_lazy(tower::service_fn(move |_uri: http::Uri| {
                    let message = message.clone();
                    async move {
                        Err::<hyper_util::rt::TokioIo<tokio::net::TcpStream>, io::Error>(
                            io::Error::new(io::ErrorKind::InvalidData, message),
                        )
                    }
                }))
            }
            None => builder.connect_lazy(),
        }
    };
    Ok(ManagedChannel {
        channel,
        authority,
        tls,
        interceptors,
        max_inbound_message_size,
        service_config,
    })
}

/// `Math.toIntExact(seconds)`, whose `ArithmeticException` reaches the
/// `catch (RuntimeException e)` of `newChannel`.
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:116 GoogleAuthUtils.newChannel
fn to_int_exact(seconds: u64, target_url: &str) -> io::Result<u64> {
    if seconds > i32::MAX as u64 {
        return Err(failed_to_connect(target_url, "integer overflow"));
    }
    Ok(seconds)
}

/// The message of an error and of its sources (`Throwable.getMessage()` of a wrapped cause).
/// The message of the innermost error of `e`'s source chain (rustls' own text).
fn innermost_message(e: &(dyn std::error::Error + 'static)) -> String {
    let mut current = e;
    while let Some(s) = current.source() {
        current = s;
    }
    current.to_string()
}

fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut message = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        message.push_str(": ");
        message.push_str(&s.to_string());
        source = s.source();
    }
    message
}

/// `GrpcUtil.checkAuthority(authority)`, which `overrideAuthority` applies.
///
/// PORT: Java parses with `new URI(null, authority, null, null, null)`; the port parses with
/// `http::uri::Authority`, so the text of the first failure is the `http` crate's.
// Java: io/grpc/internal/GrpcUtil.java:530 GrpcUtil.checkAuthority
fn check_authority(authority: &str) -> Result<http::uri::Authority, String> {
    let parsed = authority
        .parse::<http::uri::Authority>()
        .map_err(|e| format!("Invalid authority: {authority}: {e}"))?;
    // Verify that the user Info is not provided.
    if authority.contains('@') {
        return Err(format!(
            "Userinfo must not be present on authority: '{authority}'"
        ));
    }
    Ok(parsed)
}

/// Converts 'grpc(s)' into an empty protocol, because 'grpc(s)' is not a widely supported scheme
/// and is interpreted as 'dns' under the hood.
///
/// Returns the target URL with converted scheme.
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:162 GoogleAuthUtils.convertTargetScheme
fn convert_target_scheme(target: &str) -> String {
    target.replace("grpcs://", "").replace("grpc://", "")
}

/// Whether the channel to `target` is encrypted.
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:166 GoogleAuthUtils.isTlsEnabled
pub fn is_tls_enabled(target: &str) -> bool {
    // 'grpcs://' or empty prefix => TLS-enabled
    // when no schema prefix is provided in URL, bazel will treat it as a gRPC request with TLS
    // enabled
    !target.starts_with("grpc://") && !target.starts_with("unix:")
}

/// The base64 bodies of the PEM sections of `pem` whose label satisfies `label`, in order, with
/// their labels. A section whose body is not base64 is skipped.
///
/// This is the form rustls reads (`-----BEGIN <label>-----` ... `-----END <label>-----`).
fn pem_sections(pem: &str, label: impl Fn(&str) -> bool) -> Vec<(&str, Vec<u8>)> {
    use base64::Engine as _;
    let mut sections = Vec::new();
    let mut lines = pem.lines();
    while let Some(line) = lines.next() {
        let Some(section_label) = line
            .trim()
            .strip_prefix("-----BEGIN ")
            .and_then(|l| l.strip_suffix("-----"))
        else {
            continue;
        };
        let end = format!("-----END {section_label}-----");
        let mut body = String::new();
        let mut terminated = false;
        for body_line in lines.by_ref() {
            let body_line = body_line.trim();
            if body_line == end {
                terminated = true;
                break;
            }
            body.push_str(body_line);
        }
        if !terminated || !label(section_label) {
            continue;
        }
        if let Ok(der) = base64::engine::general_purpose::STANDARD.decode(body.as_bytes())
            && !der.is_empty()
        {
            sections.push((section_label, der));
        }
    }
    sections
}

/// `SslContext.toX509Certificates(file)`: the PEM text of a certificate collection file.
///
/// `Err(())` is any exception of the Java method (unreadable file, no certificate in it), which
/// `SslContextBuilder` reports as `"File does not contain valid certificates: " + file`.
///
/// PORT: Netty also parses every certificate (`CertificateFactory.generateCertificate`); here
/// the file must hold at least one `CERTIFICATE` section in base64, and rustls parses the
/// certificates when the connection configuration is built. A section rustls cannot parse is
/// not a trust root, so a file of unusable certificates trusts nothing (the connection fails)
/// instead of failing here.
// Java: io/netty/handler/ssl/SslContext.java SslContext.toX509Certificates
fn to_x509_certificates(file: &str) -> Result<Vec<u8>, ()> {
    // PORT: `new File(path)` of a relative path resolves against the working directory of the
    // process, as here. The option value is an internal string; the JVM hands `File` paths to
    // the OS as platform strings.
    let pem =
        std::fs::read(blaze_util::string_encoding::internal_to_platform(file)).map_err(|_| ())?;
    let text = std::str::from_utf8(&pem).map_err(|_| ())?;
    if pem_sections(text, |label| label == "CERTIFICATE").is_empty() {
        // CertificateException("found no certificates in input stream")
        return Err(());
    }
    Ok(pem)
}

/// `SslContext.toPrivateKey(keyFile, null)`: the PEM text of an unencrypted PKCS#8 private key
/// file.
///
/// `Err(())` is any exception of the Java method, which `SslContextBuilder` reports as
/// `"File does not contain valid private key: " + keyFile`.
///
/// PORT: Netty takes the first `... PRIVATE KEY` section and decodes it as PKCS#8
/// (`PKCS8EncodedKeySpec`), so a PKCS#1 (`RSA PRIVATE KEY`), SEC1 (`EC PRIVATE KEY`) or
/// encrypted key is refused; the port refuses them by their section label.
// Java: io/netty/handler/ssl/SslContext.java SslContext.toPrivateKey
fn to_private_key(key_file: &str) -> Result<Vec<u8>, ()> {
    let pem = std::fs::read(blaze_util::string_encoding::internal_to_platform(key_file))
        .map_err(|_| ())?;
    let text = std::str::from_utf8(&pem).map_err(|_| ())?;
    let sections = pem_sections(text, |label| label.ends_with("PRIVATE KEY"));
    match sections.first() {
        Some((label, _)) if *label == "PRIVATE KEY" => Ok(pem),
        // KeyException("could not find a PKCS #8 private key in input stream ...") or the
        // InvalidKeySpecException of a key in another format.
        _ => Err(()),
    }
}

/// Builds the TLS configuration of a channel (`io.netty.handler.ssl.SslContext`).
///
/// Trust: with `--tls_certificate` the certificates of that file are the only trust roots
/// (`SslContextBuilder.trustManager(File)` replaces the default trust manager); without it the
/// roots are the webpki roots, standing in for the JDK's bundled `cacerts`
/// (docs/decisions.md 2026-11-18, "TLS trust").
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:173 GoogleAuthUtils.createSSlContext
fn create_ssl_context(
    root_cert: Option<&str>,
    client_cert: Option<&str>,
    client_key: Option<&str>,
) -> io::Result<ClientTlsConfig> {
    // sslContextBuilder = GrpcSslContexts.forClient();
    let mut ssl_context_builder = ClientTlsConfig::new();
    match root_cert {
        Some(root_cert) => {
            // sslContextBuilder.trustManager(new File(rootCert));
            match to_x509_certificates(root_cert) {
                Ok(pem) => {
                    ssl_context_builder =
                        ssl_context_builder.ca_certificate(Certificate::from_pem(pem));
                }
                Err(()) => {
                    let message = format!(
                        "Failed to init TLS infrastructure using '{}' as root certificate: {}",
                        root_cert,
                        format_args!("File does not contain valid certificates: {root_cert}")
                    );
                    return Err(io_exception(message));
                }
            }
        }
        None => {
            // The default trust manager of the JVM.
            ssl_context_builder = ssl_context_builder.with_webpki_roots();
        }
    }
    if let (Some(client_cert), Some(client_key)) = (client_cert, client_key) {
        // sslContextBuilder.keyManager(new File(clientCert), new File(clientKey));
        let key_cert_chain = to_x509_certificates(client_cert).map_err(|()| {
            io_exception(format!(
                "Failed to init TLS infrastructure using '{}' as client certificate: {}",
                client_cert,
                format_args!("File does not contain valid certificates: {client_cert}")
            ))
        })?;
        let key = to_private_key(client_key).map_err(|()| {
            io_exception(format!(
                "Failed to init TLS infrastructure using '{}' as client certificate: {}",
                client_cert,
                format_args!("File does not contain valid private key: {client_key}")
            ))
        })?;
        ssl_context_builder = ssl_context_builder.identity(Identity::from_pem(key_cert_chain, key));
    }
    // return sslContextBuilder.build(): see `new_channel`.
    Ok(ssl_context_builder)
}

// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:224 GoogleAuthUtils.newUnixNettyChannelBuilder
fn new_unix_netty_channel_builder(_target: &str) -> io::Result<Endpoint> {
    blaze_util::unported!(
        "GoogleAuthUtils.newUnixNettyChannelBuilder (unix domain socket channels: unix: targets and --remote_proxy=unix:...)"
    )
}

/// The channel builder of a target.
///
/// * `tls`: the `NegotiationType` Java sets on the builder afterwards (see [`new_channel`]).
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:238 GoogleAuthUtils.newNettyChannelBuilder
fn new_netty_channel_builder(
    target_url: &str,
    proxy: Option<&str>,
    tls: bool,
) -> io::Result<Endpoint> {
    if target_url.starts_with("unix:") {
        return new_unix_netty_channel_builder(target_url);
    }

    let proxy = proxy.unwrap_or("");
    if proxy.is_empty() {
        // return NettyChannelBuilder.forTarget(targetUrl).defaultLoadBalancingPolicy("round_robin");
        // PORT: a target without a name resolver scheme is `dns:///<target>` for gRPC: the
        // authority is the target and the default port is 443 for TLS and 80 for plaintext
        // (`GrpcUtil.DEFAULT_PORT_SSL`/`DEFAULT_PORT_PLAINTEXT`), which are the default ports of
        // `https` and `http`. One connection is made (crate::grpc::channel: no round robin).
        if target_url.starts_with("dns:") {
            blaze_util::unported!(
                "GoogleAuthUtils.newNettyChannelBuilder (NettyChannelBuilder.forTarget with a name resolver scheme)",
                "{}",
                target_url
            );
        }
        let scheme = if tls { "https" } else { "http" };
        return Endpoint::from_shared(format!("{scheme}://{target_url}"))
            .map_err(|e| failed_to_connect(target_url, error_chain(&e)));
    }

    if !proxy.starts_with("unix:") {
        return Err(io_exception(format!("Remote proxy unsupported: {proxy}")));
    }

    // return newUnixNettyChannelBuilder(proxy).overrideAuthority(targetUrl);
    new_unix_netty_channel_builder(proxy)
}

/// `MoreCallCredentials.from(creds)`: [`Credentials`] as gRPC call credentials.
///
/// The headers are attached to every call made with these call credentials, on a plaintext
/// channel too: Java's `GoogleAuthLibraryCallCredentials` requires a channel with
/// `SecurityLevel.PRIVACY_AND_INTEGRITY` only for `GoogleCredentials` (`requirePrivacy`), and no
/// `GoogleCredentials` exist in this port ([`new_google_credentials`] is not ported).
// Java: io/grpc/auth/GoogleAuthLibraryCallCredentials.java GoogleAuthLibraryCallCredentials
pub struct GoogleAuthLibraryCallCredentials {
    creds: Arc<dyn Credentials>,
}

impl GoogleAuthLibraryCallCredentials {
    /// `MoreCallCredentials.from(creds)`.
    // Java: io/grpc/auth/MoreCallCredentials.java MoreCallCredentials.from
    pub fn new(creds: Arc<dyn Credentials>) -> GoogleAuthLibraryCallCredentials {
        GoogleAuthLibraryCallCredentials { creds }
    }
}

#[async_trait]
impl CallCredentials for GoogleAuthLibraryCallCredentials {
    // Java: io/grpc/auth/GoogleAuthLibraryCallCredentials.java GoogleAuthLibraryCallCredentials.applyRequestMetadata
    async fn get_request_metadata(&self, uri: &str) -> io::Result<Vec<(String, Vec<String>)>> {
        let uri =
            JavaUri::create(uri).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        // Credentials is expected to manage caching internally if the metadata is fetched over
        // the network.
        let metadata: Headers = self
            .creds
            .get_request_metadata(&uri)
            .map_err(io::Error::other)?;
        Ok(metadata.into_iter().collect())
    }
}

/// Create a new [`CallCredentials`] object from the authentication flags, or `None` if no flags
/// are set.
///
/// Fails in case the credentials can't be constructed.
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:262 GoogleAuthUtils.newGoogleCallCredentials
pub fn new_google_call_credentials(
    options: &AuthAndTLSOptions,
) -> io::Result<Option<Arc<dyn CallCredentials>>> {
    let creds = new_google_credentials(options)?;
    Ok(creds.map(|creds| -> Arc<dyn CallCredentials> {
        Arc::new(GoogleAuthLibraryCallCredentials::new(creds))
    }))
}

/// Create a new [`CallCredentialsProvider`] object from [`Credentials`] or return
/// [`NO_CREDENTIALS`] if it is `None`.
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:275 GoogleAuthUtils.newCallCredentialsProvider
pub fn new_call_credentials_provider(
    creds: Option<Arc<dyn Credentials>>,
) -> Arc<dyn CallCredentialsProvider> {
    if let Some(creds) = creds {
        return Arc::new(GoogleAuthCallCredentialsProvider::new(creds));
    }
    NO_CREDENTIALS.clone()
}

/// Environment for running credential helpers in.
///
/// PORT: the Java record lives in `lib/authandtls/credentialhelper`, which is not ported; this
/// is the record itself, for the fields `GoogleAuthUtils` reads.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperEnvironment.java:39 CredentialHelperEnvironment
pub struct CredentialHelperEnvironment<'a> {
    /// The reporter for reporting events related to credential helpers.
    pub event_reporter: &'a dyn EventHandler,
    /// The absolute path to the workspace, or `None` if Bazel was invoked outside a workspace.
    pub workspace_path: Option<Path>,
    /// The environment from the Bazel client.
    pub client_environment: IndexMap<String, String>,
    /// The execution timeout for the helper subprocess.
    pub helper_execution_timeout: Duration,
}

/// A provider for credential helpers.
///
/// PORT: only the provider without helpers exists: [`new_credential_helper_provider`] cannot
/// resolve the path of a helper (`CommandLinePathFactory` is not ported) and there is no
/// credential helper runner (`CredentialHelper.getCredentials`), so a configured helper fails
/// loudly where its path would be resolved.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:37 CredentialHelperProvider
#[derive(Debug)]
pub struct CredentialHelperProvider {
    _no_helpers: (),
}

/// `CommandLinePathFactory.create(env, value)`.
fn command_line_path_factory_create(helper: &CredentialHelperOption) -> ! {
    blaze_util::unported!(
        "CommandLinePathFactory.create (missing hook: blaze_runtime CommandLinePathFactory; CredentialHelper.getCredentials, the credential helper runner, is not ported either)",
        "--credential_helper={}",
        match &helper.scope {
            Some(scope) => format!("{scope}={}", helper.path),
            None => helper.path.clone(),
        }
    )
}

/// `GoogleAuthUtils.newCredentialHelperProvider(environment, pathFactory, helpers)`.
///
/// PORT: the `CommandLinePathFactory` parameter is absent (see [`CredentialHelperProvider`]).
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:416 GoogleAuthUtils.newCredentialHelperProvider
pub fn new_credential_helper_provider(
    _environment: &CredentialHelperEnvironment<'_>,
    helpers: &[CredentialHelperOption],
) -> io::Result<CredentialHelperProvider> {
    // CredentialHelperProvider.Builder builder = CredentialHelperProvider.builder();
    if let Some(helper) = helpers.first() {
        // Path path = pathFactory.create(environment.clientEnvironment(), helper.path());
        command_line_path_factory_create(helper);
    }
    Ok(CredentialHelperProvider { _no_helpers: () })
}

/// Implementation of [`Credentials`] which fetches credentials by invoking a credential helper
/// as subprocess, falling back to another [`Credentials`] if no suitable helper exists.
///
/// PORT: the provider never has a helper (see [`CredentialHelperProvider`]), so the credential
/// cache (`Cache<URI, GetCredentialsResponse>`) would stay empty and is not kept.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:33 CredentialHelperCredentials
struct CredentialHelperCredentials {
    _credential_helper_provider: CredentialHelperProvider,
    fallback_credentials: Option<Arc<dyn Credentials>>,
}

impl blaze_repo_fetch::authandtls::Credentials for CredentialHelperCredentials {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:60 CredentialHelperCredentials.getRequestMetadata
    fn get_request_metadata(&self, uri: &JavaUri) -> Result<Headers, IoException> {
        // response = credentialCache.get(uri, this::getCredentialsFromHelper);
        // `credentialHelperProvider.findCredentialHelper(uri)` is empty, so `response` is null.

        if let Some(fallback_credentials) = &self.fallback_credentials {
            return fallback_credentials.get_request_metadata(uri);
        }

        Ok(Headers::new())
    }
}

impl Credentials for CredentialHelperCredentials {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:114 CredentialHelperCredentials.refresh
    fn refresh(&self) -> io::Result<()> {
        if let Some(fallback_credentials) = &self.fallback_credentials {
            fallback_credentials.refresh()?;
        }

        // credentialCache.invalidateAll(): nothing is cached.
        Ok(())
    }
}

/// Create a new [`Credentials`] retrieving call credentials in the following order:
///
/// 1. If a Credential Helper is configured for the scope, use the credentials provided by the
///    helper.
/// 2. If (Google) authentication is enabled by flags, use it to create credentials.
/// 3. Use `.netrc` to provide credentials if exists.
///
/// Fails in case the credentials can't be constructed.
///
/// PORT: the `credentialCache` and `commandLinePathFactory` parameters are absent (see
/// [`CredentialHelperProvider`]).
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:294 GoogleAuthUtils.newCredentials
pub fn new_credentials(
    credential_helper_environment: &CredentialHelperEnvironment<'_>,
    file_system: &FileSystemRef,
    auth_and_tls_options: &AuthAndTLSOptions,
) -> io::Result<Arc<dyn Credentials>> {
    let mut fallback_credentials = new_google_credentials(auth_and_tls_options)?;

    if fallback_credentials.is_none() {
        // Fallback to .netrc if it exists.
        match new_credentials_from_netrc(
            &credential_helper_environment.client_environment,
            file_system,
        ) {
            Ok(credentials) => fallback_credentials = credentials,
            Err(e) => {
                // TODO(yannic): Make this fail the build.
                credential_helper_environment
                    .event_reporter
                    .handle(&Event::warn(e.to_string()));
            }
        }
    }

    Ok(Arc::new(CredentialHelperCredentials {
        _credential_helper_provider: new_credential_helper_provider(
            credential_helper_environment,
            &auth_and_tls_options.credential_helpers,
        )?,
        fallback_credentials,
    }))
}

/// Create a new [`Credentials`] object from the authentication flags, or `None` if no flags are
/// set.
///
/// Fails in case the credentials can't be constructed.
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:335 GoogleAuthUtils.newGoogleCredentials
fn new_google_credentials(options: &AuthAndTLSOptions) -> io::Result<Option<Arc<dyn Credentials>>> {
    if options.google_credentials.is_some() {
        // Credentials from file
        blaze_util::unported!(
            "GoogleAuthUtils.newGoogleCredentials (--google_default_credentials/--google_credentials)"
        );
    } else if options.use_google_default_credentials {
        // Google Application Default Credentials
        blaze_util::unported!(
            "GoogleAuthUtils.newGoogleCredentials (--google_default_credentials/--google_credentials)"
        );
    }
    Ok(None)
}

/// Create a new [`Credentials`] object by parsing the .netrc file with following order to search
/// it:
///
/// 1. If environment variable $NETRC exists, use it as the path to the .netrc file
/// 2. Fallback to $HOME/.netrc
///
/// Returns the [`Credentials`] object or `None` if there is no .netrc file.
///
/// Fails in case the credentials can't be constructed.
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:393 GoogleAuthUtils.newCredentialsFromNetrc
pub fn new_credentials_from_netrc(
    client_env: &IndexMap<String, String>,
    file_system: &FileSystemRef,
) -> io::Result<Option<Arc<dyn Credentials>>> {
    let netrc_file_string = match client_env.get("NETRC") {
        Some(netrc) => netrc.clone(),
        None => match client_env.get("HOME") {
            Some(home) => format!("{home}/.netrc"),
            None => return Ok(None),
        },
    };

    let netrc_file = Path::create_from_str(&netrc_file_string, file_system);
    if !netrc_file.exists() {
        return Ok(None);
    }

    let netrc = netrc_file
        .get_input_stream()
        .map_err(IoException::from)
        .and_then(netrc_parser::parse_and_close);
    match netrc {
        Ok(netrc) => Ok(Some(Arc::new(NetrcCredentials::new(netrc)))),
        Err(e) => Err(io_exception(format!(
            "Failed to parse {}: {}",
            netrc_file.get_path_string(),
            e.get_message()
        ))),
    }
}
