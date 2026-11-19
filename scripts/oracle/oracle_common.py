"""Shared paths and helpers for the oracle recorder, comparer and scoreboard.

Standard library only. Imported by scripts/oracle/{record,compare,scoreboard}.
"""

import hashlib
import json
import os
import subprocess
import sys

HOME = os.path.expanduser("~")
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CACHE = os.path.join(HOME, ".cache", "blaze")
RECORDS = os.path.join(CACHE, "oracle-records")
WORK = os.path.join(CACHE, "work")
OUTPUT_ROOTS = os.path.join(CACHE, "oracle-out")
REPO_CACHE = os.path.join(CACHE, "repo-cache")
CORPUS_MANIFEST = os.path.join(REPO_ROOT, "corpus", "projects.json")
CORPUS_SUMMARIES = os.path.join(REPO_ROOT, "corpus", "oracle")
CORPUS_SCORES = os.path.join(REPO_ROOT, "corpus", "scores")
ALLOW_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "allow.json")
REFERENCE_LABEL = "oracle-9.3.2"

# Comparison surfaces, in report order. Each maps to items/<surface>.json in a record.
SURFACES = [
    "exit_codes",
    "mod_graph",
    "lockfile",
    "query_label_kind",
    "query_jsonproto",
    "cquery_label_kind",
    "cquery_jsonproto",
    "aquery_actions",
    "build_stderr",
    "execlog",
    "bep",
    "outputs",
    "tests",
]

# Recording steps (what --surfaces selects in `record`) and the comparison surfaces each feeds.
STEPS = {
    "mod": ["mod_graph", "lockfile"],
    "build": ["build_stderr", "execlog", "bep", "outputs", "tests"],
    "query": ["query_label_kind", "query_jsonproto"],
    "cquery": ["cquery_label_kind", "cquery_jsonproto"],
    "aquery": ["aquery_actions"],
}
STEP_ORDER = ["mod", "build", "query", "cquery", "aquery"]


def load_projects():
    with open(CORPUS_MANIFEST) as f:
        return json.load(f)["projects"]


def expand(path):
    return os.path.expanduser(path)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def write_json(path, obj, indent=1):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(obj, f, indent=indent, sort_keys=True, ensure_ascii=False)
        f.write("\n")
    os.replace(tmp, path)


def read_json(path):
    with open(path) as f:
        return json.load(f)


def record_dir(label, project):
    return os.path.join(RECORDS, label, project)


def labels_available():
    if not os.path.isdir(RECORDS):
        return []
    return sorted(os.listdir(RECORDS))


def projects_in(label):
    d = os.path.join(RECORDS, label)
    if not os.path.isdir(d):
        return []
    return sorted(p for p in os.listdir(d) if os.path.isfile(os.path.join(d, p, "manifest.json")))


def die(msg, code=2):
    print("error: " + msg, file=sys.stderr)
    sys.exit(code)


def canonical(obj):
    """Stable string form of a JSON value, used for keys and equality."""
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def load_manifest(label, project):
    """manifest.json of one recording, or None when the project was not recorded under label."""
    path = os.path.join(record_dir(label, project), "manifest.json")
    if not os.path.isfile(path):
        return None
    return read_json(path)


