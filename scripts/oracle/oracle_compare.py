"""Item-by-item comparison of two recordings, with the allow list. Used by compare and scoreboard."""

import json
import os
import re

import oracle_common as oc
import surfaces as _surfaces

RESULT_KINDS = ("MATCH", "DIFF", "MISSING", "EXTRA", "ALLOWED")

# Status of one project/surface comparison. Only MATCH is a pass.
#   MATCH         both sides produced the surface and every item is equal or allowed
#   DIFF          both sides produced it and some item is DIFF, MISSING or EXTRA
#   FAILED        a side ran the surface's commands but could not produce it, an items file does
#                 not match its recorded sha256, or both sides are empty and that could not be
#                 verified as genuine (see _empty_check)
#   MISSING       A (the reference) has the surface, B did not record it: the project is absent
#                 from B or B skipped the step. Every reference item counts as failed.
#   NO_REFERENCE  B has the surface, A did not record it: nothing to judge B against
#   NOT_RECORDED  neither side recorded it. Excluded from the comparison, and reported as such.
SURFACE_STATUSES = ("MATCH", "DIFF", "FAILED", "MISSING", "NO_REFERENCE", "NOT_RECORDED")
NOT_PRODUCED = ("SKIPPED", "ABSENT")


# ---------------------------------------------------------------------------------------------
# allow list


class AllowListError(Exception):
    pass


class AllowList:
    """scripts/oracle/allow.json. Each entry permits a residual difference under a condition.

    Entry fields:
      id          short name, printed in reports
      projects    list of project names, or "*"
      surface     comparison surface name
      item        regex matched (re.search) against the item key
      kinds       subset of ["DIFF", "MISSING", "EXTRA"] the entry may excuse
      only_fields for DIFF: regexes (re.fullmatch) over the JSON paths of the differing leaves;
                  the entry applies only if every differing path matches one of them
      max_items   required. How many items the entry may excuse in one project and surface:
                  {"<project>": N, ...}, taken from the oracle compared with itself. A project
                  that is not named has the cap 0. An entry that would excuse more excuses none:
                  it is reported as violated and its items count as differences.
      condition   the condition above, in words
      reason      why real Bazel differs from itself here
      only_fields_in_daemon_mode
                  optional (referee batch 3): the only_fields that apply when the candidate was
                  recorded on Blaze's persistent server (`record --blaze-server daemon`). An
                  only_fields regex left out of it exists only because a one-process Blaze runs
                  every command on a cold server while the oracle's build runs on a warm one, and
                  is inactive in daemon mode. Every other mode uses only_fields.
    """

    def __init__(self, path=oc.ALLOW_FILE):
        self.entries = []
        if path and os.path.isfile(path):
            self.entries = oc.read_json(path).get("entries", [])
        for e in self.entries:
            self.prepare(e)

    @staticmethod
    def prepare(e):
        """Validate one entry and compile its regexes."""
        m = e.get("max_items")
        if not isinstance(m, dict) or not all(
                isinstance(k, str) and isinstance(v, int) and not isinstance(v, bool) and v >= 0
                for k, v in m.items()):
            raise AllowListError(
                "allow-list entry %r: max_items is required and must be an object "
                "{project: non-negative integer}" % e.get("id"))
        # Audit of batch 3, A3: an empty field list would excuse every difference of the item.
        if "DIFF" in e.get("kinds", ["DIFF"]):
            for name in ("only_fields", "only_fields_in_daemon_mode"):
                if (name == "only_fields" or name in e) and not e.get(name):
                    raise AllowListError(
                        "allow-list entry %r excuses DIFF items, so %s must name at least one field "
                        "(an empty list would excuse every difference)" % (e.get("id"), name))
        e["_item_re"] = re.compile(e.get("item", ".*"))
        e["_field_res"] = [re.compile(f) for f in e.get("only_fields", [])]
        if "only_fields_in_daemon_mode" in e:
            e["_daemon_field_res"] = [re.compile(f) for f in e["only_fields_in_daemon_mode"]]
        return e

    @staticmethod
    def cap(entry, project):
        return entry["max_items"].get(project, 0)

    def match(self, project, surface, key, kind, diff_paths, mode=None):
        """The entry that excuses this item, or None. mode "daemon": the candidate is judged by
        the daemon-mode rules (oc.strict_server_rules), so only_fields_in_daemon_mode applies."""
        for e in self.entries:
            if e.get("surface") != surface:
                continue
            projects = e.get("projects", "*")
            if projects != "*" and project not in projects:
                continue
            if kind not in e.get("kinds", ["DIFF"]):
                continue
            if not e["_item_re"].search(key):
                continue
            field_res = e["_daemon_field_res"] if mode == "daemon" and "_daemon_field_res" in e else e["_field_res"]
            if kind == "DIFF":
                # No field list excuses nothing (A3; prepare refuses such an entry anyway).
                if not field_res or not diff_paths or not all(
                        any(r.fullmatch(p) for r in field_res) for p in diff_paths):
                    continue
            return e
        return None


