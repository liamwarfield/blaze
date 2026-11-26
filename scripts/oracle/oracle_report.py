"""What compare and scoreboard print: where both recordings came from, declared exceptions, the
denominators and the verdict; and the score document that is committed under corpus/scores/.

Everything here is derived from an oracle_compare.Comparison. Nothing in a score document depends
on when or where it was rendered, so the same recordings and referee always give the same bytes.
"""

import hashlib
import json
import os

import oracle_common as oc
import oracle_compare as ocmp

SCORE_SCHEMA = 1
# Hex digits of sha256(item key) kept per matching item in a score row (audit R2A-4).
KEY_DIGEST_HEX = 10
COUNT_KEYS = ("total", "compared", "matching", "allowed", "failed", "extra")


def short(sha, n=12):
    return sha[:n] if isinstance(sha, str) and sha else "not recorded"


def home_relative(path):
    """~/... for a path under $HOME (score documents are committed; keep them machine-neutral)."""
    if isinstance(path, str) and (path == oc.HOME or path.startswith(oc.HOME + os.sep)):
        return "~" + path[len(oc.HOME):]
    return path


def tree_text(p):
    """'commit 0123456789ab clean' / '... DIRTY (diff 89abcdef0123, 4 paths)' / 'commit not recorded'."""
    if not p.get("repo_commit"):
        return "repo commit not recorded"
    verified = p.get("commit_verified")
    v = ("binary from that commit verified" if verified else "binary origin NOT VERIFIED" if verified is False
         else "binary origin not checked")
    if p.get("repo_dirty"):
        return "repo commit %s DIRTY (diff %s), %s" % (
            short(p["repo_commit"]), short(p.get("repo_dirty_diff_sha256")), v)
    return "repo commit %s clean, %s" % (short(p["repo_commit"]), v)


def server_text(p):
    """'server mode daemon; cold: mod_graph; warm: build, ...' (states read from stderr are marked)."""
    states = p.get("server_states") or {}
    by = {}
    for cmd, st in states.items():
        name = "unknown" if not st else st[0] + (" (from stderr)" if st[1] == "stderr" else "")
        by.setdefault(name, []).append(cmd)
    parts = ["server mode %s" % (p.get("server_mode") or "unset")]
    parts += ["%s: %s" % (k, ", ".join(v)) for k, v in sorted(by.items())]
    return "; ".join(parts)


def server_state_lines(cmp):
    """Commands that ran on a cold server on one side and a warm one on the other (audit 3, B1;
    decision D12), and a final shutdown that failed or left a server alive."""
    out = []
    for project in cmp.projects:
        a, b = cmp.provenance[project]["a"], cmp.provenance[project]["b"]
        if not (a["recorded"] and b["recorded"]):
            continue
        sa, sb = a.get("server_states") or {}, b.get("server_states") or {}
        differ = [c for c in sa if c in sb and sa[c] and sb[c] and sa[c][0] != sb[c][0]]
        if differ:
            out.append("%s: commands on a %s server in %s and a %s one in %s: %s" % (
                project, sa[differ[0]][0], cmp.a, sb[differ[0]][0], cmp.b, ", ".join(differ))
                if all(sa[c][0] == sa[differ[0]][0] for c in differ) else
                "%s: server states differ between %s and %s: %s" % (
                    project, cmp.a, cmp.b, ", ".join("%s %s/%s" % (c, sa[c][0], sb[c][0]) for c in differ)))
        for side, label in (("a", cmp.a), ("b", cmp.b)):
            sd = cmp.provenance[project][side].get("shutdown") or {}
            if sd.get("shutdown_exit_code") not in (None, 0) or sd.get("alive_after_shutdown"):
                out.append("%s/%s: final shutdown exited %s; servers alive after it: %s" % (
                    label, project, sd.get("shutdown_exit_code"), sd.get("alive_after_shutdown") or "none"))
    return out


