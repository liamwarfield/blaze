# 07c: Byte strings, the execution crates' boundary sites

This is the inventory of string-boundary sites in blaze_exec, blaze_sky_exec, blaze_buildtool,
blaze_remote, blaze_skyframe and blaze, written for the conversion described in
`07-byte-strings-rules.md`. The rules are in that document. It was surveyed on tree `e98c83c`
plus the working tree on 2026-11-18, with `STRINGS_ARE_INTERNAL == true`. Java references are
relative to `src/main/java` at the 9.3.2 pin.

The classes are:

- **a**: right as it is after the switch.
- **b**: wrong after the switch for non-ASCII input. The line gives the replacement.
- **c**: a seam that is correct now but should become the plain function.
- **d**: unsure; the line says what to check.

`RESERVED` marks files the remote-execution lead is editing. `free` marks the others. Several
identical sites in one file share a line.

## 1. Summary

Counts are lines of section 3. A number in parentheses is the count of individual calls on
those lines.

| crate | files | a | b | c | d |
|---|---|---|---|---|---|
| blaze_exec | free | 25 (47) | 11 | 8 (21) | 3 |
| blaze_exec | RESERVED | 5 | 0 | 1 | 0 |
| blaze_sky_exec | free | 1 | 1 | 0 | 0 |
| blaze_sky_exec | RESERVED (`skyframe_action_executor.rs`) | 2 | 0 | 0 | 0 |
| blaze_buildtool | free | 7 | 0 | 0 | 0 |
| blaze_buildtool | RESERVED (`execution_tool.rs`) | 0 | 0 | 0 | 0 |
| blaze_remote | RESERVED (all) | 19 (73) | 6 | 8 | 1 (2) |
| blaze_skyframe | free | 0 | 0 | 0 | 0 |
| blaze (binary) | free | 0 | 1 | 0 | 0 |
| **total** | free | 33 | 13 | 8 | 3 |
| **total** | RESERVED | 26 | 6 | 9 | 1 |

The 3 (d) lines in blaze_exec are `local_spawn_runner.rs:283` plus two unported sites, listed
in section 3.

blaze_skyframe is the generic evaluator. It has no string boundary: no I/O, no encoders, no
non-ASCII literals.

These are outside the scoped crates but are where the local non-ASCII build's bytes are
decided:

- `FileWriteAction`, `ParameterFileWriteAction`, `LazyWrite*`, `SourceManifestAction` and
  `RepoMappingManifestAction` writers, and action keys: blaze_actions.
- Subprocess argv, env and cwd: blaze_shell, already converted.
- `TargetCompleteEvent`'s three `internalToUnicode` calls: blaze_bep.
- `DependencySet`: blaze_cc_features.
- The `RegexPatternConverter` `internalToUnicode`: blaze_options `converters.rs:660`, still an
  identity marker.

## 2. Needed for a local non-ASCII build

The `scripts/bytes/workspace` build (genrule, `ctx.actions.write`, `run_shell` argv and env) does
not depend on any (b) site in these crates. Its argv and env go through blaze_shell, its file
contents through blaze_actions, and the action-cache and exec-log encodings are already right.
The list below is what a slightly broader ordinary local build needs: a non-ASCII label or
argument on the command line, a test, a non-ASCII output base, or a dangling symlink in
runfiles. Free files come first.

1. `crates/blaze/src/main.rs:44`: replace `std::env::args()` with `blaze_client::argv_from_os()`.
   - Today a non-ASCII argument such as `build //:fg_é` arrives as Unicode instead of internal,
     so the label does not match.
   - Argv that is not UTF-8 panics.
   - Java ref: `src/main/cpp/main.cc:24` gives argv as raw bytes.
2. `crates/blaze_exec/src/exec/test_log_helper.rs:35`: the test-output header with a non-ASCII
   test name is written as UTF-8. Replace it with
   `latin1_encode_lossy(&format!(..))` (`exec/TestLogHelper.java:58`, a default-charset
   `PrintStream`).
3. `crates/blaze_exec/src/util/file_out_err.rs:139`: `OpenOptions::open(&str)` on an internal
   path. Use `internal_to_platform(p.get_path_string())`, or `blaze_vfs`
   `Path::get_output_stream_with(append)` (`util/io/FileOutErr.java:400`). This matters only
   for a non-ASCII output base.
