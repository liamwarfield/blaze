# Oracle-driven Rust tests that pass without their evidence (audit S2-9)

For the Rust owners. Each site below prints "skipped" (or `SKIP`, "absent", "not present") and
returns when a file outside the repository is missing, so `cargo test` counts the test as passed
without having compared anything. The audit (round 2B) counted 22 such sites in 13 test files;
the list here is what `scripts/gate/check-oracle-tests --list` finds at commit `12ec0c9`: 22
sites in 15 files, among them the two sandbox runner test files, whose evidence is the oracle's
install base (I did not have the audit's own list to reconcile the file count). Until each test fails instead of skipping (with one documented opt-out
variable, audit round 2B item D3), the gate runs `scripts/gate/check-oracle-tests` before
`scripts/gate/run-tests`: it exits non-zero when any path below is missing on the machine or a
new print-and-return site appears that is not in its table.

`~` is `$HOME`; `REC` is `~/.cache/blaze/oracle-records/oracle-9.3.2`; `INSTALL` is
`~/.cache/blaze/oracle-out/ur/install/58cf1ecbff441bac762b0050d6da21e9`. Line numbers are those at
`12ec0c9`; the tool matches sites by file and message, not by line.

| # | site (crates/...) | test | needs | on this machine (2026-11-18) |
|---|---|---|---|---|
| 1 | `blaze_actions/tests/oracle_action_keys.rs:546` | `oracle_action_keys_match` | `REC/{brotli,re2}/raw/aquery_deps_jsonproto.stdout`, `manifest.json` | present |
| 2 | `blaze_actions/tests/oracle_aquery_dump.rs:271` | `brotli_/re2_aquery_jsonproto_bytes_match_oracle` | same as 1 | present |
| 3 | `blaze_actions/tests/oracle_platform_probe.rs:54` | `empty_platform_fingerprint_matches_oracle_symlink_key` | `REC/brotli/raw/aquery_deps_jsonproto.stdout`, `manifest.json` | present |
| 4 | `blaze_actions/tests/oracle_platform_probe.rs:114` | `host_platform_fingerprint_matches_oracle_file_write_key` | same as 3 | present |
| 5 | `blaze_actions/tests/symlink_action_test.rs:319` | `oracle_symlink_keys` | same as 1 | present |
| 6 | `blaze_actions/tests/symlink_action_test.rs:362` | `oracle_solib_symlink_keys_and_paths` | same as 1 | present |
| 7 | `blaze_actions/tests/template_expansion_action_test.rs:483` | `oracle_re2_template_expansion_keys` | `REC/re2/raw/aquery_deps_jsonproto.stdout`, `manifest.json` | present |
| 8 | `blaze_bep/tests/oracle_bep_test.rs:180` | `replay_brotli`, `replay_re2` | `REC/{brotli,re2}/raw/bep.json`, `manifest.json` | present |
| 9 | `blaze_bep/tests/oracle_bep_test.rs:338` | `reconstruct_brotli`, `reconstruct_re2` | same as 8 | present |
| 10 | `blaze_client/tests/blaze_test.rs:224` | `oracle_cmdline_file_matches_recorded_list` | `~/.cache/blaze/oracle-out/ur-w1/73a93e10e881efba110a09db7263fe1d/server/cmdline` (absolute path hardcoded in the test) | **MISSING: this test passes today without comparing anything** |
| 11 | `blaze_exec/tests/analysis_actions_oracle_execution_test.rs:389` | `brotli_execution_matches_oracle` | `REC/brotli/raw/aquery_deps_jsonproto.stdout`, `manifest.json`, `items/outputs.json` | present |
| 12 | `blaze_exec/tests/analysis_actions_oracle_execution_test.rs:403` | `re2_execution_matches_oracle` | the same for re2 | present |
| 13 | `blaze_exec/tests/spawn_log_oracle_test.rs:29` | spawn log oracle tests | `REC/{brotli,re2}/raw/execlog.json` | present |
| 14 | `blaze_exec/tests/sandbox_linux_sandboxed_spawn_runner_test.rs:161` | linux-sandbox runner tests | `INSTALL/linux-sandbox` | present |
| 15 | `blaze_exec/tests/sandbox_linux_sandboxed_spawn_runner_test.rs:270` | linux-sandbox runner tests | `INSTALL/linux-sandbox`, and a machine that supports it | present |
| 16 | `blaze_exec/tests/sandbox_process_wrapper_sandboxed_spawn_runner_test.rs:158` | process-wrapper runner tests | `INSTALL/process-wrapper` | present |
| 17 | `blaze_repo_fetch/tests/decompressor_test.rs:272` | `http_bzl_documentation` | `~/.cache/blaze/src/bazel-9.3.2/tools/build_defs/repo/http.bzl` | present |
| 18 | `blaze_repo_fetch/tests/oracle_extraction_test.rs:118` | `platforms_tarball_from_shared_cache_matches_oracle_extraction` | `~/.cache/blaze/oracle-out/ur*/*/external/platforms/BUILD` | present |
| 19 | `blaze_repo_fetch/tests/oracle_extraction_test.rs:122` | same | `~/.cache/blaze/repo-cache/content_addressable/sha256/3384eb1c…19a8/file` | present |
| 20 | `starlark_conformance/tests/byte_strings.rs:377` | byte string cases | `~/.cache/blaze/work/bytes/third_party/telemetry/sha1.bzl` (from `scripts/bytes/run`) | present |
| 21 | `starlark_conformance/tests/byte_strings.rs:422` | byte string cases | same as 20 | present |
| 22 | `starlark_conformance/tests/script_test.rs:11` | `bazel_script_tests_on_starlark_rust` | `~/.cache/blaze/src/bazel-9.3.2/src/test/java/net/starlark/java/eval/testdata` | present |

What the Rust owners should do: make each site `panic!` (or `assert!`) with the path it needs,
unless one opt-out variable (for example `BLAZE_ALLOW_MISSING_ORACLE=1`, documented in
AGENTS.md) is set; site 10 should not depend on a server directory of an old output user root
at all (record the file under `~/.cache/blaze/oracle-records` or drop the test). When a site is
fixed, `check-oracle-tests` prints its table entry as STALE; remove the entry in the next
referee batch.