def recording_problems(cmp):
    """Why the candidate's recordings cannot back an accepted score (audit R2A-3): a project
    recorded without a commit, from a dirty tree, or with a binary whose origin was not verified.
    `scoreboard --write` refuses a score with any of these unless --allow-dirty "REASON"."""
    out = []
    for project in cmp.projects:
        p = cmp.provenance[project]["b"]
        if not p["recorded"]:
            continue
        where = "%s/%s" % (cmp.b, project)
        if not p.get("repo_commit"):
            out.append("%s: no recorded commit" % where)
            continue
        if p.get("repo_dirty"):
            out.append("%s: recorded from a dirty tree (diff %s)" % (where, short(p.get("repo_dirty_diff_sha256"))))
        if p.get("commit_verified") is None:
            out.append("%s: binary origin not checked (recorded before binary_origin existed)" % where)
        elif not p.get("commit_verified"):
            out.append("%s: binary origin not verified: %s" % (
                where, "; ".join(p.get("commit_unverified_reasons") or ["no reason recorded"])))
    return out


def provenance_lines(cmp):
    """Per side and project: binary, date, repository commit and whether the tree was dirty."""
    lines = []
    for side, label, role in (("a", cmp.a, "A, reference"), ("b", cmp.b, "B, candidate")):
        lines.append("%s (%s)" % (label, role))
        for project in cmp.projects:
            p = cmp.provenance[project][side]
            if not p["recorded"]:
                lines.append("  %-8s NOT RECORDED under this label" % project)
                continue
            lines.append("  %-8s binary sha256 %s %s; recorded %s; %s" % (
                project, short(p.get("binary_sha256")), home_relative(p.get("binary")), p.get("created"),
                tree_text(p)))
    return lines


def server_lines(cmp):
    """Per side and project: the server mode and which commands ran on a cold or warm server."""
    lines = []
    for side, label in (("a", cmp.a), ("b", cmp.b)):
        for project in cmp.projects:
            p = cmp.provenance[project][side]
            if p["recorded"]:
                lines.append("server %s/%s: %s" % (label, project, server_text(p)))
    return lines


def warning_lines(cmp):
    """Things that weaken a comparison without changing an item: mixed binaries, stale items."""
    out = []
    if oc.STAGE:
        out.append("staged: recordings read from %s, the reference checked against %s "
                   "(BLAZE_ORACLE_STAGE), not the shared recordings and the committed summaries" % (
                       home_relative(oc.RECORDS), home_relative(oc.CORPUS_SUMMARIES)))
    out.extend(server_state_lines(cmp))
    for side, label in (("a", cmp.a), ("b", cmp.b)):
        recorded = [cmp.provenance[p][side] for p in cmp.projects if cmp.provenance[p][side]["recorded"]]
        for field, what in (("binary_sha256", "binaries"), ("repo_commit", "repository commits"),
                            ("repo_dirty_diff_sha256", "uncommitted states")):
            values = sorted({str(p.get(field)) for p in recorded})
            if len(values) > 1:
                out.append("label %s mixes %s across projects: %s" % (
                    label, what, ", ".join(short(v) for v in values)))
        for project in cmp.projects:
            p = cmp.provenance[project][side]
            if not p["recorded"]:
                continue
            if p.get("replaced"):
                r = p["replaced"]
                out.append("%s/%s was recorded with --force over an earlier recording (created %s, binary %s)" % (
                    label, project, r.get("created"), short(r.get("binary_sha256"))))
            for f in p.get("forced") or []:
                out.append("%s/%s was recorded with --force despite: %s" % (label, project, f))
            if side == "b" and p.get("repo_commit") and p.get("commit_verified") is False:
                out.append("%s/%s: the recorded commit is not verified as the binary's origin: %s" % (
                    label, project, "; ".join(p.get("commit_unverified_reasons") or [])))
        if side == "b":
            if any(p.get("repo_commit") is None for p in recorded):
                out.append("label %s does not say which commit it was recorded from" % label)
            elif any(p.get("repo_dirty") for p in recorded):
                out.append("label %s was recorded from a dirty tree: the commit alone does not reproduce it" % label)
    out.extend(cmp.notes)
    current = oc.extractor_sha256()
    for project in cmp.projects:
        for side, label in (("a", cmp.a), ("b", cmp.b)):
            p = cmp.provenance[project][side]
            if p["recorded"] and p.get("extractor_sha256") != current:
                out.append("%s/%s items were extracted by another version of surfaces.py/normalise.py "
                           "(%s, now %s): run `record --renormalise --label %s --project %s`" % (
                               label, project, short(p.get("extractor_sha256")), short(current), label, project))
    return out


