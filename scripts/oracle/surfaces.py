"""Turn a record's raw/ outputs into normalised files (norm/) and comparable items (items/).

Every surface becomes a dict item-key -> JSON value. `compare` diffs those dicts key by key.
Item keys are chosen so that run-specific identifiers (proto ids, named-set ids, sandbox numbers)
never appear in them: actions are keyed by primary output path, targets by label (+ configuration
checksum), BEP events by their canonical event id.
"""

import base64
import hashlib
import json
import os
import re
import xml.etree.ElementTree as ET

import oracle_common as oc
from normalise import stderr_summary, stderr_tail

# Files under bazel-testlogs (and spawn outputs under bazel-out/*/testlogs) carry test runtimes,
# start times and test logs, so their bytes differ between identical runs. They are recorded by
# existence only; their meaning is compared through the `tests` surface.
TESTLOGS_RE = re.compile(r"(^|/)testlogs/")

# BEP payload keys that hold wall-clock values.
BEP_TIMING_KEY_RE = re.compile(
    r"(Millis|Ms|Nanos|InMs|Duration|DurationMillis|Epoch|^startTime|^finishTime|^firstStartTime"
    r"|^lastStopTime|Time$|^testAttempt(Start|Duration).*|^wallTime|^cpuTime|^userTime|^systemTime)"
)
# Client-to-server options whose values are durations measured by the client.
TIMING_OPTIONS = ("startup_time", "command_wait_time", "extract_data_time", "connect_time")
TIMING_OPTION_RE = re.compile(r"^--(%s)=\S*$" % "|".join(TIMING_OPTIONS))
# workspaceStatus keys whose values are host, user or clock.
VOLATILE_STATUS_KEYS = {"BUILD_HOST", "BUILD_USER", "BUILD_TIMESTAMP", "FORMATTED_DATE"}
# buildMetrics sub-messages that only measure the machine (memory, time, network, workers).
BEP_METRICS_DROP = {
    "memoryMetrics", "timingMetrics", "cumulativeMetrics", "networkMetrics", "workerMetrics",
    "workerPoolMetrics", "dynamicExecutionMetrics", "artifactMetrics",
}


# query runs with --keep_going: exit 3 means "partial result" (e.g. a select() branch whose
# repository cannot be fetched); the output is still recorded and the exit code compared.
QUERY_OK = (0, 3)


class SurfaceFailed(Exception):
    pass


# ---------------------------------------------------------------------------------------------
# helpers


def _cmd(manifest, name):
    for c in manifest.get("commands", []):
        if c["name"] == name:
            return c
    return None


def _read(rd, name, binary=False):
    path = os.path.join(rd, "raw", name)
    if not os.path.isfile(path):
        raise SurfaceFailed("missing raw/" + name)
    with open(path, "rb") as f:
        data = f.read()
    return data if binary else data.decode("utf-8", "replace")


def _stdout(rd, manifest, name, ok_codes=(0,)):
    c = _cmd(manifest, name)
    if c is None:
        raise SurfaceFailed("command %s was not run" % name)
    if c["exit_code"] not in ok_codes:
        raise SurfaceFailed("command %s exited %d" % (name, c["exit_code"]))
    return _read(rd, name + ".stdout")


def _write_norm(rd, name, obj):
    path = os.path.join(rd, "norm", name)
    if isinstance(obj, (bytes, str)):
        with open(path, "wb") as f:
            f.write(obj if isinstance(obj, bytes) else obj.encode())
    else:
        oc.write_json(path, obj)


def _concat_json(text):
    """Parse a stream of JSON values (one per line, or pretty-printed back to back)."""
    dec = json.JSONDecoder()
    i, n, out = 0, len(text), []
    while True:
        while i < n and text[i].isspace():
            i += 1
        if i >= n:
            return out
        obj, i = dec.raw_decode(text, i)
        out.append(obj)


def _unique_key(items, key):
    if key not in items:
        return key
    k = 2
    while "%s #%d" % (key, k) in items:
        k += 1
    return "%s #%d" % (key, k)


# ---------------------------------------------------------------------------------------------
# surfaces


def s_exit_codes(rd, manifest, ctx):
    return {c["name"]: c["exit_code"] for c in manifest["commands"]}


