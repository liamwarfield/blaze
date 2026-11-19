#!/usr/bin/env python3
"""The verdict of one scripts/remote/matrix run. See `matrix_verdict.py help`.

Standard library only. Called by scripts/remote/matrix at the end of a run, and on its own to
judge an existing matrix output again.
"""

import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ALLOW_FILE = os.path.join(HERE, "matrix-allow.json")
WORK = os.path.join(os.path.expanduser("~"), ".cache", "blaze", "work", "remote")

USAGE = """\
usage: scripts/remote/matrix_verdict.py <project> <tag> [--allow FILE]
       scripts/remote/matrix_verdict.py help

Reads ~/.cache/blaze/work/remote/matrix-<tag>.txt (written by scripts/remote/matrix) and prints
one line per failing step, then `verdict: PASS` or `verdict: FAIL (<reasons>)`.

A run passes only when:
  * every build step (D1..D4, C1..C5, E1, E2) has its `N processes` line and exit code 0;
  * every gRPC log comparison (`C3 vs C2`, `C4 vs C1`, `E2 vs E1`) is RESULT: MATCH, or is
    DIFFERENT and an entry of the allow file (default scripts/remote/matrix-allow.json) for this
    project and step holds. The allowance is checked on the full list of unmatched RPCs:
    compare-grpc-log is run again with --verbose (output kept in
    matrix-<tag>-<C4-vs-C1>.unmatched.txt) and every unmatched RPC on either side must belong to an
    action whose mnemonic is in `only_mnemonics`, at most `max_actions` distinct actions may be
    involved, and (with `same_count_per_method`) both sides must have the same number of RPCs per
    method;
  * the run finished (`matrix <tag> done`) and has every step.
Exit status: 0 PASS, 1 FAIL, 2 usage error or unreadable input.
"""

BUILD_STEPS = ["D1", "D2", "D3", "D4", "C1", "C2", "C3", "C4", "C5", "E1", "E2"]
# Comparison step -> (reference run, candidate run), as scripts/remote/matrix lays them out.
COMPARISONS = {
    "C3 vs C2 gRPC log": ("c1/run2", "c3/run1"),
    "C4 vs C1 gRPC log": ("c1/run1", "c4/run1"),
    "E2 vs E1 gRPC log": ("e1/run1", "e2/run1"),
}
UNMATCHED = re.compile(r"^\s*\[only in (reference|candidate) x(\d+)\] (.*)$")
METHOD_ROW = re.compile(r"^(\S+/\S+)\s+(\d+)\s+(\d+)\s+(\d+)$")


def load_allow(path):
    with open(path) as f:
        entries = json.load(f).get("entries", [])
    for e in entries:
        for k in ("id", "project", "step", "only_mnemonics", "max_actions", "condition", "reason", "source"):
            if k not in e:
                raise ValueError("matrix allow entry %r lacks %r" % (e.get("id"), k))
        if e["step"] not in COMPARISONS:
            raise ValueError("matrix allow entry %r: unknown step %r" % (e["id"], e["step"]))
    return entries


def parse_matrix(text):
    """{step: line text} for the build and comparison steps, and whether the run finished."""
    steps, done = {}, False
    for line in text.splitlines():
        if re.match(r"^matrix \S+ done$", line.strip()):
            done = True
            continue
        name, sep, rest = line.partition(": ")
        if sep and (name in BUILD_STEPS or name in COMPARISONS):
            steps[name] = rest
    return steps, done


def build_step_problem(rest):
    if "NO PROCESSES LINE" in rest:
        return "no `N processes` line"
    m = re.search(r"\[exit (?:exit code: )?(\d*)\]", rest)
    if not m or m.group(1) != "0":
        return "exit code %s" % (m.group(1) if m and m.group(1) else "unknown")
    return None


