//! Port of `authandtls/AuthAndTLSOptions.java` (the `OptionsBase` subclass and what it uses).

use std::time::Duration;

use blaze_options::{
    CommaSeparatedOptionListConverter, ConversionContext, Converter, DurationConverter,
    OptionScalar, OptionsParsingException, Value, converter::non_null, option_field_type, options,
};

// Java: com/google/devtools/build/lib/authandtls/AuthAndTLSOptions.java:37 AuthAndTLSOptions
options! {
    /// Common options for authentication and TLS.
    pub struct AuthAndTLSOptions {
        #[option(
            name = "google_default_credentials",
            old_name = "auth_enabled",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Whether to use 'Google Application Default Credentials' for authentication.\n",
                "See [Authentication methods at Google - Google Cloud][gc-auth-methods] for details.\n",
                "Disabled by default.\n",
                "\n",
                "[gc-auth-methods]: https://cloud.google.com/docs/authentication\n",
            ),
        )]
        pub use_google_default_credentials: bool,

        #[option(
            name = "google_auth_scopes",
            old_name = "auth_scope",
            default_value = "https://www.googleapis.com/auth/cloud-platform",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "A comma-separated list of Google Cloud authentication scopes.",
        )]
        pub google_auth_scopes: Vec<String>,

        #[option(
            name = "google_credentials",
            old_name = "auth_credentials",
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies the file to get authentication credentials from. See ",
                "https://cloud.google.com/docs/authentication for details.",
            ),
        )]
        pub google_credentials: Option<String>,

        #[option(
            name = "tls_certificate",
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Specify a path to a TLS certificate that is trusted to sign server certificates.",
        )]
        pub tls_certificate: Option<String>,

        #[option(
            name = "tls_client_certificate",
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specify the TLS client certificate to use; you also need to provide a client key to ",
                "enable client authentication.",
            ),
        )]
        pub tls_client_certificate: Option<String>,

        #[option(
            name = "tls_client_key",
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specify the TLS client key to use; you also need to provide a client certificate to ",
                "enable client authentication.",
            ),
        )]
        pub tls_client_key: Option<String>,

        #[option(
            name = "tls_authority_override",
            default_value = "null",
            metadata_tags = [HIDDEN],
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "TESTING ONLY! Can be used with a self-signed certificate to consider the specified ",
                "value a valid TLS authority.",
            ),
        )]
        pub tls_authority_override: Option<String>,

        #[option(
            name = "grpc_keepalive_time",
            default_value = "60s",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Configures keep-alive pings for outgoing gRPC connections. If this is set, then Bazel\n",
                "sends pings after this much time of no read operations on the connection, but\n",
                "only if there is at least one pending gRPC call. The value 0 disables the keep-alives.\n",
            ),
        )]
        pub grpc_keepalive_time: Duration,

        #[option(
            name = "grpc_keepalive_timeout",
            default_value = "20s",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Configures a keep-alive timeout for outgoing gRPC connections. If keep-alive pings are\n",
                "enabled with `--grpc_keepalive_time`, then Bazel times out a connection if it does\n",
                "not receive a ping reply after this much time. Times are treated as second\n",
                "granularity; it is an error to set a value less than one second. If keep-alive\n",
                "pings are disabled, then this setting is ignored.\n",
            ),
        )]
        pub grpc_keepalive_timeout: Duration,

        #[option(
            name = "grpc_tcp_keepalive",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Whether to enable TCP keep-alive (the SO_KEEPALIVE socket option) for outgoing gRPC\n",
                "connections. This operates at the transport layer and is independent of the\n",
                "application-level keep-alive pings configured with `--grpc_keepalive_time`. When enabled,\n",
                "the keep-alive behavior is controlled by `--grpc_tcp_keepalive_time`,\n",
                "`--grpc_tcp_keepalive_interval` and `--grpc_tcp_keepalive_count`.\n",
            ),
        )]
        pub grpc_tcp_keepalive: bool,

        #[option(
            name = "grpc_tcp_keepalive_time",
            default_value = "60s",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "When `--grpc_tcp_keepalive` is enabled, the amount of idle time on a gRPC connection\n",
                "before the first TCP keep-alive probe is sent (TCP_KEEPIDLE). Has no effect if TCP\n",
                "keep-alive is disabled.\n",
            ),
        )]
        pub grpc_tcp_keepalive_time: Duration,

        #[option(
            name = "grpc_tcp_keepalive_interval",
            default_value = "10s",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "When `--grpc_tcp_keepalive` is enabled, the amount of time between successive TCP\n",
                "keep-alive probes (TCP_KEEPINTVL). Has no effect if TCP keep-alive is disabled.\n",
            ),
        )]
        pub grpc_tcp_keepalive_interval: Duration,

        #[option(
            name = "grpc_tcp_keepalive_count",
            default_value = "3",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "When `--grpc_tcp_keepalive` is enabled, the number of unacknowledged TCP keep-alive probes\n",
                "to send before considering the connection dead (TCP_KEEPCNT). Has no effect if TCP\n",
                "keep-alive is disabled.\n",
            ),
        )]
        pub grpc_tcp_keepalive_count: i32,

        #[option(
            name = "credential_helper",
            old_name = "experimental_credential_helper",
            default_value = "null",
            allow_multiple = true,
            converter = CredentialHelperOptionConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Configures a credential helper conforming to the [Credential Helper Specification][ch-spec]\n",
                "to use for retrieving authorization credentials for repository\n",
                "fetching, remote caching and execution, and the build event service.\n",
                "\n",
                "Credentials supplied by a helper take precedence over credentials supplied by\n",
                "`--google_default_credentials`, `--google_credentials`, a `.netrc` file, or the\n",
                " auth parameter to `repository_ctx.download()` and\n",
                "`repository_ctx.download_and_extract()`.\n",
                "\n",
                "May be specified multiple times to set up multiple helpers.\n",
                "\n",
                "See [Configuring Bazel's Credential Helper - Engflow Blog][ch-example] for instructions.\n",
                "\n",
                "[ch-spec]: https://github.com/EngFlow/credential-helper-spec\n",
                "[ch-example]: https://blog.engflow.com/2023/10/09/configuring-bazels-credential-helper/\n",
            ),
        )]
        pub credential_helpers: Vec<CredentialHelperOption>,

        #[option(
            name = "credential_helper_timeout",
            old_name = "experimental_credential_helper_timeout",
            default_value = "10s",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Configures the timeout for a credential helper.\n",
                "\n",
                "Credential helpers failing to respond within this timeout will fail the invocation.\n",
            ),
        )]
        pub credential_helper_timeout: Duration,

        #[option(
            name = "credential_helper_cache_duration",
            old_name = "experimental_credential_helper_cache_duration",
            default_value = "30m",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "How long to cache credentials for if the credential helper doesn't return an expiration",
                " time. Changing the value of this flag clears the cache.",
            ),
        )]
        pub credential_helper_cache_timeout: Duration,
    }
}