def s_mod_graph(rd, manifest, ctx):
    graph = ctx.deep(json.loads(_stdout(rd, manifest, "mod_graph")))
    _write_norm(rd, "mod_graph.json", graph)
    items = {}

    def visit(node):
        key = node.get("key", "?")
        rec = {k: v for k, v in node.items() if k not in ("dependencies", "indirectDependencies")}
        for dk in ("dependencies", "indirectDependencies"):
            if dk in node:
                rec[dk] = [d.get("key") for d in node[dk]]
        if node.get("unexpanded") and key in items:
            pass
        elif key not in items or items[key].get("unexpanded"):
            items[key] = rec
        for dk in ("dependencies", "indirectDependencies"):
            for d in node.get(dk, []):
                visit(d)

    visit(graph)
    return items


def s_lockfile(rd, manifest, ctx):
    # The workspace starts with the project's own lockfile (if any); only a successful
    # `mod graph` makes the copied file the binary's output.
    _stdout(rd, manifest, "mod_graph")
    data, _ = ctx.bytes(_read(rd, "MODULE.bazel.lock", binary=True))
    _write_norm(rd, "MODULE.bazel.lock", data)
    lock = json.loads(data.decode())
    items = {"__bytes_sha256__": hashlib.sha256(data).hexdigest()}
    for k, v in lock.items():
        if isinstance(v, dict) and v:
            for k2, v2 in v.items():
                items["%s[%s]" % (k, k2)] = v2
        else:
            items[k] = v
    return items


def _label_kind_lines(text):
    out = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        kind, _, label = line.rpartition(" ")
        out.append((label, kind))
    return out


def s_query_label_kind(rd, manifest, ctx):
    items = {}
    for prefix, name in (("pat", "query_label_kind"), ("deps", "query_deps_label_kind")):
        text = ctx.text(_stdout(rd, manifest, name, QUERY_OK))
        _write_norm(rd, name + ".txt", text)
        lines = _label_kind_lines(text)
        for label, kind in lines:
            items["%s %s" % (prefix, label)] = kind
        items["__order__ %s" % prefix] = [l for l, _ in lines]
    return items


def _target_name(t):
    for k in ("rule", "sourceFile", "generatedFile", "packageGroup", "environmentGroup"):
        if k in t:
            return t[k].get("name")
    return oc.canonical(t)


def s_query_jsonproto(rd, manifest, ctx):
    items = {}
    for prefix, name in (("pat", "query_jsonproto"), ("deps", "query_deps_jsonproto")):
        targets = [ctx.deep(t) for t in _concat_json(_stdout(rd, manifest, name, QUERY_OK))]
        _write_norm(rd, name + ".jsonl", "".join(oc.canonical(t) + "\n" for t in targets))
        order = []
        for t in targets:
            label = _target_name(t)
            order.append(label)
            items[_unique_key(items, "%s %s" % (prefix, label))] = t
        items["__order__ %s" % prefix] = order
    return items


def s_cquery_label_kind(rd, manifest, ctx):
    text = ctx.text(_stdout(rd, manifest, "cquery_deps_label_kind"))
    _write_norm(rd, "cquery_deps_label_kind.txt", text)
    items = {}
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        m = re.match(r"^(.*) (\S+) \(([^()]*)\)$", line)
        if not m:
            items[_unique_key(items, "unparsed " + line)] = line
            continue
        kind, label, cfg = m.groups()
        items[_unique_key(items, "deps %s (%s)" % (label, cfg))] = kind
    return items


def s_cquery_jsonproto(rd, manifest, ctx):
    doc = ctx.deep(json.loads(_stdout(rd, manifest, "cquery_jsonproto")))
    _write_norm(rd, "cquery_jsonproto.json", doc)
    cfg_by_id = {c.get("id"): c.get("checksum") for c in doc.get("configurations", [])}
    items = {}
    for c in doc.get("configurations", []):
        c = {k: v for k, v in c.items() if k != "id"}
        items[_unique_key(items, "configuration %s" % c.get("checksum"))] = c
    for r in doc.get("results", []):
        r = dict(r)
        if "configurationId" in r:
            r["configurationId"] = cfg_by_id.get(r["configurationId"], r["configurationId"])
        cfg = (r.get("configuration") or {}).get("checksum") or r.get("configurationId")
        items[_unique_key(items, "target %s (%s)" % (_target_name(r.get("target", {})), cfg))] = r
    return items


