"""Parse the output of `cargo test --message-format=json-render-diagnostics` into per-name results.

Usage: python3 -I scripts/gate/parse_test_log.py <log> <out.json> <checkout root> [cargo exit status]
       python3 -I scripts/gate/parse_test_log.py help

Called by scripts/gate/run-tests. The log holds, mixed: Cargo's JSON messages (one per line; the
`compiler-artifact` messages say which package and target each test executable belongs to),
Cargo's `Running ... (target/debug/deps/<name>-<hash>)` and `Doc-tests <crate>` lines, and
libtest's `test <name> ... ok|FAILED|ignored` lines.

Result names are `<package>::<target>::<test>`, where <target> is `lib`, `bin/<name>`,
`test/<name>` (a file under tests/), `example/<name>`, `bench/<name>` or `doc`. The package comes
from the artifact message, never from the executable's file name: two crates may both have a
tests/label_test.rs.

Output: {"commit", "tree_dirty", "dirty_diff_sha256", "cargo_exit", "counts": {pass, fail,
ignored}, "binaries": {"<package>::<target>": {pass, fail, ignored, "finished": bool}},
"problems": [...], "results": {name: "pass"|"fail"|"ignored"}}.

`problems` lists what makes the result unfit to be a baseline: a test executable Cargo did not
announce (the tree changed during the run), a name reported twice, a test executable that printed
no `test result:` line (it crashed or was killed; its remaining tests are absent from `results`),
and a test executable whose parsed names do not add up to its own `test result:` counts.

The last check exists because the log mixes streams: a test that writes to its real stderr can
put text between libtest's `test <name> ... `, its `ok` and the newline (libtest flushes after
each piece). The parser accepts text glued after the status and a status that arrives on a later
line; whatever it still misses shows up as a count that does not add up, never as a silently
shorter result.
"""

import hashlib
import json
import os
import re
import subprocess
import sys

RUN_RE = re.compile(r"^\s*Running (?:unittests )?(\S+) \((\S+)\)$")
DOC_RE = re.compile(r"^\s*Doc-tests (\S+)$")
# `test <name> ... <status>`; the status may be followed by its reason (`ignored, why`) or by
# foreign text glued on before the newline, or may be missing (it then starts a later line).
TEST_RE = re.compile(r"^test (.+?) \.\.\. (?:(ok|FAILED|ignored).*)?")
LATE_STATUS_RE = re.compile(r"^(ok|FAILED|ignored)")
RESULT_RE = re.compile(r"^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; ")
STATUS = {"ok": "pass", "FAILED": "fail", "ignored": "ignored"}


def package_name(package_id):
    """The package name in a Cargo package id (`path+file:///x/crates/blaze_util#0.1.0`,
    `path+file:///x/dir#name@0.1.0`, `registry+https://...#serde@1.0.0`, or the old
    `name 0.1.0 (path+file:///...)`)."""
    if "#" not in package_id:
        return package_id.split(" ", 1)[0]
    head, frag = package_id.rsplit("#", 1)
    if "@" in frag:
        return frag.split("@", 1)[0]
    return head.rstrip("/").rsplit("/", 1)[-1]


def target_label(kind, name):
    if "lib" in kind or "rlib" in kind or "proc-macro" in kind or "cdylib" in kind or "dylib" in kind:
        return "lib"
    for k in ("bin", "test", "example", "bench"):
        if k in kind:
            return "%s/%s" % (k, name)
    return "%s/%s" % ("+".join(kind) or "target", name)