# ---------------------------------------------------------------------------------------------
# diffing values


def diff_paths(a, b, path="", out=None):
    """JSON paths of every leaf where a and b differ (lists of unequal length: the list path).

    Never truncated: the allow list judges a DIFF by all of its paths, so a cut-off list would let
    many allowed paths hide one that is not allowed (audit R2A-1). Only what is printed is cut.
    """
    if out is None:
        out = []
    if type(a) is not type(b):
        out.append(path or "$")
    elif isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            p = "%s.%s" % (path, k) if path else k
            if k not in a or k not in b:
                out.append(p)
            else:
                diff_paths(a[k], b[k], p, out)
    elif isinstance(a, list):
        if len(a) != len(b):
            out.append(path or "$")
        else:
            for i, (x, y) in enumerate(zip(a, b)):
                diff_paths(x, y, "%s[%d]" % (path, i), out)
    elif a != b:
        out.append(path or "$")
    return out


def get_path(obj, path):
    if path == "$":
        return obj
    for part in re.findall(r"[^.\[\]]+|\[\d+\]", path):
        if part.startswith("["):
            idx = int(part[1:-1])
            obj = obj[idx] if isinstance(obj, list) and idx < len(obj) else None
        else:
            obj = obj.get(part) if isinstance(obj, dict) else None
    return obj


def _short(v, n=160):
    s = json.dumps(v, sort_keys=True, ensure_ascii=False)
    return s if len(s) <= n else s[: n - 3] + "..."


def describe_diff(a, b, paths, max_paths=8, total=None):
    """Human-readable lines for one DIFF item. `total`: how many paths differ in all, when
    `paths` is a truncated list."""
    total = len(paths) if total is None else total
    lines = []
    for p in paths[:max_paths]:
        va, vb = get_path(a, p), get_path(b, p)
        if isinstance(va, list) and isinstance(vb, list):
            sa = {oc.canonical(x): x for x in va}
            sb = {oc.canonical(x): x for x in vb}
            gone = [sa[k] for k in sa if k not in sb]
            new = [sb[k] for k in sb if k not in sa]
            lines.append("  %s: list of %d vs %d" % (p, len(va), len(vb)))
            for x in gone[:6]:
                lines.append("    - %s" % _short(x))
            for x in new[:6]:
                lines.append("    + %s" % _short(x))
            if not gone and not new:
                lines.append("    (same elements, different order or multiplicity)")
            first = next((i for i, (x, y) in enumerate(zip(va, vb)) if x != y), min(len(va), len(vb)))
            if first < max(len(va), len(vb)):
                lines.append("    first difference at index %d:" % first)
                lines.append("    - %s" % (_short(va[first]) if first < len(va) else "(end of list)"))
                lines.append("    + %s" % (_short(vb[first]) if first < len(vb) else "(end of list)"))
        else:
            lines.append("  %s:" % p)
            lines.append("    - %s" % _short(va))
            lines.append("    + %s" % _short(vb))
    if total > min(max_paths, len(paths)):
        lines.append("  ... %d more differing paths" % (total - min(max_paths, len(paths))))
    return lines


# ---------------------------------------------------------------------------------------------
# loading and comparing