def aquery_summary(doc):
    """Resolve aquery jsonproto ids into one self-contained record per action.

    Keyed by primary output exec path. Values: mnemonic, owner target and rule class,
    configuration (mnemonic + checksum), aspects, argv, env, execution info, sorted input and
    output path sets, and the optional param-file / file-write / template contents.
    """
    frag = {p["id"]: p for p in doc.get("pathFragments", [])}
    path_cache = {}

    def frag_path(fid):
        if fid in path_cache:
            return path_cache[fid]
        parts, cur = [], fid
        while cur:
            p = frag[cur]
            parts.append(p["label"])
            cur = p.get("parentId")
        path_cache[fid] = "/".join(reversed(parts))
        return path_cache[fid]

    art = {a["id"]: frag_path(a["pathFragmentId"]) + ("/" if a.get("isTreeArtifact") else "")
           for a in doc.get("artifacts", [])}
    depsets = {d["id"]: d for d in doc.get("depSetOfFiles", [])}
    flat_cache = {}

    def flatten(did):
        if did in flat_cache:
            return flat_cache[did]
        d = depsets[did]
        s = set(art[a] for a in d.get("directArtifactIds", []))
        for t in d.get("transitiveDepSetIds", []):
            s |= flatten(t)
        flat_cache[did] = frozenset(s)
        return flat_cache[did]

    rule_classes = {r["id"]: r["name"] for r in doc.get("ruleClasses", [])}
    targets = {t["id"]: t for t in doc.get("targets", [])}
    configs = {c["id"]: c for c in doc.get("configuration", [])}
    aspects = {a["id"]: a for a in doc.get("aspectDescriptors", [])}
    out = {}
    for a in doc.get("actions", []):
        t = targets.get(a.get("targetId"), {})
        c = configs.get(a.get("configurationId"), {})
        inputs = set()
        for did in a.get("inputDepSetIds", []):
            inputs |= flatten(did)
        rec = {
            "mnemonic": a.get("mnemonic"),
            "target": t.get("label"),
            "ruleClass": rule_classes.get(t.get("ruleClassId")),
            "configuration": {k: c.get(k) for k in ("mnemonic", "platformName", "checksum", "isTool") if k in c},
            "aspects": [{k: v for k, v in aspects[x].items() if k != "id"} for x in a.get("aspectDescriptorIds", [])],
            "actionKey": a.get("actionKey"),
            "argv": a.get("arguments", []),
            "env": {e["key"]: e.get("value", "") for e in a.get("environmentVariables", [])},
            "executionInfo": {e["key"]: e.get("value", "") for e in a.get("executionInfo", [])},
            "inputs": sorted(inputs),
            "outputs": sorted(art[o] for o in a.get("outputIds", [])),
        }
        for k in ("executionPlatform", "discoversInputs", "isExecutable", "templateContent",
                  "substitutions", "fileContents", "unresolvedSymlinkTarget"):
            if k in a:
                rec[k] = a[k]
        if "paramFiles" in a:
            rec["paramFiles"] = a["paramFiles"]
        primary = art.get(a.get("primaryOutputId")) or (rec["outputs"][0] if rec["outputs"] else "?")
        out[_unique_key(out, primary)] = rec
    return out


def s_aquery_actions(rd, manifest, ctx):
    doc = ctx.deep(json.loads(_stdout(rd, manifest, "aquery_deps_jsonproto")))
    _write_norm(rd, "aquery_deps_jsonproto.json", doc)
    summary = aquery_summary(doc)
    _write_norm(rd, "aquery_actions.json", summary)
    return summary


def s_build_stderr(rd, manifest, ctx):
    c = _cmd(manifest, "build")
    if c is None:
        raise SurfaceFailed("command build was not run")
    text = _read(rd, "build.stderr")
    _write_norm(rd, "build.stderr.txt", ctx.text(text))
    return stderr_summary(ctx, text)