4. `crates/blaze_sky_exec/src/recursive_filesystem_traversal_function.rs:503`: a dangling-symlink
   digest computed over `as_bytes()`. Use `latin1_encode_lossy(..)`
   (`skyframe/RecursiveFilesystemTraversalValue.java:583`, `hashString(.., ISO_8859_1)`).
5. `crates/blaze_exec/src/exec/test_policy.rs:43`: `to_string_lossy` of the passwd name. Use
   `platform_to_internal(OsStr::from_bytes(cstr.to_bytes()))` (`util/UserUtils.java:37`,
   `user.name` decoded with the Latin-1 JNU charset).
6. `crates/blaze_exec/src/exec/local/local_spawn_runner.rs:587`: the host name. Use
   `latin1_decode` / `platform_to_internal` (`util/NetUtil.java:69`).
7. RESERVED: none is needed for a local build.
   - `blaze_remote` runs only with `--remote_*`, `--disk_cache` or BEP upload.
   - `remote_spawn_runner.rs:762` (server message written as UTF-8) affects only remote builds.
   - `disk_cache_client.rs:500` (`File::open(&str)`) affects only a non-ASCII `--disk_cache`
     directory.

## 3. Sites

### blaze_exec (free)

```
crates/blaze_exec/src/exec/spawn_log_context.rs:109-111  c  free  internal() wrapper over to_internal  ->  delete; callers pass the string as is  (seam)
crates/blaze_exec/src/exec/spawn_log_context.rs:119-126  c  free  local internal_to_unicode wrapper with strings_are_internal() branch  ->  blaze_util::string_encoding::internal_to_unicode  (util/StringEncoding.java:126)
crates/blaze_exec/src/exec/spawn_log_context.rs:130-137  c  free  local unicode_to_internal wrapper with branch  ->  blaze_util::string_encoding::unicode_to_internal  (util/StringEncoding.java:137)
crates/blaze_exec/src/exec/spawn_log_context.rs:141-145  a  free  java_string_compare: UTF-16 order  ->  keep  (String.compareTo)
crates/blaze_exec/src/exec/spawn_log_context.rs:170  a  free  env sorted by as_bytes(): UTF-8 order of chars <= U+00FF equals Java char order  ->  keep; fix comment  (exec/SpawnLogContext.java:114)
crates/blaze_exec/src/exec/spawn_log_context.rs:174,175  a  free  internalToUnicode of env name/value  ->  keep (plain fn)  (exec/SpawnLogContext.java:117,118)
crates/blaze_exec/src/exec/spawn_log_context.rs:235  a  free  internalToUnicode(remoteExecutionProperties) before text-format parse  ->  keep  (analysis/platform/PlatformUtils.java:134)
crates/blaze_exec/src/exec/spawn_log_context.rs:260,261  a  free  unicodeToInternal of parsed property name/value  ->  keep  (PlatformUtils.java:147)
crates/blaze_exec/src/exec/spawn_log_context.rs:283  a  free  internalToUnicode of platform properties  ->  keep  (PlatformUtils.java:172,173)
crates/blaze_exec/src/exec/compact_spawn_log_context.rs:143,144,146,169,187,189,210,225,237,239,271,272,277,279,446,522,547,585,688,714,760,761  a  free  internalToUnicode into ExecLogEntry fields (22 calls via the local wrapper)  ->  keep, import the plain fn  (exec/CompactSpawnLogContext.java:214,215,217,238,249,251,267,278,285,287,315,316,320,322,460,506,544,573,654,681,726,727)
crates/blaze_exec/src/exec/compact_spawn_log_context.rs:617  c  free  internal(p) for empty_files (Java does NOT convert here)  ->  p.get_path_string().to_string()  (exec/CompactSpawnLogContext.java:592-594)
crates/blaze_exec/src/exec/expanded_spawn_log_context.rs:135,167,208,209,229,243,253,273,274,290,299,305,308,353  c  free  internal(...) on args/paths/status/runner/mnemonic/label (Java stores internal strings unconverted)  ->  drop the wrapper  (exec/ExpandedSpawnLogContext.java:159,182,206,207,223,243,262,263,277,286,292,295,371)
crates/blaze_exec/src/exec/expanded_spawn_log_context.rs:334  a  free  dirents sorted by as_bytes() == Java char order  ->  keep  (ExpandedSpawnLogContext.java:356)
crates/blaze_exec/src/exec/local/local_spawn_runner.rs:283  d  free  to_internal(message).as_bytes(); Java: unicodeToInternal(pattern) then getBytes(UTF_8)  ->  drop to_internal; wrap allowed_local_action.pattern() in unicode_to_internal in the same change that makes blaze_options RegexPatternConverter (converters.rs:660) apply internalToUnicode; until then the pattern is internal and the identity is right  (exec/local/LocalSpawnRunner.java:326-329)
crates/blaze_exec/src/exec/local/local_spawn_runner.rs:440-442  a  free  "Action failed to execute: ..." getBytes(UTF_8)  ->  keep  (LocalSpawnRunner.java:431)
crates/blaze_exec/src/exec/local/local_spawn_runner.rs:587  b  free  gethostname bytes via from_utf8_lossy  ->  latin1_decode(&buf[..len]); the to_socket_addrs lookup needs the platform bytes (check for non-ASCII host names)  (util/NetUtil.java:69)
crates/blaze_exec/src/exec/test_policy.rs:43  b  free  passwd name via to_string_lossy  ->  platform_to_internal(OsStr::from_bytes(CStr::to_bytes()))  (util/UserUtils.java:37, user.name with Latin-1 JNU)
crates/blaze_exec/src/analysis/actions/local_template_expansion_strategy.rs:130  c  free  internal_string_bytes(expanded)  ->  latin1_encode_lossy (getBytes(ISO_8859_1) writes '?', the seam panics on > U+00FF)  (analysis/actions/LocalTemplateExpansionStrategy.java:52)
crates/blaze_exec/src/analysis/actions/local_template_expansion_strategy.rs:162  c  free  internal_string_from_bytes + unported! for non-UTF-8  ->  latin1_decode(&bytes); delete the Err arm and the PORT comment  (analysis/actions/Template.java:107)
crates/blaze_exec/src/util/file_out_err.rs:117-123  c  free  blaze_protos::java::decode_latin1  ->  latin1_decode  (util/io/FileOutErr.java:166,175)
crates/blaze_exec/src/util/file_out_err.rs:60-70  a  free  dump copies raw bytes  ->  keep  (FileOutErr.java:219-220)
crates/blaze_exec/src/util/file_out_err.rs:139  b  free  std OpenOptions::open(internal &str)  ->  open(internal_to_platform(p.get_path_string())) or blaze_vfs append stream  (FileOutErr.java:400)
crates/blaze_exec/src/sandbox/abstract_sandbox_spawn_runner.rs:305  a  free  msg.getBytes(UTF_8)  ->  keep  (sandbox/AbstractSandboxSpawnRunner.java:241)
crates/blaze_exec/src/sandbox/abstract_sandbox_spawn_runner.rs:483  b  free  sandbox debug output via from_utf8_lossy  ->  java_new_string_utf8(&bytes) (differs only on malformed UTF-8: JDK replacement count)  (AbstractSandboxSpawnRunner.java:329)
crates/blaze_exec/src/sandbox/sandbox_module.rs:580  a  free  Windows-only literal getBytes(UTF_8)  ->  keep  (sandbox/WindowsSandboxUtil.java:69)
crates/blaze_exec/src/sandbox/linux_sandbox_command_line_builder.rs:228-292  a  free  internal paths pushed into argv; blaze_shell converts at exec  ->  keep  (sandbox/LinuxSandboxCommandLineBuilder.java:233; no charset in Java)
crates/blaze_exec/src/analysis/actions/bazel_workspace_status_module.rs:224  b  free  stdout via from_utf8_lossy  ->  java_new_string_utf8(&bytes) (a Unicode string, as Java)  (bazel/BazelWorkspaceStatusModule.java:120)
crates/blaze_exec/src/actions/cache/persistent_string_indexer.rs:199  b  free  key via from_utf8_lossy  ->  java_new_string_utf8 (same on valid files; JDK replacement on corrupt)  (actions/cache/PersistentStringIndexer.java:195)
crates/blaze_exec/src/actions/cache/persistent_string_indexer.rs:207  a  free  key.getBytes(UTF_8) (double encoding on disk, as Bazel)  ->  keep  (PersistentStringIndexer.java:205)
crates/blaze_exec/src/actions/cache/persistent_string_indexer.rs:122-128  b  free  dump via writeln! (UTF-8)  ->  write latin1_encode_lossy(&line); default-charset PrintStream  (PersistentStringIndexer.java:173)
crates/blaze_exec/src/actions/cache/action_cache.rs:267-303  b  free  Entry.dump via writeln! of internal paths  ->  latin1_encode_lossy  (actions/cache/ActionCache.java:196)
crates/blaze_exec/src/actions/cache/compact_persistent_action_cache.rs:1210-1236  b  free  dump via writeln!  ->  latin1_encode_lossy  (actions/cache/CompactPersistentActionCache.java:766)
crates/blaze_exec/src/actions/cache/map_codec.rs:213-230  a  free  writeUTF modified UTF-8 over UTF-16 units  ->  keep  (DataOutputStream.writeUTF)
crates/blaze_exec/src/exec/test_log_helper.rs:35  b  free  header "Test output for <name>" via writeln! (UTF-8)  ->  out.write_all(&latin1_encode_lossy(&format!(..)))  (exec/TestLogHelper.java:58)
crates/blaze_exec/src/exec/test_log_helper.rs:46,53  a  free  ASCII-only lines  ->  keep (optionally the same encoder)  (TestLogHelper.java:67,71)
crates/blaze_exec/src/exec/test_log_helper.rs:83-93  a  free  header filter pushes b as char (Latin-1), counts chars  ->  keep  (TestLogHelper.java:106-115)
crates/blaze_exec/src/exec/io/message_input_stream_wrapper.rs:166  a  free  byte scan for ASCII delimiter "\n}{\n"  ->  keep
crates/blaze_exec/src/exec/io/message_input_stream_wrapper.rs:199  b  free  Scanner(UTF_8) via from_utf8_lossy  ->  java_new_string_utf8 (malformed input only)  (util/io/MessageInputStreamWrapper.java:70)
crates/blaze_exec/src/exec/io/message_output_stream_wrapper.rs:80  a  free  print(m).getBytes(UTF_8)  ->  keep  (util/io/MessageOutputStreamWrapper.java:64)
crates/blaze_exec/src/exec/stable_sort.rs:229  a  free  TextFormat escapeBytes(copyFromUtf8(text))  ->  keep  (protobuf TextFormatEscaper)
crates/blaze_exec/src/exec/spawn_strategy_registry.rs:861-865  a  free  String.hashCode over UTF-16 units  ->  keep
crates/blaze_exec/src/runtime/spawn_stats.rs:210  a  free  String.compareTo over UTF-16  ->  keep
crates/blaze_exec/src/bin/blaze-exec-replay.rs:236-495; bin/blaze-test-replay.rs:553-789  a  free  dev tools (env::args, std::fs on ASCII record paths); not ports  ->  keep
crates/blaze_exec/src/exec/runfiles_tree_updater.rs:314-378; exec/single_build_file_cache.rs:176-213; exec/evaluation_stack.rs:98  a  free  #[cfg(test)] std::fs / Command on ASCII temp paths  ->  keep
```

