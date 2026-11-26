# Response to the review of execution, second part (`exec-2`), 2026-11-25

Implementer: `exec-fix-1` (Opus 5.5). Review: `docs/reviews/2026-11-25-exec-2.md`, STOP at e8b79c0.
Tree: uncommitted on top of aa55920. The oracle evidence comes from the binary
`~/.cache/blaze/work/exec-fix-1/bin/blaze-a` (sha256 1f36a41ffce7a02b…, built from this tree).
The pre-fix comparison uses root's suite binary `~/.cache/blaze/work/root-runs/blaze-e8b79c0`.

## Blocking findings

**E2-1 fixed.** `crates/blaze_runtime/src/commands/exclusive_test_order.rs`:
- `putAll` pre-sizes with the formula of the oracle's JDK 25:
  `tableSizeFor((int) Math.ceil(s / (double) 0.75f))` (`put_all_table_size`).
- The two maps are now replayed by `HashMapReplay`, a port of `HashMap.putVal`, `resize` and
  `treeifyBin`. It covers the default first table, the pre-sized copy, and the early resize of a
  9-entry bin below 64 buckets. A tree bin at 64 buckets or more fails loudly (see E2-4).
- The caller (`commands/build.rs`, `run_exclusive_tests`) now passes every distinct key of the
  result, with its Java hash code where Blaze computes it (configured-target `BuildDriverKey`s),
  so the other entries count toward bins. `BuildInfoKey` hashes by identity, which makes Java's
  own result nondeterministic in the 9-entry corner case; this is noted in the code.
- Tests: `put_all_table_size_matches_jdk_25` (1 to 96 entries, against the reviewer's
  measurements); `order_with_{two,three,six,twelve}_entries`; the reviewer's case
  `test excl_b excl_c` (excl_c runs first); `treeify_bin_resizes_a_small_table`;
  `treeify_bin_of_a_large_table_fails_loudly`.
- Oracle: workspace `~/.cache/blaze/work/exec-fix-1/excl-ws` with ten exclusive tests
  (`excl_a` to `excl_j`; each prints its start time). Every pair (45 runs, a 3-entry result
  map) and ten sets of five (6 entries) were run on the oracle, the fixed binary and the pre-fix
  binary, with `--nocache_test_results`.
  - Fixed binary: all **55 of 55 orders equal** to the oracle.
  - Pre-fix binary: **9 of 55 differ**: the pairs c/e, c/i, d/e and d/i, and the five-sets
    `f g h i j`, `b d f h j`, `c d h i e`, `j i h g f` and `d g j c h`.
  - Files: `excl-oracle.txt`, `excl-blaze.txt`, `excl-old.txt` in `~/.cache/blaze/work/exec-fix-1/`.
  - The fixed binary exits 37 after the tests: a crash in `CoverageReportActionBuilder` from
    another agent's in-progress coverage work in the same tree. The tests ran before the crash,
    so the orders are valid; the exit codes are not compared.
- The eleven-target case the reviewer suggested is not run.

**E2-2 fixed.** `JavaException::from_io_error` (`worker/error_message.rs`):
- An `io::Error` carrying an errno now has the JDK's message `strerror(errno)`, through
  `blaze_shell::java_process::java_io_message`. A Blaze-made message is kept, and an empty
  message is Java's null.
- This covers every caller: the WorkRequest write, prepare, finish, "during worker execution",
  the multiplexer's read error, and `@`-argument reads.
- Tests: `from_io_error_uses_the_jdk_message_for_an_errno`, `from_io_error_keeps_a_given_message`.
- Oracle: workspace `~/.cache/blaze/work/exec-fix-1/worker-ws`, a proto worker that exits at
  startup, with 1 input (`//p:small`) and 1500 inputs (`//p:big`).
  - Bazel, small request: `java.io.IOException: Broken pipe` with **no** stack frames.
  - Bazel, large request: `java.io.IOException: Stream closed` followed by JVM frames. The JDK's
    process reaper replaced stdin with `ProcessBuilder$NullOutputStream` once the child had
    exited.
  - Blaze, large request: `java.io.IOException: Broken pipe` (was `Broken pipe (os error 32)`).
  - Blaze, small request: its write finished before the worker died, so it reports
    `Worker process did not return a WorkResponse:`.
  - Which message appears is a timing race on both sides. The JDK's reaper swap (writes after
    the child's exit fail with "Stream closed") is not ported; it is a separate item below.