def load_surface(label, project, surface, manifest=None, summary=None):
    """items/<surface>.json of one recording; {"status": "ABSENT"} when there is none.

    With a manifest (and, for the reference, the committed summary corpus/oracle/<project>.json)
    the file's sha256 is checked against the recorded one. A file that does not match is FAILED:
    it was edited or rebuilt after the manifest (or the committed summary) was written.
    """
    path = os.path.join(oc.record_dir(label, project), "items", surface + ".json")
    if not os.path.isfile(path):
        return {"status": "ABSENT", "items": {}}
    with open(path, "rb") as f:
        data = f.read()
    res = json.loads(data)
    name = "items/%s.json" % surface
    for what, hashes in (("manifest.json", (manifest or {}).get("files")),
                         ("the committed corpus/oracle/%s.json" % project, (summary or {}).get("files_sha256"))):
        if hashes is None:
            continue
        if hashes.get(name) != oc.sha256_bytes(data):
            res = dict(res, status="FAILED",
                       error="%s/%s/%s does not match the sha256 in %s" % (label, project, name, what))
            break
    return res


def _empty_check(surface, exits_a, exits_b):
    """Why an empty-vs-empty surface is not a genuine match, or None when it is one.

    Zero items on both sides is a match only when the reference is genuinely empty and both sides
    ran: every command the surface is extracted from was run on both sides, succeeded on A, and
    exited with the same code on B.
    """
    commands = _surfaces.SURFACE_COMMANDS.get(surface)
    if not commands:
        return "0 items on both sides"
    for c in commands:
        ea, eb = (exits_a or {}).get(c), (exits_b or {}).get(c)
        if ea is None or eb is None:
            return "0 items on both sides and command %s has no recorded exit code on %s" % (
                c, "A" if ea is None else "B")
        ok = _surfaces.QUERY_OK if c.startswith("query_") else (0,)
        if ea not in ok:
            return "0 items on both sides and command %s exited %d on A" % (c, ea)
        if eb != ea:
            return "0 items on both sides, but command %s exited %d on A and %d on B" % (c, ea, eb)
    return None


def compare_surface(project, surface, a, b, allow, exits_a=None, exits_b=None, mode_b=None):
    """Compare two loaded surface files. Returns a result dict with counts and per-item details.

    exits_a/exits_b: {command: exit code} of each side (its exit_codes items), used to decide
    whether an empty-vs-empty surface is genuine. mode_b: the candidate's blaze_server_mode
    (AllowList.match).
    """
    res = {"project": project, "surface": surface, "status_a": a.get("status"), "status_b": b.get("status"),
           "counts": {k: 0 for k in RESULT_KINDS}, "total_a": len(a.get("items", {})),
           "total_b": len(b.get("items", {})), "details": [], "allow_used": {}, "allow_violations": [],
           "exceptions": [], "empty": False}
    absent_a, absent_b = a.get("status") in NOT_PRODUCED, b.get("status") in NOT_PRODUCED
    if absent_a or absent_b:
        res["status"] = "NOT_RECORDED" if absent_a and absent_b else "NO_REFERENCE" if absent_a else "MISSING"
        return res
    if b.get("status") == "FAILED" or a.get("status") == "FAILED":
        res["status"] = "FAILED"
        res["failure"] = {"a": {k: a.get(k) for k in ("error", "stderr_tail") if k in a},
                          "b": {k: b.get(k) for k in ("error", "stderr_tail") if k in b}}
    ia, ib = a.get("items", {}), b.get("items", {})
    per_entry = {}
    matching = []
    for key in sorted(set(ia) | set(ib)):
        if key in ia and key in ib:
            if oc.canonical(ia[key]) == oc.canonical(ib[key]):
                res["counts"]["MATCH"] += 1
                matching.append(key)
                continue
            kind = "DIFF"
            paths = diff_paths(ia[key], ib[key])
        else:
            kind = "MISSING" if key in ia else "EXTRA"
            paths = []
        # The allow list sees every differing path; only the stored and printed list is cut.
        entry = allow.match(project, surface, key, kind, paths, mode_b)
        detail = {"key": key, "kind": kind, "paths": paths[:50], "paths_total": len(paths)}
        if entry is not None:
            per_entry.setdefault(entry["id"], []).append(detail)
            detail["allowed_by"] = entry["id"]
        res["details"].append(detail)
    for eid, details in per_entry.items():
        entry = next(e for e in allow.entries if e["id"] == eid)
        cap = allow.cap(entry, project)
        if len(details) > cap:
            res["allow_violations"].append("%s excuses %d items > max_items %d for project %s" % (
                eid, len(details), cap, project))
            for d in details:
                d.pop("allowed_by", None)
        else:
            res["allow_used"][eid] = len(details)
    for d in res["details"]:
        res["counts"]["ALLOWED" if "allowed_by" in d else d["kind"]] += 1
    if "status" not in res and not ia and not ib:
        res["empty"] = True
        why = _empty_check(surface, exits_a, exits_b)
        if why:
            res["status"] = "FAILED"
            res["failure"] = {"a": {"error": why}, "b": {"error": why}}
    if "status" not in res:
        bad = res["counts"]["DIFF"] + res["counts"]["MISSING"] + res["counts"]["EXTRA"]
        res["status"] = "MATCH" if bad == 0 else "DIFF"
    res["_a"], res["_b"], res["_matching"] = ia, ib, matching
    return res


