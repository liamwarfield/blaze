"""Shared code of the gate tools (compare-tests, baseline, compare-scores). Standard library only."""

import gzip
import hashlib
import io
import json
import os

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
STATE_DIR = os.path.join(REPO_ROOT, "docs", "state")
BASELINE = os.path.join(STATE_DIR, "tests-baseline.json.gz")
BASELINE_SHA = BASELINE + ".sha256"

# What a baseline keeps of a run-tests result, besides what `baseline` adds.
BASELINE_KEYS = ("commit", "tree_dirty", "dirty_diff_sha256", "cargo_exit", "counts", "binaries", "results")


def load_results(path):
    """A run-tests result or a baseline: plain JSON or gzip (by content, not by file name)."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:2] == b"\x1f\x8b":
        data = gzip.decompress(data)
    doc = json.loads(data)
    if not isinstance(doc, dict) or not isinstance(doc.get("results"), dict):
        raise ValueError("%s is not a per-name test result (no `results` object)" % path)
    return doc


def read_name_map(path):
    """TSV `old_name<TAB>new_name<TAB>reason` for renamed or moved tests. The reason is required."""
    mapping = {}
    with open(path) as f:
        for n, line in enumerate(f, 1):
            line = line.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 3 or not all(p.strip() for p in parts[:3]):
                raise ValueError("%s:%d: expected old_name<TAB>new_name<TAB>reason" % (path, n))
            mapping[parts[0]] = parts[1]
    return mapping


def compare_tests(base, new, mapping=None):
    """Apply the acceptance rule to two {name: status} maps.

    Every name that passed in `base` must pass in `new`. A name that no longer passes is `lost`
    (now fail or ignored) or `absent` (not in `new` at all: deleted, renamed, not built, not run).
    New passes never compensate. Returns {"retained": n, "recovered": [...], "added": [...],
    "lost": [(name, status)], "absent": [...]}.
    """
    mapping = mapping or {}
    out = {"retained": 0, "recovered": [], "added": [], "lost": [], "absent": []}
    for name in sorted(base):
        cur = new.get(mapping.get(name, name))
        if base[name] == "pass":
            if cur is None:
                out["absent"].append(name)
            elif cur != "pass":
                out["lost"].append((name, cur))
            else:
                out["retained"] += 1
        elif cur == "pass":
            out["recovered"].append(name)
    known = {mapping.get(n, n) for n in base}
    out["added"] = sorted(n for n, st in new.items() if n not in known and st == "pass")
    return out


def regressions(cmp):
    return len(cmp["lost"]) + len(cmp["absent"])


def canonical_json(doc):
    """The bytes a baseline is made of: sorted keys, one-space indent, trailing newline."""
    return (json.dumps(doc, indent=1, sort_keys=True, ensure_ascii=False) + "\n").encode()


def gzip_bytes(data):
    """gzip without a file name or a timestamp in the header (as `gzip -n`), so that the same
    JSON always gives the same file."""
    buf = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=buf, compresslevel=9, mtime=0) as f:
        f.write(data)
    return buf.getvalue()


def sha256(data):
    return hashlib.sha256(data).hexdigest()