Not ported. Each has a boundary to get right when it is ported:

```
crates/blaze_exec/src/analysis/actions/spawn_action.rs:253  d  free  unused_inputs_list unported  ->  when ported: read lines with latin1_decode  (analysis/actions/StarlarkAction.java:423-425 InputStreamReader ISO_8859_1)
crates/blaze_exec/src/exec/executor_setup.rs:340  d  free  WorkerSpawnRunner.exec unported  ->  when ported: internal_to_unicode for WorkRequest input paths; flagfile lines as java_new_string_utf8  (worker/WorkerSpawnRunner.java:281,320; WorkerMultiplexer 3 sites)
crates/blaze_exec/src/exec/runfiles_tree_updater.rs:188  a  free  isRunfilesDirectoryPopulated skipped (Windows only)  ->  none  (exec/RunfilesTreeUpdater.java:160)
```

### blaze_exec (RESERVED)

```
crates/blaze_exec/src/actions/file_artifact_value.rs:451  c  RESERVED  internal_string_bytes(symlink_target) for the unresolved-symlink digest  ->  latin1_encode_lossy (hashString(.., ISO_8859_1) writes '?'; the seam panics on > U+00FF)  (actions/FileArtifactValue.java:1157)
crates/blaze_exec/src/exec/standalone_test_strategy.rs:651  a  RESERVED  ASCII literal getBytes(UTF_8)  ->  keep  (exec/StandaloneTestStrategy.java:700)
crates/blaze_exec/src/exec/standalone_test_strategy.rs:926-930  a  RESERVED  appendStderr raw byte copy  ->  keep
crates/blaze_exec/src/analysis/test/test_runner_action.rs:654,692  a  RESERVED  TestResultData proto bytes (strings internal, as Java)  ->  keep  (analysis/test/TestRunnerAction.java:608,628)
crates/blaze_exec/src/actions/input_metadata_provider.rs:678-857  a  RESERVED  #[cfg(test)] std::fs, CString, to_str on ASCII temp paths  ->  keep
crates/blaze_exec/src/actions/action_output_directory_helper.rs:357-421  a  RESERVED  #[cfg(test)] std::fs on ASCII temp paths  ->  keep
```