def normalise_spawn(ctx, spawn):
    s = {k: v for k, v in spawn.items() if k not in ("metrics", "walltime")}
    s = ctx.deep(s)
    for o in s.get("actualOutputs", []):
        if TESTLOGS_RE.search(o.get("path", "")) and "digest" in o:
            o["digest"] = "<timing-dependent>"
    return s


def s_execlog(rd, manifest, ctx):
    c = _cmd(manifest, "build")
    if c is None:
        raise SurfaceFailed("command build was not run")
    spawns = [normalise_spawn(ctx, s) for s in _concat_json(_read(rd, "execlog.json"))]
    _write_norm(rd, "execlog.json", spawns)
    items, order = {}, []
    for s in spawns:
        outs = s.get("listedOutputs") or []
        key = outs[0] if outs else "%s %s" % (s.get("mnemonic"), oc.canonical(s.get("commandArgs"))[:200])
        key = _unique_key(items, key)
        order.append(key)
        items[key] = s
    items["__order__"] = order
    return items


def _drop_timing(obj):
    if isinstance(obj, dict):
        out = {}
        for k, v in obj.items():
            if BEP_TIMING_KEY_RE.search(k):
                continue
            out[k] = _drop_timing(v)
        if out.get("optionName") in TIMING_OPTIONS:
            out["optionValue"] = "<DUR>"
            out["combinedForm"] = "--%s=<DUR>" % out["optionName"]
        return out
    if isinstance(obj, list):
        return [_drop_timing(x) for x in obj]
    if isinstance(obj, str) and TIMING_OPTION_RE.match(obj):
        return obj.split("=", 1)[0] + "=<DUR>"
    return obj


def _rc_group(value):
    """'1:build:macos=--cxxopt=x' -> '1:build:macos' (rc file index and command[:config])."""
    return value.split("=", 1)[0]


def sort_default_overrides(seq, value_of):
    """Stable-sort each contiguous run of --default_override entries by rc group.

    The client sends .bazelrc lines as --default_override=<rc>:<command>=<option>, grouped by
    command in an order that changes between runs (hash-map iteration); the order of lines within
    one rc file and command is kept, and that is the order that has meaning.
    """
    out, run = [], []
    for x in list(seq) + [None]:
        v = value_of(x) if x is not None else None
        if v is not None:
            run.append((v, x))
            continue
        if run:
            out.extend(x2 for _, x2 in sorted(run, key=lambda p: _rc_group(p[0])))
            run = []
        if x is not None:
            out.append(x)
    return out


def _arg_override(x):
    if isinstance(x, str) and x.startswith("--default_override="):
        return x[len("--default_override="):]
    return None


def _opt_override(x):
    if isinstance(x, dict) and x.get("optionName") == "default_override":
        return x.get("optionValue", "")
    return None


def _sort_overrides_deep(obj):
    if isinstance(obj, dict):
        return {k: _sort_overrides_deep(v) for k, v in obj.items()}
    if isinstance(obj, list):
        obj = [_sort_overrides_deep(v) for v in obj]
        if any(_arg_override(x) is not None for x in obj):
            obj = sort_default_overrides(obj, _arg_override)
        if any(_opt_override(x) is not None for x in obj):
            obj = sort_default_overrides(obj, _opt_override)
        return obj
    return obj


