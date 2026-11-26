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
# Startup options whose value is the installation's identity, not behaviour: Bazel's
# --install_md5 is the md5 of its embedded install files, Blaze's is its own install key.
# The value becomes a placeholder; the option itself is still compared.
MASKED_OPTION_VALUES = {"install_md5": "<INSTALL_MD5>"}
MASKED_OPTION_RE = re.compile(r"^--(%s)=\S*$" % "|".join(MASKED_OPTION_VALUES))
# Startup options the recorder sets for its own sake: --max_idle_secs (README "Server mode",
# AGENTS.md "Memory"). The reference was recorded without it, so it is removed on both sides from
# every BEP startup-option list (parsed, explicit, canonical with Bazel's default value, and the
# original command line). There the client follows every startup option by an empty one
# (blaze.cc BlazeServer::Communicate calls request.add_startup_options() twice, so Bazel prints
# `{}` after each), and that empty entry goes with it.
# The client also names every explicit startup option in --option_sources
# (name:source:name:source..., names escaped by EscapeForOptionSource: "_" -> "_U", ":" -> "_C";
# blaze.cc), so the option's pair is removed from that value too.
# Likewise the recorder's environment variable BLAZE_SERVER_MODE (daemon mode): the client
# forwards its environment as --client_env=NAME=VALUE (Bazel's client forwards every variable),
# so that option is removed wherever it appears.
RECORDER_STARTUP_OPTIONS = ("max_idle_secs",)
RECORDER_CLIENT_ENV = ("BLAZE_SERVER_MODE",)
RECORDER_OPTION_RE = re.compile(r"^--((%s)=|client_env=(%s)=)\S*$" % (
    "|".join(RECORDER_STARTUP_OPTIONS), "|".join(RECORDER_CLIENT_ENV)))
RECORDER_OPTION_SOURCE_KEYS = {o.replace("_", "_U").replace(":", "_C") for o in RECORDER_STARTUP_OPTIONS}
# buildMetrics sub-messages that only measure the machine (memory, time, network, workers):
# compared by presence (which fields are set), not by value (referee batch 3; review
# 2026-11-25-reporting S1-1: dropping them let an empty message score as identical).
BEP_METRICS_PRESENCE = {
    "memoryMetrics", "timingMetrics", "networkMetrics", "workerMetrics", "workerPoolMetrics",
    "dynamicExecutionMetrics",
}
# artifactMetrics groups whose sizeInBytes depends on the output root path (runfiles manifests
# name absolute execroot paths) or on test timing (test logs and test.xml are outputs): the three
# oracle recordings differ there (brotli outputArtifactsSeen 7671064/7671088/7671088, re2
# 89006013/89050518/89050526). Their counts, and sourceArtifactsRead (sizes equal in all three),
# are compared as they are.
ARTIFACT_SIZE_MASKED = {"outputArtifactsSeen", "outputArtifactsFromActionCache", "topLevelArtifacts"}


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
    items, order = {}, []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        m = re.match(r"^(.*) (\S+) \(([^()]*)\)$", line)
        if not m:
            key = _unique_key(items, "unparsed " + line)
            items[key] = line
            order.append(key)
            continue
        kind, label, cfg = m.groups()
        key = _unique_key(items, "deps %s (%s)" % (label, cfg))
        items[key] = kind
        order.append(key)
    # Bazel prints cquery results in a deterministic order (identical in three oracle recordings
    # of both projects), so the order is an item of its own, as for query.
    items["__order__ deps"] = order
    return items


def s_cquery_jsonproto(rd, manifest, ctx):
    doc = ctx.deep(json.loads(_stdout(rd, manifest, "cquery_jsonproto")))
    _write_norm(rd, "cquery_jsonproto.json", doc)
    cfg_by_id = {c.get("id"): c.get("checksum") for c in doc.get("configurations", [])}
    items, order = {}, []
    for c in doc.get("configurations", []):
        c = {k: v for k, v in c.items() if k != "id"}
        items[_unique_key(items, "configuration %s" % c.get("checksum"))] = c
    for r in doc.get("results", []):
        r = dict(r)
        if "configurationId" in r:
            r["configurationId"] = cfg_by_id.get(r["configurationId"], r["configurationId"])
        cfg = (r.get("configuration") or {}).get("checksum") or r.get("configurationId")
        key = _unique_key(items, "target %s (%s)" % (_target_name(r.get("target", {})), cfg))
        items[key] = r
        order.append(key)
    # The order of the results and the order and numbering of the configurations are deterministic
    # in Bazel (the whole normalised document is identical in three oracle recordings).
    items["__order__ results"] = order
    items["__order__ configurations"] = [[c.get("id"), c.get("checksum")] for c in doc.get("configurations", [])]
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


