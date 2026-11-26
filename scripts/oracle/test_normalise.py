"""Tests for the oracle normaliser. Run: python3 -I -m unittest discover -s scripts/oracle"""

import unittest

from normalise import Context, stderr_summary

ROOT = "/home/u/.cache/blaze/oracle-out/rec-x"
OB = ROOT + "/0123456789abcdef0123456789abcdef"


def ctx():
    return Context(
        {
            "execroot": OB + "/execroot/_main",
            "output_base": OB,
            "install_base": ROOT + "/install/58cf",
            "output_user_root": ROOT,
            "workspace": "/home/u/.cache/blaze/work/x/brotli",
            "record_dir": "/home/u/.cache/blaze/oracle-records/x/brotli",
            "binary": "/home/u/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64",
            "binary_dir": "/home/u/.cache/blaze/oracle",
            "home": "/home/u",
        },
        user="alice",
        host="buildhost",
    )


class ContextTextTest(unittest.TestCase):
    def test_most_specific_placeholder_wins(self):
        c = ctx()
        self.assertEqual(c.text(OB + "/execroot/_main/bazel-out/k8"), "<EXECROOT>/bazel-out/k8")
        self.assertEqual(c.text(OB + "/external/foo"), "<OUTPUT_BASE>/external/foo")
        self.assertEqual(c.text(ROOT + "/install/58cf/embedded_tools"), "<INSTALL_BASE>/embedded_tools")
        self.assertEqual(c.text(ROOT + "/other"), "<OUTPUT_USER_ROOT>/other")
        self.assertEqual(c.text("/home/u/.cache/blaze/work/x/brotli/BUILD.bazel:3:1"), "<WORKSPACE>/BUILD.bazel:3:1")
        self.assertEqual(c.text("/home/u/.bashrc"), "<HOME>/.bashrc")

    def test_binary_before_binary_dir(self):
        c = ctx()
        self.assertEqual(c.text("--binary_path=/home/u/.cache/blaze/oracle/bazel-9.3.2-linux-x86_64"),
                         "--binary_path=<BINARY>")
        self.assertEqual(c.text("/home/u/.cache/blaze/oracle/other"), "<BINARY_DIR>/other")

    def test_paths_inside_longer_strings(self):
        c = ctx()
        self.assertEqual(c.text("file://%s/a.o and %s/b" % (OB + "/execroot/_main", "/home/u/.cache/blaze/work/x/brotli")),
                         "file://<EXECROOT>/a.o and <WORKSPACE>/b")

    def test_sandbox_numbers(self):
        c = ctx()
        self.assertEqual(c.text(OB + "/sandbox/linux-sandbox/123/execroot/_main/x"),
                         "<OUTPUT_BASE>/sandbox/linux-sandbox/<N>/execroot/_main/x")
        self.assertEqual(c.text(OB + "/sandbox/processwrapper-sandbox/7/execroot"),
                         "<OUTPUT_BASE>/sandbox/processwrapper-sandbox/<N>/execroot")

    def test_uuid_and_timestamp(self):
        c = ctx()
        self.assertEqual(c.text("id 123e4567-e89b-12d3-a456-426614174000 x"), "id <UUID> x")
        self.assertEqual(c.text("at 2026-11-18T01:31:24.691Z."), "at <TIMESTAMP>.")
        self.assertEqual(c.text("2026-11-18 01:31:24"), "<TIMESTAMP>")
        self.assertEqual(c.text("version 1.2-3"), "version 1.2-3")

    def test_user_and_host_whole_words_only(self):
        c = ctx()
        self.assertEqual(c.text("built by alice on buildhost"), "built by <USER> on <HOST>")
        self.assertEqual(c.text("malice buildhosts alice_x"), "malice buildhosts alice_x")

    def test_short_user_names_are_not_replaced(self):
        c = Context({}, user="u", host="h")
        self.assertEqual(c.text("u h"), "u h")

    def test_non_strings_untouched(self):
        c = ctx()
        self.assertEqual(c.text(""), "")
        self.assertIsNone(c.text(None))

    def test_deep_replaces_keys_and_values(self):
        c = ctx()
        got = c.deep({OB + "/k": [OB + "/v", 3, {"x": True}]})
        self.assertEqual(got, {"<OUTPUT_BASE>/k": ["<OUTPUT_BASE>/v", 3, {"x": True}]})

    def test_round_trip_dict(self):
        c = ctx()
        c2 = Context.from_dict(c.to_dict())
        self.assertEqual(c2.text(OB + "/execroot/_main"), "<EXECROOT>")