`remote_action_file_system.rs`, `action_adapters.rs`, `action_execution_metadata.rs`,
`exec_exception.rs`, `action_execution.rs`, `exec/action_execution_context.rs`,
`abstract_spawn_strategy.rs`, `spawn_runner.rs`: no sites. Their Java has no charset call either.
`RemoteExternalOverlayFileSystem` (4 `unicodeToInternal` calls) is not ported anywhere.

### blaze_sky_exec

```
crates/blaze_sky_exec/src/recursive_filesystem_traversal_function.rs:503  b  free  sha256(link_value.as_bytes()) for a dangling symlink  ->  sha256(&latin1_encode_lossy(link_value.get_path_string())); delete the PORT comment  (skyframe/RecursiveFilesystemTraversalValue.java:583)
crates/blaze_sky_exec/src/action_execution_function.rs:1627-2445  a  free  #[cfg(test)] std::fs on ASCII temp paths  ->  keep
crates/blaze_sky_exec/src/skyframe_action_executor.rs:1850,1851  a  RESERVED  action stdout/stderr bytes handed to the reporter raw  ->  keep  (FileOutErr.dumpOutAsLatin1)
crates/blaze_sky_exec/src/skyframe_action_executor.rs:3916-4451  a  RESERVED  #[cfg(test)] std::fs on ASCII temp paths  ->  keep
```