// Java: com/google/devtools/build/lib/authandtls/AuthAndTLSOptions.java:264 AuthAndTLSOptions.CredentialHelperOption
/// One of the values of the `--credential_helper` flag.
///
/// `scope`: the scope of the credential helper (if any). The scope is a valid ASCII domain name
/// with an optional leading '.*' wildcard. `path`: the (unparsed) path of the credential helper.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialHelperOption {
    pub scope: Option<String>,
    pub path: String,
}

impl OptionScalar for CredentialHelperOption {
    // Java record toString: CredentialHelperOption[scope=Optional[x], path=y].
    fn java_to_string(&self) -> String {
        format!(
            "CredentialHelperOption[scope={}, path={}]",
            match &self.scope {
                Some(s) => format!("Optional[{}]", s),
                None => "Optional.empty".to_string(),
            },
            self.path
        )
    }
}

option_field_type!(
    CredentialHelperOption,
    "com.google.devtools.build.lib.authandtls.AuthAndTLSOptions$CredentialHelperOption",
    raw = CredentialHelperOption {
        scope: None,
        path: String::new()
    }
);

// Java: com/google/devtools/build/lib/authandtls/AuthAndTLSOptions.java:273 AuthAndTLSOptions.CredentialHelperOptionConverter
/// A `Converter` for the `--credential_helper` flag.
#[derive(Clone, Copy, Debug, Default)]
pub struct CredentialHelperOptionConverter;