def normalise_bep(ctx, events):
    """Normalised BEP events keyed by canonical event id. Named sets are inlined, progress dropped."""
    named = {}
    for e in events:
        if "namedSet" in e.get("id", {}):
            named[e["id"]["namedSet"]["id"]] = e.get("namedSetOfFiles", {})
    flat_cache = {}

    def flatten(nid):
        if nid in flat_cache:
            return flat_cache[nid]
        flat_cache[nid] = []
        ns = named.get(nid, {})
        files = {oc.canonical(f): f for f in ns.get("files", [])}
        for sub in ns.get("fileSets", []):
            for f in flatten(sub["id"]):
                files[oc.canonical(f)] = f
        flat_cache[nid] = [files[k] for k in sorted(files)]
        return flat_cache[nid]

    def keep_child(c):
        return "progress" not in c and "namedSet" not in c

    out = {}
    for e in events:
        eid = e.get("id", {})
        kind = next(iter(eid), "?")
        if kind in ("progress", "namedSet"):
            continue
        p = {k: v for k, v in e.items() if k != "id"}
        if "children" in p:
            p["children"] = sorted((c for c in p["children"] if keep_child(c)), key=oc.canonical)
        if "started" in p:
            p["started"] = {k: v for k, v in p["started"].items()
                            if k not in ("uuid", "serverPid", "host", "user")}
        if "workspaceStatus" in p:
            for it in p["workspaceStatus"].get("item", []):
                if it.get("key") in VOLATILE_STATUS_KEYS:
                    it["value"] = "<%s>" % it["key"]
        if "buildToolLogs" in p:
            p["buildToolLogs"] = {"log": sorted(l.get("name", "") for l in p["buildToolLogs"].get("log", []))}
        if "buildMetrics" in p:
            p["buildMetrics"] = {k: v for k, v in p["buildMetrics"].items() if k not in BEP_METRICS_DROP}
        if "completed" in p:
            for g in p["completed"].get("outputGroup", []):
                if "fileSets" in g:
                    files = {}
                    for fs in g.pop("fileSets"):
                        for f in flatten(fs["id"]):
                            files[oc.canonical(f)] = f
                    g["files"] = [files[k] for k in sorted(files)]
        if "expanded" in p:
            # Test-suite expansions come from an unordered set (pairs swap between runs).
            exps = p["expanded"].get("testSuiteExpansions", [])
            for x in exps:
                x["testLabels"] = sorted(x.get("testLabels", []))
            p["expanded"]["testSuiteExpansions"] = sorted(exps, key=lambda x: x.get("suiteLabel", ""))
        if "testResult" in p:
            tr = p["testResult"]
            if isinstance(tr.get("executionInfo"), dict):
                tr["executionInfo"].pop("timingBreakdown", None)
            for f in tr.get("testActionOutput", []):
                f.pop("digest", None)
                f.pop("length", None)
        p = _sort_overrides_deep(_drop_timing(ctx.deep(p)))
        out[_unique_key(out, oc.canonical(ctx.deep(eid)))] = p
    return out


def s_bep(rd, manifest, ctx):
    if _cmd(manifest, "build") is None:
        raise SurfaceFailed("command build was not run")
    events = [json.loads(l) for l in _read(rd, "bep.json").splitlines() if l.strip()]
    items = normalise_bep(ctx, events)
    _write_norm(rd, "bep.json", items)
    return items


def s_outputs(rd, manifest, ctx):
    if _cmd(manifest, "build") is None:
        raise SurfaceFailed("command build was not run")
    walk = json.loads(_read(rd, "outputs.walk.json"))
    if isinstance(walk, dict):
        if "bin" not in walk.get("roots", []):
            raise SurfaceFailed("no bazel-bin convenience symlink after the build")
        entries = walk["entries"]
    else:
        entries = walk
    items = {}
    for e in entries:
        path = e["path"]
        if e["type"] == "file" and path.startswith("testlogs/"):
            v = {"type": "file", "timing_dependent": True}
        elif e["type"] == "file":
            v = {"type": "file", "sha256": e["sha256"], "executable": e["executable"],
                 "path_normalised": e["path_normalised"]}
        else:
            v = {k: x for k, x in e.items() if k != "path"}
        items[path] = v
    _write_norm(rd, "outputs.json", items)
    return items


def _label_to_testlog_dir(label):
    if not label.startswith("//"):
        return None
    pkg, _, name = label[2:].partition(":")
    return "/".join(x for x in (pkg, name) if x)


# Test case names that embed object reprs (Python parameterized tests: "<function f at 0x7f..>").
ADDR_RE = re.compile(r"\b0x[0-9a-fA-F]{6,}\b")


def parse_test_xml(data):
    """[(suite, case, status)] from a JUnit-style test.xml, ignoring times and logs.

    Memory addresses in names become 0x<ADDR>.
    """
    root = ET.fromstring(data)
    out = []
    suites = [root] if root.tag == "testsuite" else root.iter("testsuite")
    for s in suites:
        for c in s.findall("testcase"):
            status = "passed"
            if c.find("failure") is not None:
                status = "failed"
            elif c.find("error") is not None:
                status = "error"
            elif c.find("skipped") is not None or c.get("status") == "notrun":
                status = "skipped"
            out.append([ADDR_RE.sub("0x<ADDR>", s.get("name", "")),
                        ADDR_RE.sub("0x<ADDR>", c.get("name", "")), status])
    return sorted(out)


