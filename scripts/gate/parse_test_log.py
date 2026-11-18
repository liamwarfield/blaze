"""Parse `cargo test` text output into per-name results. Usage: parse_test_log.py LOG OUT ROOT"""
import json, re, subprocess, sys

log, out, root = sys.argv[1:4]
results = {}
crate = None
# "Running unittests src/lib.rs (target/debug/deps/blaze_cmdline-abc123)"
run_re = re.compile(r"^\s*Running (?:unittests )?(\S+) \(target/[^)]*/deps/([A-Za-z0-9_]+)-[0-9a-f]+\)")
doc_re = re.compile(r"^\s*Doc-tests (\S+)")
test_re = re.compile(r"^test (\S+)(?: - .*)? \.\.\. (ok|FAILED|ignored(?:, .*)?)$")
for line in open(log, errors="replace"):
    line = line.rstrip("\n")
    m = run_re.match(line)
    if m:
        src, binary = m.groups()
        crate = binary if src.startswith("src/") else f"{binary}[{src}]"
        continue
    m = doc_re.match(line)
    if m:
        crate = f"{m.group(1)}[doc]"
        continue
    m = test_re.match(line)
    if m and crate:
        name, st = m.groups()
        status = "pass" if st == "ok" else "fail" if st == "FAILED" else "ignored"
        results[f"{crate}::{name}"] = status
def git(*a):
    return subprocess.run(["git", "-C", root, *a], capture_output=True, text=True).stdout.strip()
doc = {"commit": git("rev-parse", "HEAD"), "tree_dirty": bool(git("status", "--porcelain")),
       "counts": {k: sum(1 for v in results.values() if v == k) for k in ("pass", "fail", "ignored")},
       "results": dict(sorted(results.items()))}
json.dump(doc, open(out, "w"), indent=1)
print(json.dumps(doc["counts"]))