**E2-3 fixed (with E2-5).** The `test.xml` reader is no longer `quick-xml`. The new
`exec/test_xml_output_parser/xml_stream_reader.rs` ports the part of the JDK's StAX reader that
`TestXmlOutputParser` uses:
- Encoding detection, as Xerces does it: byte order marks, UTF-16 without a BOM, and the encoding
  declaration resolved through Xerces' `EncodingMap` (dumped from the oracle's JDK). UTF-8,
  UTF-16, ISO-8859-1 (all 9 aliases) and US-ASCII (12 aliases) are read. Other encodings the
  JDK reads through a `Charset` call `unported!`. Unknown names are errors.
- Line-end normalization.
- XML 1.0 character validity, and `Name` / `NCName` from Xerces' `XMLChar` tables (dumped from the
  oracle's JDK; second-edition tables).
- QNames exactly as `XMLEntityScanner.scanQName` reads them (a leading colon is not a prefix; a
  second colon ends the name; `IllegalQName`).
- Attribute normalization; character and predefined entity references; comments, PIs, CDATA,
  and the `]]>` rule.
- Namespace binding rules; content in the prolog and after the root.
- Decoding errors surface only when the scanner reaches them, as in Xerces: a bad byte after the
  root is never seen.
- `unported!`: a DTD with an internal subset or an external ID (Java reads its entities), XML
  1.1, UCS-4, EBCDIC.

Tests: `crates/blaze_exec/tests/test_xml_output_parser_test.rs`, **145 documents**.
- The expected results come from Bazel's own `TestXmlOutputParser` (the oracle's `A-server.jar`)
  run on the oracle's embedded JDK.
- The documents include the reviewer's Latin-1, UTF-16 and `&nbsp;` cases, a raw `0x1B`,
  unbound prefixes, and 13 encoding aliases.
- 139 compare the parsed tree or the rejection, and 6 are documents Java accepts and the port
  `unported!`s. Result: **145 passed, 0 failed**.
- Generator and probe: `~/.cache/blaze/work/exec-fix-1/xmlprobe/` (`gen/make_cases*.py`,
  `src/XmlProbe.java`, `gen/make_rust_tests.py`).

## S2

- **E2-4 fixed.** `JavaStringHashMap` (`analysis/actions/spawn_action.rs`) fails loudly
  (`unported!("HashMap.treeifyBin")`) where Java would turn a bin into a red-black tree. The
  wrong comment is replaced. The same rule is in `exclusive_test_order.rs`, which also gains the
  early `treeifyBin` resize. Tests: `treeify_bin_resizes_a_small_table` and
  `treeify_bin_of_a_large_table_fails_loudly` (colliding "Aa"/"BB" strings).
