"""Tests for what a recording says about itself: commit binding, label reuse, declared exceptions,
and the staged rebuild of norm/ and items/.

Run: python3 -I -m unittest discover -s scripts/oracle
"""

import contextlib
import io
import os
import shutil
import stat
import subprocess
import tempfile
import unittest

import oracle_common as oc
import surfaces
from fixture_records import Records, load_tool, run_main

record = load_tool("record")


def git(root, *args):
    env = {"PATH": "/usr/bin:/bin", "HOME": root, "GIT_CONFIG_NOSYSTEM": "1",
           "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.invalid",
           "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.invalid"}
    return subprocess.run(["git", "-C", root] + list(args), check=True, env=env,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout.decode().strip()


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


class RepoStateTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="oracle-test-")
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        git(self.root, "init", "-q")
        write(os.path.join(self.root, "a.rs"), "fn a() {}\n")
        git(self.root, "add", "a.rs")
        git(self.root, "commit", "-q", "-m", "first")
        self.head = git(self.root, "rev-parse", "HEAD")

    def test_clean_tree(self):
        self.assertEqual(oc.repo_state(self.root),
                         {"commit": self.head, "dirty": False, "dirty_paths": 0, "dirty_diff_sha256": None})

    def test_modified_file_makes_it_dirty_and_the_hash_follows_the_content(self):
        write(os.path.join(self.root, "a.rs"), "fn a() { 1 }\n")
        one = oc.repo_state(self.root)
        self.assertEqual((one["commit"], one["dirty"], one["dirty_paths"]), (self.head, True, 1))
        self.assertRegex(one["dirty_diff_sha256"], "^[0-9a-f]{64}$")
        self.assertEqual(oc.repo_state(self.root), one)  # stable
        write(os.path.join(self.root, "a.rs"), "fn a() { 2 }\n")
        self.assertNotEqual(oc.repo_state(self.root)["dirty_diff_sha256"], one["dirty_diff_sha256"])

    def test_staged_and_untracked_files_count(self):
        write(os.path.join(self.root, "new.rs"), "one\n")
        untracked = oc.repo_state(self.root)
        self.assertTrue(untracked["dirty"])
        write(os.path.join(self.root, "new.rs"), "two\n")
        changed = oc.repo_state(self.root)
        self.assertNotEqual(changed["dirty_diff_sha256"], untracked["dirty_diff_sha256"])
        git(self.root, "add", "new.rs")
        self.assertTrue(oc.repo_state(self.root)["dirty"])
        git(self.root, "commit", "-q", "-m", "second")
        self.assertFalse(oc.repo_state(self.root)["dirty"])

    def test_outside_a_checkout(self):
        d = tempfile.mkdtemp(prefix="oracle-test-")
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        state = oc.repo_state(d)
        self.assertEqual(state, {"commit": None, "dirty": None, "dirty_paths": None, "dirty_diff_sha256": None})

    def test_this_checkout(self):
        state = oc.repo_state()
        self.assertRegex(state["commit"], "^[0-9a-f]{40}$")
        self.assertIsInstance(state["dirty"], bool)