def check_allowance(entry, unmatched_text):
    """Why the allowance does not hold for this compare-grpc-log --verbose output; [] if it holds."""
    why, actions, rows = [], set(), {}
    for line in unmatched_text.splitlines():
        r = METHOD_ROW.match(line.strip())
        if r:
            rows[r.group(1)] = (int(r.group(2)), int(r.group(3)), int(r.group(4)))
            continue
        m = UNMATCHED.match(line)
        if not m:
            continue
        try:
            rpc = json.loads(m.group(3))
        except ValueError:
            why.append("an unmatched RPC could not be parsed")
            continue
        md = rpc.get("metadata") or {}
        mnemonic = md.get("actionMnemonic")
        if mnemonic not in entry["only_mnemonics"]:
            why.append("unmatched %s RPC of a %s action (allowed: %s)" % (
                (rpc.get("methodName") or "?").split(".")[-1], mnemonic, ", ".join(entry["only_mnemonics"])))
        actions.add(md.get("actionId"))
    if not actions and not why:
        why.append("no unmatched RPC found in the --verbose output")
    if len(actions) > entry["max_actions"]:
        why.append("%d actions involved > max_actions %d" % (len(actions), entry["max_actions"]))
    if entry.get("same_count_per_method"):
        for method, (ref, cand, _) in sorted(rows.items()):
            if ref != cand:
                why.append("%s: %d RPCs in the reference, %d in the candidate" % (method, ref, cand))
    # Report each distinct reason once.
    return sorted(set(why))


def unmatched_output(tag, step, work=WORK):
    """compare-grpc-log --verbose for one comparison step, kept next to the matrix output."""
    ref, cand = COMPARISONS[step]
    t = "mx-" + tag
    out = os.path.join(work, "matrix-%s-%s.unmatched.txt" % (tag, step.replace(" gRPC log", "").replace(" ", "-")))
    p = subprocess.run([os.path.join(HERE, "compare-grpc-log"),
                        os.path.join(work, "%s-%s" % (t, ref.split("/")[0]), ref.split("/")[1], "grpc.log"),
                        os.path.join(work, "%s-%s" % (t, cand.split("/")[0]), cand.split("/")[1], "grpc.log"),
                        "--verbose", "--max", "1000000000"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    text = p.stdout.decode("utf-8", "replace")
    with open(out, "w") as f:
        f.write(text)
    return text


def verdict(project, matrix_text, allow, unmatched=None):
    """[failure reasons] for one matrix run; [] is a pass. `unmatched(step)` returns the
    compare-grpc-log --verbose output of a DIFFERENT comparison step."""
    steps, done = parse_matrix(matrix_text)
    why = []
    for s in BUILD_STEPS:
        if s not in steps:
            why.append("%s: step missing" % s)
            continue
        p = build_step_problem(steps[s])
        if p:
            why.append("%s: %s" % (s, p))
    for s in COMPARISONS:
        if s not in steps:
            why.append("%s: step missing" % s)
            continue
        rest = steps[s]
        if rest.rstrip().endswith("RESULT: MATCH"):
            continue
        if not rest.rstrip().endswith("RESULT: DIFFERENT"):
            why.append("%s: no RESULT line" % s)
            continue
        entries = [e for e in allow if e["project"] == project and e["step"] == s]
        if not entries:
            why.append("%s: DIFFERENT (no allowance for %s)" % (s, project))
            continue
        text = unmatched(s) if unmatched else ""
        for e in entries:
            fail = check_allowance(e, text)
            if not fail:
                print("ALLOWED %s: DIFFERENT under %s: %s" % (s, e["id"], e["condition"]))
                break
            why.append("%s: DIFFERENT and allowance %s does not hold: %s" % (s, e["id"], "; ".join(fail[:5])))
    if not done:
        why.append("the run did not finish (no `matrix <tag> done` line)")
    return why


def main(argv):
    if not argv or argv[0] in ("help", "-h", "--help"):
        print(USAGE, end="")
        return 0 if argv else 2
    allow_path = ALLOW_FILE
    if "--allow" in argv:
        i = argv.index("--allow")
        allow_path = argv[i + 1]
        argv = argv[:i] + argv[i + 2:]
    if len(argv) != 2:
        print(USAGE, end="", file=sys.stderr)
        return 2
    project, tag = argv
    path = os.path.join(WORK, "matrix-%s.txt" % tag)
    try:
        allow = load_allow(allow_path)
        with open(path) as f:
            text = f.read()
    except (OSError, ValueError) as e:
        print("error: %s" % e, file=sys.stderr)
        return 2
    why = verdict(project, text, allow, lambda step: unmatched_output(tag, step))
    for w in why:
        print("FAIL %s" % w)
    print("verdict: %s" % ("PASS" if not why else "FAIL (%d step%s: %s)" % (
        len(why), "" if len(why) == 1 else "s", ", ".join(w.split(":")[0] for w in why))))
    return 1 if why else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
