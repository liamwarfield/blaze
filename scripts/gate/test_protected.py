"""Tests for the protected-path rule and the git hooks. Run: python3 -I -m unittest discover -s scripts/gate"""

import os
import shutil
import subprocess
import tempfile
import unittest

import protected

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
RULES = [("scripts/oracle/", "protected"), ("corpus/", "protected"), ("corpus/scores/", "append-only"),
         ("UPSTREAM.json", "protected")]
OK = "batch R7: order items for cquery and aquery"


class RulesTest(unittest.TestCase):
    def test_longest_rule_decides(self):
        self.assertEqual(protected.rule_for("corpus/projects.json", RULES), ("corpus/", "protected"))
        self.assertEqual(protected.rule_for("corpus/scores/blaze-1.json", RULES), ("corpus/scores/", "append-only"))
        self.assertEqual(protected.rule_for("UPSTREAM.json", RULES), ("UPSTREAM.json", "protected"))
        self.assertIsNone(protected.rule_for("UPSTREAM.json.bak", RULES))
        self.assertIsNone(protected.rule_for("crates/blaze/src/main.rs", RULES))
        self.assertIsNone(protected.rule_for("scripts/oracle_other/x", RULES))

    def test_violations(self):
        changes = [("M", "crates/a.rs"), ("M", "scripts/oracle/allow.json"), ("A", "scripts/oracle/new.py"),
                   ("D", "corpus/oracle/re2.json"), ("A", "corpus/scores/blaze-1.json"),
                   ("M", "corpus/scores/blaze-0.json"), ("D", "corpus/scores/blaze-0.md")]
        self.assertEqual(protected.violations(changes, RULES), [
            ("M", "scripts/oracle/allow.json", "scripts/oracle/"), ("A", "scripts/oracle/new.py", "scripts/oracle/"),
            ("D", "corpus/oracle/re2.json", "corpus/"), ("M", "corpus/scores/blaze-0.json", "corpus/scores/"),
            ("D", "corpus/scores/blaze-0.md", "corpus/scores/")])

    def test_check_needs_a_named_approval(self):
        changes = [("M", "scripts/oracle/allow.json")]
        self.assertEqual(protected.check([("M", "crates/a.rs")], RULES, {}), (0, []))
        rc, lines = protected.check(changes, RULES, {})
        self.assertEqual(rc, 1)
        text = "\n".join(lines)
        self.assertIn("M scripts/oracle/allow.json   (rule: scripts/oracle/)", text)
        self.assertIn("COMMIT REJECTED.", text)
        self.assertIn('BLAZE_PROTECTED_OK="<batch>: <why>" git commit', text)
        for weak in ("1", "yes", "ok", "   x   ", "approved"):
            self.assertEqual(protected.check(changes, RULES, {protected.ENV: weak})[0], 1, weak)
        rc, lines = protected.check(changes, RULES, {protected.ENV: OK})
        self.assertEqual(rc, 0)
        self.assertEqual(lines[-1], "approved by BLAZE_PROTECTED_OK: " + OK)

    def test_trailer(self):
        self.assertEqual(protected.add_trailer("Subject\n", OK), "Subject\n\nProtected-Paths-OK: %s\n" % OK)
        self.assertEqual(protected.add_trailer("Subject\n\nBody text.\n", OK),
                         "Subject\n\nBody text.\n\nProtected-Paths-OK: %s\n" % OK)
        with_trailer = "Subject\n\nBody.\n\nCo-Authored-By: A <a@example.invalid>\n"
        got = protected.add_trailer(with_trailer, OK)
        self.assertEqual(got, with_trailer + "Protected-Paths-OK: %s\n" % OK)
        self.assertEqual(protected.add_trailer(got, OK), got)

    def test_the_real_list(self):
        rules = dict(protected.load_rules())
        for path in ("scripts/oracle/", "scripts/gate/", "scripts/git-hooks/", "corpus/", "docs/state/",
                     "UPSTREAM.json", "third_party/starlark-rust/PATCHES.md"):
            self.assertEqual(rules.get(path), "protected", path)
        self.assertEqual(rules.get("corpus/scores/"), "append-only")
        real = protected.load_rules()
        for path in ("scripts/oracle/allow.json", "scripts/gate/protected-paths.txt", "scripts/git-hooks/pre-commit",
                     "corpus/oracle/brotli.json", "corpus/projects.json", "docs/state/tests-baseline.json.gz"):
            self.assertEqual(protected.violations([("M", path)], real)[0][1], path)
        self.assertEqual(protected.violations([("A", "corpus/scores/blaze-abc.json")], real), [])
        self.assertEqual(protected.violations([("M", "crates/blaze/src/main.rs"), ("M", "docs/STATUS.md")], real), [])
        # Every protected file or directory exists (a typo in the list would protect nothing).
        for prefix, _ in real:
            if prefix != "corpus/scores/":
                self.assertTrue(os.path.exists(os.path.join(REPO, prefix)), prefix)

    def test_malformed_list(self):
        d = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        for bad in ("/abs/path\n", "a/../b\n", "append-only file.txt\n", "two words\n"):
            path = os.path.join(d, "list.txt")
            with open(path, "w") as f:
                f.write(bad)
            with self.assertRaises(ValueError, msg=bad):
                protected.load_rules(path)


