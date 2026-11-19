# Response: Build without the Bytes review (STOP on e7451bb)

- Responding to `docs/reviews/2026-11-19-remote-bwob.md`. The fixes are uncommitted on top of
  `e7451bb`, together with the test.xml fix found on the real RBE (described in `docs/remote.md`,
  "Fix after the RBE run").
- Oracle: `~/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64`; Blaze: `target/debug/blaze` of the
  fixed tree. Both run through `scripts/remote/bwob-matrix` (one fresh worker per tool and
  scenario, `--batch --nosystem_rc --nohome_rc`, one `--output_base` per tool; steps: cold, warm
  after `clean`, rebuild without `clean`), compared by `scripts/remote/bwob-compare` (exit code,
  summary lines, set and sha256 of the files under `bazel-out`, JSON exec log entries without
  `metrics`, `compare-grpc-log`). Workspace `~/.cache/blaze/bench/remote-ws-bwob`; runs in
  `~/.cache/blaze/bench/bwob-runs/m7`.

## B1 (S1) remote tree child dropped: fixed

- `construct_tree_artifact_value_from_filesystem` compares the parent-relative path of each
  listed child with the remote entry's for equality (Java merges per directory by name,
  `RemoteActionFileSystem.java:835`).
- Bench: `//:dup_tree` writes `x.txt` and `sub/x.txt`; `//:dup_consumer` (remote) and
  `//:dup_consumer_local` (`no-remote`) list and concatenate every file of it.
- `build //:dup_consumer //:dup_consumer_local --remote_executor=grpc://localhost:18981
  --remote_download_outputs=minimal --remote_download_regex=.*/sub/.*`: MATCH in all three steps
  (files 10/10, exec log 3/3, gRPC log MATCH).
- Reproduction with the old `ends_with` comparison (same scenario, cold): `dup_tree_dir/x.txt`
  missing from Blaze's tree, `dup_consumer_local.txt` differs, 1 of 3 exec log entries identical,
  gRPC log DIFFERENT (the consumer's input root lacks `x.txt`).
- `prefetchFiles_downloadRemoteTrees_partial` ported (passes; the test helper now builds local
  children from a digest without writing them, as Java's does).

## B2 (S1) tree artifact that is a symlink to a remote tree: fixed

- The tree is listed through its canonical path (local symlinks resolved one link at a time):
  local children of the target directory plus the remote output tree's entries under the target,
  merged by parent-relative path; the root counts as a directory when either exists or the
  canonical path is an input tree of the action. When the root is a symlink and the canonical
  path is an input of the action (Java's `FileStatusWithMetadata` stat), the value gets the
  resolved path (`metadata.getResolvedPath()` or the canonical path,
  `ActionOutputMetadataStore.java:300-312`).
- `RemoteActionFileSystem` (`remote_output_tree.rs`) canonicalizes every component through local
  symlinks (`resolveSymbolicLinks`, `:326`) before looking up remote files and directories, so
  `stat` and `readdir` of a symlinked directory see the target's remote entries.
- Bench: `//:sym_tree` is `ctx.actions.symlink(output = declare_directory(...), target_file =
  //:tree_for_sym)`; `//:sym_tree_consumer` (remote) and `//:sym_tree_consumer_local`
  (`no-remote`) list and concatenate every file.
- MATCH in all three steps for: `--remote_executor --remote_download_outputs=minimal` (files
  11/11); `--remote_cache --remote_download_outputs=minimal`, whose warm step is the remote cache
  hit case (files 12/12, 11/11, 11/11); `build //:sym_tree //:sym_tree_consumer --remote_executor
  --remote_download_outputs=toplevel` (files 11/11); and all five new targets with
  `--remote_download_outputs=all`, remote exec and remote cache (files 18/18).
- Following Java, a symlinked tree whose target is an input now carries the resolved path with
  any cache, `all` included; the `all` rows above match the oracle.
- `prefetchFiles_downloadRemoteTrees_withResolvedPath` ported (passes).
- Not reproduced before the fix (the reviewer's trace is the evidence); the old code had no path
  from the symlink to the target's remote entries.

## Non-blocking

- N1: documented in `docs/remote.md` (Phase 2, "A lost remote blob is a sticky, loud failure"):
  the failure repeats until `blaze clean` or one build with `--remote_download_outputs=all`.
  Recovery without rewinding is not ported (open).
- N2: fixed. `do_download_file` first does the part of `CombinedCache.downloadFile` that Java runs
  before returning the future (create the temporary file's directory, write an empty file) and
  returns its errors as they are; an interruption is returned as it is; only other errors of the
  download become `CacheNotFoundException`. The Java comment is carried over.
- N3: comment fixed (`abstract_action_input_prefetcher.rs`) and the gap described in
  `docs/remote.md`: Java passes the `ActionOutputDirectoryHelper` for every build and replaces a
  file or symlink found where a directory is needed; Blaze fails with an I/O error there (open).
- N4: `..._partial` and `..._withResolvedPath` ported; `..._forActionTemplateExpansion` and
  `prefetchFiles_ignoreNonRemoteFiles_tree` remain ignored (open).
- The other non-blocking findings stand as written (open).

## Tests

`scripts/cargo-capped test -p <crate> --no-fail-fast` on the fixed tree: blaze_remote 445 passed,
0 failed, 112 ignored (`remote_action_input_fetcher_test`: 14 passed, 17 ignored); blaze_sky_exec
42/0/11; blaze_exec 399/0/73. `clippy --no-deps -p blaze_remote -p blaze_exec -p blaze_sky_exec
-p blaze_buildtool -p blaze_runtime --all-targets -- -D warnings` clean.
