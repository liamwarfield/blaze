"""Shared paths and helpers for the oracle recorder, comparer and scoreboard.

Standard library only. Imported by scripts/oracle/{record,compare,scoreboard}.
"""

import hashlib
import json
import os
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