class HookTest(unittest.TestCase):
    """The hooks, run by git in a scratch repository that holds copies of them."""

    def setUp(self):
        self.repo = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, self.repo, ignore_errors=True)
        for rel in ("scripts/gate/protected.py", "scripts/gate/protected-paths.txt", "scripts/git-hooks/pre-commit",
                    "scripts/git-hooks/commit-msg", "scripts/git-hooks/install"):
            dest = os.path.join(self.repo, rel)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            shutil.copy(os.path.join(REPO, rel), dest)
        self.write("crates/a.rs", "fn a() {}\n")
        self.write("corpus/projects.json", "{}\n")
        self.git("init", "-q")
        self.git("add", "-A")
        self.git("commit", "-q", "--no-verify", "-m", "initial")
        p = subprocess.run([os.path.join(self.repo, "scripts/git-hooks/install")], stdout=subprocess.PIPE,
                           env=self.env())
        self.assertEqual(p.returncode, 0)
        self.assertEqual(self.git("config", "--get", "core.hooksPath").stdout.strip(), "scripts/git-hooks")

    def env(self, **extra):
        env = {"PATH": "/usr/bin:/bin", "HOME": self.repo, "GIT_CONFIG_NOSYSTEM": "1", "GIT_AUTHOR_NAME": "t",
               "GIT_AUTHOR_EMAIL": "t@example.invalid", "GIT_COMMITTER_NAME": "t",
               "GIT_COMMITTER_EMAIL": "t@example.invalid"}
        env.update(extra)
        return env

    def git(self, *args, **extra_env):
        return subprocess.run(["git", "-C", self.repo] + list(args), env=self.env(**extra_env),
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, universal_newlines=True)

    def write(self, rel, text):
        path = os.path.join(self.repo, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text)

    def head(self):
        return self.git("rev-parse", "HEAD").stdout.strip()

    def test_ordinary_commit_passes_untouched(self):
        self.write("crates/a.rs", "fn a() { 1 }\n")
        p = self.git("commit", "-q", "-am", "port a")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(self.git("log", "-1", "--format=%B").stdout.strip(), "port a")

    def test_protected_change_is_rejected_without_approval(self):
        before = self.head()
        self.write("corpus/projects.json", '{"projects": {}}\n')
        self.write("crates/a.rs", "fn a() { 2 }\n")
        p = self.git("commit", "-q", "-am", "sneak")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("M corpus/projects.json   (rule: corpus/)", p.stderr)
        self.assertIn("COMMIT REJECTED.", p.stderr)
        self.assertIn("BLAZE_PROTECTED_OK", p.stderr)
        self.assertEqual(self.head(), before)
        p = self.git("commit", "-q", "-am", "sneak", BLAZE_PROTECTED_OK="yes")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("too short", p.stderr)
        self.assertEqual(self.head(), before)

    def test_approved_change_is_committed_with_a_trailer(self):
        self.write("corpus/projects.json", '{"projects": {}}\n')
        p = self.git("commit", "-q", "-am", "Add a project", BLAZE_PROTECTED_OK=OK)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("approved by BLAZE_PROTECTED_OK", p.stderr)
        self.assertEqual(self.git("log", "-1", "--format=%B").stdout.strip(),
                         "Add a project\n\nProtected-Paths-OK: " + OK)

    def test_scores_are_append_only(self):
        self.write("corpus/scores/blaze-1.json", "{}\n")
        self.git("add", "-A")
        p = self.git("commit", "-q", "-m", "score blaze-1")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.write("corpus/scores/blaze-1.json", '{"matching": 999}\n')
        p = self.git("commit", "-q", "-am", "improve the score by hand")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("M corpus/scores/blaze-1.json   (rule: corpus/scores/)", p.stderr)
        self.git("rm", "-q", "--cached", "corpus/scores/blaze-1.json")
        self.assertNotEqual(self.git("commit", "-q", "-m", "drop the score").returncode, 0)

    def test_rename_out_of_a_protected_path_is_rejected(self):
        before = self.head()
        self.git("mv", "corpus/projects.json", "projects.json")
        p = self.git("commit", "-q", "-m", "move it away")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("D corpus/projects.json", p.stderr)
        self.assertEqual(self.head(), before)

    def test_the_hooks_and_the_list_protect_themselves(self):
        before = self.head()
        # The list a commit is judged by is the committed one: emptying it in the same commit
        # protects nothing less.
        self.write("scripts/gate/protected-paths.txt", "# nothing is protected\n")
        self.write("corpus/projects.json", '{"projects": {"x": {}}}\n')
        p = self.git("commit", "-q", "-am", "empty the list")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("M scripts/gate/protected-paths.txt", p.stderr)
        self.assertIn("M corpus/projects.json", p.stderr)
        self.git("checkout", "-q", "--", "scripts/gate/protected-paths.txt", "corpus/projects.json")
        # git runs the working copy of a hook, so a replaced pre-commit lets its own commit
        # through; the commit-msg hook still refuses it.
        self.write("scripts/git-hooks/pre-commit", "#!/bin/sh\nexit 0\n")
        p = self.git("commit", "-q", "-am", "disable the hook")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("staged changes to protected paths and no approval", p.stderr)
        self.assertEqual(self.head(), before)

    def test_a_broken_committed_list_rejects_every_commit(self):
        self.write("scripts/gate/protected-paths.txt", "/absolute\n")
        p = self.git("commit", "-q", "-am", "break the list", BLAZE_PROTECTED_OK=OK)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.write("crates/a.rs", "fn a() { 3 }\n")
        p = self.git("commit", "-q", "-am", "port a")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("cannot check protected paths", p.stderr)

    def test_committed_rules_fall_back_to_the_working_file(self):
        d = tempfile.mkdtemp(prefix="gate-test-")
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        self.assertEqual(protected.committed_rules(d), protected.load_rules())

    def test_install_status_and_uninstall(self):
        tool = os.path.join(self.repo, "scripts/git-hooks/install")
        self.assertEqual(subprocess.run([tool, "status"], stdout=subprocess.PIPE, env=self.env()).returncode, 0)
        self.assertEqual(subprocess.run([tool, "uninstall"], stdout=subprocess.PIPE, env=self.env()).returncode, 0)
        self.assertEqual(subprocess.run([tool, "status"], stdout=subprocess.PIPE, env=self.env()).returncode, 1)
        self.assertEqual(subprocess.run([tool, "help"], stdout=subprocess.PIPE, env=self.env()).returncode, 0)
        self.assertEqual(subprocess.run([tool, "bogus"], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                        env=self.env()).returncode, 2)


if __name__ == "__main__":
    unittest.main()
