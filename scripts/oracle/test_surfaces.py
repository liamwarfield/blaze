"""Tests for surface extraction and comparison. Run: python3 -I -m unittest discover -s scripts/oracle"""

import json
import unittest

import oracle_compare as ocmp
import surfaces
from normalise import Context

OB = "/r/0123"
CTX = Context({"execroot": OB + "/execroot/_main", "output_base": OB, "workspace": "/w"})


class ConcatJsonTest(unittest.TestCase):
    def test_pretty_and_line_streams(self):
        self.assertEqual(surfaces._concat_json('{\n "a": 1\n}{"b": [2]}\n\n{"c": {}}\n'),
                         [{"a": 1}, {"b": [2]}, {"c": {}}])
        self.assertEqual(surfaces._concat_json("  \n"), [])


class AquerySummaryTest(unittest.TestCase):
    DOC = {
        "pathFragments": [
            {"id": 1, "label": "bazel-out"}, {"id": 2, "label": "k8", "parentId": 1},
            {"id": 3, "label": "a.o", "parentId": 2}, {"id": 4, "label": "a.c"},
            {"id": 5, "label": "a.h"}, {"id": 6, "label": "tree", "parentId": 2},
        ],
        "artifacts": [
            {"id": 10, "pathFragmentId": 3}, {"id": 11, "pathFragmentId": 4},
            {"id": 12, "pathFragmentId": 5}, {"id": 13, "pathFragmentId": 6, "isTreeArtifact": True},
        ],
        "depSetOfFiles": [
            {"id": 1, "directArtifactIds": [11], "transitiveDepSetIds": [2]},
            {"id": 2, "directArtifactIds": [12, 13]},
        ],
        "targets": [{"id": 1, "label": "//:a", "ruleClassId": 1}],
        "ruleClasses": [{"id": 1, "name": "cc_library"}],
        "configuration": [{"id": 1, "mnemonic": "k8-fastbuild", "platformName": "k8", "checksum": "abc"}],
        "actions": [{
            "targetId": 1, "actionKey": "k", "mnemonic": "CppCompile", "configurationId": 1,
            "arguments": ["gcc", "-c", "a.c"], "environmentVariables": [{"key": "PATH", "value": "/bin"}],
            "inputDepSetIds": [1], "outputIds": [10], "primaryOutputId": 10,
            "executionInfo": [{"key": "requires-x", "value": ""}],
        }],
    }

    def test_ids_resolved_and_keyed_by_primary_output(self):
        got = surfaces.aquery_summary(self.DOC)
        self.assertEqual(list(got), ["bazel-out/k8/a.o"])
        a = got["bazel-out/k8/a.o"]
        self.assertEqual(a["inputs"], ["a.c", "a.h", "bazel-out/k8/tree/"])
        self.assertEqual(a["outputs"], ["bazel-out/k8/a.o"])
        self.assertEqual(a["env"], {"PATH": "/bin"})
        self.assertEqual(a["executionInfo"], {"requires-x": ""})
        self.assertEqual(a["target"], "//:a")
        self.assertEqual(a["ruleClass"], "cc_library")
        self.assertEqual(a["configuration"]["checksum"], "abc")

    def test_renumbered_ids_give_the_same_summary(self):
        doc = json.loads(json.dumps(self.DOC))
        remap = {10: 20, 11: 21, 12: 22, 13: 23}
        for a in doc["artifacts"]:
            a["id"] = remap[a["id"]]
        for d in doc["depSetOfFiles"]:
            d["directArtifactIds"] = [remap[x] for x in d["directArtifactIds"]]
        doc["actions"][0]["outputIds"] = [20]
        doc["actions"][0]["primaryOutputId"] = 20
        self.assertEqual(surfaces.aquery_summary(doc), surfaces.aquery_summary(self.DOC))

    def test_order_items(self):
        doc = json.loads(json.dumps(self.DOC))
        doc["actions"].append(dict(doc["actions"][0], primaryOutputId=13, outputIds=[13, 10], inputDepSetIds=[2, 1]))
        keys = list(surfaces.aquery_summary(doc))
        order = surfaces.aquery_order(doc, keys)
        self.assertEqual(order["__order__ actions"], {"order": ["bazel-out/k8/a.o", "bazel-out/k8/tree/"]})
        # key, targetId, configurationId, primaryOutputId, inputDepSetIds, outputIds, aspectDescriptorIds
        self.assertEqual(order["__order__ action ids"], {"order": [
            ["bazel-out/k8/a.o", 1, 1, 10, [1], [10], None],
            ["bazel-out/k8/tree/", 1, 1, 13, [2, 1], [13, 10], None]]})
        for name in ("targets", "artifacts", "depSetOfFiles", "pathFragments", "configuration", "ruleClasses"):
            self.assertEqual(order["__order__ " + name], {"order": doc[name]})
        self.assertNotIn("__order__ aspectDescriptors", order)  # the document has none
        # Every item of the surface is an object, action or order.
        self.assertTrue(all(isinstance(v, dict) for v in order.values()))

    def test_action_order_is_compared(self):
        doc = json.loads(json.dumps(self.DOC))
        doc["actions"].append(dict(doc["actions"][0], primaryOutputId=13, outputIds=[13]))
        swapped = dict(doc, actions=doc["actions"][::-1])
        a, b = surfaces.aquery_summary(doc), surfaces.aquery_summary(swapped)
        self.assertEqual(a, b)  # as sets of actions they are equal
        a.update(surfaces.aquery_order(doc, list(a)))
        b.update(surfaces.aquery_order(swapped, list(b)))
        r = ocmp.compare_surface("p", "aquery_actions", {"status": "OK", "items": a}, {"status": "OK", "items": b},
                                 ocmp.AllowList(None))
        self.assertEqual(r["status"], "DIFF")
        self.assertEqual(sorted(d["key"] for d in r["details"]), ["__order__ action ids", "__order__ actions"])
        self.assertEqual(r["counts"]["MATCH"], 2 + 6)

    def test_renumbered_ids_are_an_order_difference(self):
        doc = json.loads(json.dumps(self.DOC))
        other = json.loads(json.dumps(self.DOC))
        for a in other["artifacts"]:
            a["id"] += 100
        for d in other["depSetOfFiles"]:
            d["directArtifactIds"] = [x + 100 for x in d["directArtifactIds"]]
        other["actions"][0]["outputIds"] = [110]
        other["actions"][0]["primaryOutputId"] = 110
        a, b = surfaces.aquery_summary(doc), surfaces.aquery_summary(other)
        self.assertEqual(a, b)
        oa, ob = surfaces.aquery_order(doc, list(a)), surfaces.aquery_order(other, list(b))
        differing = sorted(k for k in oa if oa[k] != ob[k])
        self.assertEqual(differing, ["__order__ action ids", "__order__ artifacts", "__order__ depSetOfFiles"])

    def test_duplicate_primary_outputs_get_suffix(self):
        doc = json.loads(json.dumps(self.DOC))
        doc["actions"].append(dict(doc["actions"][0], mnemonic="Other"))
        self.assertEqual(sorted(surfaces.aquery_summary(doc)), ["bazel-out/k8/a.o", "bazel-out/k8/a.o #2"])