SHUTDOWN_ITEM = "__shutdown__"


def server_checked_exit_codes(a, b, states_a, states_b, shutdown_b):
    """exit_codes of both sides with the daemon-mode server checks (audit of batch 3, A1).

    For a candidate under the daemon-mode rules: a command that ran on a cold server on one side
    and a warm one on the other becomes a DIFF of its exit_codes item (path `server_state`); a
    final `shutdown` that exited non-zero, left a server alive or was not recorded becomes the
    candidate-only item SHUTDOWN_ITEM (EXTRA). Decided here, at compare time, so that no
    recording's items change. Returns new (a, b) surface dicts; the inputs are not modified.
    """
    ia, ib = dict(a.get("items", {})), dict(b.get("items", {}))
    for cmd in sorted(set(ia) & set(ib)):
        sa, sb = (states_a or {}).get(cmd), (states_b or {}).get(cmd)
        sa, sb = sa[0] if sa else None, sb[0] if sb else None
        if sa != sb:
            ia[cmd] = {"exit_code": ia[cmd], "server_state": sa}
            ib[cmd] = {"exit_code": ib[cmd], "server_state": sb}
    sd = shutdown_b or {}
    if sd.get("shutdown_exit_code") != 0 or sd.get("alive_after_shutdown"):
        ib[SHUTDOWN_ITEM] = {"shutdown_exit_code": sd.get("shutdown_exit_code"),
                             "alive_after_shutdown": sd.get("alive_after_shutdown")}
    return dict(a, items=ia), dict(b, items=ib)


def manifest_exceptions(manifest):
    """The declared exceptions of a recording: [{"id", "surfaces", "explanation", ...}].

    An exception says that part of the recording was not produced by the recorder's protocol (for
    example an output spliced in by hand). A malformed entry is kept and applies to every surface.
    """
    out = []
    for i, e in enumerate((manifest or {}).get("exceptions") or []):
        if not isinstance(e, dict):
            e = {"id": "malformed-%d" % i, "explanation": "malformed exception entry: %r" % (e,)}
        e = dict(e)
        e.setdefault("id", "unnamed-%d" % i)
        if not isinstance(e.get("surfaces"), list) or not e["surfaces"]:
            e["surfaces"] = list(oc.SURFACES)
        out.append(e)
    return out