### blaze_buildtool

```
crates/blaze_buildtool/src/buildtool/build_result_printer.rs:497-570  a  free  "Target ... up-to-date" lines via blaze_events OutErr.print_err (latin1_encode_lossy)  ->  keep  (buildtool/BuildResultPrinter.java:269,312)
crates/blaze_buildtool/src/runtime/test_summary_printer.rs:61,282  a  free  Strings.padEnd / 78 - length() counted in chars  ->  keep
crates/blaze_buildtool/src/runtime/test_summary_printer.rs:89-121  a  free  numeric formatting (ASCII digits)  ->  keep
crates/blaze_buildtool/src/runtime/test_summary_printer.rs:188,207  a  free  cached test log streamed raw through AnsiTerminalPrinter's stream (blaze_events encodes print/printLn as Latin-1)  ->  keep  (util/io/AnsiTerminalPrinter.java:83)
crates/blaze_buildtool/src/buildtool/symlink_forest.rs:461  a  free  Ascii.toLowerCase  ->  keep  (buildtool/SymlinkForest.java:400)
crates/blaze_buildtool/src/buildtool/output_directory_links_utils.rs:531  a  free  slice of an OS error message  ->  keep
crates/blaze_buildtool/src/bin/blaze-build-replay.rs:423-674; bin/replay/recording.rs:31-154  a  free  dev tools (env::args/var, std::fs on ASCII record paths)  ->  keep
crates/blaze_buildtool/src/buildtool/execution_tool.rs  -  RESERVED  no sites; --explain log unported (when ported: PrintWriter(UTF_8) of internal messages = as_bytes)  (buildtool/ExecutionTool.java:854)
```

### blaze (binary)