class BepTest(unittest.TestCase):
    def events(self, set_ids=("0", "1")):
        s0, s1 = set_ids
        return [
            {"id": {"started": {}}, "children": [{"progress": {}}, {"pattern": {"pattern": ["//..."]}}],
             "started": {"uuid": "u", "startTimeMillis": "1", "command": "build", "serverPid": "9"}},
            {"id": {"progress": {}}, "progress": {"stderr": "x"}},
            {"id": {"namedSet": {"id": s1}}, "namedSetOfFiles": {"files": [{"name": "b", "uri": "file://%s/b" % OB}]}},
            {"id": {"namedSet": {"id": s0}}, "namedSetOfFiles": {"files": [{"name": "a"}], "fileSets": [{"id": s1}]}},
            {"id": {"targetCompleted": {"label": "//:t", "configuration": {"id": "c"}}},
             "completed": {"success": True, "outputGroup": [{"name": "default", "fileSets": [{"id": s0}]}]}},
            {"id": {"workspaceStatus": {}}, "workspaceStatus": {"item": [
                {"key": "BUILD_HOST", "value": "h"}, {"key": "BUILD_EMBED_LABEL"}]}},
            {"id": {"buildFinished": {}}, "finished": {"overallSuccess": True, "finishTimeMillis": "5",
                                                     "finishTime": "2026-01-01T00:00:00Z", "exitCode": {"name": "SUCCESS"}}},
            {"id": {"unstructuredCommandLine": {}}, "unstructuredCommandLine": {"args": ["build", "--startup_time=7", "--x=1"]}},
            {"id": {"testResult": {"label": "//:t", "run": 1}}, "testResult": {
                "status": "PASSED", "testAttemptDurationMillis": "5", "testAttemptStart": "2026-01-01T00:00:00Z",
                "testActionOutput": [{"name": "test.log", "uri": "file://x", "digest": "d", "length": "3"}],
                "executionInfo": {"strategy": "linux-sandbox", "timingBreakdown": {"time": "1s"}}}},
        ]

    def test_normalisation(self):
        got = surfaces.normalise_bep(CTX, self.events())
        self.assertNotIn('{"progress":{}}', got)
        self.assertFalse(any("namedSet" in k for k in got))
        self.assertEqual(got['{"started":{}}'], {"children": [{"pattern": {"pattern": ["//..."]}}],
                                                "started": {"command": "build"}})
        tc = got['{"targetCompleted":{"configuration":{"id":"c"},"label":"//:t"}}']
        self.assertEqual(tc["completed"]["outputGroup"][0]["files"],
                         [{"name": "a"}, {"name": "b", "uri": "file://<OUTPUT_BASE>/b"}])
        self.assertEqual(got['{"workspaceStatus":{}}']["workspaceStatus"]["item"][0]["value"], "<BUILD_HOST>")
        self.assertEqual(got['{"buildFinished":{}}']["finished"], {"overallSuccess": True, "exitCode": {"name": "SUCCESS"}})
        self.assertEqual(got['{"unstructuredCommandLine":{}}']["unstructuredCommandLine"]["args"],
                         ["build", "--startup_time=<DUR>", "--x=1"])
        tr = got['{"testResult":{"label":"//:t","run":1}}']["testResult"]
        self.assertEqual(tr, {"status": "PASSED", "testActionOutput": [{"name": "test.log", "uri": "file://x"}],
                              "executionInfo": {"strategy": "linux-sandbox"}})

    def test_named_set_numbering_does_not_matter(self):
        self.assertEqual(surfaces.normalise_bep(CTX, self.events(("0", "1"))),
                         surfaces.normalise_bep(CTX, self.events(("7", "3"))))

    def test_test_suite_expansion_sorted(self):
        ev = [{"id": {"pattern": {"pattern": ["//:all"]}}, "expanded": {"testSuiteExpansions": [
            {"suiteLabel": "//:z", "testLabels": ["//:b", "//:a"]}, {"suiteLabel": "//:s", "testLabels": ["//:c"]}]}}]
        got = list(surfaces.normalise_bep(CTX, ev).values())[0]["expanded"]["testSuiteExpansions"]
        self.assertEqual(got, [{"suiteLabel": "//:s", "testLabels": ["//:c"]},
                               {"suiteLabel": "//:z", "testLabels": ["//:a", "//:b"]}])

    def test_structured_timing_option(self):
        ev = [{"id": {"structuredCommandLine": {"commandLineLabel": "original"}}, "structuredCommandLine": {
            "sections": [{"optionList": {"option": [
                {"combinedForm": "--startup_time=12", "optionName": "startup_time", "optionValue": "12"}]}}]}}]
        got = surfaces.normalise_bep(CTX, ev)
        opt = list(got.values())[0]["structuredCommandLine"]["sections"][0]["optionList"]["option"][0]
        self.assertEqual(opt, {"combinedForm": "--startup_time=<DUR>", "optionName": "startup_time", "optionValue": "<DUR>"})


