"""Tests for the gate tools. Run: python3 -I -m unittest discover -s scripts/gate"""

import contextlib
import gzip
import importlib.machinery
import importlib.util
import io
import json
import os
import shutil
import subprocess
import tempfile
import unittest

import gate_lib
import parse_test_log

HERE = os.path.dirname(os.path.abspath(__file__))


def load_tool(name):
    loader = importlib.machinery.SourceFileLoader("tool_" + name.replace("-", "_"), os.path.join(HERE, name))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


def run(mod, argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        rc = mod.main(list(argv))
    return rc, out.getvalue(), err.getvalue()


def artifact(package_dir, kind, name, exe, test=True):
    return json.dumps({"reason": "compiler-artifact", "package_id": "path+file:///r/crates/%s#0.1.0" % package_dir,
                       "target": {"kind": [kind], "name": name}, "profile": {"test": test},
                       "executable": exe}, separators=(",", ":"))


LOG = "\n".join([
    artifact("blaze_a", "lib", "blaze_a", "/r/target/debug/deps/blaze_a-1111"),
    artifact("blaze_a", "test", "label_test", "/r/target/debug/deps/label_test-2222"),
    artifact("blaze_b", "test", "label_test", "/r/target/debug/deps/label_test-3333"),
    artifact("blaze_b", "bin", "blaze", "/r/target/debug/deps/blaze-4444"),
    artifact("blaze_b", "lib", "blaze_b", None, test=False),
    '{"reason":"build-finished","success":true}',
    "    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.68s",
    "     Running unittests src/lib.rs (target/debug/deps/blaze_a-1111)",
    "",
    "running 3 tests",
    "test tests::one ... ok",
    "test tests::two ... FAILED",
    "test tests::three ... ignored, needs the harness",
    "",
    "test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s",
    "     Running tests/label_test.rs (target/debug/deps/label_test-2222)",
    "test parse ... ok",
    "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
    "     Running tests/label_test.rs (target/debug/deps/label_test-3333)",
    "test parse ... FAILED",
    "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
    "     Running unittests src/main.rs (target/debug/deps/blaze-4444)",
    "test cli::help - should panic ... ok",
    "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
    "   Doc-tests blaze_b",
    "test crates/blaze_b/src/lib.rs - parse (line 3) ... ok",
    "test crates/blaze_b/src/lib.rs - parse (line 9) ... ok",
    "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
]) + "\n"


class ParseTest(unittest.TestCase):
    def test_names_carry_package_and_target(self):
        results, binaries, problems = parse_test_log.parse(LOG.splitlines(True))
        self.assertEqual(problems, [])
        self.assertEqual(results, {
            "blaze_a::lib::tests::one": "pass",
            "blaze_a::lib::tests::two": "fail",
            "blaze_a::lib::tests::three": "ignored",
            # Two crates with a tests/label_test.rs: both results survive.
            "blaze_a::test/label_test::parse": "pass",
            "blaze_b::test/label_test::parse": "fail",
            "blaze_b::bin/blaze::cli::help - should panic": "pass",
            "blaze_b::doc::crates/blaze_b/src/lib.rs - parse (line 3)": "pass",
            "blaze_b::doc::crates/blaze_b/src/lib.rs - parse (line 9)": "pass",
        })
        self.assertEqual(binaries["blaze_a::lib"], {"pass": 1, "fail": 1, "ignored": 1, "finished": True})
        self.assertEqual(sorted(binaries), ["blaze_a::lib", "blaze_a::test/label_test", "blaze_b::bin/blaze",
                                            "blaze_b::doc", "blaze_b::test/label_test"])

    def test_crashed_executable_is_a_problem(self):
        log = LOG.replace("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n"
                          "     Running tests/label_test.rs (target/debug/deps/label_test-3333)",
                          "     Running tests/label_test.rs (target/debug/deps/label_test-3333)")
        _, binaries, problems = parse_test_log.parse(log.splitlines(True))
        self.assertFalse(binaries["blaze_a::test/label_test"]["finished"])
        self.assertEqual(len(problems), 1)
        self.assertIn("blaze_a::test/label_test printed no `test result:` line", problems[0])

    def test_interleaved_stderr_does_not_lose_a_name(self):
        log = "\n".join([
            artifact("blaze_a", "test", "t", "/r/target/debug/deps/t-1"),
            "     Running tests/t.rs (target/debug/deps/t-1)",
            "running 4 tests",
            "test glued_after_status ... okExtracting Bazel installation...",
            "",
            "test status_on_a_later_line ... Corrupt installation: file is missing",
            "more foreign text",
            "FAILED",
            "test plain ... ok",
            "test skipped ... ignored, needs the harness",
            "",
            "failures:",
            "",
            "---- status_on_a_later_line stdout ----",
            "test nested_harness_output ... ok",
            "",
            "test result: FAILED. 2 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s",
        ]) + "\n"
        results, binaries, problems = parse_test_log.parse(log.splitlines(True))
        self.assertEqual(problems, [])
        self.assertEqual(results, {"blaze_a::test/t::glued_after_status": "pass",
                                   "blaze_a::test/t::status_on_a_later_line": "fail",
                                   "blaze_a::test/t::plain": "pass", "blaze_a::test/t::skipped": "ignored"})

    def test_counts_that_do_not_add_up_are_a_problem(self):
        log = "\n".join([
            artifact("blaze_a", "test", "t", "/r/target/debug/deps/t-1"),
            "     Running tests/t.rs (target/debug/deps/t-1)",
            "foreign text without a newline test lost ... ok",
            "test kept ... ok",
            "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
            "     Running tests/t.rs (target/debug/deps/t-1)",
            "test no_status ... foreign",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
        ]) + "\n"
        results, _, problems = parse_test_log.parse(log.splitlines(True))
        self.assertEqual(results, {"blaze_a::test/t::kept": "pass"})
        self.assertEqual(len(problems), 4, problems)
        self.assertIn("parsed 1 passed, 0 failed, 0 ignored, but its `test result:` line says 2, 0, 0", problems[0])
        self.assertIn("ran more than once", problems[1])
        self.assertIn("parsed 0 passed, 0 failed, 0 ignored, but its `test result:` line says 1, 0, 0", problems[2])
        self.assertIn("blaze_a::test/t::no_status has no status in the log", problems[3])

    def test_unannounced_executable_is_a_problem(self):
        log = ("     Running tests/x.rs (target/debug/deps/x-9999)\ntest a ... ok\n"
               "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n")
        results, _, problems = parse_test_log.parse(log.splitlines(True))
        self.assertEqual(results, {"UNKNOWN::x-9999::a": "pass"})
        self.assertIn("was not announced by a compiler-artifact message", problems[0])

    def test_build_failure_gives_no_results(self):
        log = "error[E0063]: missing fields\nerror: could not compile `blaze_x` (lib) due to 1 previous error\n"
        self.assertEqual(parse_test_log.parse(log.splitlines(True)), ({}, {}, []))

    def test_package_names(self):
        self.assertEqual(parse_test_log.package_name("path+file:///r/crates/blaze_util#0.1.0"), "blaze_util")
        self.assertEqual(parse_test_log.package_name("path+file:///r/dir#name@0.1.0"), "name")
        self.assertEqual(parse_test_log.package_name("registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0"), "serde")
        self.assertEqual(parse_test_log.package_name("blaze_util 0.1.0 (path+file:///r/crates/blaze_util)"), "blaze_util")

    def test_document_names_the_tree(self):
        root = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, root, ignore_errors=True)
        env = {"PATH": "/usr/bin:/bin", "HOME": root, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.invalid",
               "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.invalid"}
        for cmd in (["init", "-q"], ["commit", "-q", "--allow-empty", "-m", "x"]):
            subprocess.run(["git", "-C", root] + cmd, check=True, env=env, stdout=subprocess.DEVNULL)
        doc = parse_test_log.document(LOG.splitlines(True), root, 101)
        self.assertRegex(doc["commit"], "^[0-9a-f]{40}$")
        self.assertEqual((doc["tree_dirty"], doc["dirty_diff_sha256"], doc["cargo_exit"]), (False, None, 101))
        self.assertEqual(doc["counts"], {"pass": 5, "fail": 2, "ignored": 1})
        self.assertEqual(list(doc["results"]), sorted(doc["results"]))
        with open(os.path.join(root, "new.rs"), "w") as f:
            f.write("x")
        doc = parse_test_log.document([], root)
        self.assertTrue(doc["tree_dirty"])
        self.assertRegex(doc["dirty_diff_sha256"], "^[0-9a-f]{64}$")