```
crates/blaze/src/main.rs:44  b  free  std::env::args() (UTF-8 decode, panics on other bytes)  ->  blaze_client::argv_from_os()  (src/main/cpp/main.cc:24)
```

### blaze_remote (all RESERVED)

```
crates/blaze_remote/src/remote_execution_service.rs:76-84  c  RESERVED  local wrapper internal_to_unicode + "PORT: StringEncoding identity"  ->  delete; plain internal_to_unicode at callers  (util/StringEncoding.java:126)
crates/blaze_remote/src/remote_execution_service.rs:88-96  c  RESERVED  local wrapper unicode_to_internal + marker  ->  delete; plain unicode_to_internal  (StringEncoding.java:137)
crates/blaze_remote/src/remote_execution_service.rs:1284  c  RESERVED  to_internal(workspace) into CacheSalt.workspace  ->  workspace.clone()  (remote/RemoteExecutionService.java:384)
crates/blaze_remote/src/remote_execution_service.rs:1610  c  RESERVED  to_internal(output path) into GetActionResult inline_output_files  ->  drop to_internal  (RemoteExecutionService.java:788-789)
crates/blaze_remote/src/remote_execution_service.rs:106,697,1137,1148,1191,1201,1202,1206,2052  a  RESERVED  internalToUnicode / unicodeToInternal / PROTO_STRING_COMPARATOR as Java  ->  keep (wrapper -> plain fn)  (RemoteExecutionService.java:176,717,283,292,317,324,325,330,2044)
crates/blaze_remote/src/remote_execution_service_download.rs:118-123  b  RESERVED  Path.hashCode emulation loops over .bytes() of the internal path  ->  loop over chars(): h = h*31 + c as i32; delete the PORT marker  (vfs/Path.java:229 -> String.hashCode; only orders the realToTmpPath map)
crates/blaze_remote/src/remote_execution_service_download.rs:392,402,403,408,475,566,588,594,1017,1021,1703  a  RESERVED  unicodeToInternal of proto names/paths, internalToUnicode for progress/cache paths  ->  keep  (RemoteExecutionService.java:1086,1096,1097,1101,1134,1177,1195,1196,872,878,1625)
crates/blaze_remote/src/upload_manifest.rs:195-207  c  RESERVED  local wrapper internal_to_unicode with strings_are_internal() branch  ->  plain internal_to_unicode
crates/blaze_remote/src/upload_manifest.rs:476,477,491,492,504,520,877,976,997,1000  a  RESERVED  internalToUnicode into proto fields  ->  keep  (remote/UploadManifest.java:331,332,341,342,351,563,444,532,544,545)
crates/blaze_remote/src/upload_manifest.rs:211  a  RESERVED  encode_utf16 sort key on Unicode node names  ->  keep  (String.compareTo)
crates/blaze_remote/src/upload_manifest.rs:729; remote_execution_cache.rs:246  a  RESERVED  text-format escape of digest hash (ASCII hex)  ->  keep
crates/blaze_remote/src/merkletree/merkle_tree_computer.rs:1365-1377  c  RESERVED  local wrapper internal_to_unicode  ->  plain fn
crates/blaze_remote/src/merkletree/merkle_tree_computer.rs:844,862,907  a  RESERVED  internalToUnicode of names / symlink target  ->  keep  (remote/merkletree/MerkleTreeComputer.java:629,639,659)
crates/blaze_remote/src/platform_utils.rs:19-30,33-41  c  RESERVED  local wrappers internal_to_unicode / unicode_to_internal (2)  ->  plain fns
crates/blaze_remote/src/platform_utils.rs:69,70,168,190,191,227,228  a  RESERVED  as Java  ->  keep  (analysis/platform/PlatformUtils.java:60,61,134,147,147,172,173)
crates/blaze_remote/src/remote_spawn_runner.rs:762  b  RESERVED  outErr.printErr("Remote server execution message: "+message) written as UTF-8; Java PrintWriter is default charset  ->  write_err(&latin1_encode_lossy(&format!(..)))  (remote/RemoteSpawnRunner.java:533, util/io/OutErr.java:134)
crates/blaze_remote/src/disk/disk_cache_client.rs:500  b  RESERVED  std::fs::File::open(internal &str)  ->  File::open(internal_to_platform(path.get_path_string()))  (remote/disk/DiskCacheClient.java:455)
crates/blaze_remote/src/authandtls/google_auth_utils.rs:415  b  RESERVED  std::fs::read(--tls_certificate value)  ->  std::fs::read(internal_to_platform(file))  (authandtls/GoogleAuthUtils.java:185)
crates/blaze_remote/src/authandtls/google_auth_utils.rs:435  b  RESERVED  std::fs::read(--tls_client_key value)  ->  internal_to_platform(key_file)  (GoogleAuthUtils.java:194)
crates/blaze_remote/src/remote_grpc_service_config.rs:59  b  RESERVED  read_to_string fails on non-UTF-8; Java InputStreamReader(UTF_8) gives U+FFFD  ->  read_to_end + java_new_string_utf8 (Unicode, for serde_json as Java gives Gson)  (remote/RemoteGrpcServiceConfig.java:86)
crates/blaze_remote/src/authandtls/google_auth_utils.rs:392,416,436  a  RESERVED  PEM text / base64 (ASCII by format)  ->  keep
crates/blaze_remote/src/authandtls/google_auth_utils.rs:784-798  a  RESERVED  $HOME/.netrc via blaze_vfs Path  ->  keep
crates/blaze_remote/src/util/digest_util.rs:27-28  a  RESERVED  hash hex compare  ->  keep
crates/blaze_remote/src/util/digest_util.rs:111  a  RESERVED  computeAsUtf8 = getBytes(UTF_8)  ->  keep  (remote/util/DigestUtil.java:150)
crates/blaze_remote/src/util/utils.rs:344  a  RESERVED  TextFormat escape of Any.type_url (Unicode)  ->  keep
crates/blaze_remote/src/grpc/channel.rs:152,563,578  a  RESERVED  ASCII metadata key bytes  ->  keep
crates/blaze_remote/src/grpc/channel.rs:162-166  a  RESERVED  header value per char, non-ASCII -> '?' (getBytes(US_ASCII)); on an internal value one '?' per byte  ->  keep  (io/grpc/Metadata.java:981)
crates/blaze_remote/src/grpc/channel.rs:570  a  RESERVED  base64 decode of credential-helper value  ->  keep
crates/blaze_remote/src/grpc/channel.rs:859,860; disk/disk_cache_garbage_collector.rs:218,219; remote_server_capabilities.rs:168  a  RESERVED  ASCII number formatting  ->  keep
crates/blaze_remote/src/combined_cache.rs:855  a  RESERVED  write_content(path, &[])  ->  keep  (remote/CombinedCache.java:765)
crates/blaze_remote/src/byte_stream_build_event_artifact_uploader.rs:299,300,602  a  RESERVED  regex on internal path; "file://"+internal path into a BEP proto string (double encoded, as Java)  ->  keep  (remote/ByteStreamBuildEventArtifactUploader.java:290,291,503)
crates/blaze_remote/src/scrubber.rs:287,299  d  RESERVED  Rust regex on internal paths/args: \w \s \d and (?i) are Unicode-aware on Latin-1 chars where Java Pattern defaults are ASCII  ->  check against Java Pattern flags  (remote/Scrubber.java:167,175)
crates/blaze_remote/src/bin/blaze-remote-grpc-log.rs:22,27; bin/blaze-remote-replay.rs:318-707; bin/blaze-remote-fake-caps.rs:141  a  RESERVED  dev tools, not ports  ->  keep
```