class ContextBytesTest(unittest.TestCase):
    def test_bytes_replaced_and_flagged(self):
        c = ctx()
        data, changed = c.bytes(b"\x00" + (OB + "/execroot/_main/x").encode() + b"\xff")
        self.assertTrue(changed)
        self.assertEqual(data, b"\x00<EXECROOT>/x\xff")

    def test_bytes_unchanged(self):
        data, changed = ctx().bytes(b"\x7fELF plain")
        self.assertFalse(changed)
        self.assertEqual(data, b"\x7fELF plain")

    def test_bytes_sandbox(self):
        data, _ = ctx().bytes((OB + "/sandbox/linux-sandbox/42/execroot").encode())
        self.assertEqual(data, b"<OUTPUT_BASE>/sandbox/linux-sandbox/<N>/execroot")

    def test_no_word_rules_on_bytes(self):
        data, changed = ctx().bytes(b"alice")
        self.assertFalse(changed)
        self.assertEqual(data, b"alice")


class StderrSummaryTest(unittest.TestCase):
    def test_keeps_info_drops_progress_and_timing(self):
        text = "\n".join([
            "Computing main repo mapping: ",
            "Loading: 0 packages loaded",
            "Analyzing: 13 targets (5 packages loaded, 6 targets configured)",
            "INFO: Invocation ID: 123e4567-e89b-12d3-a456-426614174000",
            "INFO: Analyzed 13 targets (89 packages loaded, 594 targets configured).",
            "[37 / 53] Compiling c/enc/x.c; 0s linux-sandbox ... (12 actions, 10 running)",
            "INFO: Elapsed time: 2.510s, Critical Path: 1.24s",
            "INFO: 53 processes: 10 internal, 43 linux-sandbox.",
            "//:a_test                                  PASSED in 0.3s",
            "//:b_test                                  PASSED in 12.0s",
            "Executed 2 out of 2 tests: 2 tests pass.",
            "WARNING: %s/BUILD:1:1: something" % (OB + "/external/x"),
        ])
        got = stderr_summary(ctx(), text)
        self.assertEqual(got, {
            "INFO: Analyzed 13 targets (89 packages loaded, 594 targets configured).": 1,
            "INFO: 53 processes: 10 internal, 43 linux-sandbox.": 1,
            "//:a_test PASSED in <DUR>": 1,
            "//:b_test PASSED in <DUR>": 1,
            "Executed 2 out of 2 tests: 2 tests pass.": 1,
            "WARNING: <OUTPUT_BASE>/external/x/BUILD:1:1: something": 1,
        })

    def test_counts_duplicates_and_strips_ansi_and_pids(self):
        text = "\x1b[32mINFO:\x1b[0m server pid=1234 x\nINFO: server pid=99 x\n"
        self.assertEqual(stderr_summary(ctx(), text), {"INFO: server pid=<PID> x": 2})


class ServerStartLineTest(unittest.TestCase):
    """Referee batch 3: the client's server-start line has no prefix and is dropped, except for a
    recording made on Blaze's persistent server, which asks to keep it."""

    TEXT = "Starting local Bazel server (9.3.2) and connecting to it...\nINFO: Found 3 targets...\n"

    def test_dropped_by_default(self):
        self.assertEqual(stderr_summary(ctx(), self.TEXT), {"INFO: Found 3 targets...": 1})

    def test_kept_on_request(self):
        self.assertEqual(stderr_summary(ctx(), self.TEXT, keep_server_start=True),
                         {"Starting local Bazel server (9.3.2) and connecting to it...": 1,
                          "INFO: Found 3 targets...": 1})


if __name__ == "__main__":
    unittest.main()