class DefaultOverrideTest(unittest.TestCase):
    def test_groups_sorted_order_within_group_kept(self):
        a = ["test", "--x", "--default_override=1:build:macos=--m", "--default_override=1:build=--a",
             "--default_override=1:build=--b", "--default_override=1:test=--t", "--y"]
        b = ["test", "--x", "--default_override=1:test=--t", "--default_override=1:build=--a",
             "--default_override=1:build=--b", "--default_override=1:build:macos=--m", "--y"]
        want = ["test", "--x", "--default_override=1:build=--a", "--default_override=1:build=--b",
                "--default_override=1:build:macos=--m", "--default_override=1:test=--t", "--y"]
        self.assertEqual(surfaces._sort_overrides_deep(a), want)
        self.assertEqual(surfaces._sort_overrides_deep(b), want)

    def test_within_group_order_is_significant(self):
        a = ["--default_override=1:build=--b", "--default_override=1:build=--a"]
        self.assertEqual(surfaces._sort_overrides_deep(a), a)

    def test_structured_options(self):
        mk = lambda v: {"optionName": "default_override", "optionValue": v, "combinedForm": "--default_override=" + v}
        got = surfaces._sort_overrides_deep({"option": [mk("1:test=--t"), mk("1:build=--a")]})
        self.assertEqual([o["optionValue"] for o in got["option"]], ["1:build=--a", "1:test=--t"])