def provenance(label, project, manifest):
    """What a recording says about where it came from (None values: not recorded)."""
    if manifest is None:
        return {"label": label, "project": project, "recorded": False}
    repo = manifest.get("blaze_repo") or {}
    origin = manifest.get("binary_origin")
    return {
        "label": label, "project": project, "recorded": True,
        "created": manifest.get("created"),
        "binary": manifest.get("binary"), "binary_sha256": manifest.get("binary_sha256"),
        "repo_commit": repo.get("commit"), "repo_dirty": repo.get("dirty"),
        "repo_dirty_diff_sha256": repo.get("dirty_diff_sha256"),
        # None: recorded before the check existed (audit R2A-2).
        "commit_verified": origin.get("commit_verified") if isinstance(origin, dict) else None,
        "commit_unverified_reasons": list(origin.get("reasons") or []) if isinstance(origin, dict) else None,
        "replaced": manifest.get("replaced"),
        "forced": manifest.get("forced"),
        "extractor_sha256": manifest.get("extractor_sha256"),
        "steps": manifest.get("steps"),
        "exceptions": manifest_exceptions(manifest),
        # Referee batch 3: BLAZE_SERVER_MODE the recorder set (None: unset), what it saw of the
        # server per command (oc.server_states), and the final shutdown.
        "server_mode": oc.blaze_server_mode(manifest),
        # Batch 3b (A2): judged by the daemon-mode rules (mode daemon, or a warm server observed).
        "strict_server_rules": oc.strict_server_rules(manifest),
        "server_states": oc.server_states(label, project, manifest),
        "shutdown": {k: v for k, v in (manifest.get("server") or {}).items()
                     if k in ("shutdown_exit_code", "alive_after_shutdown")} or None,
    }


def reference_exceptions(manifest, summary):
    """The reference's exceptions: those of the committed summary corpus/oracle/<project>.json
    (hash-anchored, audit R2A-5) together with any the manifest in ~/.cache adds. An exception
    only makes surfaces fail, so the union never lets a surface pass that either source covers;
    an exception removed from the unanchored manifest still counts."""
    out, seen = [], set()
    for e in manifest_exceptions({"exceptions": (summary or {}).get("exceptions") or []}) + manifest_exceptions(manifest):
        k = oc.canonical({k: v for k, v in e.items()})
        if k not in seen:
            seen.add(k)
            out.append(e)
    return out


def exception_source_mismatch(manifest, summary):
    """Why the reference manifest's exceptions differ from the committed summary's, or None."""
    if summary is None:
        return None
    m = sorted(oc.canonical(e) for e in (manifest or {}).get("exceptions") or [])
    c = sorted(oc.canonical(e) for e in summary.get("exceptions") or [])
    return None if m == c else "manifest exceptions %s differ from the committed summary's %s" % (
        [e.get("id") for e in (manifest or {}).get("exceptions") or [] if isinstance(e, dict)],
        [e.get("id") for e in summary.get("exceptions") or [] if isinstance(e, dict)])


def committed_summary(label, project, manifest):
    """corpus/oracle/<project>.json when `label` is the reference label, else None.

    The reference recording must be the one whose file hashes are committed: without a committed
    summary no items file of the reference can match.
    """
    if label != oc.REFERENCE_LABEL or manifest is None:
        return None
    path = os.path.join(oc.CORPUS_SUMMARIES, project + ".json")
    return oc.read_json(path) if os.path.isfile(path) else {"files_sha256": {}}


