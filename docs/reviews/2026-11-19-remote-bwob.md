# Review: remote phase 2, build without the bytes

- Commit reviewed: `e7451bb` against `3bb3adf` (`git show e7451bb:<path>`, `git diff 3bb3adf
  e7451bb -- <path>`). All `file:line` references are at `e7451bb`.
- Reviewer rules (AGENTS.md "Reviewers"): read-only, no Cargo, no build, no Bazel, no worker.
  Every finding below comes from reading the Rust against the pinned Java. Nothing was run, so
  the two blocking findings are stated as what the code does, not as observed output. The
  evidence table in `docs/remote.md` was read, not re-measured.
- Java paths are relative to `src/main/java/com/google/devtools/build/lib/` at the pin.

## Verdict

**STOP**, bound to `e7451bb`. Two blocking findings (B1, B2), both S1, both in the tree artifact
path of the build-wide remote output tree. Both produce a `TreeArtifactValue` with children
missing and no error. Neither is reachable from the bench workspace, which is why the evidence
table is all MATCH.

## Blocking findings

### B1 (S1) A remote tree child is dropped when a local child's path ends with its relative path

`crates/blaze_sky_exec/src/skyframe_action_executor.rs:1929-1941`
(`construct_tree_artifact_value_from_filesystem`). After the local `visit_tree`, the remote
output tree's entries under the root are merged in, skipping those already listed locally. The
test for "already listed" is

```rust
tree.iter().any(|(child, _)| child.get_exec_path().ends_with(&parent_relative_path))
```

`PathFragment::ends_with` is a segment suffix match (`crates/blaze_vfs/src/path_fragment.rs:403`),
and `child.get_exec_path()` is the child's whole exec path. So a remote child `x.txt` is treated
as already present when any local child is `<anything>/x.txt`.

Java: `remote/RemoteActionFileSystem.java:835` `getDirectoryContents` merges local and remote
entries by name per directory; `skyframe/ActionOutputMetadataStore.java:245-283` then puts every
visited child. No child is dropped.

A tree with both local and remote children is produced by
`remote/RemoteExecutionService.java:1343` (`shouldDownload` is evaluated per tree file), that is,
by `--remote_download_regex` matching some children of a tree.

Failing scenario: a `declare_directory` action writing `x.txt` and `sub/x.txt`, built with
`--remote_executor=... --remote_download_outputs=minimal
--remote_download_regex='.*/sub/.*'`. `sub/x.txt` is downloaded, `x.txt` is injected as remote.
The local visit yields `<tree>/sub/x.txt`; the remote `x.txt` is skipped because
`<tree>/sub/x.txt` ends with `x.txt`. The action's `TreeArtifactValue` has one child. `check_outputs`
passes. A remote consumer's input root has no `<tree>/x.txt`, a local consumer has it neither
prefetched nor in its sandbox, the tree digest differs from Bazel's (different action keys and
remote cache keys for every consumer), and the action cache stores the short tree. The build
reports success unless a consumer happens to open the missing file.

The bench tree (`defs.bzl` `make_tree`: `x.txt`, `sub/y.txt`) has distinct base names, so the
regex row of the evidence table cannot show this. Duplicate base names across directories of one
tree are the normal case in real trees (`index.js`, `__init__.py`, `mod.rs`).

Wanted: compare the parent-relative path of the local child for equality; a bench tree with a
repeated base name and the regex row re-measured against the oracle; port
`prefetchFiles_downloadRemoteTrees_partial` (see N4).

### B2 (S1) A tree artifact that is a symlink to a remote tree gets an empty (or missing) value

`crates/blaze_sky_exec/src/skyframe_action_executor.rs:1884-1952`. For a tree output `S` that
`ctx.actions.symlink(output = declare_directory(...), target_file = <tree T>)` materialized as a
symlink (reachable: `crates/blaze_analysis/src/starlark/starlark_action_factory.rs:1488-1522`),
with `T` remote:

- `:1891` stats `S` on the local file system, following the symlink. `T`'s directory exists
  locally and is empty: the executor creates it before `T`'s action runs
  (`crates/blaze_exec/src/actions/action_output_directory_helper.rs:107`) and a remote execution
  or cache hit puts nothing in it.
- `visit_tree` lists the local directory: no children.
- `:1929-1941` merges `remote_output_tree.files_under(<path of S>)`. The remote children are
  keyed under `T`'s path, so nothing is found.
- The result is `TreeArtifactValue::build([], None, None)`, the empty tree. No resolved path is
  set; the comment at `:1946-1952` says only an action file system hands out a stat with
  metadata, which was true before this commit made the build-wide tree stand in for one.

The file case was handled in this commit (`remote_symlink_target_metadata`, `:1716`); the tree
case was not. `RemoteActionFileSystem::stat`/`readdir`
(`crates/blaze_sky_exec/src/remote_output_tree.rs:231-249, 336-359`) likewise follow a symlink to
a remote file but not a symlinked directory.