class SpawnTest(unittest.TestCase):
    def test_metrics_dropped_and_test_outputs_timing_dependent(self):
        s = {"metrics": {"startTime": "x"}, "walltime": "1s", "listedOutputs": ["bazel-out/k8/testlogs/t/test.log"],
             "actualOutputs": [{"path": "bazel-out/k8/testlogs/t/test.log", "digest": {"hash": "h"}},
                               {"path": "bazel-out/k8/bin/x", "digest": {"hash": "g"}}],
             "commandArgs": [OB + "/execroot/_main/x"]}
        got = surfaces.normalise_spawn(CTX, s)
        self.assertNotIn("metrics", got)
        self.assertNotIn("walltime", got)
        self.assertEqual(got["actualOutputs"][0]["digest"], "<timing-dependent>")
        self.assertEqual(got["actualOutputs"][1]["digest"], {"hash": "g"})
        self.assertEqual(got["commandArgs"], ["<EXECROOT>/x"])


class TestXmlTest(unittest.TestCase):
    def test_parse_ignores_times_and_logs(self):
        xml = b"""<?xml version="1.0"?><testsuites tests="3" time="1.2" timestamp="2026-01-01T00:00:00">
          <testsuite name="S" tests="3" time="0.3"><testcase name="a" time="0.1"/>
          <testcase name="b" time="0.2"><failure message="m"/></testcase>
          <testcase name="c" status="notrun"/><system-out>log 0.123s</system-out></testsuite></testsuites>"""
        self.assertEqual(surfaces.parse_test_xml(xml), [["S", "a", "passed"], ["S", "b", "failed"], ["S", "c", "skipped"]])

    def test_addresses_in_names(self):
        xml = b'<testsuite name="S"><testcase name="t (&lt;function f at 0x7fe2a408ea20&gt;)"/></testsuite>'
        self.assertEqual(surfaces.parse_test_xml(xml), [["S", "t (<function f at 0x<ADDR>>)", "passed"]])

    def test_label_to_dir(self):
        self.assertEqual(surfaces._label_to_testlog_dir("//:a_test"), "a_test")
        self.assertEqual(surfaces._label_to_testlog_dir("//python:x"), "python/x")
        self.assertIsNone(surfaces._label_to_testlog_dir("@r//:x"))


class LabelKindTest(unittest.TestCase):
    def test_parse(self):
        self.assertEqual(surfaces._label_kind_lines("cc_library rule //:a\nsource file //:a.c\n\n"),
                         [("//:a", "cc_library rule"), ("//:a.c", "source file")])