def compare_labels(label_a, label_b, projects=None, surfaces=None, allow=None):
    """Compare label_b (the candidate) with label_a (the reference). Returns a Comparison.

    Without `projects`/`surfaces` the scope is every project recorded under either label and every
    surface: what one side lacks is MISSING or NO_REFERENCE, never left out. `projects` and
    `surfaces` restrict the scope explicitly; the restriction and what it leaves out are part of
    the result (Comparison.restriction, Comparison.out_of_scope).
    """
    allow = allow if allow is not None else AllowList()
    every = sorted(set(oc.projects_in(label_a)) | set(oc.projects_in(label_b)))
    scope_projects = list(projects) if projects else every
    scope_surfaces = list(surfaces) if surfaces else list(oc.SURFACES)
    cmp = Comparison(label_a, label_b)
    cmp.restriction = {"projects": sorted(projects) if projects else None,
                       "surfaces": [s for s in oc.SURFACES if s in surfaces] if surfaces else None}
    cmp.projects = sorted(scope_projects)
    for p in sorted(set(every) | set(scope_projects)):
        ma, mb = oc.load_manifest(label_a, p), oc.load_manifest(label_b, p)
        cmp.provenance[p] = {"a": provenance(label_a, p, ma), "b": provenance(label_b, p, mb)}
        sa, sb = committed_summary(label_a, p, ma), committed_summary(label_b, p, mb)
        for side, label, m, summ in (("a", label_a, ma, sa), ("b", label_b, mb, sb)):
            if summ is not None:
                cmp.provenance[p][side]["exceptions"] = reference_exceptions(m, summ)
                why = exception_source_mismatch(m, summ)
                if why:
                    cmp.notes.append("%s/%s: %s; both are applied" % (label, p, why))
                path = os.path.join(oc.CORPUS_SUMMARIES, p + ".json")
                if os.path.isfile(path) and oc.head_version_differs(path):
                    cmp.notes.append("%s differs from its committed version at HEAD: the reference is "
                                     "checked against an uncommitted summary" % os.path.relpath(path, oc.REPO_ROOT))
        for s in oc.SURFACES:
            if p not in scope_projects or s not in scope_surfaces:
                n = ((ma or {}).get("surfaces") or {}).get(s, {}).get("items", 0)
                cmp.out_of_scope.append({"project": p, "surface": s, "reference_items": n})
        if p not in scope_projects:
            continue
        exits_a = load_surface(label_a, p, "exit_codes", ma, sa).get("items")
        exits_b = load_surface(label_b, p, "exit_codes", mb, sb).get("items")
        excs = [(side, e) for side in ("a", "b") for e in cmp.provenance[p][side].get("exceptions", [])]
        # Audit of batch 3, A1 and A2: the daemon-mode rules follow the server the recorder
        # observed, not only the mode it was asked for.
        strict = mb is not None and oc.strict_server_rules(mb)
        for s in scope_surfaces:
            sur_a, sur_b = load_surface(label_a, p, s, ma, sa), load_surface(label_b, p, s, mb, sb)
            if strict and s == "exit_codes" and sur_a.get("status") == "OK" and sur_b.get("status") == "OK":
                pa, pb = cmp.provenance[p]["a"], cmp.provenance[p]["b"]
                sur_a, sur_b = server_checked_exit_codes(sur_a, sur_b, pa.get("server_states"),
                                                         pb.get("server_states"), pb.get("shutdown"))
            r = compare_surface(p, s, sur_a, sur_b, allow, exits_a, exits_b, "daemon" if strict else None)
            r["exceptions"] = [{"side": side, "id": e["id"]} for side, e in excs if s in e["surfaces"]]
            cmp.results.append(r)
    return cmp


class Comparison:
    """The results of compare_labels: one result per project and surface in scope, plus what was
    left out and where both recordings came from."""

    def __init__(self, label_a, label_b):
        self.a, self.b = label_a, label_b
        self.results = []
        self.provenance = {}
        self.restriction = {"projects": None, "surfaces": None}
        self.projects = []  # the projects in scope
        self.out_of_scope = []
        self.notes = []  # warnings found while loading (exception sources, uncommitted summary)

    def __iter__(self):
        return iter(self.results)

    def __len__(self):
        return len(self.results)

    def restricted(self):
        return bool(self.restriction["projects"] or self.restriction["surfaces"])

    def exceptions(self):
        """[(side, label, project, exception)] for every declared exception of both recordings."""
        out = []
        for p in self.projects:
            for side, label in (("a", self.a), ("b", self.b)):
                for e in self.provenance[p][side].get("exceptions", []):
                    out.append((side, label, p, e))
        return out

    def rows(self):
        return [score_row(r) for r in self.results]

    def totals(self, rows=None):
        return totals(self.rows() if rows is None else rows)

    def failures(self):
        """Why the comparison is not a pass: [] when it is one."""
        why = []
        rows = self.rows()
        t = totals(rows)
        for status in ("DIFF", "FAILED", "MISSING", "NO_REFERENCE"):
            n = t["surfaces"][status]
            if n:
                why.append("%d surface%s %s" % (n, "" if n == 1 else "s", status))
        n = sum(len(r["allow_violations"]) for r in self.results)
        if n:
            why.append("%d allow-list entr%s over max_items" % (n, "y" if n == 1 else "ies"))
        n = sum(1 for r in self.results if r["exceptions"])
        if n:
            why.append("%d surface%s under a declared recording exception" % (n, "" if n == 1 else "s"))
        if t["surfaces"]["MATCH"] + t["surfaces"]["DIFF"] == 0:
            why.append("nothing was compared")
        return why

    def clean(self):
        return not self.failures()

    def verdict(self):
        """One line: the result, the restriction it holds under and what was excluded."""
        why = self.failures()
        t = self.totals()
        text = "100% MATCH" if not why else "FAIL (%s)" % "; ".join(why)
        notes = []
        if self.restricted():
            notes.append("restricted to %s" % self.restriction_text())
        if t["surfaces"]["NOT_RECORDED"]:
            notes.append("%d surfaces recorded by neither side" % t["surfaces"]["NOT_RECORDED"])
        return text + ("".join(" [%s]" % n for n in notes))

    def restriction_text(self):
        parts = []
        if self.restriction["projects"]:
            parts.append("projects " + ",".join(self.restriction["projects"]))
        if self.restriction["surfaces"]:
            parts.append("surfaces " + ",".join(self.restriction["surfaces"]))
        return "; ".join(parts) or "nothing"