# The id-valued fields of an aquery action: which target, configuration, depsets, artifacts and
# aspects it names, in the order it names them.
AQUERY_ACTION_ID_FIELDS = ("targetId", "configurationId", "primaryOutputId", "inputDepSetIds", "outputIds",
                           "aspectDescriptorIds")
# The other top-level lists of the aquery container. Each entry is only ids, names and paths.
AQUERY_ID_LISTS = ("targets", "artifacts", "depSetOfFiles", "pathFragments", "configuration", "ruleClasses",
                   "aspectDescriptors")


def aquery_order(doc, action_keys):
    """`__order__` items for an aquery jsonproto container: what aquery_summary leaves out.

    aquery_summary resolves ids and sorts, so that actions compare one by one whatever their order
    and numbering. Bazel's order and numbering are deterministic, though: every list of the
    normalised container, ids included, is identical in three oracle recordings of both projects
    (the only field that varies is the actionKey of runfiles manifest actions, see allow.json). So
    each list is also compared as printed:

      __order__ actions     the action keys (primary output paths) in output order
      __order__ action ids  per action, in output order: its key and its id-valued fields
      __order__ <list>      targets, artifacts, depSetOfFiles, pathFragments, configuration,
                            ruleClasses, aspectDescriptors: the list as printed, ids included

    Together with the per-action items these decide the whole normalised container. Each value is
    an object {"order": [...]}: every item of this surface stays an object, as readers of
    items/aquery_actions.json expect.
    """
    actions = doc.get("actions", [])
    items = {
        "__order__ actions": {"order": list(action_keys)},
        "__order__ action ids": {"order": [
            [key] + [a.get(f) for f in AQUERY_ACTION_ID_FIELDS] for key, a in zip(action_keys, actions)
        ]},
    }
    for name in AQUERY_ID_LISTS:
        if name in doc:
            items["__order__ " + name] = {"order": doc[name]}
    return items


def s_aquery_actions(rd, manifest, ctx):
    doc = ctx.deep(json.loads(_stdout(rd, manifest, "aquery_deps_jsonproto")))
    _write_norm(rd, "aquery_deps_jsonproto.json", doc)
    summary = aquery_summary(doc)
    _write_norm(rd, "aquery_actions.json", summary)
    # aquery_summary adds one key per action, in the order of doc["actions"].
    return {**summary, **aquery_order(doc, list(summary))}


def s_build_stderr(rd, manifest, ctx):
    c = _cmd(manifest, "build")
    if c is None:
        raise SurfaceFailed("command build was not run")
    text = _read(rd, "build.stderr")
    _write_norm(rd, "build.stderr.txt", ctx.text(text))
    # The server-start line is compared only for a recording made on Blaze's persistent server
    # (see normalise.SERVER_START_RE); every other recording drops it, as before batch 3.
    return stderr_summary(ctx, text, keep_server_start=oc.blaze_server_mode(manifest) == "daemon")


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


def mask_option_values(obj):
    """MASKED_OPTION_VALUES: `--install_md5=<hex>` becomes `--install_md5=<INSTALL_MD5>`, as a
    string and in an option message (optionValue and combinedForm)."""
    if isinstance(obj, dict):
        out = {k: mask_option_values(v) for k, v in obj.items()}
        ph = MASKED_OPTION_VALUES.get(out.get("optionName"))
        if ph is not None:
            if "optionValue" in out:
                out["optionValue"] = ph
            if "combinedForm" in out:
                out["combinedForm"] = "--%s=%s" % (out["optionName"], ph)
        return out
    if isinstance(obj, list):
        return [mask_option_values(x) for x in obj]
    if isinstance(obj, str) and MASKED_OPTION_RE.match(obj):
        name = obj[2:].split("=", 1)[0]
        return "--%s=%s" % (name, MASKED_OPTION_VALUES[name])
    return obj