class CompareTestsTest(unittest.TestCase):
    BASE = {"a": "pass", "b": "pass", "c": "pass", "d": "fail", "e": "ignored", "old": "pass"}
    NEW = {"a": "pass", "b": "fail", "d": "pass", "e": "ignored", "f": "pass", "renamed": "pass"}

    def test_rule(self):
        cmp = gate_lib.compare_tests(self.BASE, self.NEW)
        self.assertEqual(cmp, {"retained": 1, "recovered": ["d"], "added": ["f", "renamed"],
                               "lost": [("b", "fail")], "absent": ["c", "old"]})
        self.assertEqual(gate_lib.regressions(cmp), 3)

    def test_ignoring_a_passing_test_is_a_loss(self):
        cmp = gate_lib.compare_tests({"a": "pass"}, {"a": "ignored"})
        self.assertEqual(cmp["lost"], [("a", "ignored")])

    def test_new_passes_do_not_compensate(self):
        cmp = gate_lib.compare_tests({"a": "pass"}, {"x%d" % i: "pass" for i in range(100)})
        self.assertEqual(gate_lib.regressions(cmp), 1)

    def test_name_map(self):
        cmp = gate_lib.compare_tests(self.BASE, self.NEW, {"old": "renamed"})
        self.assertEqual(cmp["absent"], ["c"])
        self.assertEqual(cmp["added"], ["f"])
        self.assertEqual(cmp["retained"], 2)

    def write(self, d, name, results, gz=False, **extra):
        doc = {"commit": "c" * 40, "tree_dirty": False, "results": results,
               "counts": {k: sum(1 for v in results.values() if v == k) for k in ("pass", "fail", "ignored")}, **extra}
        path = os.path.join(d, name)
        data = gate_lib.canonical_json(doc)
        with open(path, "wb") as f:
            f.write(gate_lib.gzip_bytes(data) if gz else data)
        return path

    def test_tool_exit_status_and_gzip_input(self):
        d = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        tool = load_tool("compare-tests")
        base = self.write(d, "base.json.gz", self.BASE, gz=True)
        new = self.write(d, "new.json", self.NEW, problems=["x crashed"])
        rc, out, _ = run(tool, [base, new])
        self.assertEqual(rc, 1)
        self.assertIn("retained 1  recovered 1  new-pass 2  lost 1  absent 2", out)
        self.assertIn("  LOST   b -> fail", out)
        self.assertIn("  ABSENT c", out)
        self.assertIn("PROBLEM in new: x crashed", out)
        self.assertIn("result: FAIL (3 names that passed no longer pass)", out)
        rc, out, _ = run(tool, [base, base])
        self.assertEqual(rc, 0)
        self.assertIn("result: PASS", out)
        with open(os.path.join(d, "map.tsv"), "w") as f:
            f.write("# renames\nold\trenamed\tmoved to another module\n")
        rc, out, _ = run(tool, [base, new, "--name-map", os.path.join(d, "map.tsv")])
        self.assertIn("lost 1  absent 1", out)
        self.assertEqual(run(tool, [base])[0], 2)
        self.assertEqual(run(tool, ["help"])[0], 0)
        with open(os.path.join(d, "bad.tsv"), "w") as f:
            f.write("old\trenamed\n")
        self.assertEqual(run(tool, [base, new, "--name-map", os.path.join(d, "bad.tsv")])[0], 2)


