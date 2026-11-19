"""Tests for the verdicts of the evidence tools outside the referee (referee batch 2):
scripts/gate/check-oracle-tests, scripts/w5r/compare-repos, scripts/remote/matrix_verdict.py and
the step classes of scripts/w4/probe. Nothing here runs a binary or touches ~/.cache/blaze.

Run: python3 -I -m unittest discover -s scripts/gate
"""

import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
import shutil
import tempfile
import sys
import unittest

sys.dont_write_bytecode = True  # do not leave __pycache__ next to the tools loaded below

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPTS = os.path.dirname(HERE)


def load(path, name):
    loader = importlib.machinery.SourceFileLoader(name, path)
    spec = importlib.util.spec_from_loader(name, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


def run(mod, argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        rc = mod.main(list(argv))
    return rc, out.getvalue(), err.getvalue()


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


class CheckOracleTestsTest(unittest.TestCase):
    """S2-9: tests that print "skipped" and return when their recording is absent."""

    def setUp(self):
        self.tool = load(os.path.join(HERE, "check-oracle-tests"), "check_oracle_tests")
        self.root = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.evidence = os.path.join(self.root, "evidence.json")
        write(os.path.join(self.root, "crates", "x", "tests", "t.rs"), (
            "#[test]\nfn t() {\n    let Some(r) = load() else {\n"
            "        eprintln!(\"no oracle recording; skipped\");\n        return;\n    };\n}\n"
            "fn u() {\n    // eprintln!(\"skipped\"); return;\n    println!(\"not found anywhere\");\n}\n"))
        write(os.path.join(self.root, "crates", "x", "src", "lib.rs"),
              "fn f() { eprintln!(\"skipping\"); return; }\n")  # no #[test]: not a test file
        self.table = [("x/tests/t.rs", "no oracle recording", [self.evidence], "t")]

    def test_finds_print_and_return_sites_in_test_files_only(self):
        sites = self.tool.find_sites(os.path.join(self.root, "crates"))
        self.assertEqual([(rel, n) for rel, n, _ in sites], [("x/tests/t.rs", 4)])

    def test_missing_evidence_fails_present_evidence_passes(self):
        saved = list(self.tool.SITES)
        self.addCleanup(lambda: self.tool.SITES.__setitem__(slice(None), saved))
        self.tool.SITES[:] = self.table
        rc, out, _ = run(self.tool, ["--checkout", self.root])
        self.assertEqual(rc, 1)
        self.assertIn("MISSING  %s" % self.evidence, out)
        write(self.evidence, "{}")
        rc, out, _ = run(self.tool, ["--checkout", self.root])
        self.assertEqual(rc, 0, out)
        self.assertIn("sites 1 in 1 files; paths missing 0; unregistered sites 0", out)

    def test_unregistered_site_fails_and_stale_entry_is_reported(self):
        saved = list(self.tool.SITES)
        self.addCleanup(lambda: self.tool.SITES.__setitem__(slice(None), saved))
        self.tool.SITES[:] = [("x/tests/gone.rs", "whatever", [], "gone")]
        rc, out, _ = run(self.tool, ["--checkout", self.root])
        self.assertEqual(rc, 1)
        self.assertIn("UNREGISTERED", out)
        self.assertIn("STALE table entry (no such site any more): crates/x/tests/gone.rs", out)

    def test_every_site_of_this_checkout_is_registered(self):
        results, _ = self.tool.classify(self.tool.find_sites(os.path.join(self.tool.ROOT, "crates")))
        self.assertEqual([s for s, e in results if e is None], [])

    def test_usage(self):
        self.assertEqual(run(self.tool, ["help"])[0], 0)
        self.assertEqual(run(self.tool, ["--bogus"])[0], 2)


class CompareReposVerdictTest(unittest.TestCase):
    """Round 2B: compare-repos must fail on a failed or panicking fetch and on any lockfile diff."""

    @classmethod
    def setUpClass(cls):
        cls.tool = load(os.path.join(SCRIPTS, "w5r", "compare-repos"), "compare_repos")

    def failures(self, run_info=None, match=81, n=81, lock="MATCH", hidden="MATCH"):
        counts = {"MATCH": match, "DIFF": n - match, "MISSING": 0}
        return self.tool.project_failures(run_info if run_info is not None else {"exit_code": 0}, counts, n,
                                          {"status": lock}, {"status": hidden})

    def test_all_match_passes(self):
        self.assertEqual(self.failures(), [])

    def test_the_stored_re2_case_fails(self):
        # The auditor's case: repos 81/81 and workspace lockfile MATCH, fetch panicked, hidden DIFF.
        why = self.failures({"exit_code": 1, "panic": "thread 'x' panicked at stubs.rs:169:9:"}, hidden="DIFF")
        self.assertEqual(why, ["fetch exited 1", "fetch panicked", "hidden lockfile DIFF"])

    def test_each_condition(self):
        self.assertEqual(self.failures({}), ["fetch exit code unknown (no run recorded)"])
        self.assertEqual(self.failures({"exit_code": -9, "timed_out": True}), ["fetch timed out"])
        self.assertEqual(self.failures(match=80), ["repos 80/81 MATCH (DIFF 1, MISSING 0)"])
        self.assertEqual(self.failures(lock="MISSING"), ["workspace lockfile MISSING"])

    def test_log_panic(self):
        d = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        log = os.path.join(d, "run.log")
        write(log, "INFO: ok\n")
        self.assertIsNone(self.tool.log_panic(log))
        write(log, "INFO: ok\nthread 'main' panicked at src/x.rs:1:2:\nboom\n")
        self.assertEqual(self.tool.log_panic(log), "thread 'main' panicked at src/x.rs:1:2:")
        self.assertIsNone(self.tool.log_panic(os.path.join(d, "absent.log")))


MATRIX_OK = """\
D1: INFO: 53 processes: 10 internal, 43 linux-sandbox. [exit exit code: 0]
D2: INFO: 53 processes: 43 disk cache hit, 10 internal. [exit exit code: 0]
D3: INFO: 53 processes: 10 internal, 43 linux-sandbox. [exit exit code: 0]
D4: INFO: 53 processes: 43 disk cache hit, 10 internal. [exit exit code: 0]
C1: INFO: 53 processes: 10 internal, 43 linux-sandbox. [exit exit code: 0]
C2: INFO: 53 processes: 43 remote cache hit, 10 internal. [exit exit code: 0]
C3: INFO: 53 processes: 43 remote cache hit, 10 internal. [exit exit code: 0]
C3 vs C2 gRPC log: RPCs: reference 51, candidate 51, matched 51 (exact comparison; 51 and 51 RPCs) RESULT: MATCH 
C4: INFO: 53 processes: 10 internal, 43 linux-sandbox. [exit exit code: 0]
C4 vs C1 gRPC log: RPCs: reference 295, candidate 295, matched 295 (exact comparison) RESULT: MATCH 
C5: INFO: 53 processes: 43 remote cache hit, 10 internal. [exit exit code: 0]
E1: INFO: 53 processes: 10 internal, 43 remote. [exit exit code: 0]
E2: INFO: 53 processes: 10 internal, 43 remote. [exit exit code: 0]
E2 vs E1 gRPC log: items: reference 1274, candidate 1274, matched 1274 RESULT: MATCH 
matrix t done
"""


def unmatched(entries, ref_cand_rows=(("ActionCache/UpdateActionResult", 21, 21, 0),)):
    lines = ["method                                           reference candidate  matched"]
    for m, a, b, c in ref_cand_rows:
        lines.append("%-48s %9d %9d %8d" % (m, a, b, c))
    for side, mnemonic, action in entries:
        rpc = {"methodName": "build.bazel.remote.execution.v2.ActionCache/UpdateActionResult",
               "metadata": {"actionMnemonic": mnemonic, "actionId": action}}
        lines.append("  [only in %s x1] %s" % (side, json.dumps(rpc)))
    lines.append("RESULT: DIFFERENT")
    return "\n".join(lines) + "\n"


class MatrixVerdictTest(unittest.TestCase):
    """Round 2B: scripts/remote/matrix needs a verdict; DIFFERENT fails unless an allowance holds."""

    @classmethod
    def setUpClass(cls):
        cls.tool = load(os.path.join(SCRIPTS, "remote", "matrix_verdict.py"), "matrix_verdict")
        cls.allow = cls.tool.load_allow(cls.tool.ALLOW_FILE)

    def verdict(self, text, project="re2", unmatched_text=""):
        with contextlib.redirect_stdout(io.StringIO()):
            return self.tool.verdict(project, text, self.allow, lambda step: unmatched_text)

    def test_all_match_passes(self):
        self.assertEqual(self.verdict(MATRIX_OK), [])

    def test_different_without_allowance_fails(self):
        text = MATRIX_OK.replace("matched 1274 RESULT: MATCH", "RESULT: DIFFERENT")
        self.assertEqual(self.verdict(text), ["E2 vs E1 gRPC log: DIFFERENT (no allowance for re2)"])

    def test_allowed_step_holds_only_under_its_condition(self):
        text = MATRIX_OK.replace("matched 295 (exact comparison) RESULT: MATCH", "RESULT: DIFFERENT")
        tests = [(side, "TestRunner", "a%d" % i) for i in range(21) for side in ("reference", "candidate")]
        self.assertEqual(self.verdict(text, unmatched_text=unmatched(tests)), [])
        # The same step on brotli has no allowance.
        self.assertEqual(self.verdict(text, project="brotli", unmatched_text=unmatched(tests)),
                         ["C4 vs C1 gRPC log: DIFFERENT (no allowance for brotli)"])
        # A CppCompile RPC among the unmatched ones breaks the condition.
        why = self.verdict(text, unmatched_text=unmatched(tests + [("reference", "CppCompile", "c")]))
        self.assertEqual(len(why), 1)
        self.assertIn("unmatched ActionCache/UpdateActionResult RPC of a CppCompile action", why[0])
        # More than 21 actions.
        many = [("reference", "TestRunner", "a%d" % i) for i in range(22)]
        self.assertIn("22 actions involved > max_actions 21", self.verdict(text, unmatched_text=unmatched(many))[0])
        # Unequal RPC counts per method.
        why = self.verdict(text, unmatched_text=unmatched(tests, (("ByteStream/Write", 40, 39, 0),)))
        self.assertIn("ByteStream/Write: 40 RPCs in the reference, 39 in the candidate", why[0])
        # No unmatched RPC visible: the allowance cannot be checked.
        self.assertIn("no unmatched RPC found", self.verdict(text, unmatched_text="")[0])

    def test_failed_or_missing_steps_and_unfinished_run_fail(self):
        text = MATRIX_OK.replace("D2: INFO: 53 processes: 43 disk cache hit, 10 internal. [exit exit code: 0]",
                                 "D2: NO PROCESSES LINE [exit exit code: 1]")
        self.assertEqual(self.verdict(text), ["D2: no `N processes` line"])
        text = MATRIX_OK.replace("[exit exit code: 0]\nE2:", "[exit exit code: 2]\nE2:")
        self.assertEqual(self.verdict(text), ["E1: exit code 2"])
        text = "\n".join(l for l in MATRIX_OK.splitlines() if not l.startswith(("C5", "matrix")))
        self.assertEqual(self.verdict(text), ["C5: step missing",
                                              "the run did not finish (no `matrix <tag> done` line)"])

    def test_allow_file_entries_are_complete(self):
        self.assertEqual([(e["id"], e["project"], e["step"]) for e in self.allow],
                         [("re2-cold-test-actions", "re2", "C4 vs C1 gRPC log")])
        with self.assertRaises(ValueError):
            d = tempfile.mkdtemp(prefix="gate-test-")
            self.addCleanup(shutil.rmtree, d, ignore_errors=True)
            write(os.path.join(d, "a.json"), json.dumps({"entries": [{"id": "x", "step": "C4 vs C1 gRPC log"}]}))
            self.tool.load_allow(os.path.join(d, "a.json"))

    def test_usage(self):
        self.assertEqual(run(self.tool, ["help"])[0], 0)
        self.assertEqual(run(self.tool, [])[0], 2)


class ProbeStepClassTest(unittest.TestCase):
    """Round 2B: the probe summary separates steps that run the binary from file checks."""

    @classmethod
    def setUpClass(cls):
        cls.tool = load(os.path.join(SCRIPTS, "w4", "probe"), "probe")

    def test_classes(self):
        c = self.tool.step_class
        self.assertEqual([c("=== 1: run build //..."), c("=== 2: aquery //:a"), c("=== 3: cat bazel-bin/x"),
                          c("=== 4: stat bazel-bin/x")], ["binary", "binary", "file", "file"])

    def test_counts_and_summary_are_split(self):
        want = {"=== 1: run build //:a": ["exit 0", "--- stdout", "--- stderr", "INFO: x"],
                "=== 2: cat out.txt": ["hello"], "=== 3: stat out.txt": ["file 644"]}
        got = {"=== 1: run build //:a": ["exit 0", "--- stdout", "--- stderr", "INFO: y"],
               "=== 2: cat out.txt": ["hello"], "=== 3: stat out.txt": ["file 644"]}
        with contextlib.redirect_stdout(io.StringIO()):
            counts = self.tool.count(want, got)
        self.assertEqual(counts["full"], 2)
        self.assertEqual(counts["classes"]["binary"], {"steps": 1, "full": 0, "masked": 0, "messages": 0})
        self.assertEqual(counts["classes"]["file"], {"steps": 2, "full": 2, "masked": 2, "messages": 2})
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            tot, n = self.tool.print_table([("s", counts, 3)], [("t", "--skip", 7)])
        text = out.getvalue()
        self.assertIn("probe: 1 scenarios, 3 steps; full 2, masked 2, messages 2", text)
        self.assertRegex(text, r"steps that run the binary \(run, aquery\): +1 steps; full 0, masked 0, messages 0")
        self.assertRegex(text, r"file checks \(cat, stat\): +2 steps; full 2, masked 2, messages 2")
        self.assertIn("not counted: t (--skip; 7 oracle steps)", text)
        self.assertEqual(n, 3)


if __name__ == "__main__":
    unittest.main()