These Java sites are not ported in blaze_remote:

- `RemoteExternalOverlayFileSystem` 304/334/345/360
- `RemoteRepoContentsCacheImpl` 183/280 (`ISO_8859_1`)
- `BazelOutputService` 109 (`hashString` `UTF_8`)
- `Scrubber.parse` (`Scrubber.java:74`, a UTF-8 reader)

## 4. Tests that will change meaning

```
crates/blaze_exec/tests/analysis_actions_execute_test.rs:229-240  free  test_with_special_characters: substitution is the Unicode literal "Š©±½_strøget" (Š = U+0160); the switch made internal_string_bytes panic on it  ->  Substitution::of("%key%", &unicode_to_internal(SPECIAL_CHARS)); delete the PORT comment  (TemplateExpansionActionTest.java:253)
crates/blaze_exec/tests/action_cache_checker_test.rs:125  free  write_content_as_latin1 uses as_bytes  ->  latin1_encode; all callers ASCII, no change today
crates/blaze_exec/tests/exec_test_log_helper_test.rs:47,118,133,154  free  blaze_protos::java::decode_latin1  ->  latin1_decode; no change
crates/blaze_remote/tests/remote_execution_service_test.rs:1470-1487  RESERVED  build_remote_action_golden_test feeds Unicode "path/ﾐ", "path/🔥" where Java wraps them in unicodeToInternal; panics now  ->  unicode_to_internal(..); drop the PORT marker  (RemoteExecutionServiceTest.java:484-485)
crates/blaze_remote/tests/util/fake_action_input_file_cache.rs:110  RESERVED  writeContentAsLatin1 port uses as_bytes; double encodes once the golden test names are internal  ->  latin1_encode(content)
crates/blaze_remote/tests/remote_execution_service_test.rs:2396-2405  RESERVED  download_outputs_output_directories_with_unicode_filenames_works looks up Unicode literal paths; fails now  ->  unicode_to_internal  (RemoteExecutionServiceTest.java:1279-1282)
crates/blaze_remote/tests/remote_execution_service_test.rs:4855-4858  RESERVED  build_remote_action_code_point_sorting: same panic  ->  unicode_to_internal on paths and env names  (RemoteExecutionServiceTest.java:3162-3177)
crates/blaze_remote/tests/remote_execution_service_test.rs:2416  RESERVED  download_outputs_non_ascii_names_are_unicode_on_both_sides_of_the_seam  ->  delete the false half and the with_strings_are_internal_for_testing wrapper
crates/blaze_remote/tests/merkle_tree_computer_test.rs:1179  RESERVED  build_for_files_non_ascii_names_are_unicode_on_both_sides_of_the_seam  ->  same
crates/blaze_remote/tests/remote_execution_service_test.rs:678; upload_manifest_test.rs:158; remote_execution_service_test.rs:3719  RESERVED  seam helpers  ->  plain unicode_to_internal (no change in meaning)
crates/blaze_remote/tests/merkle_tree_computer_test.rs:135; remote_execution_cache_test.rs:340  RESERVED  write_content_as_latin1 helpers use as_bytes  ->  latin1_encode; callers ASCII, no change today
crates/blaze_remote/src/grpc/channel.rs:1480-1490  RESERVED  header_values_are_sent_as_us_ascii feeds Unicode "aé" (the option value is internal), expects "a?"; Java gives "a??"  ->  input "a\u{c3}\u{a9}", expected value to confirm with oracle probe tc-hdr-o (no record found on disk)
```