impl CredentialHelperOptionConverter {
    // Java: com/google/devtools/build/lib/authandtls/AuthAndTLSOptions.java:288 AuthAndTLSOptions.CredentialHelperOptionConverter.convert
    pub fn convert_option(
        &self,
        input: &str,
    ) -> Result<CredentialHelperOption, OptionsParsingException> {
        if let Some(pos) = input.find('=') {
            let scope = Self::parse_scope(&input[..pos])?;
            let path = Self::parse_path(&input[pos + 1..])?;
            return Ok(CredentialHelperOption {
                scope: Some(scope),
                path,
            });
        }

        // `input` does not specify a scope.
        Ok(CredentialHelperOption {
            scope: None,
            path: Self::parse_path(input)?,
        })
    }

    // Java: com/google/devtools/build/lib/authandtls/AuthAndTLSOptions.java:302 AuthAndTLSOptions.CredentialHelperOptionConverter.parseScope
    fn parse_scope(scope: &str) -> Result<String, OptionsParsingException> {
        if scope.is_empty() {
            return Err(OptionsParsingException::new(
                "Credential helper scope must not be empty",
            ));
        }
        let mut wildcard = "";
        let mut domain_name = scope;
        // Java: scope.length() > 2 counts UTF-16 units; "*." is two ASCII bytes either way.
        if scope.starts_with("*.") && scope.chars().count() > 2 {
            wildcard = "*.";
            domain_name = &scope[2..];
        }
        // Check that the domain name is either ASCII and conforming to RFC 1122 and RFC 1123,
        // or non-ASCII and conforming to RFC 3490. In the latter case, convert it to Punycode.
        // See https://en.wikipedia.org/wiki/Punycode.
        match idn_to_ascii_std3(domain_name) {
            Ok(ascii) => Ok(format!("{}{}", wildcard, ascii)),
            Err(cause) => Err(OptionsParsingException::with_cause(
                format!(
                    "Credential helper scope '{}' must be a valid domain name with an optional \
                     leading '*.' wildcard",
                    scope
                ),
                cause,
            )),
        }
    }

    // Java: com/google/devtools/build/lib/authandtls/AuthAndTLSOptions.java:326 AuthAndTLSOptions.CredentialHelperOptionConverter.parsePath
    fn parse_path(path: &str) -> Result<String, OptionsParsingException> {
        if path.is_empty() {
            return Err(OptionsParsingException::new(
                "Credential helper path must not be empty",
            ));
        }
        Ok(path.to_string())
    }
}

impl Converter for CredentialHelperOptionConverter {
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        // Preconditions.checkNotNull(input)
        Ok(Some(Value::new(self.convert_option(non_null(input))?)))
    }

    // Java: com/google/devtools/build/lib/authandtls/AuthAndTLSOptions.java:279 AuthAndTLSOptions.CredentialHelperOptionConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        concat!(
            "Path to a credential helper. It may be absolute, relative to the PATH environment",
            " variable, or %workspace%-relative. The path be optionally prefixed by a scope ",
            " followed by an '='. The scope is a domain name, optionally with a single leading '*'",
            " wildcard component. A helper applies to URIs matching its scope, with more specific",
            " scopes preferred. If a helper has no scope, it applies to every URI.",
        )
        .into()
    }
}