class RecordRunTest(unittest.TestCase):
    """Runs `record` end to end with a stand-in binary on a scratch project."""

    def setUp(self):
        self.rec = Records(self)
        root = self.rec.root
        self.checkout = os.path.join(root, "checkout")
        write(os.path.join(self.checkout, "BUILD"), "")
        saved = (oc.WORK, oc.OUTPUT_ROOTS, oc.load_projects, record.maybe_reexec_in_scope, oc.repo_state)
        oc.WORK = os.path.join(root, "work")
        oc.OUTPUT_ROOTS = os.path.join(root, "out")
        oc.load_projects = lambda: {"toy": {"checkout": self.checkout, "commit": "0" * 40,
                                            "commands": [["build", "//..."]]},
                                    "toy2": {"checkout": self.checkout, "commit": "0" * 40,
                                             "commands": [["build", "//..."]]}}
        record.maybe_reexec_in_scope = lambda args: None
        self.state = {"commit": "c" * 40, "dirty": True, "dirty_paths": 2, "dirty_diff_sha256": "d" * 64}
        oc.repo_state = lambda root=None: dict(self.state)

        def restore():
            oc.WORK, oc.OUTPUT_ROOTS, oc.load_projects, record.maybe_reexec_in_scope, oc.repo_state = saved

        self.addCleanup(restore)
        self.binary = self.fake_binary("one", "//:a")
        self.other = self.fake_binary("two", "//:b")

    def fake_binary(self, name, label):
        """A stand-in for bazel: `query` prints one label, every other command prints nothing."""
        path = os.path.join(self.rec.root, "bin-" + name)
        write(path, "#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in query) echo 'cc_library rule %s';; esac; done\n" % label)
        os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
        return path

    def run_record(self, *argv):
        return run_main(record, list(argv))

    def test_manifest_binds_the_recording_to_commit_tree_binary_and_date(self):
        rc, out, err = self.run_record("--binary", self.binary, "--label", "blaze-x", "--project", "toy",
                                       "--surfaces", "query")
        self.assertEqual(rc, 0, out + err)
        m = oc.load_manifest("blaze-x", "toy")
        self.assertEqual(m["blaze_repo"], self.state)
        self.assertEqual(m["binary"], self.binary)
        self.assertEqual(m["binary_sha256"], oc.sha256_file(self.binary))
        self.assertRegex(m["created"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\+00:00$")
        self.assertEqual(m["extractor_sha256"], oc.extractor_sha256())
        self.assertNotIn("replaced", m)
        self.assertNotIn("renormalised", m)
        self.assertEqual(m["surfaces"]["query_label_kind"], {"status": "OK", "items": 4})
        self.assertFalse(os.path.exists(os.path.join(oc.record_dir("blaze-x", "toy"), ".stage")))
        for name, digest in m["files"].items():
            self.assertEqual(oc.sha256_file(os.path.join(oc.record_dir("blaze-x", "toy"), name)), digest, name)
        self.assertIn("items/query_label_kind.json", m["files"])
        self.assertIn("raw/query_label_kind.stdout", m["files"])

    def test_a_label_is_never_silently_reused(self):
        args = ["--binary", self.binary, "--label", "blaze-x", "--project", "toy", "--surfaces", "query"]
        self.assertEqual(self.run_record(*args)[0], 0)
        before = oc.load_manifest("blaze-x", "toy")
        rc, _, err = self.run_record(*args)
        self.assertEqual(rc, 2)
        self.assertIn("label 'blaze-x' is already in use, and labels are never reused", err)
        self.assertIn("pass --force", err)
        self.assertEqual(oc.load_manifest("blaze-x", "toy"), before)
        # --force replaces it, and the new manifest says what it replaced.
        self.state = {"commit": "e" * 40, "dirty": False, "dirty_paths": 0, "dirty_diff_sha256": None}
        self.assertEqual(self.run_record(*args, "--force")[0], 0)
        m = oc.load_manifest("blaze-x", "toy")
        self.assertEqual(m["blaze_repo"]["commit"], "e" * 40)
        self.assertEqual(m["replaced"], {"created": before["created"], "binary_sha256": before["binary_sha256"],
                                         "blaze_repo": before["blaze_repo"]})

    def test_an_unfinished_recording_also_blocks_the_label(self):
        os.makedirs(oc.record_dir("blaze-x", "toy"))
        rc, _, err = self.run_record("--binary", self.binary, "--label", "blaze-x", "--project", "toy")
        self.assertEqual(rc, 2)
        self.assertIn("exists without a manifest", err)

    def test_a_label_names_one_binary(self):
        self.assertEqual(self.run_record("--binary", self.binary, "--label", "blaze-x", "--project", "toy",
                                         "--surfaces", "query")[0], 0)
        rc, _, err = self.run_record("--binary", self.other, "--label", "blaze-x", "--project", "toy2",
                                     "--surfaces", "query")
        self.assertEqual(rc, 2)
        self.assertIn("already holds toy recorded with another binary", err)
        self.assertIsNone(oc.load_manifest("blaze-x", "toy2"))
        # The same binary may add the label's second project.
        self.assertEqual(self.run_record("--binary", self.binary, "--label", "blaze-x", "--project", "toy2",
                                         "--surfaces", "query")[0], 0)
        self.assertEqual(oc.projects_in("blaze-x"), ["toy", "toy2"])

    def test_renormalise_keeps_a_history_and_does_not_need_force(self):
        self.assertEqual(self.run_record("--binary", self.binary, "--label", "blaze-x", "--project", "toy",
                                         "--surfaces", "query")[0], 0)
        before = oc.load_manifest("blaze-x", "toy")
        rc, out, err = self.run_record("--renormalise", "--label", "blaze-x", "--project", "toy")
        self.assertEqual(rc, 0, out + err)
        after = oc.load_manifest("blaze-x", "toy")
        self.assertEqual(len(after["renormalised"]), 1)
        self.assertEqual(after["renormalised"][0]["extractor_sha256"], oc.extractor_sha256())
        self.assertEqual(after["files"], before["files"])
        self.assertEqual(after["blaze_repo"], before["blaze_repo"])
        self.assertEqual(after["created"], before["created"])

    def test_a_failed_rebuild_leaves_the_recording_as_it_was(self):
        self.assertEqual(self.run_record("--binary", self.binary, "--label", "blaze-x", "--project", "toy",
                                         "--surfaces", "query")[0], 0)
        rd = oc.record_dir("blaze-x", "toy")
        before = oc.load_manifest("blaze-x", "toy")
        real = surfaces.extract_all

        def boom(rd_, manifest, ctx):
            real(rd_, manifest, ctx)
            raise MemoryError("out of memory half way")

        surfaces.extract_all = boom
        self.addCleanup(setattr, surfaces, "extract_all", real)
        with self.assertRaises(MemoryError), contextlib.redirect_stdout(io.StringIO()):
            record.finish(rd, renormalise=True)
        self.assertEqual(oc.load_manifest("blaze-x", "toy"), before)
        for name, digest in before["files"].items():
            self.assertEqual(oc.sha256_file(os.path.join(rd, name)), digest, name)
        # The next rebuild starts from a clean stage and does not follow its link into raw/.
        surfaces.extract_all = real
        with contextlib.redirect_stdout(io.StringIO()):
            record.finish(rd, renormalise=True)
        self.assertFalse(os.path.exists(os.path.join(rd, ".stage")))
        self.assertTrue(os.path.isfile(os.path.join(rd, "raw", "query_label_kind.stdout")))


class ExceptionsTest(unittest.TestCase):
    GOOD = {"id": "aquery-spliced", "kind": "spliced", "surfaces": ["aquery_actions", "exit_codes"],
            "explanation": "why"}

    def test_well_formed(self):
        self.assertEqual(record.validate_exceptions({"exceptions": [self.GOOD]}), [self.GOOD])
        self.assertEqual(record.validate_exceptions({}), [])

    def test_malformed_exceptions_are_refused(self):
        for bad in ("text", {**self.GOOD, "surfaces": []}, {**self.GOOD, "surfaces": ["nope"]},
                    {**self.GOOD, "explanation": " "}, {k: v for k, v in self.GOOD.items() if k != "id"},
                    {k: v for k, v in self.GOOD.items() if k != "kind"}):
            with self.assertRaises(SystemExit, msg=repr(bad)):
                with contextlib.redirect_stderr(io.StringIO()):
                    record.validate_exceptions({"exceptions": [bad]})

    def test_summary_carries_exceptions_and_commit(self):
        rec = Records(self)
        repo = {"commit": "c" * 40, "dirty": False, "dirty_paths": 0, "dirty_diff_sha256": None}
        rec.add(oc.REFERENCE_LABEL, "workerd", {"aquery_actions": {"o": {}}},
                manifest={"exceptions": [self.GOOD], "blaze_repo": repo,
                          "commands": [{"name": "info", "exit_code": 0, "duration_s": 0.1}]})
        record.write_summary(oc.REFERENCE_LABEL, "workerd")
        s = oc.read_json(os.path.join(oc.CORPUS_SUMMARIES, "workerd.json"))
        self.assertEqual(s["exceptions"], [self.GOOD])
        self.assertEqual(s["blaze_repo"], repo)
        self.assertEqual(s["extractor_sha256"], oc.extractor_sha256())
        self.assertIn("items/aquery_actions.json", s["files_sha256"])

    def test_the_committed_workerd_summary_declares_its_splice(self):
        s = oc.read_json(os.path.join(oc.REPO_ROOT, "corpus", "oracle", "workerd.json"))
        self.assertEqual(record.validate_exceptions(s), s["exceptions"])
        splice = [e for e in s["exceptions"] if e["kind"] == "spliced"]
        self.assertEqual(len(splice), 1)
        self.assertEqual(sorted(splice[0]["surfaces"]), ["aquery_actions", "exit_codes"])
        self.assertEqual(splice[0]["protocol_exit_code"], 8)
        self.assertEqual(splice[0]["stored_exit_code"], 0)
        self.assertEqual(s["exit_codes"]["aquery_deps_jsonproto"], splice[0]["stored_exit_code"])


if __name__ == "__main__":
    unittest.main()