def score_row(r):
    """The denominators of one project/surface result, counted in reference items.

      total     items in the reference (A)
      allowed   reference items whose difference an allow-list entry excuses (not judged)
      compared  reference items judged against the candidate: total - allowed when both sides
                produced the surface, 0 otherwise
      matching  reference items the candidate reproduces exactly
      failed    reference items not reproduced: DIFF and MISSING items, and every item of a
                surface that is FAILED, MISSING or NO_REFERENCE. total = matching + allowed + failed
      extra     items only the candidate has (not excused)
    """
    ia = r.get("_a", {})
    total = r["total_a"]
    if r["status"] in ("MATCH", "DIFF"):
        allowed = sum(1 for d in r["details"] if "allowed_by" in d and d["key"] in ia)
        matching = r["counts"]["MATCH"]
        compared = total - allowed
    else:
        allowed = matching = compared = 0
    extra = sum(1 for d in r["details"] if d["kind"] == "EXTRA" and "allowed_by" not in d)
    return {"project": r["project"], "surface": r["surface"], "status": r["status"],
            "total": total, "compared": compared, "matching": matching, "allowed": allowed,
            "failed": total - allowed - matching, "extra": extra, "empty": r.get("empty", False),
            "exceptions": [e["id"] for e in r.get("exceptions", [])],
            "allow_used": dict(r.get("allow_used", {})), "allow_violations": list(r.get("allow_violations", []))}


def totals(rows):
    t = {k: 0 for k in ("total", "compared", "matching", "allowed", "failed", "extra")}
    t["surfaces"] = {s: 0 for s in SURFACE_STATUSES}
    for row in rows:
        for k in ("total", "compared", "matching", "allowed", "failed", "extra"):
            t[k] += row[k]
        t["surfaces"][row["status"]] += 1
    return t


def percent(row):
    """matching as a percentage of every reference item that is not allow-listed.

    Rounded down to one decimal, so that 100.0 is printed only when nothing is missing.
    """
    judged = row["total"] - row["allowed"]
    return "%.1f" % ((1000 * row["matching"] // judged) / 10.0) if judged else "-"


def is_clean(r):
    return r["status"] == "MATCH" and not r["allow_violations"] and not r.get("exceptions")


def report_json(label_a, label_b, results):
    out = {"a": label_a, "b": label_b, "results": []}
    if isinstance(results, Comparison):
        out.update({"restriction": results.restriction, "provenance": results.provenance,
                    "out_of_scope": results.out_of_scope, "verdict": results.verdict(),
                    "totals": results.totals(), "referee_sha256": oc.referee_sha256()})
    for r in results:
        out["results"].append({
            k: v for k, v in r.items() if not k.startswith("_") and k != "details"
        } | {"non_matching": r["details"]})
    return out