/// `java.net.IDN.toASCII(input, IDN.USE_STD3_ASCII_RULES)`. The error is the message of Java's
/// `IllegalArgumentException`.
///
/// PORT: only ASCII labels are ported; a non-ASCII label needs nameprep (RFC 3491) and Punycode
/// (RFC 3492) and fails loudly.
fn idn_to_ascii_std3(input: &str) -> Result<String, String> {
    // Java: IDN.isRootLabel
    if matches!(input, "." | "\u{3002}" | "\u{FF0E}" | "\u{FF61}") {
        return Ok(".".to_string());
    }
    let is_dot = |c: char| matches!(c, '.' | '\u{3002}' | '\u{FF0E}' | '\u{FF61}');
    let mut out = String::new();
    let mut rest = input;
    while !rest.is_empty() {
        let (label, more) = match rest.find(is_dot) {
            Some(q) => {
                let dot_len = rest[q..].chars().next().unwrap().len_utf8();
                (&rest[..q], Some(&rest[q + dot_len..]))
            }
            None => (rest, None),
        };
        out.push_str(&idn_to_ascii_label(label)?);
        match more {
            // has more labels, or keep the trailing dot as at present
            Some(m) => {
                out.push('.');
                rest = m;
            }
            None => rest = "",
        }
    }
    Ok(out)
}

