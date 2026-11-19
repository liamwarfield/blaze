# Proxy authentication: oracle recordings

Recorded 2026-11-19 with the oracle (Bazel 9.3.2, embedded JDK Zulu 25.0.2) by
`scripts/proxy/run oracle <dir> <scenario.txt fields>`: a repository rule downloads the URL(s)
through `scripts/proxy/fake-proxy`, which answers the n-th request with the n-th response of the
list (the last repeats) and writes each request head byte for byte. Every run is a fresh server.
Tests: `tests/proxy_authentication_test.rs` (one per directory).

Each directory:

- `scenario.txt`: `<url>[,<url>] <VAR>=<proxy url, PORT = the fake proxy's port> <responses>`
  (`scripts/proxy/fake-proxy help` describes the responses).
- `NN-cC.req`: request NN (over the whole run) on connection C, as received. Only the first 45
  are kept; for scenarios Blaze stops at with `unported!` (Digest, NTLM, Negotiate) the first 3.
  The TLS ClientHello sent inside a tunnel (`NN-cC.tunnel`) is not kept.
- `events.txt`: the complete log (accept, request, response, close), so the total request
  count is there.
- `stderr.txt`: Bazel's `WARNING: Download ...` and `Error in download` lines (output base
  masked). `exit-code`: Bazel's (`query @r//:all`).

The https downloads cannot succeed (the proxy ends the tunnel after the ClientHello); what
counts is what reaches the proxy. HttpConnector retries an https failure: 8 connection objects
per run. A plain-http 407 is not retried ("GET returned 407 ...", UnrecoverableHttpException).

`https-407basic-after-direct` comes from the first version of the harness, where the registry
files were fetched directly (`NO_PROXY=bcr.bazel.build`, no repository cache) before the proxied
download: that direct fetch made the first `HttpURLConnection` before ProxyHelper cleared
`jdk.http.auth.tunneling.disabledSchemes`, so Basic stays disabled for tunnels
(`-Xlog:class+init`: `HttpURLConnection` initialised at 1.83 s, ProxyHelper's `Authenticator`
at 4.88 s). In all other directories the proxied download is the server's first HTTP
connection. With `--host_jvm_args=-Djdk.http.auth.tunneling.disabledSchemes=` the same
scenario answers Basic (not kept; Blaze has no JVM options).

## What the oracle does

| scenario | requests | credentials on the wire | result |
| --- | --- | --- | --- |
| https-200-creds | 1 per attempt | none (no challenge) | tunnel, TLS fails |
| https-407basic-ok / -close | 2, then 1 per attempt | 1st none; after the 407 `Proxy-authorization: Basic` on a new connection; later CONNECTs send it first (cache) | tunnel |
| https-407basic-wrong | 20 per attempt | 1st none, 19 Basic | `Unable to tunnel through proxy. Proxy returns "HTTP/1.1 407 Proxy Authentication Required"` |
| https-407basic-nocreds | 1 per attempt | none | same error |
| https-407basic-after-direct | 1 per attempt | none (Basic disabled) | same error |
| https-cache-evict | 2; then 20; then 20 ... | cached Basic refused: dropped from the cache, Authenticator's Basic 19 times; next attempt starts with none | same error |
| https-enc-raw / -enc-pct | 2 | `Basic` of ISO-8859-1 of the Unicode string (`é中:pé` → `6T86cOk=`; `%C3%A9%40x:p%3Aw%25` → `é@x:p:w%` → `6UB4OnA6dyU=`) | tunnel |
| https-enc-raw-utf8 | 2 | `charset="UTF-8"`: UTF-8 (`w6nkuK06cMOp`) | tunnel |
| https-port | 2 | Basic; `Host: origin.example:8443` | tunnel |
| https-407digest (MD5) | 1 per attempt | none (MD5 disabled in the JDK) | error |
| https-407digest256 | 20 per attempt | Digest SHA-256 | error |
| https-407ntlm | NTLM handshake | NTLM type 1 | error |
| https-407negotiate | 1 per attempt | none (no Kerberos) | error |
| https-407multi (Negotiate, NTLM, Digest MD5, Basic) | 1 per attempt | none: Digest is chosen and cannot answer; no fall back | error |
| https-407digestmd5-basic | 1 per attempt | none: no fall back to Basic | error |
| https-407ntlm-basic | NTLM | NTLM, not Basic | tunnel |
| https-407ntlm-nocreds / -digest256-nocreds | 1 per attempt | none | error |
| http-200-creds | 1 | Bazel's own `Proxy-Authorization` (HttpConnector, the internal bytes) | 200 |
| http-200-nocreds | 1 | none | 200 |
| http-407basic-ok | 2 | Bazel's header; after the 407 the JDK's Basic replaces its value in place, new connection | 200 |
| http-407basic-wrong | 20 | as above | `GET returned 407 Proxy Authentication Required` |
| http-407basic-nocreds | 1 | none | same |
| http-407digest (MD5) / http-407multi | 1 | Bazel's Basic only | same |
| http-407digest256 | 2 | Digest SHA-256 replaces Bazel's | 200 |
| http-407ntlm | 39 | NTLM | 407 |
| http-enc-raw / -enc-pct | 1 | Bazel's: UTF-8 bytes (`w6nkuK06cMOp`, `w6lAeDpwOncl`) | 200 |
| http-enc-raw-407 | 2 | Bazel's `w6nkuK06cMOp`, then the JDK's `6T86cOk=` | 200 |
| http-enc-raw-407-twice | 3 | the second download starts with the cached `6T86cOk=` (and reuses the keep-alive connection) | 200, 200 |
| http-enc-raw-utf8 | 2 | Bazel's, then UTF-8 | 200 |
| http-cache-evict | 2 + 20 | second download: cached value refused, 19 more | 407 |