No test in blaze_sky_exec, blaze_buildtool, blaze_skyframe or blaze has a non-ASCII literal or
uses `with_strings_are_internal_for_testing`.

## 5. Java inconsistencies to preserve

- The two execution logs disagree. `CompactSpawnLogContext` converts every path with
  `internalToUnicode` except `empty_files` (`CompactSpawnLogContext.java:592-594`), which stays
  internal and is therefore double encoded. `ExpandedSpawnLogContext` converts nothing. The
  Rust copies both behaviours; keep them when the wrappers go.
- In remote execution, `Execute` sends `inline_output_files` through `internalToUnicode`
  (`RemoteExecutionService.java:2043`). `lookupCache` sends the same paths unconverted
  (`:788-789`), and `CacheSalt.workspace` is the raw internal string (`:384`).
- The BEP `file://` URI from the ByteStream uploader is the internal path, unconverted
  (`ByteStreamBuildEventArtifactUploader.java:503`). `TargetCompleteEvent` converts its names
  (blaze_bep).
- Workspace status decodes the command's stdout as UTF-8 (Unicode). It then mixes in internal
  `BUILD_HOST` and `BUILD_USER` values (`NetUtil` `platformToInternal`, `user.name`) and writes
  everything with `getBytes(UTF_8)`. A non-ASCII host or user name is therefore double encoded
  in `stable-status.txt`; non-ASCII command output is not.
- Two hashes encode the same string differently. `FileArtifactValue` and
  `RecursiveFilesystemTraversalValue` hash a symlink target with `hashString(.., ISO_8859_1)`,
  which turns a char above U+00FF into `?`. The seam `internal_string_bytes` panics on such a
  char instead.