def _is_recorder_option(x):
    if isinstance(x, str):
        return bool(RECORDER_OPTION_RE.match(x))
    if isinstance(x, dict):
        return (x.get("optionName") in RECORDER_STARTUP_OPTIONS
                or bool(RECORDER_OPTION_RE.match(x.get("combinedForm") or "")))
    return False


def _strip_option_sources(value):
    """The --option_sources value without the pairs of RECORDER_STARTUP_OPTIONS."""
    tokens = value.split(":")
    if len(tokens) % 2:
        return value
    pairs = [(tokens[i], tokens[i + 1]) for i in range(0, len(tokens), 2)]
    return ":".join("%s:%s" % p for p in pairs if p[0] not in RECORDER_OPTION_SOURCE_KEYS)


def drop_recorder_options(obj):
    """Remove RECORDER_STARTUP_OPTIONS and RECORDER_CLIENT_ENV from every list (`--max_idle_secs=N`
    and `--client_env=BLAZE_SERVER_MODE=...` strings and option messages, and the empty option
    that follows an original-command-line entry: {"combinedForm": "--max_idle_secs=N"}, {}), and
    their pairs from --option_sources."""
    if isinstance(obj, dict):
        out = {k: drop_recorder_options(v) for k, v in obj.items()}
        if out.get("optionName") == "option_sources":
            if "optionValue" in out:
                out["optionValue"] = _strip_option_sources(out["optionValue"])
            if "combinedForm" in out:
                out["combinedForm"] = "--option_sources=" + _strip_option_sources(
                    out["combinedForm"][len("--option_sources="):])
        return out
    if isinstance(obj, str) and obj.startswith("--option_sources="):
        return "--option_sources=" + _strip_option_sources(obj[len("--option_sources="):])
    if isinstance(obj, list):
        out, after_original = [], False
        for x in obj:
            if after_original and x == {}:
                after_original = False
                continue
            after_original = False
            if _is_recorder_option(x):
                after_original = isinstance(x, dict) and set(x) == {"combinedForm"}
                continue
            out.append(drop_recorder_options(x))
        return out
    return obj


def presence(obj):
    """The shape of a measurement: which fields are set. Scalars become "<measured>", a list
    (proto3 JSON prints only non-empty ones) "<list>"; message fields are kept by name."""
    if isinstance(obj, dict):
        return {k: presence(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return "<list>"
    return "<measured>"


def normalise_build_metrics(ctx, bm):
    """buildMetrics: counters compared, machine measurements compared by presence.

    actionSummary, targetMetrics, packageMetrics, buildGraphMetrics, cumulativeMetrics and the
    rest: compared as they are, without wall-clock keys (BEP_TIMING_KEY_RE). The varying
    buildGraphMetrics and action-cache fields are the allow list's business
    (build-metrics-volatile-counters), evaluatedValues among them by name (decisions
    2026-11-26). artifactMetrics: counts compared, sizes as ARTIFACT_SIZE_MASKED says.
    BEP_METRICS_PRESENCE: presence() only (taken before the wall-clock keys are dropped).
    """
    counters, shapes = {}, {}
    for k, v in bm.items():
        if k in BEP_METRICS_PRESENCE:
            shapes[k] = presence(v)
        elif k == "artifactMetrics" and isinstance(v, dict):
            am = {}
            for group, g in v.items():
                g = dict(g) if isinstance(g, dict) else g
                if group in ARTIFACT_SIZE_MASKED and isinstance(g, dict) and "sizeInBytes" in g:
                    g["sizeInBytes"] = "<path-dependent>"
                am[group] = g
            counters[k] = am
        else:
            counters[k] = v
    out = _drop_timing(ctx.deep(counters))
    out.update(shapes)
    return out


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
        metrics = p.pop("buildMetrics", None)
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
        p = drop_recorder_options(mask_option_values(_sort_overrides_deep(_drop_timing(ctx.deep(p)))))
        if metrics is not None:
            p["buildMetrics"] = normalise_build_metrics(ctx, metrics)
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
                        with open(p, errors="replace") as f:
                            tails[name] = [ctx.text(l) for l in stderr_tail(f.read())]
                res = {"surface": surface, "status": "FAILED", "items": {},
                       "error": "%s: %s" % (type(ex).__name__, ex), "stderr_tail": tails}
        oc.write_json(os.path.join(rd, "items", surface + ".json"), res, indent=None)
        results[surface] = res
    return results