def _git(root, *args):
    p = subprocess.run(["git", "--no-optional-locks", "-C", root] + list(args),
                       stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    return p.stdout if p.returncode == 0 else None


def repo_state(root=None):
    """The commit and cleanliness of the Blaze repository at `root` (default: this checkout).

    {"commit": <git rev-parse HEAD>, "dirty": bool, "dirty_paths": N, "dirty_diff_sha256": hex}.
    The tree is dirty when `git status --porcelain` lists anything (modified, staged or untracked,
    ignored files excepted). dirty_diff_sha256 identifies the uncommitted state: sha256 over
    `git diff HEAD --binary` followed by "<path>\0<sha256 of content>\n" for every untracked file
    in path order; null for a clean tree. Every value is null outside a git checkout.
    """
    root = root or REPO_ROOT
    head = _git(root, "rev-parse", "HEAD")
    if head is None:
        return {"commit": None, "dirty": None, "dirty_paths": None, "dirty_diff_sha256": None}
    status = _git(root, "status", "--porcelain=v1", "-z", "--untracked-files=all") or b""
    entries = [e for e in status.split(b"\0") if e]
    state = {"commit": head.decode().strip(), "dirty": bool(entries), "dirty_paths": len(entries),
             "dirty_diff_sha256": None}
    if entries:
        h = hashlib.sha256()
        h.update(_git(root, "diff", "HEAD", "--binary") or b"")
        untracked = _git(root, "ls-files", "--others", "--exclude-standard", "-z") or b""
        for path in sorted(p for p in untracked.split(b"\0") if p):
            full = os.path.join(root.encode(), path)
            digest = sha256_file(full) if os.path.isfile(full) and not os.path.islink(full) else "-"
            h.update(path + b"\0" + digest.encode() + b"\n")
        state["dirty_diff_sha256"] = h.hexdigest()
    return state


def referee_sha256():
    """sha256 over the files that decide what a recording's items are and how they compare."""
    here = os.path.dirname(os.path.abspath(__file__))
    h = hashlib.sha256()
    for name in ("normalise.py", "surfaces.py", "oracle_compare.py", "allow.json"):
        h.update(name.encode() + b"\0" + sha256_file(os.path.join(here, name)).encode() + b"\n")
    return h.hexdigest()


def extractor_sha256():
    """sha256 over the files that turn raw/ into norm/ and items/ (normalise.py, surfaces.py)."""
    here = os.path.dirname(os.path.abspath(__file__))
    h = hashlib.sha256()
    for name in ("normalise.py", "surfaces.py"):
        h.update(name.encode() + b"\0" + sha256_file(os.path.join(here, name)).encode() + b"\n")
    return h.hexdigest()


def _iso(ts):
    import datetime
    return datetime.datetime.fromtimestamp(ts, datetime.timezone.utc).isoformat(timespec="seconds")


def binary_origin(binary, root=None):
    """Whether `binary` can have been built from the checkout at `root` (default: this one).

    Audit R2A-2: the commit in a manifest is the recorder's checkout, and `--binary` may be any
    file. Without a commit stamped into the binary, the check is what the file system says:
      * the binary's real path lies inside <root>/target (the checkout's Cargo target dir;
        AGENTS.md forbids CARGO_TARGET_DIR), and
      * the binary was modified at or after the checkout's HEAD commit time (a binary older than
        HEAD was built from an earlier commit).
    {"commit_verified": bool, "reasons": [why not], "binary_realpath", "target_dir",
     "binary_mtime", "head_commit_time"}. Nothing here proves that a dirty tree's uncommitted state
    is what was built: that is the dirty flag's business (scoreboard --write refuses it).
    """
    root = os.path.realpath(root or REPO_ROOT)
    real = os.path.realpath(binary)
    target = os.path.join(root, "target")
    out = {"commit_verified": False, "reasons": [], "binary_realpath": real, "target_dir": target,
           "binary_mtime": None, "head_commit_time": None}
    if not (real + os.sep).startswith(target + os.sep):
        out["reasons"].append("binary is not inside the recorder's checkout target dir %s" % target)
    try:
        mtime = os.stat(real).st_mtime
        out["binary_mtime"] = _iso(mtime)
    except OSError as e:
        out["reasons"].append("cannot stat the binary: %s" % e)
        mtime = None
    ct = _git(root, "log", "-1", "--format=%ct", "HEAD")
    if ct is None or not ct.strip():
        out["reasons"].append("the recorder's checkout has no HEAD commit")
    else:
        head_time = int(ct.decode().strip())
        out["head_commit_time"] = _iso(head_time)
        if mtime is not None and mtime < head_time:
            out["reasons"].append("binary (modified %s) is older than the checkout's HEAD commit (%s)" % (
                out["binary_mtime"], out["head_commit_time"]))
    out["commit_verified"] = not out["reasons"]
    return out


def head_version_differs(path):
    """True when `path` (a file of this repository) differs from its version at HEAD or is not
    committed; None when it is not inside this repository or git is unavailable."""
    real = os.path.realpath(path)
    root = os.path.realpath(REPO_ROOT)
    if not real.startswith(root + os.sep):
        return None
    rel = os.path.relpath(real, root)
    head = _git(root, "show", "HEAD:" + rel)
    if head is None:
        return True
    try:
        with open(real, "rb") as f:
            return f.read() != head
    except OSError:
        return True