- **E2-5 fixed** with E2-3; every input the reviewer named is among the 145 cases.
- **E2-6 fixed.**
  - `SpawnExecutionContext.lock_output_files` and `StopConcurrentSpawns.stop` now take
    `Option<&FileOutErr>` (Java's `@Nullable`).
  - The worker runner passes `None` at both of Java's sites (`WorkerSpawnRunner.java:443`, `:485`).
  - An `InterruptedException` of the second `lockOutputFiles` now replaces the original
    exception, as Java's catch block does; Blaze used to drop it.
- **E2-7 fixed.**
  - `JavaException::interrupted_io` and `is_interrupted_io_exception` represent
    `InterruptedIOException`.
  - `restore_interrupt` (`WorkerSpawnRunner.restoreInterrupt`) now runs at each of Java's sites:
    prepare, put, get response, finish, and the outer `IOException` catch.
  - The multiplexer's read error checks `InterruptedIOException`, not `ErrorKind::Interrupted`.
  - The `From<io::Error>` conversions are unchanged and documented: the only `io::Error` they
    turn into an interrupt is the one Blaze's interrupt checks make (`sandbox_helpers::
    interrupted_error`), and those are Java `InterruptedException`s (`WorkerExecRoot.createInputs`,
    `expandArgument`, ...). Nothing in the port raises an `InterruptedIOException` today; the
    marker keeps one an I/O failure if a future port does.
- **E2-8 fixed.**
  - `get_worker_files_with_digests` keeps Java's nullable digest (`Option<Vec<u8>>`).
  - `get_combined_hash` panics with the `NullPointerException` on a null digest, as Java crashes.
  - `WorkerParser` converts the map back to non-null for the key, after the hash.

## S3

- **Exception details.** From the oracle cases above:
  - An exception whose trace Java prints with frames shows `\tat …` lines, which Blaze cannot
    produce (the `Stream closed` case: 40 frames, server-internal).
  - The JDK's `Broken pipe` from the native write showed **no** frames, so that block is
    byte-identical.
  - **Ruling (accepted by root, 2026-11-25):** the JVM stack-frame lines (`\tat …`) of an
    "Exception details" block are a declared gap. The first line (class and message), the
    `Caused by:` lines and everything around the block must be equal to Bazel's.
  - The JDK's process reaper is not ported (a write after the worker exited gives `Stream closed`
    in Java). It is racy in Java and is left as an open item.
- **Exit hooks** now run the rest of `destroy()`:
  - `SingleplexWorker`: `status.setKilled()`, plus a `SandboxedWorker`'s `workDir.deleteTree()`
    (Java's hook calls the virtual `destroy()`).
  - `WorkerMultiplexer`: the work-dir deletion and `setKilled()` of `destroyMultiplexer()`.
  - The 100 ms lock wait and the 10 s SIGKILL limit stay as declared `PORT` items.
- **81 ignored worker tests: not ported in this round.** Root reprioritized the remaining time to
  the worker failures of the e8b79c0 suite run. The list in `tests/worker_unported_tests.rs` is
  unchanged.
- **S4 fixed.** The dead `let _ = &worker_owner;` and `let _ = &self.key;` are removed
  (`WorkerKeyPool.key` is marked as read only by skipped logging).

## Not done

- The 81 ignored worker tests.
- quick-xml was removed from `crates/blaze_exec/Cargo.toml` after root's acceptance (no user is
  left).
- The cgroup port that root named for the worker suites.
  - With `--incompatible_use_new_cgroup_implementation` (default true), Java uses
    `VirtualCgroupFactory("worker_", VirtualCgroup.getInstance(), …, alwaysCreate =
    useCgroupsOnLinux)` for every worker, and `VirtualCgroup.getInstance().memory() != null` for
    the metrics.
  - So the port needed is `lib/sandbox/cgroups` (VirtualCgroup, Hierarchy, Mount, the v1/v2
    controllers, the factory). `CgroupsInfo` is only the old implementation's path.
- Dynamic execution (Part B). A draft of `Branch` / `ShrinkableSemaphore` is in
  `~/.cache/blaze/work/exec-fix-1/dynamic-draft/`, outside the tree.

## Round 2

Implementer: `workers-3` (Opus 5.5). Review: `docs/reviews/2026-11-25-exec-2-round2.md`, STOP at
f71e692. Tree: uncommitted on top of d212972 (HEAD when measured). Binary for the Blaze evidence:
`~/.cache/blaze/work/workers-3/bin/blaze-w3` (sha256 d584d347…), built from that tree; the
pre-fix comparison uses root's suite binary `~/.cache/blaze/work/root-runs/blaze-5f68157`. All
probe files are in `~/.cache/blaze/work/workers-3/`.

### E2-9 fixed (finding confirmed on the oracle)

- `crates/blaze_runtime/src/commands/build.rs`, `run_exclusive_tests`: the entries given to
  `exclusive_test_order::evaluation_result_values_order` are now the roots in Java's order
  (`SkyframeExecutor.evaluateBuildDriverKeys`: the configured-target keys, the aspect keys, then
  the other values such as the workspace status artifact). A key skipped as incompatible is
  added at its place among the roots with its `BuildDriverKey` hash (Java evaluates it and
  `BuildDriverFunction.compute` returns a skipped `BuildDriverValue`). The caller passes the
  full key list and the skipped keys. A check asserts that every evaluated `BuildDriverKey` is
  a root.
- Unit test `order_with_a_skipped_target` (synthetic hashes: three entries keep a table of 4,
  four make it 8 and swap the two tests).
- Oracle evidence. Workspace `excl-ws` (the ten exclusive tests of round 1, plus `skp:s1` and
  `skp2:s2`, `sh_test`s with `target_compatible_with` an unsatisfiable constraint, reached by
  `//skp:all`). Script `excl-ws/run_e29.sh`: 15 cases, 2 to 5 exclusive tests with 0, 1 or 2
  skipped targets.
  - The oracle confirms the finding: `test excl_c excl_e` runs e then c, `test excl_c excl_e
    //skp:all` runs c then e (`e29-oracle.txt`).
  - The reviewer's own example (`excl_b excl_c` plus one skipped target) does not change order
    on the oracle (c then b in both); the hashes of round 1 came from another configuration.
    The finding holds for other pairs.
  - Fixed binary: **15 of 15 orders equal** to the oracle (`e29-blaze.txt`).
  - Pre-fix binary: **8 of 15 differ** (every case with a skipped target where the count
    crosses a table size: `c e`, `c i`, `d e`, `d i` and four five-test sets).
  - Regression check: the 55 orders of round 1 (`run_all.sh`, no skipped target) are all equal
    to the oracle's recorded results with the fixed binary (`excl55-blaze.txt`).
- First runs used `--skip_incompatible_explicit_targets` with explicit targets; that flag is
  part of the configuration, so it changes every key's hash. The evidence above uses wildcards
  and no extra flag.

### E2-11 fixed (finding confirmed on the oracle)

- Oracle run (`xmlws`, three tests writing their own `test.xml`: a UTF-8 document with `\303(`,
  a US-ASCII document with `\351`, and a good one; `test --test_summary=detailed //p:all`): the
  console shows `[Fatal Error] :-1:-1: Byte "233" is not a member of the (7-bit) ASCII character
  set.` and `[Fatal Error] :-1:-1: Invalid byte 2 of 2-byte UTF-8 sequence.`
  (`e211-oracle.err`).
- `xml_stream_reader.rs`: a decoding error now carries the message of the JDK reader that throws
  it, and whether it is a reader's `MalformedByteSequenceException`. When the scanner reaches
  one, `next()` prints `[Fatal Error] :-1:-1: <message>` to `System.err` (`blaze_events::io::
  system_err_println`, the console during a command), once, before the parse fails.
  - `UTF8Reader`: `Invalid byte N of M-byte UTF-8 sequence.`, `Expected byte N of M-byte UTF-8
    sequence.` (at the end of the input), `High surrogate bits in UTF-8 sequence must not exceed
    0x10 but found 0x<hex>.`, classified exactly as the JDK reader's branches
    (`utf8_reader_error`).
  - `ASCIIReader`: `Byte "<n>" is not a member of the (7-bit) ASCII character set.`
  - `UTF16Reader`: an odd last byte is `Expected byte 2 of 2-byte UTF-8 sequence.`
  - An unpaired UTF-16 surrogate is a scanner error (an invalid character), not printed.
- Measured on the oracle's JDK with Bazel's parser: 12 printing documents (`xml/cases5`), whose
  messages equal the code's classification.
- Blaze run, same workspace and command: both lines are printed with the oracle's text; stdout
  is byte-equal. The two lines' relative order depends on which test finishes first, on both
  sides.
- The module comment now says which messages are Java's and printed.

### E2-10 fixed

- Limits of the oracle's JDK, measured with Bazel's parser (`xml/cases6`, `oracle6.txt`):
  - element depth over 100 fails;
  - more than 200 attributes on an element fails; namespace declarations do not count (one
    `xmlns:p` and 200 attributes pass);
  - a name over 1,000 characters fails, counted per part of a qualified name (a 1,000-character
    prefix with a 1,000-character local part passes; a 1,001-character prefix or local part
    fails; a 1,001-character PI target fails).
- All three are ported (`MAX_ELEMENT_DEPTH`, `ELEMENT_ATTRIBUTE_LIMIT`, `check_name_limit`).
  Their messages are not Java's (Bazel drops them, and they are not printed).
- Eager ASCII and UTF-16 errors: the error becomes visible from the start of the chunk that
  holds the bad byte (chunks of 8192 bytes for ASCII, 4096 units for UTF-16, counted from the
  end of the XML declaration).
  - Measured: a bad ASCII byte 8,121 bytes after the declaration is rejected, 9,021 bytes after
    is accepted (`ascii_high_8100`, `ascii_high_9000`). A UTF-16 `<testsuite/>` with an odd
    byte is rejected.
  - `PORT`: Xerces loads a shorter chunk when a token spans the end of its buffer, which moves
    later boundaries. That is not modelled.

### E2-12 fixed

`java_parse_hex_float` ports `FloatingDecimal.parseHexString`: Java's pattern, the binary
exponent required, and correct rounding to `float` (ties to even, subnormals, overflow).
Oracle: `0x1.8p1` is 3000 ms, `0X.8P-3` 63, `-0x1.0p0d` -1000, `0x1.fffffep127` saturates to
2147483647, `0x1.000001p0` and `0x1.0000018p0` are 1000, and `0x1.8` (no exponent) is rejected.

### S3 / S4

- `SingleplexWorker::put_request` carries the `PORT` note for the JDK's process reaper
  (`Stream closed` versus `Broken pipe`, timing dependent in Java).
- The CESU-8 comment is corrected: Xerces rejects `ED A0..BF` as `Invalid byte 2 of 3-byte
  UTF-8 sequence.`, as the port does.

### Tests

- `crates/blaze_exec/tests/test_xml_output_parser_review_test.rs`: **112 documents**, all
  passing. They are the reviewer's 79 (`cases`) and 10 (`cases2`) plus 23 of mine (`cases5`,
  `cases6`), with the oracle's results.
  - Generated by `~/.cache/blaze/work/workers-3/xml/make_review_tests.py`.
  - Not included: `text_big` (20 MB) and `attrs_10001` (`attrs_201` covers that limit).
- The 145 tests of round 1 still pass.