class BaselineTest(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, self.d, ignore_errors=True)
        self.tool = load_tool("baseline")
        self.out = os.path.join(self.d, "state", "tests-baseline.json.gz")

    def result(self, name, results, **over):
        doc = {"commit": "c" * 40, "tree_dirty": False, "dirty_diff_sha256": None, "cargo_exit": 0,
               "counts": {k: sum(1 for v in results.values() if v == k) for k in ("pass", "fail", "ignored")},
               "binaries": {"p::lib": {"pass": 1, "fail": 0, "ignored": 0, "finished": True}},
               "problems": [], "results": results, "log": "/somewhere/else"}
        doc.update(over)
        path = os.path.join(self.d, name)
        with open(path, "w") as f:
            json.dump(doc, f)  # unsorted, compact: the baseline must not depend on this formatting
        return path

    def accept(self, path, *args):
        return run(self.tool, [path, "--out", self.out, *args])

    def test_written_deterministically_with_sha256(self):
        src = self.result("r.json", {"b": "pass", "a": "fail"})
        rc, out, err = self.accept(src)
        self.assertEqual(rc, 0, err)
        with open(self.out, "rb") as f:
            first = f.read()
        self.assertEqual(first[:4], b"\x1f\x8b\x08\x00")            # gzip, no FNAME flag
        self.assertEqual(first[4:8], b"\x00\x00\x00\x00")           # no timestamp
        doc = json.loads(gzip.decompress(first))
        self.assertEqual(list(doc["results"]), ["a", "b"])
        self.assertEqual(doc["commit"], "c" * 40)
        self.assertNotIn("log", doc)
        self.assertEqual(doc["accepted_losses"], [])
        with open(self.out + ".sha256") as f:
            self.assertEqual(f.read(), "%s  tests-baseline.json.gz\n" % gate_lib.sha256(first))
        os.remove(self.out)
        other = self.result("r2.json", {"a": "fail", "b": "pass"})
        self.assertEqual(self.accept(other)[0], 0)
        with open(self.out, "rb") as f:
            self.assertEqual(f.read(), first)
        self.assertEqual(gate_lib.load_results(self.out)["results"], {"a": "fail", "b": "pass"})

    def test_refuses_dirty_tree_problems_and_empty_results(self):
        rc, _, err = self.accept(self.result("r.json", {"a": "pass"}, tree_dirty=True, dirty_diff_sha256="d" * 64))
        self.assertEqual(rc, 1)
        self.assertIn("dirty tree", err)
        self.assertFalse(os.path.exists(self.out))
        self.assertEqual(self.accept(self.result("r.json", {"a": "pass"}, tree_dirty=True), "--allow-dirty")[0], 0)
        self.assertTrue(gate_lib.load_results(self.out)["tree_dirty"])
        os.remove(self.out)
        rc, _, err = self.accept(self.result("r.json", {"a": "pass"}, problems=["p::lib crashed"]))
        self.assertEqual(rc, 1)
        self.assertIn("p::lib crashed", err)
        rc, _, err = self.accept(self.result("r.json", {}))
        self.assertEqual(rc, 1)
        self.assertIn("no passing test", err)
        rc, _, err = self.accept(self.result("r.json", {"a": "pass"}, commit=None))
        self.assertEqual(rc, 1)

    def test_no_name_that_passed_may_be_lost(self):
        self.assertEqual(self.accept(self.result("r1.json", {"a": "pass", "b": "pass", "c": "fail"}))[0], 0)
        with open(self.out, "rb") as f:
            before = f.read()
        worse = self.result("r2.json", {"a": "pass", "c": "pass", "d": "pass"}, commit="e" * 40)
        rc, out, err = self.accept(worse)
        self.assertEqual(rc, 1)
        self.assertIn("ABSENT b", out)
        self.assertIn("1 names that pass in the current baseline no longer pass", err)
        with open(self.out, "rb") as f:
            self.assertEqual(f.read(), before)
        rc, out, err = self.accept(worse, "--allow-loss", "b tested a seam that was removed (batch R7)")
        self.assertEqual(rc, 0, err)
        doc = gate_lib.load_results(self.out)
        self.assertEqual(doc["commit"], "e" * 40)
        self.assertEqual(doc["accepted_losses"], [{"name": "b", "now": "absent", "since_commit": "c" * 40,
                                                   "reason": "b tested a seam that was removed (batch R7)"}])
        # A better result is accepted without flags.
        better = self.result("r3.json", {"a": "pass", "c": "pass", "d": "pass", "e": "pass"}, commit="f" * 40)
        self.assertEqual(self.accept(better)[0], 0)

    def test_show_checks_the_sha256(self):
        self.accept(self.result("r.json", {"a": "pass"}))
        rc, out, _ = run(self.tool, ["show", self.out])
        self.assertEqual(rc, 0)
        self.assertIn("commit %s (clean tree)" % ("c" * 40), out)
        self.assertIn("pass 1, fail 0, ignored 0; 1 names", out)
        with open(self.out, "ab") as f:
            f.write(b"\0")
        rc, out, _ = run(self.tool, ["show", self.out])
        self.assertEqual(rc, 1)
        self.assertIn("DOES NOT MATCH", out)
        self.assertEqual(run(self.tool, ["show", os.path.join(self.d, "none.gz")])[0], 1)

    def test_usage(self):
        self.assertEqual(run(self.tool, ["help"])[0], 0)
        self.assertEqual(run(self.tool, [])[0], 2)
        self.assertEqual(run(self.tool, ["--bogus"])[0], 2)

    def test_committed_baseline_is_intact_if_present(self):
        if not os.path.isfile(gate_lib.BASELINE):
            self.skipTest("no committed baseline yet")
        rc, out, _ = run(self.tool, ["show"])
        self.assertEqual(rc, 0, out)
        doc = gate_lib.load_results(gate_lib.BASELINE)
        with open(gate_lib.BASELINE, "rb") as f:
            self.assertEqual(f.read(), gate_lib.gzip_bytes(gate_lib.canonical_json(doc)))