def exception_lines(cmp):
    out = []
    for side, label, project, e in cmp.exceptions():
        out.append("%s/%s exception %s (kind %s) affects surfaces %s: %s" % (
            label, project, e["id"], e.get("kind", "unspecified"), ", ".join(e["surfaces"]),
            e.get("explanation", "no explanation given")))
    return out


def excluded(cmp):
    """[{project, surface, reason, reference_items}]: what the comparison did not judge, and why."""
    out = []
    for r in cmp.results:
        if r["status"] == "NOT_RECORDED":
            out.append({"project": r["project"], "surface": r["surface"], "reference_items": 0,
                        "reason": "recorded by neither side"})
    for o in cmp.out_of_scope:
        out.append({"project": o["project"], "surface": o["surface"], "reference_items": o["reference_items"],
                    "reason": "outside the requested restriction (%s)" % cmp.restriction_text()})
    return out


def _excluded_groups(excl):
    """Excluded surfaces grouped by (reason, project): [(reason, project, [surfaces], items)]."""
    groups = {}
    for e in excl:
        g = groups.setdefault((e["reason"], e["project"]), [[], 0])
        g[0].append(e["surface"])
        g[1] += e["reference_items"]
    return [(reason, project, s, n) for (reason, project), (s, n) in sorted(groups.items())]


def denominator_lines(cmp):
    t = cmp.totals()
    s = t["surfaces"]
    judged = t["total"] - t["allowed"]
    lines = [
        "reference items: total %d = matching %d + allowed %d + failed/missing %d; compared %d; extra %d" % (
            t["total"], t["matching"], t["allowed"], t["failed"], t["compared"], t["extra"]),
        "matching %s%% of the %d reference items that are not allow-listed" % (ocmp.percent(t), judged),
        "surfaces in scope: %d = MATCH %d + DIFF %d + FAILED %d + MISSING %d + NO_REFERENCE %d + NOT_RECORDED %d" % (
            len(cmp.results), s["MATCH"], s["DIFF"], s["FAILED"], s["MISSING"], s["NO_REFERENCE"],
            s["NOT_RECORDED"]),
    ]
    groups = _excluded_groups(excluded(cmp))
    if groups:
        lines.append("excluded from the comparison:")
        for reason, project, surfaces, n in groups:
            lines.append("  %s: %d surfaces (%d reference items), %s: %s" % (
                project, len(surfaces), n, reason, ", ".join(surfaces)))
    else:
        lines.append("excluded from the comparison: nothing")
    return lines


def allow_usage(cmp, allow):
    """{entry id: {"items": n, "reason": ...}} for the allow-list entries that excused something."""
    used = {}
    for r in cmp.results:
        for eid, n in r["allow_used"].items():
            used[eid] = used.get(eid, 0) + n
    by_id = {e["id"]: e for e in allow.entries}
    return {eid: {"items": n, "reason": by_id.get(eid, {}).get("reason", "")} for eid, n in sorted(used.items())}


def status_text(row):
    """The row's status with its marks: '*' under a declared exception, '(empty)' for 0 vs 0."""
    return row["status"] + ("*" if row["exceptions"] else "") + (" (empty)" if row["empty"] else "")


# ---------------------------------------------------------------------------------------------
# the score document (corpus/scores/<label>.json and .md)


def key_digests(keys):
    """The compact form of a set of item keys stored in a score row: the first KEY_DIGEST_HEX hex
    digits of sha256 of each key (UTF-8), sorted, space-separated. compare-scores ratchets these."""
    return " ".join(sorted(hashlib.sha256(k.encode("utf-8")).hexdigest()[:KEY_DIGEST_HEX] for k in keys))