// Java: java/net/IDN.java IDN.toASCIIInternal (USE_STD3_ASCII_RULES set)
fn idn_to_ascii_label(label: &str) -> Result<String, String> {
    if !label.is_ascii() {
        blaze_util::unported!("IDN.toASCII (non-ASCII domain names: nameprep and Punycode)");
    }
    if label.is_empty() {
        return Err("Empty label is not a legal name".into());
    }
    // isNonLDHAsciiCodePoint
    if label
        .bytes()
        .any(|c| !(c.is_ascii_alphanumeric() || c == b'-'))
    {
        return Err("Contains non-LDH ASCII characters".into());
    }
    if label.starts_with('-') || label.ends_with('-') {
        return Err("Has leading or trailing hyphen".into());
    }
    if label.len() > 63 {
        return Err("The label in the input is too long".into());
    }
    Ok(label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<AuthAndTLSOptions>(&[
            "google_default_credentials",
            "google_auth_scopes",
            "google_credentials",
            "tls_certificate",
            "tls_client_certificate",
            "tls_client_key",
            "tls_authority_override",
            "grpc_keepalive_time",
            "grpc_keepalive_timeout",
            "grpc_tcp_keepalive",
            "grpc_tcp_keepalive_time",
            "grpc_tcp_keepalive_interval",
            "grpc_tcp_keepalive_count",
            "credential_helper",
            "credential_helper_timeout",
            "credential_helper_cache_duration",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<AuthAndTLSOptions>(&[
            "--credential_helper=/bin/helper",
            "--credential_helper=*.example.com=%workspace%/h",
            "--google_auth_scopes=a,b",
            "--grpc_keepalive_time=0",
        ])
        .unwrap();
        assert_eq!(
            o.credential_helpers,
            vec![
                CredentialHelperOption {
                    scope: None,
                    path: "/bin/helper".into()
                },
                CredentialHelperOption {
                    scope: Some("*.example.com".into()),
                    path: "%workspace%/h".into()
                },
            ]
        );
        assert_eq!(o.google_auth_scopes, vec!["a", "b"]);
        assert_eq!(o.grpc_keepalive_time, Duration::ZERO);
        assert_eq!(
            AuthAndTLSOptions::default().google_auth_scopes,
            vec!["https://www.googleapis.com/auth/cloud-platform"]
        );
    }

    #[test]
    fn credential_helper_errors() {
        let c = CredentialHelperOptionConverter;
        assert_eq!(
            c.convert_option("=x").unwrap_err().get_message(),
            "Credential helper scope must not be empty"
        );
        assert_eq!(
            c.convert_option("a.com=").unwrap_err().get_message(),
            "Credential helper path must not be empty"
        );
        assert_eq!(
            c.convert_option("a_b.com=x").unwrap_err().get_message(),
            "Credential helper scope 'a_b.com' must be a valid domain name with an optional \
             leading '*.' wildcard"
        );
        assert!(c.convert_option("-a.com=x").is_err());
        assert!(c.convert_option("a..com=x").is_err());
        assert_eq!(
            c.convert_option("a.com.=x").unwrap().scope.as_deref(),
            Some("a.com.")
        );
        assert_eq!(
            c.convert_option("Foo.COM=x").unwrap().java_to_string(),
            "CredentialHelperOption[scope=Optional[Foo.COM], path=x]"
        );
    }

    // Java tests: src/test/java/com/google/devtools/build/lib/authandtls/CredentialHelperOptionConverterTest.java

    fn convert(s: &str) -> Result<CredentialHelperOption, OptionsParsingException> {
        CredentialHelperOptionConverter.convert_option(s)
    }

    #[test]
    fn exact_scope() {
        let helper1 = convert("example.com=foo").unwrap();
        assert_eq!(helper1.scope.as_deref(), Some("example.com"));
        assert_eq!(helper1.path, "foo");
    }

    #[test]
    fn wildcard_scope() {
        let helper1 = convert("*.example.com=foo").unwrap();
        assert_eq!(helper1.scope.as_deref(), Some("*.example.com"));
        assert_eq!(helper1.path, "foo");
    }

    #[test]
    #[ignore = "needs IDN.toASCII for non-ASCII labels (nameprep and Punycode)"]
    fn punycode_scope() {
        let helper1 = convert("münchen.de=foo").unwrap();
        assert_eq!(helper1.scope.as_deref(), Some("xn--mnchen-3ya.de"));
        assert_eq!(helper1.path, "foo");

        let helper2 = convert("*.köln.de=foo").unwrap();
        assert_eq!(helper2.scope.as_deref(), Some("*.xn--kln-sna.de"));
        assert_eq!(helper2.path, "foo");
    }

    #[test]
    fn absolute_path() {
        let helper1 = convert("/absolute/path").unwrap();
        assert_eq!(helper1.scope, None);
        assert_eq!(helper1.path, "/absolute/path");
    }

    #[test]
    fn root_relative_path() {
        let helper1 = convert("%workspace%/path").unwrap();
        assert_eq!(helper1.scope, None);
        assert_eq!(helper1.path, "%workspace%/path");
    }

    #[test]
    fn path_lookup() {
        let helper1 = convert("foo").unwrap();
        assert_eq!(helper1.scope, None);
        assert_eq!(helper1.path, "foo");
    }

    #[test]
    fn empty_option() {
        let t = convert("").unwrap_err();
        assert!(
            t.get_message()
                .contains("Credential helper path must not be empty")
        );
    }

    #[test]
    fn empty_scope() {
        let t = convert("=/foo").unwrap_err();
        assert!(
            t.get_message()
                .contains("Credential helper scope must not be empty")
        );
    }

    #[test]
    fn empty_path() {
        let t = convert("foo=").unwrap_err();
        assert!(
            t.get_message()
                .contains("Credential helper path must not be empty")
        );
    }

    #[test]
    fn empty_scope_and_path() {
        let t = convert("=").unwrap_err();
        assert!(
            t.get_message()
                .contains("Credential helper scope must not be empty")
        );
    }

    #[test]
    fn invalid_scope() {
        for scope in [
            "-example.com",
            "example-.com",
            "example!.com",
            "*.",
            "example.*",
            "*.example.*",
            "foo.*.example.com",
            "*.foo.*.example.com",
            "*-foo.example.com",
            ".*.example.com",
            "foo.*.münchen.de",
            ".*.münchen.de",
            "*-foo.münchen.de",
        ] {
            let t = convert(&format!("{}=foo", scope)).unwrap_err();
            assert!(
                t.get_message().contains(&format!(
                    "Credential helper scope '{}' must be a valid domain name with an optional \
                     leading '*.' wildcard",
                    scope
                )),
                "{}",
                scope
            );
        }
    }
}
