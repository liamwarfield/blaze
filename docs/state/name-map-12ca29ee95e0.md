# Name map 88dbfa6 -> 12ca29ee95e0

`name-map-12ca29ee95e0.tsv` maps 30 of the 31 names that passed in the baseline (88dbfa6) and are
absent at 12ca29ee95e0 to the passing test that now checks the same behaviour. Every target is
`pass` in `~/.cache/blaze/work/tests-12ca29ee95e0.json`.

- 14 `HttpUtilsTest` ports: `blaze_bzlmod::lib::registry_fetcher::http::tests::*` became
  `blaze_repo_fetch::test/http_utils_test::*` (same JUnit methods). The duplicate stack was
  removed in bzlmod-repos review S1-2.
- 12 `DownloadCacheTest` ports: `blaze_bzlmod::test/registry_fetcher_test::*` became
  `blaze_repo_fetch::test/download_cache_test::sha256::*`. The Java test is parameterized by key
  type. The old port covered SHA-256 only.
- `format_grouped_uses_commas` became `download_progress_event::tests::formats`: the same
  `%,d` helper. There is no JUnit method for it.
- `in_error_bubbling_is_always_false` became `in_error_bubbling_is_true_only_in_the_rerun_after_a_failure`.
  The old test asserted the defect fixed in skyframe review S1-3.
- `struct_with_list_and_dict_fields_is_hashable` became `struct_with_frozen_list_and_dict_fields_is_hashable`.
  It gives the same results on frozen structs. Hashing a struct with a mutable field was wrong
  (loading review S2-13).
- `internal_string_double_encodes` became `add_string_of_internal_string_is_javas_double_encoding`.
  It checks the same bytes, `c3 83 c2 a9`. The encoder was removed in actions review S1-1.

## No equivalent

- `blaze_bzlmod::lib::registry_fetcher::http::tests::java_split_comma_trims_around_separators`.
  This test checked that `ProxyHelper.createProxyIfNeeded` splits `no_proxy` the way Java's
  `noProxyUrl.split("\\s*,\\s*")` does (ProxyHelper.java:85). The separator absorbs the whitespace
  around it, the first entry keeps its leading whitespace, the last keeps its trailing
  whitespace, and trailing empty entries are dropped. For example, `" a"` stays `[" a"]`.
  The `blaze_repo_fetch` port (`downloader/proxy_helper.rs:164`) uses `split(',').map(trim)`
  instead, which also trims the ends. So `no_proxy=" example.com"` bypasses the proxy in Blaze,
  while in Bazel it does not. No `ProxyHelperTest` method covers the outer whitespace. The
  coverage was lost because the stack was deleted, not because of a review finding, and it
  exposes a small divergence that the `blaze_repo_fetch` owner should fix. Once fixed, a ported
  unit test can be mapped here.