def parse(lines):
    """Returns (results, binaries, problems) for the lines of a test log."""
    executables = {}   # basename of the executable -> "<package>::<target>"
    doc_packages = {}  # lib target name -> package
    results, binaries, problems = {}, {}, []
    current = None
    pending = None        # a test name whose status has not been seen yet
    in_failures = False   # inside libtest's `failures:` section (captured output, not results)

    def new_binary(name):
        nonlocal current, pending, in_failures
        finish_current()
        current, pending, in_failures = name, None, False
        if current in binaries:
            problems.append("%s ran more than once" % current)
        binaries[current] = {"pass": 0, "fail": 0, "ignored": 0, "finished": False}

    def finish_current():
        if current is None:
            return
        if pending is not None:
            problems.append("%s::%s has no status in the log" % (current, pending))
        if not binaries[current]["finished"]:
            problems.append("%s printed no `test result:` line (crashed or killed); its unrun tests are absent"
                            % current)

    def record(test, status):
        name = "%s::%s" % (current, test)
        if name in results:
            problems.append("%s was reported twice" % name)
        results[name] = STATUS[status]
        binaries[current][STATUS[status]] += 1

    for line in lines:
        line = line.rstrip("\n")
        if line.startswith('{"reason":'):
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            if msg.get("reason") != "compiler-artifact":
                continue
            target = msg.get("target", {})
            package = package_name(msg.get("package_id", ""))
            kind = target.get("kind", [])
            if "lib" in kind or "rlib" in kind or "proc-macro" in kind:
                doc_packages[target.get("name")] = package
            if msg.get("executable") and msg.get("profile", {}).get("test"):
                executables[os.path.basename(msg["executable"])] = "%s::%s" % (
                    package, target_label(kind, target.get("name")))
            continue
        m = RUN_RE.match(line)
        if m:
            exe = os.path.basename(m.group(2))
            name = executables.get(exe)
            if name is None:
                name = "UNKNOWN::%s" % exe
                problems.append("test executable %s (%s) was not announced by a compiler-artifact message"
                                % (m.group(2), m.group(1)))
            new_binary(name)
            continue
        m = DOC_RE.match(line)
        if m:
            new_binary("%s::doc" % doc_packages.get(m.group(1), m.group(1)))
            continue
        if current is None:
            continue
        m = RESULT_RE.match(line)
        if m:
            b = binaries[current]
            b["finished"] = True
            said = tuple(int(x) for x in m.groups())
            if said != (b["pass"], b["fail"], b["ignored"]):
                problems.append("%s: parsed %d passed, %d failed, %d ignored, but its `test result:` line says "
                                "%d, %d, %d (interleaved output?)" % ((current, b["pass"], b["fail"], b["ignored"]) + said))
            in_failures = False
            continue
        if line == "failures:":
            in_failures = True
            continue
        if in_failures:
            continue
        m = TEST_RE.match(line)
        if m:
            if pending is not None:
                problems.append("%s::%s has no status in the log" % (current, pending))
                pending = None
            if m.group(2):
                record(m.group(1), m.group(2))
            else:
                pending = m.group(1)
            continue
        if pending is not None:
            m = LATE_STATUS_RE.match(line)
            if m:
                record(pending, m.group(1))
                pending = None
    finish_current()
    return results, binaries, problems


def git(root, *args):
    return subprocess.run(["git", "--no-optional-locks", "-C", root, *args], stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL).stdout


def tree_state(root):
    """(commit, dirty, sha256 of the uncommitted state or None). Same definition as
    scripts/oracle/oracle_common.repo_state: `git diff HEAD --binary` plus every untracked file."""
    commit = git(root, "rev-parse", "HEAD").decode().strip() or None
    status = git(root, "status", "--porcelain=v1", "-z", "--untracked-files=all")
    if not status:
        return commit, False, None
    h = hashlib.sha256()
    h.update(git(root, "diff", "HEAD", "--binary"))
    for path in sorted(p for p in git(root, "ls-files", "--others", "--exclude-standard", "-z").split(b"\0") if p):
        full = os.path.join(root.encode(), path)
        digest = "-"
        if os.path.isfile(full) and not os.path.islink(full):
            with open(full, "rb") as f:
                digest = hashlib.sha256(f.read()).hexdigest()
        h.update(path + b"\0" + digest.encode() + b"\n")
    return commit, True, h.hexdigest()


def document(lines, root, cargo_exit=None):
    results, binaries, problems = parse(lines)
    commit, dirty, diff = tree_state(root)
    return {
        "commit": commit, "tree_dirty": dirty, "dirty_diff_sha256": diff, "cargo_exit": cargo_exit,
        "counts": {k: sum(1 for v in results.values() if v == k) for k in ("pass", "fail", "ignored")},
        "binaries": dict(sorted(binaries.items())),
        "problems": problems,
        "results": dict(sorted(results.items())),
    }


def main(argv):
    if len(argv) < 3 or argv[0] in ("help", "-h", "--help"):
        print(__doc__)
        return 0 if argv[:1] and argv[0] in ("help", "-h", "--help") else 2
    log, out, root = argv[:3]
    cargo_exit = int(argv[3]) if len(argv) > 3 else None
    with open(log, errors="replace") as f:
        doc = document(f, root, cargo_exit)
    tmp = out + ".tmp"
    with open(tmp, "w") as f:
        json.dump(doc, f, indent=1, sort_keys=True)
        f.write("\n")
    os.replace(tmp, out)
    print(json.dumps({"commit": doc["commit"], "tree_dirty": doc["tree_dirty"], **doc["counts"],
                      "test_executables": len(doc["binaries"]), "problems": len(doc["problems"])}))
    for p in doc["problems"]:
        print("PROBLEM: " + p)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