def score_document(cmp, allow, allow_dirty=None):
    """The score document. `allow_dirty`: the reason given to `scoreboard --allow-dirty`."""
    rows = cmp.rows()
    for row, r in zip(rows, cmp.results):
        # Names, not only counts (audit R2A-4): which reference items match.
        row["matching_keys"] = key_digests(r.get("_matching", []) if row["status"] in ("MATCH", "DIFF") else [])
    prov = {}
    for project in cmp.projects:
        prov[project] = {}
        for side, role in (("a", "reference"), ("b", "candidate")):
            p = dict(cmp.provenance[project][side])
            if "binary" in p:
                p["binary"] = home_relative(p["binary"])
            p.pop("steps", None)
            p.pop("exceptions", None)
            prov[project][role] = p
    return {
        "schema": SCORE_SCHEMA,
        "label": cmp.b,
        "reference": cmp.a,
        "verdict": cmp.verdict(),
        "clean": cmp.clean(),
        "failures": cmp.failures(),
        "restriction": cmp.restriction,
        "referee_sha256": oc.referee_sha256(),
        "provenance": prov,
        "warnings": warning_lines(cmp),
        "exceptions": [{"label": label, "project": project, "id": e["id"], "kind": e.get("kind"),
                        "surfaces": e["surfaces"], "explanation": e.get("explanation")}
                       for _, label, project, e in cmp.exceptions()],
        "rows": rows,
        "project_totals": {p: ocmp.totals([r for r in rows if r["project"] == p])
                           for p in sorted({r["project"] for r in rows})},
        "totals": cmp.totals(rows),
        "allowed_by": allow_usage(cmp, allow),
        "excluded": excluded(cmp),
        "recording_problems": recording_problems(cmp),
        "allow_dirty": allow_dirty,
        "matching_keys_format": "sha256(key)[:%d], sorted, space-separated" % KEY_DIGEST_HEX,
    }


def score_json(doc):
    return json.dumps(doc, indent=1, sort_keys=True, ensure_ascii=False) + "\n"


def _prov_text(p):
    if not p["recorded"]:
        return "NOT RECORDED"
    return "binary sha256 `%s` `%s`; recorded %s; %s; %s" % (
        short(p.get("binary_sha256")), p.get("binary"), p.get("created"), tree_text(p), server_text(p))


def score_text(doc):
    out = ["scoreboard: %s vs reference %s" % (doc["label"], doc["reference"]), ""]
    for role in ("reference", "candidate"):
        out.append("%s %s" % (role, doc[role] if role == "reference" else doc["label"]))
        for project, sides in doc["provenance"].items():
            out.append("  %-8s %s" % (project, _prov_text(sides[role]).replace("`", "")))
    out.append("")
    for w in doc["warnings"]:
        out.append("WARNING: " + w)
    for e in doc["exceptions"]:
        out.append("EXCEPTION: %s/%s %s (kind %s) affects surfaces %s: %s" % (
            e["label"], e["project"], e["id"], e["kind"], ", ".join(e["surfaces"]), e["explanation"]))
    if doc["warnings"] or doc["exceptions"]:
        out.append("")
    hdr = "  %-18s %-16s %7s %8s %8s %7s %7s %6s %7s" % (
        "surface", "status", "total", "compared", "matching", "allowed", "failed", "extra", "match%")
    for project, t in doc["project_totals"].items():
        out += ["project %s" % project, hdr, "  " + "-" * (len(hdr) - 2)]
        for row in doc["rows"]:
            if row["project"] != project:
                continue
            out.append("  %-18s %-16s %7d %8d %8d %7d %7d %6d %7s" % (
                row["surface"], status_text(row), row["total"], row["compared"], row["matching"],
                row["allowed"], row["failed"], row["extra"], ocmp.percent(row)))
        out.append("  %-18s %-16s %7d %8d %8d %7d %7d %6d %7s" % (
            "TOTAL", "", t["total"], t["compared"], t["matching"], t["allowed"], t["failed"], t["extra"],
            ocmp.percent(t)))
        out.append("")
    t = doc["totals"]
    s = t["surfaces"]
    out.append("all projects: total %d = matching %d + allowed %d + failed/missing %d; compared %d; extra %d; "
               "matching %s%% of the %d items that are not allow-listed" % (
                   t["total"], t["matching"], t["allowed"], t["failed"], t["compared"], t["extra"],
                   ocmp.percent(t), t["total"] - t["allowed"]))
    out.append("surfaces in scope: %d = MATCH %d + DIFF %d + FAILED %d + MISSING %d + NO_REFERENCE %d + "
               "NOT_RECORDED %d" % (len(doc["rows"]), s["MATCH"], s["DIFF"], s["FAILED"], s["MISSING"],
                                    s["NO_REFERENCE"], s["NOT_RECORDED"]))
    if doc["allowed_by"]:
        out.append("allowed (scripts/oracle/allow.json):")
        for eid, u in doc["allowed_by"].items():
            out.append("  %-32s %5d  %s" % (eid, u["items"], u["reason"]))
    groups = _excluded_groups(doc["excluded"])
    if groups:
        out.append("excluded from the comparison:")
        for reason, project, surfaces, n in groups:
            out.append("  %s: %d surfaces (%d reference items), %s: %s" % (
                project, len(surfaces), n, reason, ", ".join(surfaces)))
    else:
        out.append("excluded from the comparison: nothing")
    for prob in doc["recording_problems"]:
        out.append("NOT ACCEPTABLE AS IS: %s" % prob)
    if doc.get("allow_dirty"):
        out.append("accepted with --allow-dirty: %s" % doc["allow_dirty"])
    out.append("referee sha256 %s" % short(doc["referee_sha256"]))
    out.append("verdict: %s" % doc["verdict"])
    return "\n".join(out) + "\n"