class CqueryOrderTest(unittest.TestCase):
    def write(self, tmp, name, text):
        import os
        os.makedirs(os.path.join(tmp, "raw"), exist_ok=True)
        os.makedirs(os.path.join(tmp, "norm"), exist_ok=True)
        with open(os.path.join(tmp, "raw", name + ".stdout"), "w") as f:
            f.write(text)
        return {"commands": [{"name": name, "exit_code": 0}]}

    def extract(self, fn, name, text):
        import shutil
        import tempfile
        tmp = tempfile.mkdtemp(prefix="oracle-test-")
        self.addCleanup(shutil.rmtree, tmp)
        return fn(tmp, self.write(tmp, name, text), CTX)

    def test_label_kind_order_item(self):
        text = "cc_library rule //:b (abc)\nsource file //:a.c (null)\ncc_library rule //:a (abc)\n"
        items = self.extract(surfaces.s_cquery_label_kind, "cquery_deps_label_kind", text)
        self.assertEqual(items["deps //:b (abc)"], "cc_library rule")
        self.assertEqual(items["__order__ deps"], ["deps //:b (abc)", "deps //:a.c (null)", "deps //:a (abc)"])
        lines = text.splitlines()
        other = self.extract(surfaces.s_cquery_label_kind, "cquery_deps_label_kind",
                             "\n".join([lines[2], lines[0], lines[1]]) + "\n")
        r = ocmp.compare_surface("p", "cquery_label_kind", {"status": "OK", "items": items},
                                 {"status": "OK", "items": other}, ocmp.AllowList(None))
        self.assertEqual((r["status"], r["counts"]["MATCH"], r["counts"]["DIFF"]), ("DIFF", 3, 1))
        self.assertEqual(r["details"][0]["key"], "__order__ deps")

    def test_jsonproto_order_items(self):
        doc = {"configurations": [{"id": 1, "checksum": "abc"}, {"id": 2, "checksum": "def"}],
               "results": [{"target": {"rule": {"name": "//:b"}}, "configurationId": 2},
                           {"target": {"rule": {"name": "//:a"}}, "configurationId": 1}]}
        items = self.extract(surfaces.s_cquery_jsonproto, "cquery_jsonproto", json.dumps(doc))
        self.assertEqual(items["__order__ results"], ["target //:b (def)", "target //:a (abc)"])
        self.assertEqual(items["__order__ configurations"], [[1, "abc"], [2, "def"]])
        # Same targets and configurations, printed in another order and numbered differently.
        other = {"configurations": [{"id": 1, "checksum": "def"}, {"id": 2, "checksum": "abc"}],
                 "results": [{"target": {"rule": {"name": "//:a"}}, "configurationId": 2},
                             {"target": {"rule": {"name": "//:b"}}, "configurationId": 1}]}
        b = self.extract(surfaces.s_cquery_jsonproto, "cquery_jsonproto", json.dumps(other))
        differing = sorted(k for k in items if items[k] != b[k])
        self.assertEqual(differing, ["__order__ configurations", "__order__ results"])