Java: `skyframe/ActionOutputMetadataStore.java:245-314`. The stat and the visit go through the
`RemoteActionFileSystem`, which canonicalizes the path first
(`remote/RemoteActionFileSystem.java:326` `resolveSymbolicLinks`, used by every read), so the
children of `T` are listed and get their remote metadata, and `:305-312` records
`tree.setResolvedPath(...)`. `AbstractActionInputPrefetcher.java:506-600`
(`maybeGetTreeRoot`, `getSymlinks`) then relies on that resolved path; the Rust ports of those are
correct but never see such a tree. Java test: `prefetchFiles_downloadRemoteTrees_withResolvedPath`
(ignored here as "port pending", N4).

Failing scenario: rule `A` declares tree `T` (two files); rule `B` does
`ctx.actions.symlink(output = ctx.actions.declare_directory("s"), target_file = T)`; rule `C`
runs `cat s/*`. `build //:c --remote_executor=... --remote_download_outputs=minimal` (or
`toplevel` with only `//:c` requested; or a warm `--remote_cache`). Bazel: `C` sees both files.
Blaze: `s` is an empty tree, `C` runs against an empty directory and the build succeeds with a
wrong output (or `C` fails on a missing file). In `toplevel` with `//:b` requested, `finalize_action`
has no child to download and `bazel-bin/s` is a symlink to an empty directory. If `T`'s local
directory does not exist the same code returns the missing-tree value and `B` fails with a
not-created error, where Bazel succeeds.

