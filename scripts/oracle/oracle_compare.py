"""Item-by-item comparison of two recordings, with the allow list. Used by compare and scoreboard."""

import json
import os
import re

import oracle_common as oc

RESULT_KINDS = ("MATCH", "DIFF", "MISSING", "EXTRA", "ALLOWED")


# ---------------------------------------------------------------------------------------------
# allow list


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
      max_items   optional cap on how many items the entry may excuse per project and surface;
                  beyond it, the entry is reported as violated and its items count as differences
      condition   the condition above, in words
      reason      why real Bazel differs from itself here
    """

    def __init__(self, path=oc.ALLOW_FILE):
        self.entries = []
        if path and os.path.isfile(path):
            self.entries = oc.read_json(path).get("entries", [])
        for e in self.entries:
            e["_item_re"] = re.compile(e.get("item", ".*"))
            e["_field_res"] = [re.compile(f) for f in e.get("only_fields", [])]

    def match(self, project, surface, key, kind, diff_paths):
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
            if kind == "DIFF" and e["_field_res"]:
                if not diff_paths or not all(any(r.fullmatch(p) for r in e["_field_res"]) for p in diff_paths):
                    continue
            return e
        return None


# ---------------------------------------------------------------------------------------------
# diffing values


def diff_paths(a, b, path="", out=None, limit=200):
    """JSON paths of the leaves where a and b differ (lists of unequal length: the list path)."""
    if out is None:
        out = []
    if len(out) >= limit:
        return out
    if type(a) is not type(b):
        out.append(path or "$")
    elif isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            p = "%s.%s" % (path, k) if path else k
            if k not in a or k not in b:
                out.append(p)
            else:
                diff_paths(a[k], b[k], p, out, limit)
    elif isinstance(a, list):
        if len(a) != len(b):
            out.append(path or "$")
        else:
            for i, (x, y) in enumerate(zip(a, b)):
                diff_paths(x, y, "%s[%d]" % (path, i), out, limit)
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


def describe_diff(a, b, paths, max_paths=8):
    """Human-readable lines for one DIFF item."""
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
        else:
            lines.append("  %s:" % p)
            lines.append("    - %s" % _short(va))
            lines.append("    + %s" % _short(vb))
    if len(paths) > max_paths:
        lines.append("  ... %d more differing paths" % (len(paths) - max_paths))
    return lines


# ---------------------------------------------------------------------------------------------
# loading and comparing


def load_surface(label, project, surface):
    path = os.path.join(oc.record_dir(label, project), "items", surface + ".json")
    if not os.path.isfile(path):
        return {"status": "ABSENT", "items": {}}
    return oc.read_json(path)


def compare_surface(project, surface, a, b, allow):
    """Compare two loaded surface files. Returns a result dict with counts and per-item details."""
    res = {"project": project, "surface": surface, "status_a": a.get("status"), "status_b": b.get("status"),
           "counts": {k: 0 for k in RESULT_KINDS}, "total_a": len(a.get("items", {})),
           "total_b": len(b.get("items", {})), "details": [], "allow_used": {}, "allow_violations": []}
    if a.get("status") in ("SKIPPED", "ABSENT") or b.get("status") in ("SKIPPED", "ABSENT"):
        res["status"] = "NOT_COMPARED"
        return res
    if b.get("status") == "FAILED" or a.get("status") == "FAILED":
        res["status"] = "FAILED"
        res["failure"] = {"a": {k: a.get(k) for k in ("error", "stderr_tail") if k in a},
                          "b": {k: b.get(k) for k in ("error", "stderr_tail") if k in b}}
    ia, ib = a.get("items", {}), b.get("items", {})
    per_entry = {}
    for key in sorted(set(ia) | set(ib)):
        if key in ia and key in ib:
            if oc.canonical(ia[key]) == oc.canonical(ib[key]):
                res["counts"]["MATCH"] += 1
                continue
            kind = "DIFF"
            paths = diff_paths(ia[key], ib[key])
        else:
            kind = "MISSING" if key in ia else "EXTRA"
            paths = []
        entry = allow.match(project, surface, key, kind, paths)
        detail = {"key": key, "kind": kind, "paths": paths[:50]}
        if entry is not None:
            per_entry.setdefault(entry["id"], []).append(detail)
            detail["allowed_by"] = entry["id"]
        res["details"].append(detail)
    for eid, details in per_entry.items():
        entry = next(e for e in allow.entries if e["id"] == eid)
        cap = entry.get("max_items")
        if cap is not None and len(details) > cap:
            res["allow_violations"].append("%s excuses %d items > max_items %d" % (eid, len(details), cap))
            for d in details:
                d.pop("allowed_by", None)
        else:
            res["allow_used"][eid] = len(details)
    for d in res["details"]:
        res["counts"]["ALLOWED" if "allowed_by" in d else d["kind"]] += 1
    if "status" not in res:
        bad = res["counts"]["DIFF"] + res["counts"]["MISSING"] + res["counts"]["EXTRA"]
        res["status"] = "MATCH" if bad == 0 else "DIFF"
    res["_a"], res["_b"] = ia, ib
    return res


def compare_labels(label_a, label_b, projects=None, surfaces=None, allow=None):
    allow = allow if allow is not None else AllowList()
    pa, pb = set(oc.projects_in(label_a)), set(oc.projects_in(label_b))
    projects = projects or sorted(pa | pb)
    surfaces = surfaces or oc.SURFACES
    results = []
    for p in projects:
        for s in surfaces:
            results.append(compare_surface(p, s, load_surface(label_a, p, s), load_surface(label_b, p, s), allow))
    return results


def is_clean(r):
    return r["status"] in ("MATCH", "NOT_COMPARED") and not r["allow_violations"]


def report_json(label_a, label_b, results):
    out = {"a": label_a, "b": label_b, "results": []}
    for r in results:
        out["results"].append({
            k: v for k, v in r.items() if not k.startswith("_") and k != "details"
        } | {"non_matching": r["details"]})
    return out