class DiffAndAllowTest(unittest.TestCase):
    def test_diff_paths(self):
        a = {"x": 1, "y": [1, 2], "z": {"k": "v"}, "w": [1]}
        b = {"x": 2, "y": [1, 3], "z": {"k": "v", "n": 1}, "w": [1, 2]}
        self.assertEqual(ocmp.diff_paths(a, b), ["w", "x", "y[1]", "z.n"])
        self.assertEqual(ocmp.diff_paths(1, "1"), ["$"])

    def allow(self, entries):
        al = ocmp.AllowList(None)
        al.entries = [ocmp.AllowList.prepare(e) for e in entries]
        return al

    def test_describe_diff_shows_where_two_lists_part(self):
        lines = ocmp.describe_diff({"o": ["a", "b", "c"]}, {"o": ["a", "c", "b", "d"]}, ["o"])
        self.assertEqual(lines[0], "  o: list of 3 vs 4")
        self.assertIn("    first difference at index 1:", lines)
        self.assertEqual(lines[-2:], ['    - "b"', '    + "c"'])
        lines = ocmp.describe_diff(["a"], ["a", "b"], ["$"])
        self.assertEqual(lines[-3:], ["    first difference at index 1:", "    - (end of list)", '    + "b"'])

    def test_only_fields_condition(self):
        al = self.allow([{"id": "ak", "surface": "aquery_actions", "projects": "*", "item": r"MANIFEST$",
                          "kinds": ["DIFF"], "only_fields": ["actionKey"], "max_items": {"p": 1}}])
        a = {"x/MANIFEST": {"actionKey": "1", "argv": []}}
        b_ok = {"x/MANIFEST": {"actionKey": "2", "argv": []}}
        b_bad = {"x/MANIFEST": {"actionKey": "2", "argv": ["z"]}}
        r = ocmp.compare_surface("p", "aquery_actions", {"status": "OK", "items": a}, {"status": "OK", "items": b_ok}, al)
        self.assertEqual((r["status"], r["counts"]["ALLOWED"]), ("MATCH", 1))
        r = ocmp.compare_surface("p", "aquery_actions", {"status": "OK", "items": a}, {"status": "OK", "items": b_bad}, al)
        self.assertEqual((r["status"], r["counts"]["DIFF"]), ("DIFF", 1))

    def test_max_items_violation(self):
        al = self.allow([{"id": "any", "surface": "s", "projects": "*", "kinds": ["EXTRA"], "max_items": {"p": 1}}])
        r = ocmp.compare_surface("p", "s", {"status": "OK", "items": {}},
                                 {"status": "OK", "items": {"a": 1, "b": 2}}, al)
        self.assertEqual(r["status"], "DIFF")
        self.assertEqual(len(r["allow_violations"]), 1)
        self.assertFalse(ocmp.is_clean(r))

    def test_counts_and_failed(self):
        al = self.allow([])
        r = ocmp.compare_surface("p", "s", {"status": "OK", "items": {"a": 1, "b": 2, "c": 3}},
                                 {"status": "OK", "items": {"a": 1, "b": 5, "d": 4}}, al)
        self.assertEqual(r["counts"], {"MATCH": 1, "DIFF": 1, "MISSING": 1, "EXTRA": 1, "ALLOWED": 0})
        r = ocmp.compare_surface("p", "s", {"status": "OK", "items": {"a": 1}},
                                 {"status": "FAILED", "items": {}, "error": "boom"}, al)
        self.assertEqual((r["status"], r["counts"]["MISSING"]), ("FAILED", 1))
        r = ocmp.compare_surface("p", "s", {"status": "OK", "items": {"a": 1}}, {"status": "SKIPPED", "items": {}}, al)
        self.assertEqual(r["status"], "MISSING")
        self.assertFalse(ocmp.is_clean(r))
        r = ocmp.compare_surface("p", "s", {"status": "ABSENT", "items": {}}, {"status": "OK", "items": {"a": 1}}, al)
        self.assertEqual(r["status"], "NO_REFERENCE")
        r = ocmp.compare_surface("p", "s", {"status": "ABSENT", "items": {}}, {"status": "SKIPPED", "items": {}}, al)
        self.assertEqual(r["status"], "NOT_RECORDED")
        self.assertFalse(ocmp.is_clean(r))

    def test_project_filter(self):
        al = self.allow([{"id": "x", "surface": "s", "projects": ["re2"], "kinds": ["DIFF"], "max_items": {"re2": 1}}])
        self.assertIsNone(al.match("brotli", "s", "k", "DIFF", ["a"]))
        self.assertIsNotNone(al.match("re2", "s", "k", "DIFF", ["a"]))

    def test_real_allow_file_is_well_formed(self):
        al = ocmp.AllowList()
        ids = [e["id"] for e in al.entries]
        self.assertEqual(len(ids), len(set(ids)))
        for e in al.entries:
            for field in ("id", "surface", "projects", "item", "kinds", "max_items", "condition", "reason"):
                self.assertIn(field, e, "%s lacks %s" % (e.get("id"), field))
            self.assertTrue(set(e["kinds"]) <= {"DIFF", "MISSING", "EXTRA"})
            if "DIFF" in e["kinds"]:
                self.assertTrue(e.get("only_fields"), "%s: a DIFF entry must name only_fields" % e["id"])


if __name__ == "__main__":
    unittest.main()