def score_markdown(doc):
    out = ["# Score: %s against %s" % (doc["label"], doc["reference"]), "",
           "Verdict: **%s**" % doc["verdict"], "",
           "Written by `scripts/oracle/scoreboard %s --write`; referee sha256 `%s`. Counts are reference "
           "items: total = matching + allowed + failed; match%% is matching over total minus allowed." % (
               doc["label"], short(doc["referee_sha256"])), "",
           "## Recordings", "", "| side | project | recording |", "|---|---|---|"]
    for role in ("reference", "candidate"):
        for project, sides in doc["provenance"].items():
            out.append("| %s `%s` | %s | %s |" % (
                role, doc["reference"] if role == "reference" else doc["label"], project, _prov_text(sides[role])))
    out.append("")
    if doc["warnings"]:
        out += ["## Warnings", ""] + ["- %s" % w for w in doc["warnings"]] + [""]
    if doc["recording_problems"]:
        out += ["## Not acceptable without --allow-dirty", ""] + ["- %s" % w for w in doc["recording_problems"]]
        if doc.get("allow_dirty"):
            out.append("- accepted with `--allow-dirty`: %s" % doc["allow_dirty"])
        out.append("")
    if doc["exceptions"]:
        out += ["## Declared recording exceptions", ""]
        for e in doc["exceptions"]:
            out.append("- `%s/%s` **%s** (kind %s), surfaces %s: %s" % (
                e["label"], e["project"], e["id"], e["kind"], ", ".join(e["surfaces"]), e["explanation"]))
        out.append("")
    out += ["## Surfaces", "",
            "| project | surface | status | total | compared | matching | allowed | failed | extra | match% |",
            "|---|---|---|---:|---:|---:|---:|---:|---:|---:|"]
    for project, t in doc["project_totals"].items():
        for row in doc["rows"]:
            if row["project"] != project:
                continue
            out.append("| %s | %s | %s | %d | %d | %d | %d | %d | %d | %s |" % (
                project, row["surface"], status_text(row), row["total"], row["compared"], row["matching"],
                row["allowed"], row["failed"], row["extra"], ocmp.percent(row)))
        out.append("| %s | **total** | | %d | %d | %d | %d | %d | %d | %s |" % (
            project, t["total"], t["compared"], t["matching"], t["allowed"], t["failed"], t["extra"],
            ocmp.percent(t)))
    t = doc["totals"]
    s = t["surfaces"]
    out.append("| all | **total** | | %d | %d | %d | %d | %d | %d | %s |" % (
        t["total"], t["compared"], t["matching"], t["allowed"], t["failed"], t["extra"], ocmp.percent(t)))
    out += ["",
            "Surfaces in scope: %d = MATCH %d + DIFF %d + FAILED %d + MISSING %d + NO_REFERENCE %d + "
            "NOT_RECORDED %d." % (len(doc["rows"]), s["MATCH"], s["DIFF"], s["FAILED"], s["MISSING"],
                                  s["NO_REFERENCE"], s["NOT_RECORDED"]), ""]
    out += ["## Allowed and excluded", ""]
    if doc["allowed_by"]:
        for eid, u in doc["allowed_by"].items():
            out.append("- allowed by `%s`: %d items. %s" % (eid, u["items"], u["reason"]))
    else:
        out.append("- allowed: nothing")
    groups = _excluded_groups(doc["excluded"])
    for reason, project, surfaces, n in groups:
        out.append("- excluded, %s, %s: %d surfaces (%d reference items): %s" % (
            project, reason, len(surfaces), n, ", ".join(surfaces)))
    if not groups:
        out.append("- excluded: nothing")
    return "\n".join(out) + "\n"