def score(rows, referee="r" * 64, **over):
    doc = {"label": "blaze-x", "reference": "oracle-9.3.2", "referee_sha256": referee, "restriction":
           {"projects": None, "surfaces": None}, "verdict": "FAIL", "rows": [], "provenance": {}, "staged": None,
           "totals": {"total": 0, "matching": 0, "allowed": 0, "failed": 0}}
    for project, surface, status, total, matching in rows:
        doc["rows"].append({"project": project, "surface": surface, "status": status, "total": total,
                            "matching": matching, "exceptions": [], "allow_violations": []})
        doc["totals"]["total"] += total
        doc["totals"]["matching"] += matching
    doc.update(over)
    return doc


class CompareScoresTest(unittest.TestCase):
    def setUp(self):
        self.tool = load_tool("compare-scores")
        self.base = score([("brotli", "query", "MATCH", 10, 10), ("brotli", "aquery", "DIFF", 60, 50),
                           ("re2", "bep", "MISSING", 100, 0)])

    def test_staged_or_unmarked_scores_are_not_comparable(self):
        # Audit of referee batch 3b (B1): a staged score, or one that cannot say, is refused.
        same = score([(r["project"], r["surface"], r["status"], r["total"], r["matching"]) for r in self.base["rows"]])
        blockers, _ = self.tool.compare(self.base, same)
        self.assertEqual(blockers, [])
        staged = dict(same, staged={"stage": "/x", "records": "/x/r", "summaries": "/x/s"})
        blockers, _ = self.tool.compare(self.base, staged)
        self.assertTrue(any("staging area" in b for b in blockers), blockers)
        unmarked = {k: v for k, v in same.items() if k != "staged"}
        blockers, _ = self.tool.compare(unmarked, same)
        self.assertTrue(any("no `staged` field" in b for b in blockers), blockers)
        warned = dict(same, warnings=["staged: records from /x"])
        blockers, _ = self.tool.compare(self.base, warned)
        self.assertTrue(any("`staged:` warning" in b for b in blockers), blockers)

    def test_no_regression(self):
        new = score([("brotli", "query", "MATCH", 10, 10), ("brotli", "aquery", "MATCH", 60, 60),
                     ("re2", "bep", "DIFF", 100, 40), ("re2", "tests", "MATCH", 21, 21)])
        self.assertEqual(self.tool.compare(self.base, new), ([], []))

    def test_regressions(self):
        new = score([("brotli", "query", "DIFF", 10, 9), ("brotli", "aquery", "DIFF", 60, 49)])
        blockers, bad = self.tool.compare(self.base, new)
        self.assertEqual(blockers, [])
        self.assertEqual(bad, ["brotli/query: status MATCH -> DIFF", "brotli/aquery: matching 50 -> 49 of 60",
                               "re2/bep: row is gone (was MISSING, matching 0 of 100)"])
        new = score([("brotli", "query", "MATCH", 10, 10), ("brotli", "aquery", "FAILED", 60, 0),
                     ("re2", "bep", "MISSING", 100, 0)])
        self.assertEqual(self.tool.compare(self.base, new)[1], ["brotli/aquery: status DIFF -> FAILED"])

    def test_gain_elsewhere_does_not_compensate(self):
        new = score([("brotli", "query", "MATCH", 10, 10), ("brotli", "aquery", "DIFF", 60, 49),
                     ("re2", "bep", "MATCH", 100, 100)])
        self.assertEqual(len(self.tool.compare(self.base, new)[1]), 1)

    def test_other_referee_or_reference_is_not_comparable(self):
        new = score([(r["project"], r["surface"], r["status"], r["total"], r["matching"]) for r in self.base["rows"]],
                    referee="s" * 64)
        self.assertEqual(len(self.tool.compare(self.base, new)[0]), 1)
        new = score([("brotli", "query", "MATCH", 11, 11), ("brotli", "aquery", "DIFF", 60, 50),
                     ("re2", "bep", "MISSING", 100, 0)])
        self.assertEqual(self.tool.compare(self.base, new)[0], ["brotli/query: reference total changed 10 -> 11"])

    def test_tool(self):
        d = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        paths = []
        for name, doc in (("base.json", self.base),
                          ("worse.json", score([("brotli", "query", "DIFF", 10, 9), ("brotli", "aquery", "DIFF", 60, 50),
                                                ("re2", "bep", "MISSING", 100, 0)])),
                          ("other.json", dict(self.base, referee_sha256="s" * 64))):
            paths.append(os.path.join(d, name))
            with open(paths[-1], "w") as f:
                json.dump(doc, f)
        self.assertEqual(run(self.tool, [paths[0], paths[0]])[0], 0)
        rc, out, _ = run(self.tool, [paths[0], paths[1]])
        self.assertEqual(rc, 1)
        self.assertIn("REGRESSION brotli/query: status MATCH -> DIFF", out)
        rc, out, _ = run(self.tool, [paths[0], paths[2]])
        self.assertEqual(rc, 2)
        self.assertIn("NOT COMPARABLE: different referees", out)
        self.assertEqual(run(self.tool, ["help"])[0], 0)
        self.assertEqual(run(self.tool, [paths[0]])[0], 2)