def s_tests(rd, manifest, ctx):
    if _cmd(manifest, "build") is None:
        raise SurfaceFailed("command build was not run")
    events = [json.loads(l) for l in _read(rd, "bep.json").splitlines() if l.strip()]
    tests = {}
    for e in events:
        eid = e.get("id", {})
        if "testSummary" in eid:
            label = eid["testSummary"]["label"]
            s = e.get("testSummary", {})
            t = tests.setdefault(label, {})
            t["overallStatus"] = s.get("overallStatus")
            t["totalRunCount"] = s.get("totalRunCount")
            t["shardCount"] = s.get("shardCount")
        elif "testResult" in eid:
            label = eid["testResult"]["label"]
            r = eid["testResult"]
            t = tests.setdefault(label, {})
            t.setdefault("results", []).append(
                ["run%s/shard%s/attempt%s" % (r.get("run"), r.get("shard"), r.get("attempt")),
                 e.get("testResult", {}).get("status")])
    xml_dir = os.path.join(rd, "raw", "testxml")
    for label, t in tests.items():
        t["results"] = sorted(t.get("results", []))
        d = _label_to_testlog_dir(label)
        path = os.path.join(xml_dir, d, "test.xml") if d else None
        if path and os.path.isfile(path):
            try:
                t["xml"] = parse_test_xml(open(path, "rb").read())
            except ET.ParseError as ex:
                t["xml"] = "unparseable: %s" % ex
        else:
            t["xml"] = None
    _write_norm(rd, "tests.json", tests)
    return tests


EXTRACTORS = {
    "exit_codes": (None, s_exit_codes),
    "mod_graph": ("mod", s_mod_graph),
    "lockfile": ("mod", s_lockfile),
    "query_label_kind": ("query", s_query_label_kind),
    "query_jsonproto": ("query", s_query_jsonproto),
    "cquery_label_kind": ("cquery", s_cquery_label_kind),
    "cquery_jsonproto": ("cquery", s_cquery_jsonproto),
    "aquery_actions": ("aquery", s_aquery_actions),
    "build_stderr": ("build", s_build_stderr),
    "execlog": ("build", s_execlog),
    "bep": ("build", s_bep),
    "outputs": ("build", s_outputs),
    "tests": ("build", s_tests),
}

# Which command's stderr explains a failed surface.
SURFACE_COMMANDS = {
    "mod_graph": ["mod_graph"], "lockfile": ["mod_graph"],
    "query_label_kind": ["query_label_kind", "query_deps_label_kind"],
    "query_jsonproto": ["query_jsonproto", "query_deps_jsonproto"],
    "cquery_label_kind": ["cquery_deps_label_kind"], "cquery_jsonproto": ["cquery_jsonproto"],
    "aquery_actions": ["aquery_deps_jsonproto"],
    "build_stderr": ["build"], "execlog": ["build"], "bep": ["build"], "outputs": ["build"],
    "tests": ["build"],
}


def extract_all(rd, manifest, ctx):
    results = {}
    steps = set(manifest.get("steps", oc.STEP_ORDER))
    for surface in oc.SURFACES:
        step, fn = EXTRACTORS[surface]
        if step is not None and step not in steps:
            res = {"surface": surface, "status": "SKIPPED", "items": {}}
        else:
            try:
                res = {"surface": surface, "status": "OK", "items": fn(rd, manifest, ctx)}
            except Exception as ex:  # a surface that fails is recorded, not fatal
                tails = {}
                for name in SURFACE_COMMANDS.get(surface, []):
                    p = os.path.join(rd, "raw", name + ".stderr")
                    if os.path.isfile(p):
                        tails[name] = [ctx.text(l) for l in stderr_tail(open(p, errors="replace").read())]
                res = {"surface": surface, "status": "FAILED", "items": {},
                       "error": "%s: %s" % (type(ex).__name__, ex), "stderr_tail": tails}
        oc.write_json(os.path.join(rd, "items", surface + ".json"), res, indent=None)
        results[surface] = res
    return results