Wanted: the tree branch of the same resolution the file branch got (children and resolved path
through the symlink, from the action's input tree metadata or the remote output tree), a
symlinked-tree target with a remote and a local consumer in the bench workspace measured against
the oracle, and the Java test ported.

## Non-blocking findings

- **N1 (S3, the most important of these) A lost remote blob is a panic that repeats on every
  later invocation, and there is one case where Bazel recovers without rewinding.**
  `crates/blaze_remote/src/common/bulk_transfer_exception.rs:56`,
  `abstract_action_input_prefetcher.rs:819-835`. Every path I traced is loud (local spawn
  inputs, `finalize_action`, remote execution uploads at `remote_spawn_runner.rs:514, 857`,
  `merkle_tree_computer.rs:596`); none is silent and none hangs. But the stale action cache
  entries with remote metadata stay on disk, so the next `blaze build` panics at the same place
  until `blaze clean` or one run with `--remote_download_outputs=all` (which makes
  `shouldTrustMetadata` reject the remote entries). The trigger needs no exotic setup: a
  `--disk_cache` directory that was deleted or garbage collected, or a remote cache that evicts,
  then an edit that makes one local action (any action with `--remote_cache` only) need an
  unchanged remote intermediate. Bazel 9.3.2 defaults: `--rewind_lost_inputs=true`
  (`buildtool/BuildRequestOptions.java:373`), so it rewinds. With `--norewind_lost_inputs` Bazel
  does not rewind and still recovers: exit 39, `remote/LeaseService.java:79-105` drops every
  action cache entry with output metadata, and `runtime/BlazeCommandDispatcher.java:851` reruns
  the command up to `--experimental_remote_cache_eviction_retries=5`
  (`exec/ExecutionOptions.java:506`). Blaze panics for that flag combination too
  (`throw_if_not_empty` does not read it), and where it returns `REMOTE_CACHE_EVICTED` instead
  (lost artifact unknown to the resolver) nothing purges the cache or retries.
  `docs/remote.md` names the panic but not that it is sticky, nor the way out.
- **N2 (S2) `do_download_file` turns every error into `CacheNotFoundException`.**
  `crates/blaze_remote/src/remote_action_input_fetcher.rs:157-180`. Java
  (`remote/RemoteActionInputFetcher.java:135-166`) catches `IOException` of the returned future
  only, and says so: errors thrown directly by `downloadFile` (creating the temp directory,
  writing an empty file, `remote/CombinedCache.java:756-763`) are left alone as local file
  system problems, and an interruption is not an `IOException`. In Rust both are one `Result`,
  so a full or unwritable `action_temps` directory and an `Interrupted` error become a "lost
  input" and end in the rewinding panic of N1 with the real cause discarded. The Java comment
  explaining the distinction was not carried over.
- **N3 (S2) The `ActionOutputDirectoryHelper` bypass is described as Java's branch, and it is
  not the branch a build takes.** `abstract_action_input_prefetcher.rs:138-140, 192`. Java's
  helper is non-null whenever there are `BuildRequestOptions`
  (`remote/RemoteModule.java:346-349`), so Bazel runs `createOutputDirectory`
  (`actions/ActionOutputDirectoryHelper.java:181-189`), which replaces a file or symlink found
  where a directory is needed. `create_directory_and_parents` fails there. Reachable when an
  output path component changed from file to directory between builds and the producing action
  is served from the action cache.
- **N4 (S3) Four tests are ignored as "port pending", not for missing infrastructure**
  (`tests/remote_action_input_fetcher_test.rs:499-509, 558-560`:
  `..._download_remote_trees_partial`, `..._with_resolved_path`,
  `..._for_action_template_expansion`, `..._ignore_non_remote_files_tree`). The code they test is
  in this commit and they cover the ground of B1 and B2. The other 15 reasons hold (immutable
  value, spied or delayed or overlay file systems, thread interruption, rewinding, the repo
  contents cache, `EMPTY_MARKER`). Counts check: 31 Java tests, 12 ported, 19 ignored. No test of
  `RemoteActionFileSystemTest` is ported for the view this commit extended (symlink following,
  `readdir` merge, download on read).
- **N5 (S3) Prefetch downloads cannot be interrupted.** `runtime.rs:36` blocks until every
  transfer of the call ends; `shutdown` is never called (`docs/remote.md` says so) and
  `on_build_interrupted` does not reach the prefetcher, where Java cancels
  (`remote/RemoteActionContextProvider.java:247`). The claim "nothing is pending in
  `afterCommand`" holds: every entry point is a `block_on` on the caller's thread, and I found
  no blocking call from inside the runtime and no lock held across an `await` (the tree and
  directory-state locks are taken only in synchronous sections, tree lock before state lock on
  both paths). A driver dropped mid-download leaves its temp file until `afterCommand` deletes
  the directory.
- **N6 (S3) `--experimental_finalize_actions` is not read** (`skyframe_action_executor.rs:854-857`,
  Java `skyframe/SkyframeActionExecutor.java:1340`); documented.
- **N7 (S4) `RemoteOutputTree` scans every entry under one mutex** for each `stat` of a path
  that is not found, each `readdir` and each tree value (`remote_output_tree.rs:69-96, 239-246`),
  and `directory_state_map` is one mutex held across `mkdir`/`chmod`
  (`abstract_action_input_prefetcher.rs:178-195`; Java computes per key).
- **N8 (S4) `tree.remove` drops exact paths only** (`skyframe_action_executor.rs:705-709`), so a
  tree's injected children would survive a re-execution. Not reachable today: only test runner
  actions re-execute after an action cache hit, and they have no tree outputs.
- **N9 (S4) `download_if_remote` fetches a tree child as a plain input**
  (`remote_output_tree.rs:194-206`), so its directory is marked permanently writable and a later
  tree-child prefetch does not restore 0555 on it.
- **N10 (S4) `FileArtifactValue::TreeArtifactComposite` compares `is_remote`** (derived `Eq`,
  `file_artifact_value.rs:116`); Java's `equals` ignores it (`skyframe/TreeArtifactValue.java:237-246`).

## Checked and found sound

- `RemoteImportantOutputHandler` not ported. The handler exists for Skymeld, where a target can
  become top-level after its actions ran (`remote/RemoteModule.java:1197-1212`). Blaze posts
  `TopLevelTargetAnalyzedEvent` from analysis (`blaze_sky_top/src/build_view.rs:260, 389`,
  `analysis_phase_runner.rs:187`) before any `BuildDriverKey` is evaluated, including targets
  dropped for action conflicts and skipped targets, under `--keep_going` too. A
  `TestAnalyzedEvent` is posted by the test's own driver before its completion key is requested,
  and only that key requests the test runner action. An output that becomes top-level on a
  rebuild is covered by `shouldTrustMetadata` (not trusted, re-executed, downloaded), including
  tree children and the `ctx.actions.symlink` file case. Top-level aspects are a loud
  `unported!`.
- `RemoteOutputChecker` wiring (`build.rs:1103-1262`) matches `remote/RemoteOutputChecker.java:104-235`:
  important groups of the requested output groups, runfiles under `_hidden_top_level_INTERNAL_`,
  test outputs not in `minimal`, the test binary excluded in `test`.
- `abstract_action_input_prefetcher.rs` is a faithful port of
  `remote/AbstractActionInputPrefetcher.java:257-900`: temp path, chmod before the move, atomic
  move, delete on failure, tree locks, permission restoration, symlink planting order,
  `finalizeAction`. Digest verification is the phase 1 download path (`combined_cache.rs:779-813`).
- The build-wide tree against per-action file systems: every source of remote metadata
  (spawn injection, `inject_file`/`inject_tree` on an action cache hit) writes to it, a path is
  written by one action and read only by its consumers after it is done, and local file system
  metadata overrides cached remote metadata on an action cache check as in
  `actions/ActionCacheChecker.java:314-414`. A stale remote entry after a failed remote attempt
  and local fallback behaves as Java's does.
- Implementer's open list: contents proxy after download, no progress events, symlink outputs to
  local inputs without a resolved path, local trees as `HashedDirectory` (same digest) are
  harmless for the reasons given. The directory helper is N3.
- R2-1, R2-2, R2-3 were done as stated (`execution_tool.rs:621-638`, `build.rs:159-198`). The
  remaining order difference for the single-error case is written down as open.

## `docs/remote.md` Phase 2 against the code

- "Remote tree artifacts are ported": not for a mixed tree with a repeated base name (B1) or a
  symlinked tree (B2).
- "Fails loudly (phase 2)": accurate, incomplete (N1: sticky, no recovery named, the
  `--norewind_lost_inputs` retry path).
- "The prefetcher creates directories with `createDirectoryAndParents` (Java's branch for a null
  `ActionOutputDirectoryHelper`)": the helper is not null in a Bazel build (N3).
- Wiring paragraph, test counts and the other gaps match the code.