def keyed(doc, keys):
    """Give each row of a score document `matching_keys` from {(project, surface): [digest, ...]}."""
    for r in doc["rows"]:
        r["matching_keys"] = " ".join(sorted(keys.get((r["project"], r["surface"]), [])))
    return doc


class CompareScoreNamesTest(unittest.TestCase):
    """Audit R2A-4: the ratchet is on item names, not only on counts."""

    def setUp(self):
        self.tool = load_tool("compare-scores")
        self.base = keyed(score([("brotli", "query", "DIFF", 3, 2)]), {("brotli", "query"): ["aaa", "bbb"]})

    def test_same_count_different_items_is_a_regression(self):
        new = keyed(score([("brotli", "query", "DIFF", 3, 2)]), {("brotli", "query"): ["aaa", "ccc"]})
        self.assertEqual(self.tool.compare(self.base, new)[1], [
            "brotli/query: 1 item that matched no longer matches (matching 2 -> 2; item key digests bbb)"])

    def test_superset_passes(self):
        new = keyed(score([("brotli", "query", "MATCH", 3, 3)]), {("brotli", "query"): ["aaa", "bbb", "ccc"]})
        self.assertEqual(self.tool.compare(self.base, new), ([], []))
        self.assertEqual(self.tool.count_only_rows(self.base, new), [])

    def test_old_scores_fall_back_to_counts_with_a_warning(self):
        old = score([("brotli", "query", "DIFF", 3, 2)])
        new = keyed(score([("brotli", "query", "DIFF", 3, 2)]), {("brotli", "query"): ["aaa", "ccc"]})
        self.assertEqual(self.tool.compare(old, new), ([], []))
        self.assertEqual(self.tool.count_only_rows(old, new), ["brotli/query"])
        d = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        paths = []
        for name, doc in (("old.json", old), ("new.json", new), ("base.json", self.base)):
            paths.append(os.path.join(d, name))
            with open(paths[-1], "w") as f:
                json.dump(doc, f)
        rc, out, _ = run(self.tool, [paths[0], paths[1]])
        self.assertEqual(rc, 0)
        self.assertIn("WARNING: 1 of 1 rows are compared by count only", out)
        rc, out, _ = run(self.tool, [paths[2], paths[1]])
        self.assertEqual(rc, 1)
        self.assertIn("REGRESSION brotli/query: 1 item that matched no longer matches", out)

    def test_recording_problems_and_allow_dirty_are_printed(self):
        new = dict(self.base, recording_problems=["blaze-x/re2: recorded from a dirty tree (diff 0123)"],
                   allow_dirty="ruling 3")
        d = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        a, b = os.path.join(d, "a.json"), os.path.join(d, "b.json")
        for path, doc in ((a, self.base), (b, new)):
            with open(path, "w") as f:
                json.dump(doc, f)
        rc, out, _ = run(self.tool, [a, b])
        self.assertEqual(rc, 0)
        self.assertIn("NOTE: new score: blaze-x/re2: recorded from a dirty tree", out)
        self.assertIn("NOTE: new score was written with --allow-dirty: ruling 3", out)


class RunTestsTest(unittest.TestCase):
    def test_help(self):
        p = subprocess.run([os.path.join(HERE, "run-tests"), "help"], stdout=subprocess.PIPE)
        self.assertEqual(p.returncode, 0)
        self.assertIn(b"Usage: scripts/gate/run-tests <out.json> [--checkout DIR]", p.stdout)
        self.assertEqual(subprocess.run([os.path.join(HERE, "run-tests")], stdout=subprocess.DEVNULL).returncode, 2)


if __name__ == "__main__":
    unittest.main()
