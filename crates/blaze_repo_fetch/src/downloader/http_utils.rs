//! Port of `HttpUtils`.

use crate::errors::IoException;
use crate::uri::JavaUri;

/// Returns true if `uri` is supported by `HttpDownloader`.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:32 HttpUtils.isUrlSupportedByDownloader
pub fn is_url_supported_by_downloader(uri: &JavaUri) -> bool {
    is_http(uri) || is_protocol(uri, "file")
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:36 HttpUtils.isHttp
pub fn is_http(uri: &JavaUri) -> bool {
    is_protocol(uri, "http") || is_protocol(uri, "https")
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:40 HttpUtils.isProtocol
pub fn is_protocol(uri: &JavaUri, protocol: &str) -> bool {
    // An implementation should accept uppercase letters as equivalent to lowercase in scheme
    // names (e.g., allow "HTTP" as well as "http") for the sake of robustness.
    uri.get_scheme()
        .is_some_and(|s| s.eq_ignore_ascii_case(protocol))
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:46 HttpUtils.checkUrlsArgument
pub fn check_urls_argument(uris: &[JavaUri]) {
    if uris.is_empty() {
        panic!("IllegalArgumentException: urls list empty");
    }
    for uri in uris {
        if !is_url_supported_by_downloader(uri) {
            panic!("IllegalArgumentException: unsupported protocol: {}", uri);
        }
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:53 HttpUtils.getExtension
pub fn get_extension(path: &str) -> String {
    match path.rfind('.') {
        None => String::new(),
        Some(i) => path[i + 1..].to_ascii_lowercase(),
    }
}

/// `HttpUtils.getLocation`: the redirect target, merged with the request URL.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:61 HttpUtils.getLocation
pub fn get_location(
    location: Option<&str>,
    connection_url: &JavaUri,
) -> Result<JavaUri, IoException> {
    let Some(new_location) = location else {
        return Err(IoException::new("Remote redirect missing Location."));
    };
    let preferred = JavaUri::create(new_location).map_err(|e| {
        // URI.create throws IllegalArgumentException, which HttpConnector turns into an
        // UnrecoverableHttpException with its message.
        IoException::of(
            crate::errors::IoExceptionKind::UnrecoverableHttpException,
            e.message,
        )
    })?;
    let result = merge_urls(&preferred, connection_url)?;
    if !is_http(&result) {
        return Err(IoException::new(format!("Bad Location: {}", new_location)));
    }
    Ok(result)
}

// Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpUtils.java:73 HttpUtils.mergeUrls
pub fn merge_urls(preferred: &JavaUri, original: &JavaUri) -> Result<JavaUri, IoException> {
    // Try to short cut to preferred to preserve the original presentation of the quoting.
    if preferred.get_host().is_some()
        && preferred.get_scheme().is_some()
        && (preferred.get_fragment().is_some() || original.get_fragment().is_none())
        // Forward user info to the same origin.
        && (preferred.get_user_info().is_some()
            || original.get_user_info().is_none()
            || !(preferred.get_host() == original.get_host()
                && preferred.get_port() == original.get_port()))
    {
        return Ok(preferred.clone());
    }
    // If the Location value provided in a 3xx (Redirection) response does not have a fragment
    // component, a user agent MUST process the redirection as if the value inherits the
    // fragment component of the URI reference used to generate the request target.
    let protocol = preferred.get_scheme().or(original.get_scheme());
    let mut user_info = preferred.get_user_info();
    let host;
    let port;
    match preferred.get_host() {
        None => {
            host = original.get_host().map(str::to_string);
            port = original.get_port();
            user_info = original.get_user_info();
        }
        Some(h) => {
            host = Some(h.to_string());
            port = preferred.get_port();
            if user_info.is_none() && Some(h) == original.get_host() && port == original.get_port()
            {
                user_info = original.get_user_info();
            }
        }
    }
    let path = preferred.get_path();
    let query = preferred.get_query();
    let fragment = preferred.get_fragment().or_else(|| original.get_fragment());
    JavaUri::from_components(
        protocol,
        user_info.as_deref(),
        host.as_deref(),
        port,
        path.as_deref(),
        query.as_deref(),
        fragment.as_deref(),
    )
    .map_err(|_| IoException::new(format!("Could not merge {} into {}", preferred, original)))
}
