"""Tests for what counts as a pass: missing evidence, empty surfaces, allow-list caps, tampered
items, declared exceptions, and the numbers compare and scoreboard print.

Run: python3 -I -m unittest discover -s scripts/oracle
"""

import json
import os
import unittest

import oracle_common as oc
import oracle_compare as ocmp
import oracle_report as orep
from fixture_records import Records, load_tool, run_main

REF = oc.REFERENCE_LABEL
CLEAN = {"blaze_repo": {"commit": "e" * 40, "dirty": False, "dirty_paths": 0, "dirty_diff_sha256": None},
         "binary_origin": {"commit_verified": True, "reasons": []}}
QUERY = {"query_label_kind": {"pat //:a": "cc_library rule", "__order__ pat": ["//:a"]}}
CQUERY = {"cquery_label_kind": {"deps //:a (abc)": "cc_library rule", "__order__ deps": ["deps //:a (abc)"]}}


def read(path):
    with open(path) as f:
        return f.read()


def by_surface(cmp, project):
    return {r["surface"]: r for r in cmp.results if r["project"] == project}


class MissingEvidenceTest(unittest.TestCase):
    def setUp(self):
        self.rec = Records(self)

    def test_project_absent_from_candidate_fails(self):
        # The audit's case: the candidate recorded brotli only and matches it fully.
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add(REF, "re2", {**QUERY, **CQUERY})
        self.rec.add("cand", "brotli", QUERY)
        cmp = ocmp.compare_labels(REF, "cand")
        self.assertEqual(by_surface(cmp, "brotli")["query_label_kind"]["status"], "MATCH")
        re2 = by_surface(cmp, "re2")
        self.assertEqual(re2["query_label_kind"]["status"], "MISSING")
        self.assertEqual(re2["cquery_label_kind"]["status"], "MISSING")
        self.assertEqual(re2["exit_codes"]["status"], "MISSING")
        self.assertFalse(cmp.clean())
        self.assertIn("MISSING", cmp.verdict())
        self.assertTrue(cmp.verdict().startswith("FAIL"))
        row = next(r for r in cmp.rows() if (r["project"], r["surface"]) == ("re2", "query_label_kind"))
        self.assertEqual((row["total"], row["compared"], row["matching"], row["failed"]), (2, 0, 0, 2))
        self.assertFalse(cmp.provenance["re2"]["b"]["recorded"])

    def test_surface_skipped_by_candidate_fails(self):
        self.rec.add(REF, "brotli", {**QUERY, **CQUERY})
        self.rec.add("cand", "brotli", QUERY)
        cmp = ocmp.compare_labels(REF, "cand")
        s = by_surface(cmp, "brotli")
        self.assertEqual(s["cquery_label_kind"]["status"], "MISSING")
        self.assertFalse(ocmp.is_clean(s["cquery_label_kind"]))
        self.assertFalse(cmp.clean())

    def test_surface_without_reference_fails(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add("cand", "brotli", {**QUERY, **CQUERY})
        cmp = ocmp.compare_labels(REF, "cand")
        self.assertEqual(by_surface(cmp, "brotli")["cquery_label_kind"]["status"], "NO_REFERENCE")
        self.assertFalse(cmp.clean())

    def test_failed_surface_fails_on_either_side(self):
        self.rec.add(REF, "brotli", {**QUERY, "mod_graph": ("FAILED", "command mod_graph exited 2")})
        self.rec.add("cand", "brotli", {**QUERY, "mod_graph": ("FAILED", "command mod_graph exited 2")})
        cmp = ocmp.compare_labels(REF, "cand")
        self.assertEqual(by_surface(cmp, "brotli")["mod_graph"]["status"], "FAILED")
        self.assertFalse(cmp.clean())

    def test_recorded_by_neither_side_is_excluded_and_reported(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add("cand", "brotli", QUERY)
        cmp = ocmp.compare_labels(REF, "cand")
        s = by_surface(cmp, "brotli")
        self.assertEqual(s["bep"]["status"], "NOT_RECORDED")
        self.assertFalse(ocmp.is_clean(s["bep"]))  # a single surface that was not compared is not a match
        self.assertTrue(cmp.clean())               # but nothing in scope failed
        self.assertIn("surfaces recorded by neither side", cmp.verdict())
        excl = orep.excluded(cmp)
        self.assertIn({"project": "brotli", "surface": "bep", "reference_items": 0,
                       "reason": "recorded by neither side"}, excl)
        self.assertEqual(len(excl), len(oc.SURFACES) - 2)  # all but exit_codes and query_label_kind

    def test_nothing_compared_fails(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add("cand", "brotli", QUERY)
        cmp = ocmp.compare_labels(REF, "cand", surfaces=["bep"])
        self.assertEqual([r["status"] for r in cmp.results], ["NOT_RECORDED"])
        self.assertFalse(cmp.clean())
        self.assertIn("nothing was compared", cmp.verdict())

    def test_restriction_is_explicit_and_part_of_the_result(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add(REF, "re2", QUERY)
        self.rec.add("cand", "brotli", QUERY)
        cmp = ocmp.compare_labels(REF, "cand", projects=["brotli"], surfaces=["exit_codes", "query_label_kind"])
        self.assertTrue(cmp.clean())
        self.assertEqual(cmp.verdict(),
                         "100% MATCH [restricted to projects brotli; surfaces exit_codes,query_label_kind]")
        left_out = {(o["project"], o["surface"]): o["reference_items"] for o in cmp.out_of_scope}
        self.assertEqual(left_out[("re2", "query_label_kind")], 2)
        self.assertEqual(left_out[("brotli", "bep")], 0)
        self.assertEqual(len(left_out), 2 * len(oc.SURFACES) - 2)
        self.assertTrue(any("outside the requested restriction" in e["reason"] for e in orep.excluded(cmp)))
        # Without the restriction the same recordings fail.
        self.assertFalse(ocmp.compare_labels(REF, "cand").clean())


class EmptySurfaceTest(unittest.TestCase):
    def setUp(self):
        self.rec = Records(self)

    def compare(self, ref_codes, cand_codes):
        self.rec.add(REF, "brotli", {"tests": {}, "exit_codes": ref_codes})
        self.rec.add("cand", "brotli", {"tests": {}, "exit_codes": cand_codes})
        return by_surface(ocmp.compare_labels(REF, "cand"), "brotli")["tests"]

    def test_genuinely_empty_and_both_ran_matches(self):
        r = self.compare({"build": 0, "info": 0}, {"build": 0, "info": 0})
        self.assertEqual((r["status"], r["empty"]), ("MATCH", True))
        self.assertEqual(orep.status_text(ocmp.score_row(r)), "MATCH (empty)")

    def test_candidate_build_failed(self):
        r = self.compare({"build": 0, "info": 0}, {"build": 1, "info": 0})
        self.assertEqual(r["status"], "FAILED")
        self.assertIn("exited 0 on A and 1 on B", r["failure"]["b"]["error"])

    def test_candidate_did_not_run_the_command(self):
        r = self.compare({"build": 0, "info": 0}, {"info": 0})
        self.assertEqual(r["status"], "FAILED")
        self.assertIn("no recorded exit code on B", r["failure"]["b"]["error"])

    def test_reference_command_failed(self):
        r = self.compare({"build": 1, "info": 0}, {"build": 1, "info": 0})
        self.assertEqual(r["status"], "FAILED")
        self.assertIn("exited 1 on A", r["failure"]["a"]["error"])

    def test_empty_check_without_exit_codes(self):
        al = ocmp.AllowList(None)
        ok = {"status": "OK", "items": {}}
        self.assertEqual(ocmp.compare_surface("p", "tests", ok, ok, al)["status"], "FAILED")
        self.assertEqual(ocmp.compare_surface("p", "tests", ok, ok, al, {"build": 0}, {"build": 0})["status"], "MATCH")
        # A surface that is not tied to a command cannot be verified.
        self.assertEqual(ocmp.compare_surface("p", "exit_codes", ok, ok, al, {}, {})["status"], "FAILED")
        # query commands may exit 3 (--keep_going partial result) on the reference.
        self.assertIsNone(ocmp._empty_check("query_label_kind",
                                            {"query_label_kind": 0, "query_deps_label_kind": 3},
                                            {"query_label_kind": 0, "query_deps_label_kind": 3}))
        self.assertIsNotNone(ocmp._empty_check("query_label_kind",
                                               {"query_label_kind": 0, "query_deps_label_kind": 3},
                                               {"query_label_kind": 0, "query_deps_label_kind": 0}))


class AllowCapTest(unittest.TestCase):
    ENTRY = {"id": "key", "surface": "aquery_actions", "projects": "*", "item": r"MANIFEST$",
             "kinds": ["DIFF"], "only_fields": ["actionKey"], "max_items": {"brotli": 2}}

    def allow(self, **over):
        al = ocmp.AllowList(None)
        al.entries = [ocmp.AllowList.prepare({**self.ENTRY, **over})]
        return al

    def sides(self, n):
        a = {"x%d/MANIFEST" % i: {"actionKey": "a"} for i in range(n)}
        b = {"x%d/MANIFEST" % i: {"actionKey": "b"} for i in range(n)}
        return {"status": "OK", "items": a}, {"status": "OK", "items": b}

    def test_max_items_is_required(self):
        for bad in (None, 5, {"brotli": "2"}, {"brotli": -1}, {"brotli": True}, [2]):
            with self.assertRaises(ocmp.AllowListError):
                ocmp.AllowList.prepare({**self.ENTRY, "max_items": bad})
        entry = dict(self.ENTRY)
        del entry["max_items"]
        with self.assertRaises(ocmp.AllowListError):
            ocmp.AllowList.prepare(entry)

    def test_within_the_cap(self):
        r = ocmp.compare_surface("brotli", "aquery_actions", *self.sides(2), self.allow())
        self.assertEqual((r["status"], r["counts"]["ALLOWED"], r["allow_used"]), ("MATCH", 2, {"key": 2}))
        row = ocmp.score_row(r)
        self.assertEqual((row["total"], row["allowed"], row["compared"], row["matching"], row["failed"]),
                         (2, 2, 0, 0, 0))

    def test_over_the_cap_excuses_nothing_and_fails(self):
        r = ocmp.compare_surface("brotli", "aquery_actions", *self.sides(3), self.allow())
        self.assertEqual((r["status"], r["counts"]["ALLOWED"], r["counts"]["DIFF"]), ("DIFF", 0, 3))
        self.assertEqual(r["allow_violations"], ["key excuses 3 items > max_items 2 for project brotli"])
        self.assertFalse(ocmp.is_clean(r))

    def test_cap_is_per_project_and_zero_for_a_project_that_is_not_named(self):
        r = ocmp.compare_surface("re2", "aquery_actions", *self.sides(1), self.allow())
        self.assertEqual((r["status"], r["counts"]["DIFF"]), ("DIFF", 1))
        self.assertEqual(len(r["allow_violations"]), 1)
        r = ocmp.compare_surface("re2", "aquery_actions", *self.sides(3),
                                 self.allow(max_items={"brotli": 2, "re2": 3}))
        self.assertEqual(r["status"], "MATCH")

    def test_real_allow_file_caps_are_the_oracle_vs_oracle_counts(self):
        caps = {e["id"]: e["max_items"] for e in ocmp.AllowList().entries}
        self.assertEqual(caps, {"runfiles-manifest-action-key": {"brotli": 2, "re2": 54},
                                "build-metrics-volatile-counters": {"brotli": 1, "re2": 1}})


class AllowVolumeTest(unittest.TestCase):
    """Audit R2A-1: many allowed differing paths must not hide one that is not allowed.

    The auditor's case on the re2 reference buildMetrics event: change `count` and
    `skyfunctionName` in the five allowed per-SkyFunction lists (244 allowed paths) and also
    `targetMetrics` and `buildGraphMetrics.outputArtifactCount`. Before the fix diff_paths stopped
    at 200 paths and the item was ALLOWED, the surface MATCH. Uses the real allow.json entry.
    """

    KEY = '{"buildMetrics":{}}'
    LISTS = ("evaluatedValues", "builtValues", "changedValues", "cleanedValues", "dirtiedValues")

    def event(self, n_per_list, tag):
        graph = {name: [{"count": "%d%s" % (i, tag), "skyfunctionName": "FN_%d_%s" % (i, tag)}
                        for i in range(n_per_list)] for name in self.LISTS}
        graph["outputArtifactCount"] = "1000"
        return {"buildMetrics": {"buildGraphMetrics": graph,
                                 "targetMetrics": {"targetsConfigured": "357"},
                                 "actionSummary": {"actionsExecuted": "780"}}}

    def compare(self, a, b):
        return ocmp.compare_surface("re2", "bep", {"status": "OK", "items": {self.KEY: a}},
                                    {"status": "OK", "items": {self.KEY: b}}, ocmp.AllowList())

    def test_auditor_case_more_than_200_allowed_paths_and_one_not_allowed_is_diff(self):
        a, b = self.event(25, "a"), self.event(25, "b")  # 5 lists x 25 entries x 2 fields = 250 paths
        b["buildMetrics"]["targetMetrics"]["targetsConfigured"] = "358"
        b["buildMetrics"]["buildGraphMetrics"]["outputArtifactCount"] = "1001"
        paths = ocmp.diff_paths(a, b)
        self.assertEqual(len(paths), 252)
        self.assertIn("buildMetrics.targetMetrics.targetsConfigured", paths)
        r = self.compare(a, b)
        self.assertEqual((r["status"], r["counts"]["DIFF"], r["counts"]["ALLOWED"]), ("DIFF", 1, 0))
        self.assertEqual(r["details"][0]["paths_total"], 252)
        self.assertEqual(len(r["details"][0]["paths"]), 50)  # what is stored for printing is cut
        self.assertFalse(ocmp.is_clean(r))

    def test_one_not_allowed_path_at_the_end_of_a_long_list(self):
        # Sorted path order puts the extra path after every allowed one.
        a, b = self.event(40, "a"), self.event(40, "b")
        b["buildMetrics"]["targetMetrics"]["targetsConfigured"] = "358"
        self.assertEqual(ocmp.diff_paths(a, b)[-1], "buildMetrics.targetMetrics.targetsConfigured")
        self.assertEqual(self.compare(a, b)["status"], "DIFF")

    def test_only_allowed_paths_are_still_excused_however_many(self):
        r = self.compare(self.event(40, "a"), self.event(40, "b"))  # 400 allowed paths
        self.assertEqual((r["status"], r["counts"]["ALLOWED"]), ("MATCH", 1))
        self.assertEqual(r["details"][0]["paths_total"], 400)

    def test_describe_diff_counts_every_path(self):
        a, b = self.event(25, "a"), self.event(25, "b")
        paths = ocmp.diff_paths(a, b)
        lines = ocmp.describe_diff(a, b, paths[:50], total=len(paths))
        self.assertEqual(lines[-1], "  ... 242 more differing paths")


class IntegrityTest(unittest.TestCase):
    def setUp(self):
        self.rec = Records(self)

    def rewrite(self, label, project, surface, items):
        path = os.path.join(oc.record_dir(label, project), "items", surface + ".json")
        oc.write_json(path, {"surface": surface, "status": "OK", "items": items}, indent=None)

    def test_items_edited_after_recording_fail(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add("cand", "brotli", {"query_label_kind": {"pat //:a": "WRONG"}})
        # Make the candidate "match" by editing its items file behind the manifest's back.
        self.rewrite("cand", "brotli", "query_label_kind", QUERY["query_label_kind"])
        r = by_surface(ocmp.compare_labels(REF, "cand"), "brotli")["query_label_kind"]
        self.assertEqual(r["status"], "FAILED")
        self.assertIn("does not match the sha256 in manifest.json", r["failure"]["b"]["error"])

    def test_reference_must_match_the_committed_summary(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add("cand", "brotli", {"query_label_kind": {"pat //:a": "WRONG"}})
        # Edit the reference and its manifest consistently: only the committed summary still knows.
        self.rewrite(REF, "brotli", "query_label_kind", {"pat //:a": "WRONG"})
        m = oc.load_manifest(REF, "brotli")
        m["files"]["items/query_label_kind.json"] = oc.sha256_file(
            os.path.join(oc.record_dir(REF, "brotli"), "items", "query_label_kind.json"))
        oc.write_json(os.path.join(oc.record_dir(REF, "brotli"), "manifest.json"), m)
        r = by_surface(ocmp.compare_labels(REF, "cand"), "brotli")["query_label_kind"]
        self.assertEqual(r["status"], "FAILED")
        self.assertIn("committed corpus/oracle/brotli.json", r["failure"]["a"]["error"])

    def test_reference_without_committed_summary_fails(self):
        self.rec.add(REF, "brotli", QUERY, summary=False)
        self.rec.add("cand", "brotli", QUERY)
        cmp = ocmp.compare_labels(REF, "cand")
        self.assertEqual(by_surface(cmp, "brotli")["query_label_kind"]["status"], "FAILED")
        self.assertFalse(cmp.clean())

    def test_load_surface_without_manifest_keeps_working(self):
        self.rec.add("x", "brotli", QUERY)
        self.assertEqual(ocmp.load_surface("x", "brotli", "query_label_kind")["items"], QUERY["query_label_kind"])
        self.assertEqual(ocmp.load_surface("x", "brotli", "nope")["status"], "ABSENT")


SPLICE = {"id": "aquery-spliced", "kind": "spliced", "surfaces": ["aquery_actions", "exit_codes"],
          "explanation": "aquery exits 8 under the protocol; this output comes from another output base"}


class ExceptionTest(unittest.TestCase):
    def setUp(self):
        self.rec = Records(self)
        items = {**QUERY, "aquery_actions": {"out/a.o": {"mnemonic": "CppCompile"}}}
        self.rec.add(REF, "workerd", items, manifest={"exceptions": [SPLICE]})
        self.rec.add("cand", "workerd", items)

    def test_surfaces_under_an_exception_are_compared_but_never_a_pass(self):
        cmp = ocmp.compare_labels(REF, "cand")
        s = by_surface(cmp, "workerd")
        self.assertEqual(s["aquery_actions"]["status"], "MATCH")
        self.assertEqual(s["aquery_actions"]["exceptions"], [{"side": "a", "id": "aquery-spliced"}])
        self.assertEqual(s["exit_codes"]["exceptions"], [{"side": "a", "id": "aquery-spliced"}])
        self.assertEqual(s["query_label_kind"]["exceptions"], [])
        self.assertFalse(ocmp.is_clean(s["aquery_actions"]))
        self.assertTrue(ocmp.is_clean(s["query_label_kind"]))
        self.assertFalse(cmp.clean())
        self.assertIn("2 surfaces under a declared recording exception", cmp.verdict())
        self.assertEqual(orep.status_text(ocmp.score_row(s["aquery_actions"])), "MATCH*")

    def test_the_exception_is_printed(self):
        cmp = ocmp.compare_labels(REF, "cand")
        lines = orep.exception_lines(cmp)
        self.assertEqual(len(lines), 1)
        self.assertIn("%s/workerd exception aquery-spliced (kind spliced) affects surfaces "
                      "aquery_actions, exit_codes: aquery exits 8" % REF, lines[0])
        rc, out, _ = run_main(load_tool("compare"), [REF, "cand", "--report", os.path.join(self.rec.root, "r.json")])
        self.assertEqual(rc, 1)
        self.assertIn("EXCEPTION: %s/workerd exception aquery-spliced" % REF, out)
        self.assertRegex(out, r"workerd +aquery_actions +MATCH\* +1 +1 ")
        doc = orep.score_document(cmp, ocmp.AllowList(None))
        self.assertEqual(doc["exceptions"][0]["surfaces"], ["aquery_actions", "exit_codes"])
        self.assertIn("aquery-spliced", orep.score_markdown(doc))
        self.assertIn("EXCEPTION: ", orep.score_text(doc))

    def test_malformed_exception_applies_to_every_surface(self):
        excs = ocmp.manifest_exceptions({"exceptions": ["oops", {"id": "x", "surfaces": []}]})
        self.assertEqual([e["surfaces"] for e in excs], [list(oc.SURFACES), list(oc.SURFACES)])


class AnchoredExceptionTest(unittest.TestCase):
    """Audit R2A-5: the reference's exceptions come from the committed summary as well."""

    EXC = {"id": "spliced", "kind": "spliced", "surfaces": ["query_label_kind"], "explanation": "by hand"}

    def setUp(self):
        self.rec = Records(self)
        self.rec.add(REF, "brotli", QUERY, manifest={"exceptions": [self.EXC]})
        self.rec.add("cand", "brotli", QUERY)
        summary = os.path.join(oc.CORPUS_SUMMARIES, "brotli.json")
        doc = oc.read_json(summary)
        doc["exceptions"] = [self.EXC]
        oc.write_json(summary, doc)

    def test_exception_removed_from_the_manifest_still_applies(self):
        m = oc.load_manifest(REF, "brotli")
        m["exceptions"] = []
        oc.write_json(os.path.join(oc.record_dir(REF, "brotli"), "manifest.json"), m)
        cmp = ocmp.compare_labels(REF, "cand")
        r = by_surface(cmp, "brotli")["query_label_kind"]
        self.assertEqual(r["exceptions"], [{"side": "a", "id": "spliced"}])
        self.assertFalse(cmp.clean())
        self.assertTrue(any("differ from the committed summary" in w for w in orep.warning_lines(cmp)))

    def test_same_exception_in_both_is_listed_once(self):
        cmp = ocmp.compare_labels(REF, "cand")
        self.assertEqual([e["id"] for _, _, _, e in cmp.exceptions()], ["spliced"])
        self.assertEqual(cmp.notes, [])


class ForceAndOriginTest(unittest.TestCase):
    """Audit R2A-2, R2A-3, R2A-5: what --force overrode and whether the commit is the binary's
    origin are printed, and scoreboard --write refuses what cannot back an accepted score."""

    def setUp(self):
        self.rec = Records(self)
        self.rec.add(REF, "brotli", QUERY)
        self.scoreboard = load_tool("scoreboard")

    def test_force_is_printed(self):
        self.rec.add("cand", "brotli", QUERY, manifest=dict(
            CLEAN, replaced={"created": "2025-01-01", "binary_sha256": "9" * 64},
            forced=["label cand already holds re2 recorded with another binary"]))
        w = orep.warning_lines(ocmp.compare_labels(REF, "cand"))
        self.assertIn("cand/brotli was recorded with --force over an earlier recording (created 2025-01-01, "
                      "binary 999999999999)", w)
        self.assertIn("cand/brotli was recorded with --force despite: label cand already holds re2 recorded "
                      "with another binary", w)

    def test_unverified_commit_is_printed(self):
        self.rec.add("cand", "brotli", QUERY, manifest=dict(
            CLEAN, binary_origin={"commit_verified": False, "reasons": ["binary is not inside x"]}))
        cmp = ocmp.compare_labels(REF, "cand")
        self.assertIn("binary origin NOT VERIFIED", orep.provenance_lines(cmp)[3])
        self.assertIn("cand/brotli: the recorded commit is not verified as the binary's origin: binary is not "
                      "inside x", orep.warning_lines(cmp))

    def write(self, *extra):
        return run_main(self.scoreboard, ["cand", "--write"] + list(extra))

    def test_write_refuses_what_cannot_back_an_accepted_score(self):
        dirty = {"commit": "e" * 40, "dirty": True, "dirty_paths": 1, "dirty_diff_sha256": "d" * 64}
        cases = {
            "no recorded commit": {},
            "recorded from a dirty tree": dict(CLEAN, blaze_repo=dirty),
            "binary origin not verified: older": dict(CLEAN, binary_origin={"commit_verified": False,
                                                                           "reasons": ["older"]}),
            "binary origin not checked": {"blaze_repo": CLEAN["blaze_repo"]},
        }
        for i, (why, manifest) in enumerate(cases.items()):
            label = "cand%d" % i
            self.rec.add(label, "brotli", QUERY, manifest=manifest)
            rc, _, err = run_main(self.scoreboard, [label, "--write"])
            self.assertEqual(rc, 2, why)
            self.assertIn("refusing to write a score for %s" % label, err)
            self.assertIn(why, err)
            self.assertFalse(os.path.exists(os.path.join(oc.CORPUS_SCORES, label + ".json")))

    def test_allow_dirty_writes_and_stores_the_reason(self):
        self.rec.add("cand", "brotli", QUERY)
        rc, _, err = self.write("--allow-dirty", " ")
        self.assertEqual(rc, 2)
        self.assertIn("--allow-dirty needs a reason", err)
        rc, out, _ = self.write("--allow-dirty", "root ruling 7: binary deleted")
        self.assertEqual(rc, 0, out)
        doc = oc.read_json(os.path.join(oc.CORPUS_SCORES, "cand.json"))
        self.assertEqual(doc["allow_dirty"], "root ruling 7: binary deleted")
        self.assertEqual(doc["recording_problems"], ["cand/brotli: no recorded commit"])
        self.assertIn("accepted with --allow-dirty: root ruling 7: binary deleted", out)
        self.assertIn("accepted with `--allow-dirty`: root ruling 7", read(os.path.join(oc.CORPUS_SCORES, "cand.md")))

    def test_clean_verified_recording_is_written(self):
        self.rec.add("cand", "brotli", QUERY, manifest=CLEAN)
        rc, out, err = self.write()
        self.assertEqual(rc, 0, err)
        doc = oc.read_json(os.path.join(oc.CORPUS_SCORES, "cand.json"))
        self.assertEqual((doc["recording_problems"], doc["allow_dirty"]), ([], None))


class MatchingKeysTest(unittest.TestCase):
    """Audit R2A-4: a score row names its matching items (as digests)."""

    def test_rows_hold_digests_of_the_matching_keys_only(self):
        rec = Records(self)
        rec.add(REF, "brotli", {"query_label_kind": {"a": 1, "b": 2, "c": 3}, "cquery_label_kind": {"x": 1}})
        rec.add("cand", "brotli", {"query_label_kind": {"a": 1, "b": 9, "c": 3, "d": 4}})
        doc = orep.score_document(ocmp.compare_labels(REF, "cand"), ocmp.AllowList(None))
        rows = {r["surface"]: r for r in doc["rows"]}
        self.assertEqual(rows["query_label_kind"]["matching_keys"], orep.key_digests(["a", "c"]))
        self.assertEqual(rows["cquery_label_kind"]["matching_keys"], "")  # MISSING: nothing matches
        self.assertEqual(len(rows["query_label_kind"]["matching_keys"].split()), rows["query_label_kind"]["matching"])
        import hashlib
        self.assertEqual(orep.key_digests(["c", "a"]), " ".join(sorted(
            hashlib.sha256(k.encode()).hexdigest()[:10] for k in "ac")))


class ScoreTest(unittest.TestCase):
    def setUp(self):
        self.rec = Records(self)
        ref = {"query_label_kind": {"a": 1, "b": 2, "c": 3, "d": 4}, "cquery_label_kind": {"x": 1, "y": 2}}
        cand = {"query_label_kind": {"a": 1, "b": 9, "c": 3, "e": 5}}
        repo = {"commit": "c" * 40, "dirty": True, "dirty_paths": 3, "dirty_diff_sha256": "d" * 64}
        self.rec.add(REF, "brotli", ref)
        self.rec.add("cand", "brotli", cand, manifest={"blaze_repo": repo})
        self.cmp = ocmp.compare_labels(REF, "cand")

    def test_denominators(self):
        rows = {r["surface"]: r for r in self.cmp.rows()}
        q = rows["query_label_kind"]
        self.assertEqual({k: q[k] for k in orep.COUNT_KEYS},
                         {"total": 4, "compared": 4, "matching": 2, "allowed": 0, "failed": 2, "extra": 1})
        c = rows["cquery_label_kind"]
        self.assertEqual((c["status"], c["total"], c["compared"], c["matching"], c["failed"]),
                         ("MISSING", 2, 0, 0, 2))
        for row in rows.values():
            self.assertEqual(row["total"], row["matching"] + row["allowed"] + row["failed"])
        t = self.cmp.totals()
        # exit_codes: the reference ran the cquery command too, the candidate did not.
        self.assertEqual((t["total"], t["matching"], t["failed"]), (4 + 2 + 4, 2 + 3, 2 + 2 + 1))
        self.assertEqual(t["surfaces"]["MISSING"], 1)
        self.assertEqual(t["surfaces"]["DIFF"], 2)
        self.assertEqual(ocmp.percent(q), "50.0")

    def test_percent_rounds_down_and_counts_missing_surfaces(self):
        self.assertEqual(ocmp.percent({"total": 3296, "allowed": 0, "matching": 3295}), "99.9")
        self.assertEqual(ocmp.percent({"total": 3, "allowed": 1, "matching": 2}), "100.0")
        self.assertEqual(ocmp.percent({"total": 0, "allowed": 0, "matching": 0}), "-")
        self.assertEqual(ocmp.percent(self.cmp.totals()), "50.0")  # 5 of 10, the MISSING surface included

    def test_commit_and_dirty_flag_of_both_sides_are_printed(self):
        lines = orep.provenance_lines(self.cmp)
        self.assertEqual(lines[0], "%s (A, reference)" % REF)
        self.assertIn("repo commit not recorded", lines[1])
        self.assertEqual(lines[2], "cand (B, candidate)")
        self.assertIn("binary sha256 %s /bin/cand" % oc.sha256_bytes(b"cand")[:12], lines[3])
        self.assertIn("recorded 2026-01-01T00:00:00+00:00", lines[3])
        self.assertIn("repo commit cccccccccccc DIRTY (diff dddddddddddd)", lines[3])
        self.assertTrue(any("recorded from a dirty tree" in w for w in orep.warning_lines(self.cmp)))

    def test_clean_tree_and_missing_commit(self):
        self.assertEqual(orep.tree_text({"repo_commit": "a" * 40, "repo_dirty": False}),
                         "repo commit aaaaaaaaaaaa clean, binary origin not checked")
        self.assertEqual(orep.tree_text({"repo_commit": "a" * 40, "repo_dirty": False, "commit_verified": True}),
                         "repo commit aaaaaaaaaaaa clean, binary from that commit verified")
        self.assertEqual(orep.tree_text({"repo_commit": "a" * 40, "repo_dirty": False, "commit_verified": False}),
                         "repo commit aaaaaaaaaaaa clean, binary origin NOT VERIFIED")
        self.assertEqual(orep.tree_text({}), "repo commit not recorded")

    def test_warnings_for_mixed_binaries_and_stale_extraction(self):
        self.rec.add(REF, "re2", QUERY)
        self.rec.add("cand", "re2", QUERY, manifest={"binary_sha256": "f" * 64, "extractor_sha256": "0" * 64})
        w = orep.warning_lines(ocmp.compare_labels(REF, "cand"))
        self.assertTrue(any("label cand mixes binaries across projects" in x for x in w), w)
        self.assertTrue(any("cand/re2 items were extracted by another version" in x for x in w), w)
        self.assertTrue(any("does not say which commit" in x for x in w), w)

    def test_score_document_is_deterministic_and_complete(self):
        allow = ocmp.AllowList(None)
        doc = orep.score_document(self.cmp, allow)
        again = orep.score_document(ocmp.compare_labels(REF, "cand"), allow)
        self.assertEqual(orep.score_json(doc), orep.score_json(again))
        self.assertEqual(json.loads(orep.score_json(doc)), doc)
        self.assertEqual((doc["label"], doc["reference"], doc["clean"]), ("cand", REF, False))
        self.assertEqual(doc["provenance"]["brotli"]["candidate"]["repo_commit"], "c" * 40)
        self.assertTrue(doc["provenance"]["brotli"]["candidate"]["repo_dirty"])
        self.assertEqual(doc["totals"], self.cmp.totals())
        self.assertEqual(doc["referee_sha256"], oc.referee_sha256())
        self.assertEqual(len(doc["rows"]), len(oc.SURFACES))
        md = orep.score_markdown(doc)
        self.assertIn("| brotli | cquery_label_kind | MISSING | 2 | 0 | 0 | 0 | 2 | 0 | 0.0 |", md)
        self.assertIn("Verdict: **FAIL (2 surfaces DIFF; 1 surface MISSING) [10 surfaces recorded by neither side]**", md)
        self.assertIn("verdict: FAIL (2 surfaces DIFF; 1 surface MISSING) [10 surfaces", orep.score_text(doc))

    def test_home_relative(self):
        self.assertEqual(orep.home_relative(os.path.join(oc.HOME, "x", "blaze")), "~/x/blaze")
        self.assertEqual(orep.home_relative("/opt/blaze"), "/opt/blaze")


class ToolsTest(unittest.TestCase):
    def setUp(self):
        self.rec = Records(self)
        self.compare = load_tool("compare")
        self.scoreboard = load_tool("scoreboard")

    def test_compare_exit_status(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add(REF, "re2", QUERY)
        self.rec.add("same", "brotli", QUERY)
        self.rec.add("same", "re2", QUERY)
        self.rec.add("partial", "brotli", QUERY)
        rc, out, _ = run_main(self.compare, [REF, "same"])
        self.assertEqual(rc, 0, out)
        self.assertIn("result: 100%% MATCH [%d surfaces recorded by neither side]" % (2 * (len(oc.SURFACES) - 2)), out)
        self.assertIn("reference items: total 10 = matching 10 + allowed 0 + failed/missing 0; compared 10; extra 0",
                      out)
        rc, out, _ = run_main(self.compare, [REF, "partial"])
        self.assertEqual(rc, 1)
        self.assertIn("re2      NOT RECORDED under this label", out)
        self.assertIn("== re2 MISSING: B did not record 2 surfaces (5 items on the other side): "
                      "exit_codes, query_label_kind", out)
        self.assertIn("result: FAIL (2 surfaces MISSING)", out)
        # The explicit restriction passes, says so, and does not overwrite the unrestricted report.
        rc, out, _ = run_main(self.compare, [REF, "partial", "--project", "brotli"])
        self.assertEqual(rc, 0)
        self.assertIn("[restricted to projects brotli]", out)
        self.assertIn("re2: 13 surfaces (5 reference items), outside the requested restriction", out)
        reports = sorted(os.listdir(os.path.join(oc.RECORDS, "compare")))
        self.assertEqual(reports, ["%s__vs__partial.json" % REF, "%s__vs__partial__only-brotli.json" % REF,
                                   "%s__vs__same.json" % REF])
        report = oc.read_json(os.path.join(oc.RECORDS, "compare", "%s__vs__partial.json" % REF))
        self.assertTrue(report["verdict"].startswith("FAIL"))
        self.assertFalse(report["provenance"]["re2"]["b"]["recorded"])

    def test_compare_prints_order_differences(self):
        self.rec.add(REF, "brotli", {"cquery_label_kind": {"deps //:a (c)": "k", "deps //:b (c)": "k",
                                                           "__order__ deps": ["deps //:a (c)", "deps //:b (c)"]}})
        self.rec.add("cand", "brotli", {"cquery_label_kind": {"deps //:a (c)": "k", "deps //:b (c)": "k",
                                                              "__order__ deps": ["deps //:b (c)", "deps //:a (c)"]}})
        rc, out, _ = run_main(self.compare, [REF, "cand"])
        self.assertEqual(rc, 1)
        self.assertIn("== brotli/cquery_label_kind DIFF __order__ deps", out)
        self.assertRegex(out, r"brotli +cquery_label_kind +DIFF +3 +2 +1 +0 +0 +0\n")

    def test_scoreboard_formats_and_write(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add("cand", "brotli", QUERY, manifest=CLEAN)
        rc, text, _ = run_main(self.scoreboard, ["cand"])
        self.assertEqual(rc, 0)
        self.assertIn("scoreboard: cand vs reference %s" % REF, text)
        self.assertIn("verdict: 100% MATCH", text)
        rc, js, _ = run_main(self.scoreboard, ["cand", "--json"])
        doc = json.loads(js)
        self.assertEqual((doc["schema"], doc["label"], doc["clean"]), (1, "cand", True))
        rc, md, _ = run_main(self.scoreboard, ["cand", "--markdown"])
        self.assertTrue(md.startswith("# Score: cand against %s\n" % REF))
        rc, out, _ = run_main(self.scoreboard, ["cand", "--write"])
        self.assertEqual(rc, 0)
        self.assertEqual(sorted(os.listdir(oc.CORPUS_SCORES)), ["cand.json", "cand.md"])
        self.assertEqual(read(os.path.join(oc.CORPUS_SCORES, "cand.json")), js)
        self.assertEqual(read(os.path.join(oc.CORPUS_SCORES, "cand.md")), md)
        path = os.path.join(self.rec.root, "out.md")
        run_main(self.scoreboard, ["cand", "--markdown", "--out", path])
        self.assertEqual(read(path), md)

    def test_scoreboard_write_takes_no_restriction(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add("cand", "brotli", QUERY)
        rc, _, err = run_main(self.scoreboard, ["cand", "--write", "--project", "brotli"])
        self.assertEqual(rc, 2)
        self.assertIn("--write scores the whole scope", err)
        self.assertFalse(os.path.exists(oc.CORPUS_SCORES))
        rc, _, err = run_main(self.scoreboard, ["cand", "--json", "--markdown"])
        self.assertEqual(rc, 2)

    def test_scoreboard_shows_a_missing_project_as_failed(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add(REF, "re2", QUERY)
        self.rec.add("cand", "brotli", QUERY)
        rc, text, _ = run_main(self.scoreboard, ["cand"])
        self.assertRegex(text, r"re2 +NOT RECORDED")
        self.assertRegex(text, r"query_label_kind +MISSING +2 +0 +0 +0 +2 +0 +0\.0\n")
        self.assertIn("verdict: FAIL (2 surfaces MISSING)", text)
        self.assertNotIn("100% MATCH", text)

    def test_usage(self):
        for tool in (self.compare, self.scoreboard):
            rc, out, _ = run_main(tool, ["help"])
            self.assertEqual(rc, 0)
            self.assertIn("usage: scripts/oracle/", out)


class ServerStateTest(unittest.TestCase):
    """Referee batch 3, item 4: which commands ran on a cold or a warm server, on each side."""

    def setUp(self):
        self.rec = Records(self)

    def add_with_stderr(self, label, stderr):
        """A recording made before batch 3: no `server` fields, only raw/<command>.stderr."""
        rd = self.rec.add(label, "brotli", QUERY, manifest=dict(CLEAN, commands=[
            {"name": n, "exit_code": 0} for n in stderr]))
        os.makedirs(os.path.join(rd, "raw"))
        for name, text in stderr.items():
            with open(os.path.join(rd, "raw", name + ".stderr"), "w") as f:
                f.write(text)

    def add_with_pids(self, label, states, mode="daemon"):
        self.rec.add(label, "brotli", QUERY, manifest=dict(CLEAN, server={"blaze_server_mode": mode}, commands=[
            {"name": n, "exit_code": 0, "server": {"pid_before": 1, "pid_after": 1, "start_line": False,
                                                    "state": st}} for n, st in states]))

    START = "Starting local Bazel server (9.3.2) and connecting to it...\n"

    def test_states_from_stderr_for_older_recordings(self):
        self.add_with_stderr(REF, {"mod_graph": self.START, "build": "INFO: x\n", "info": ""})
        m = oc.load_manifest(REF, "brotli")
        self.assertEqual(oc.server_states(REF, "brotli", m),
                         {"mod_graph": ["cold", "stderr"], "build": ["warm", "stderr"], "info": ["warm", "stderr"]})

    def test_one_process_against_a_server_is_named(self):
        self.add_with_stderr(REF, {"mod_graph": self.START, "build": "", "info": ""})
        self.add_with_stderr("cand", {"mod_graph": self.START, "build": self.START, "info": self.START})
        cmp = ocmp.compare_labels(REF, "cand")
        self.assertIn("brotli: commands on a warm server in %s and a cold one in cand: build, info" % REF,
                      orep.warning_lines(cmp))
        self.assertIn("server cand/brotli: server mode unset; cold (from stderr): mod_graph, build, info",
                      orep.server_lines(cmp))
        doc = orep.score_document(cmp, ocmp.AllowList(None))
        self.assertEqual(doc["provenance"]["brotli"]["candidate"]["server_states"]["build"], ["cold", "stderr"])

    def test_daemon_mode_against_a_server_has_no_warning(self):
        self.add_with_stderr(REF, {"mod_graph": self.START, "build": "", "info": ""})
        self.add_with_pids("cand", [("mod_graph", "cold"), ("build", "warm"), ("info", "warm")])
        cmp = ocmp.compare_labels(REF, "cand")
        self.assertFalse([w for w in orep.warning_lines(cmp) if "server" in w])
        self.assertIn("server cand/brotli: server mode daemon (daemon-mode rules); cold: mod_graph; warm: build, info",
                      orep.server_lines(cmp))

    def test_a_failed_shutdown_is_a_warning(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add("cand", "brotli", QUERY, manifest=dict(CLEAN, server={
            "blaze_server_mode": "daemon", "shutdown_exit_code": 0, "alive_after_shutdown": [{"pid": 7}]}))
        self.assertIn("cand/brotli: final shutdown exited 0; servers alive after it: [{'pid': 7}]",
                      orep.warning_lines(ocmp.compare_labels(REF, "cand")))

    def test_a_staged_comparison_says_so(self):
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add("cand", "brotli", QUERY)
        saved = oc.STAGE
        oc.STAGE = self.rec.root
        self.addCleanup(setattr, oc, "STAGE", saved)
        w = orep.warning_lines(ocmp.compare_labels(REF, "cand"))
        self.assertTrue(any(x.startswith("staged: recordings read from ") for x in w), w)


class DaemonModeAllowTest(unittest.TestCase):
    """Referee batch 3, item 4: the buildGraphMetrics fields that the oracle never varies in
    (built, changed, cleaned counts, the node count) are excused only for a one-process
    candidate; a daemon-mode candidate is held to the fields the oracle does vary in."""

    BM = '{"buildMetrics":{}}'

    def metrics(self, built, evaluated):
        return {self.BM: {"buildMetrics": {"buildGraphMetrics": {
            "builtValues": [{"skyfunctionName": "FILE", "count": built}],
            "evaluatedValues": [{"skyfunctionName": "FILE", "count": evaluated}]}}}}

    def setUp(self):
        self.rec = Records(self)
        self.rec.add(REF, "brotli", {"bep": self.metrics("10", "20")})

    def status(self, mode, built, evaluated):
        self.rec.add("cand-%s-%s-%s" % (mode, built, evaluated), "brotli", {"bep": self.metrics(built, evaluated)},
                     manifest={"server": {"blaze_server_mode": mode}})
        r = by_surface(ocmp.compare_labels(REF, "cand-%s-%s-%s" % (mode, built, evaluated), surfaces=["bep"]),
                       "brotli")["bep"]
        return r["status"], r["counts"]["ALLOWED"], r["counts"]["DIFF"]

    def test_one_process_candidate_scores_as_before(self):
        self.assertEqual(self.status(None, "15", "25"), ("MATCH", 1, 0))
        self.assertEqual(self.status("inprocess", "15", "25"), ("MATCH", 1, 0))

    def test_daemon_candidate_is_not_excused_for_built_values(self):
        self.assertEqual(self.status("daemon", "15", "25"), ("DIFF", 0, 1))

    def test_daemon_candidate_is_excused_where_the_oracle_varies(self):
        self.assertEqual(self.status("daemon", "10", "25"), ("MATCH", 1, 0))

    def test_the_real_entry_drops_the_four_cold_server_fields_in_daemon_mode(self):
        for path in ("buildMetrics.buildGraphMetrics.evaluatedValues[3].count",
                     "buildMetrics.buildGraphMetrics.dirtiedValues[2].skyfunctionName",
                     "buildMetrics.actionSummary.actionCacheStatistics.sizeInBytes"):
            self.assertIsNotNone(ocmp.AllowList().match("re2", "bep", self.BM, "DIFF", [path], "daemon"), path)
        for path in ("buildMetrics.buildGraphMetrics.builtValues[0].count",
                     "buildMetrics.buildGraphMetrics.postInvocationSkyframeNodeCount",
                     "buildMetrics.buildGraphMetrics.changedValues", "buildMetrics.buildGraphMetrics.cleanedValues"):
            self.assertIsNotNone(ocmp.AllowList().match("re2", "bep", self.BM, "DIFF", [path]), path)
            self.assertIsNone(ocmp.AllowList().match("re2", "bep", self.BM, "DIFF", [path], "daemon"), path)


class DaemonRulesTest(unittest.TestCase):
    """Batch 3b (audit of batch 3, A1, A2): under the daemon-mode rules a command that ran cold on
    one side and warm on the other, or a failed final shutdown, fails exit_codes; one-process
    candidates keep the warning. The rules follow the server the recorder observed."""

    START = "Starting local Bazel server (9.3.2) and connecting to it...\n"
    CLEAN_SHUTDOWN = {"shutdown_exit_code": 0, "alive_after_shutdown": []}

    def setUp(self):
        self.rec = Records(self)
        codes = {"mod_graph": 0, "build": 0, "info": 0}
        rd = self.rec.add(REF, "brotli", {"exit_codes": codes}, manifest=dict(CLEAN, commands=[
            {"name": n, "exit_code": 0} for n in codes]))
        os.makedirs(os.path.join(rd, "raw"))
        for name, text in (("mod_graph", self.START), ("build", ""), ("info", "")):
            with open(os.path.join(rd, "raw", name + ".stderr"), "w") as f:
                f.write(text)

    def cand(self, states, mode="daemon", shutdown=None, label="cand"):
        server = dict(self.CLEAN_SHUTDOWN if shutdown is None else shutdown, blaze_server_mode=mode)
        self.rec.add(label, "brotli", {"exit_codes": {n: 0 for n, _ in states}}, manifest=dict(
            CLEAN, server=server, commands=[
                {"name": n, "exit_code": 0, "server": {"pid_before": 1, "pid_after": 1 if st == "warm" else 2,
                                                        "start_line": st == "cold", "state": st}}
                for n, st in states]))
        cmp = ocmp.compare_labels(REF, label, surfaces=["exit_codes"])
        return cmp, by_surface(cmp, "brotli")["exit_codes"]

    SAME = [("mod_graph", "cold"), ("build", "warm"), ("info", "warm")]
    COLD_BUILD = [("mod_graph", "cold"), ("build", "cold"), ("info", "warm")]

    def test_equal_states_and_a_clean_shutdown_match(self):
        _, r = self.cand(self.SAME)
        self.assertEqual((r["status"], r["counts"]["MATCH"]), ("MATCH", 3))

    def test_a_cold_command_fails_its_exit_code_item(self):
        cmp, r = self.cand(self.COLD_BUILD)
        self.assertEqual(r["status"], "DIFF")
        self.assertEqual([(d["key"], d["kind"], d["paths"]) for d in r["details"]], [("build", "DIFF", ["server_state"])])
        self.assertEqual(r["_a"]["build"], {"exit_code": 0, "server_state": "warm"})
        self.assertEqual(r["_b"]["build"], {"exit_code": 0, "server_state": "cold"})
        # The reference's items are not changed: the same number of reference items.
        self.assertEqual(r["total_a"], 3)
        self.assertTrue(any("runs under the daemon-mode rules, so these commands fail" in w
                            for w in orep.warning_lines(cmp)))

    def test_a_failed_shutdown_fails_exit_codes(self):
        for sd in ({"shutdown_exit_code": 1, "alive_after_shutdown": []},
                   {"shutdown_exit_code": 0, "alive_after_shutdown": [{"pid": 7}]},
                   {}):
            label = "cand%d" % len(str(sd))
            _, r = self.cand(self.SAME, shutdown=sd, label=label)
            self.assertEqual(r["status"], "DIFF", sd)
            self.assertEqual([(d["key"], d["kind"]) for d in r["details"]], [(ocmp.SHUTDOWN_ITEM, "EXTRA")], sd)

    def test_one_process_candidates_keep_the_warning(self):
        cmp, r = self.cand([("mod_graph", "cold"), ("build", "cold"), ("info", "cold")], mode=None,
                           shutdown={"shutdown_exit_code": 1})
        self.assertEqual(r["status"], "MATCH")
        self.assertIn("brotli: commands on a warm server in %s and a cold one in cand: build, info" % REF,
                      orep.warning_lines(cmp))

    def test_an_observed_warm_server_gets_the_strict_rules_without_the_mode(self):
        # A2: a binary whose default is a server; the recorder was not asked for daemon mode.
        _, r = self.cand(self.COLD_BUILD, mode=None)
        self.assertEqual(r["status"], "DIFF")
        self.assertTrue(oc.strict_server_rules({"commands": [{"name": "build", "server": {"state": "warm"}}]}))
        self.assertTrue(oc.strict_server_rules({"server": {"blaze_server_mode": "daemon"}}))
        self.assertFalse(oc.strict_server_rules({"commands": [{"name": "build", "server": {"state": "cold"}}]}))
        self.assertFalse(oc.strict_server_rules({"commands": [{"name": "build", "exit_code": 0}]}))

    def test_an_observed_warm_server_gets_the_daemon_allow_fields(self):
        bm = '{"buildMetrics":{}}'

        def bep(built):
            return {bm: {"buildMetrics": {"buildGraphMetrics": {"builtValues": [{"skyfunctionName": "FILE",
                                                                                 "count": built}]}}}}
        self.rec.add(REF + "-x", "brotli", {"bep": bep("10")})
        for label, states in (("lenient", [("build", "cold")]), ("observed", [("build", "warm")])):
            self.rec.add(label, "brotli", {"bep": bep("15")}, manifest=dict(CLEAN, server={}, commands=[
                {"name": "build", "exit_code": 0, "server": {"state": st}} for _, st in states]))
        got = {l: by_surface(ocmp.compare_labels(REF + "-x", l, surfaces=["bep"]), "brotli")["bep"]["status"]
               for l in ("lenient", "observed")}
        self.assertEqual(got, {"lenient": "MATCH", "observed": "DIFF"})


class EmptyFieldListTest(unittest.TestCase):
    """Batch 3b (audit of batch 3, A3): an empty field list must excuse nothing."""

    BASE = {"id": "e", "surface": "bep", "projects": "*", "item": ".", "kinds": ["DIFF"], "max_items": {"p": 1}}

    def test_prepare_refuses_empty_lists(self):
        for extra in ({}, {"only_fields": []}, {"only_fields": ["a"], "only_fields_in_daemon_mode": []}):
            with self.assertRaises(ocmp.AllowListError, msg=extra):
                ocmp.AllowList.prepare(dict(self.BASE, **extra))
        # MISSING/EXTRA entries need no field list.
        ocmp.AllowList.prepare(dict(self.BASE, kinds=["MISSING"]))

    def test_match_excuses_nothing_without_fields(self):
        al = ocmp.AllowList(None)
        e = ocmp.AllowList.prepare(dict(self.BASE, only_fields=["a"], only_fields_in_daemon_mode=["a"]))
        e["_daemon_field_res"] = []  # as an entry with an empty list would have been prepared
        al.entries = [e]
        self.assertIsNotNone(al.match("p", "bep", "k", "DIFF", ["a"]))
        self.assertIsNone(al.match("p", "bep", "k", "DIFF", ["a"], "daemon"))
        self.assertIsNone(al.match("p", "bep", "k", "DIFF", ["b"], "daemon"))


class StagedFieldTest(unittest.TestCase):
    """Batch 3b (audit of batch 3, B1): a staged score says so in a top-level field."""

    def setUp(self):
        self.rec = Records(self)
        self.rec.add(REF, "brotli", QUERY)
        self.rec.add("cand", "brotli", QUERY)

    def doc(self):
        return orep.score_document(ocmp.compare_labels(REF, "cand"), ocmp.AllowList(None))

    def test_unstaged(self):
        self.assertIsNone(self.doc()["staged"])

    def test_staged(self):
        self.addCleanup(setattr, oc, "STAGE", oc.STAGE)
        oc.STAGE = self.rec.root
        doc = self.doc()
        self.assertEqual(sorted(doc["staged"]), ["records", "stage", "summaries"])
        self.assertEqual(doc["staged"]["records"], orep.home_relative(oc.RECORDS))
        self.assertIn('"staged": {', orep.score_json(doc))


class StageTest(unittest.TestCase):
    """BLAZE_ORACLE_STAGE moves the records, the reference summaries and the written scores."""

    def paths(self, env):
        code = "import oracle_common as oc; print(oc.STAGE); print(oc.RECORDS); print(oc.CORPUS_SUMMARIES); " \
               "print(oc.CORPUS_SCORES)"
        import subprocess
        import sys
        out = subprocess.run([sys.executable, "-I", "-c", "import sys; sys.path.insert(0, %r); %s" % (
            os.path.dirname(os.path.abspath(__file__)), code)], env=env, stdout=subprocess.PIPE, check=True)
        return out.stdout.decode().splitlines()

    def test_unset(self):
        got = self.paths({"HOME": oc.HOME, "PATH": "/usr/bin:/bin"})
        self.assertEqual(got, ["None", os.path.join(oc.CACHE, "oracle-records"),
                               os.path.join(oc.REPO_ROOT, "corpus", "oracle"),
                               os.path.join(oc.REPO_ROOT, "corpus", "scores")])

    def test_set(self):
        got = self.paths({"HOME": oc.HOME, "PATH": "/usr/bin:/bin", "BLAZE_ORACLE_STAGE": "/x/stage"})
        self.assertEqual(got, ["/x/stage", "/x/stage/oracle-records", "/x/stage/corpus-oracle", "/x/stage/scores"])


if __name__ == "__main__":
    unittest.main()
