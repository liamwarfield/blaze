#!/usr/bin/env python3
"""Classify extracted Starlark API uses and write the native-API inventory TSV + stats.

Usage: report.py <out.tsv> [stats.json]      report.py help

Reads ~/.cache/blaze/work/inventory/out/{java-index.json,<p>/files.tsv,<p>/uses.jsonl,
<p>/dump-rules.txt,<p>/show-repo.txt,<p>/skyframe-deps.txt} for p in brotli, re2.
Each use is classified as core (starlark-rust language), native (Java @StarlarkMethod or native
rule / provider), starlark (defined in .bzl: struct/provider fields, macros), dynamic
(ctx.attr.<x> etc.), ambiguous (member name matches several native types; receiver unknown) or
unknown. Native rows are aggregated per (receiver type, symbol).
"""
import collections, json, os, re, sys

HOME = os.path.expanduser("~")
OUT = f"{HOME}/.cache/blaze/work/inventory/out"
PROJECTS = ("brotli", "re2")
if len(sys.argv) < 2 or sys.argv[1] in ("help", "-h", "--help"):
    print(__doc__); sys.exit(0)

J = json.load(open(f"{OUT}/java-index.json"))
def rel(p): return p.replace("src/main/java/", "")
members = collections.defaultdict(dict)
for m in J["methods"]:
    if m["class"] == "StarlarkBaseExternalContext":
        for t in ("repository_ctx", "module_ctx"): members[t].setdefault(m["name"], m)
    else: members[m["builtin"] or m["class"]].setdefault(m["name"], m)
def ret_type(m):
    return m.get("ret_builtin") or (m.get("ret") if m.get("ret") in members else None)
import ast as _ast
def wrapper_members(path, var):
    t = _ast.parse(open(path).read())
    for s_ in t.body:
        if isinstance(s_, _ast.Assign) and getattr(s_.targets[0], "id", None) == var:
            return {k.arg for k in s_.value.keywords}
    return set()
BZ = f"{HOME}/.cache/blaze/src/bazel-9.3.2/src/main/starlark/builtins_bzl/common"
WRAPPERS = {"cc_common": wrapper_members(f"{BZ}/cc/cc_common_bazel.bzl", "cc_common"),
            "py_builtins": wrapper_members(f"{BZ}/python/py_internal.bzl", "py_internal"),
            "apple_common": wrapper_members(f"{BZ}/objc/apple_common.bzl", "apple_common_bazel") | wrapper_members(f"{BZ}/objc/apple_common.bzl", "apple_common"),
            "java_common": wrapper_members(f"{BZ}/java/java_common.bzl", "java_common_export_for_bazel") | wrapper_members(f"{BZ}/java/java_common.bzl", "java_common")}
by_member = collections.defaultdict(set)
CORE_TYPES = {"string", "list", "dict", "set", "tuple", "int", "float", "bool", "range", "bytes"}
for t, ms in members.items():
    for n in ms: by_member[n].add(t)
core_members = {n for t in CORE_TYPES for n in members.get(t, {})}
def builtin_loc(b):
    for cname, lst in J["classes"].items():
        for x in lst:
            if x.get("builtin") == b: return f"{rel(x['file'])}:{x['line']}", cname
    return "", ""
def impl_of(m):
    if m["impls"]:
        # prefer non-test, concrete
        c, f, l = m["impls"][0].split(":")
        return f"{rel(f)}:{l} {c}.{m['java_method']}"
    return f"{rel(m['file'])}:{m['line']} {m['class']}.{m['java_method']}"

GLOBAL_FN_CLASSES = {"StarlarkRuleFunctionsApi", "StarlarkBuildApiGlobals", "ConfigGlobalLibraryApi",
                     "RepositoryModuleApi", "DepsetLibrary", "SelectLibrary", "BuildGlobals", "PackageCallable"}
global_fns = {m["name"]: m for m in J["methods"] if m["class"] in GLOBAL_FN_CLASSES}
native_mod_fns = {m["name"]: m for m in J["methods"] if m["class"] == "StarlarkNativeModuleApi"}
module_fns = {m["name"]: m for m in J["methods"] if m["class"] == "ModuleFileGlobals"}
repo_fns = {m["name"]: m for m in J["methods"] if m["class"] == "RepoFileGlobals"}
core_fns = {m["name"]: m for m in J["methods"] if m["class"] == "MethodLibrary"}
CORE_CONST = {"True", "False", "None"}
OBJ_GLOBALS = {  # global name -> builtin type name used for its members
    "attr": "attr", "native": "native", "config": "config", "config_common": "config_common",
    "platform_common": "platform_common", "testing": "testing", "coverage_common": "coverage_common",
    "json": "json", "proto": "proto", "struct": "struct", "DefaultInfo": "DefaultInfo",
    "OutputGroupInfo": "OutputGroupInfo", "RunEnvironmentInfo": "RunEnvironmentInfo",
    "InstrumentedFilesInfo": "InstrumentedFilesInfo", "AnalysisTestResultInfo": "AnalysisTestResultInfo",
    "PackageSpecificationInfo": "PackageSpecificationInfo", "cc_common": "cc_common",
    "apple_common": "apple_common", "java_common": "java_common", "py_internal": "py_builtins",
    "_builtins": "_builtins", "proto_common_do_not_use": "proto_common", "Actions": "Actions",
    "CcInfo": "CcInfo", "CcToolchainConfigInfo": "CcToolchainConfigInfo", "DebugPackageInfo": "DebugPackageInfo",
    "CcSharedLibraryInfo": "CcSharedLibraryInfo", "CcSharedLibraryHintInfo": "CcSharedLibraryHintInfo",
    "AnalysisFailureInfo": "AnalysisFailureInfo", "MaterializedDepsInfo": "MaterializedDepsInfo",
}
OBJ_NOTES = {
    "cc_common": "builtins_bzl struct (common/cc/cc_common_bazel.bzl) wrapping native CcModule; 15 members",
    "py_internal": "builtins_bzl struct (common/python/py_internal.bzl) wrapping native py_builtins",
    "apple_common": "builtins_bzl struct (common/objc/apple_common.bzl) wrapping native AppleStarlarkCommon",
    "java_common": "builtins_bzl struct (common/java/java_common.bzl)",
    "CcInfo": "placeholder symbol: rules_cc defines CcInfo in Starlark (cc/common/cc_info.bzl)",
    "DebugPackageInfo": "placeholder symbol (rules_cc Starlark)",
    "CcSharedLibraryInfo": "placeholder symbol (rules_cc Starlark)",
    "CcSharedLibraryHintInfo": "placeholder symbol (rules_cc Starlark)",
    "CcToolchainConfigInfo": "guarded None outside allowed repos",
}
EMPTY_RULES = {"cc_binary", "cc_import", "cc_library", "cc_shared_library", "cc_static_library", "cc_test",
               "cc_toolchain", "cc_toolchain_suite", "fdo_prefetch_hints", "fdo_profile", "java_binary",
               "java_import", "java_library", "java_plugin", "java_runtime", "java_test", "java_toolchain",
               "memprof_profile", "objc_import", "objc_library", "propeller_optimize"}

def pb_rule_names(path):
    b = open(path, "rb").read()
    def varint(b, i):
        r = s = 0
        while True:
            c = b[i]; i += 1; r |= (c & 0x7f) << s; s += 7
            if c < 0x80: return r, i
    def fields(b):
        i = 0; out = []
        while i < len(b):
            k, i = varint(b, i); f, w = k >> 3, k & 7
            if w == 0: v, i = varint(b, i)
            elif w == 2: l, i = varint(b, i); v = b[i:i + l]; i += l
            elif w == 5: v = b[i:i + 4]; i += 4
            elif w == 1: v = b[i:i + 8]; i += 8
            out.append((f, v))
        return out
    return {w.decode() for f, v in fields(b) if f == 1 for g, w in fields(v) if g == 1}
NATIVE_RULES = pb_rule_names(f"{OUT}/brotli/build-language.pb") - EMPTY_RULES
rule_java = {}
for cname, lst in J["classes"].items():
    pass
# Java location of native rule classes: grep RuleDefinition files for .name("x") / builder("x")
SRCJ = f"{HOME}/.cache/blaze/src/bazel-9.3.2/src/main/java"
RULE_DEFS = {  # checked by hand against the pinned source (paths relative to src/main/java)
    "alias": "com/google/devtools/build/lib/rules/Alias.java:37",
    "config_setting": "com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:132",
    "toolchain": "com/google/devtools/build/lib/rules/platform/ToolchainRule.java:35",
    "filegroup": "com/google/devtools/build/lib/rules/filegroup/FilegroupRule.java:77",
    "constraint_value": "com/google/devtools/build/lib/rules/platform/ConstraintValueRule.java:34",
    "constraint_setting": "com/google/devtools/build/lib/rules/platform/ConstraintSettingRule.java:34",
    "toolchain_type": "com/google/devtools/build/lib/rules/ToolchainType.java:79",
    "label_flag": "com/google/devtools/build/lib/rules/LabelBuildSettings.java:120",
    "label_setting": "com/google/devtools/build/lib/rules/LabelBuildSettings.java:107",
    "platform": "com/google/devtools/build/lib/rules/platform/PlatformRule.java:39",
    "genrule": "com/google/devtools/build/lib/bazel/rules/genrule/BazelGenRuleRule.java:54",
    "test_suite": "com/google/devtools/build/lib/rules/test/TestSuiteRule.java:116",
    "cc_libc_top_alias": "com/google/devtools/build/lib/rules/cpp/CcLibcTopAlias.java:21",
    "java_plugins_flag_alias": "com/google/devtools/build/lib/rules/java/JavaPluginsFlagAliasRule.java:65",
    "starlark_doc_extract": "com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractRule.java:134",
}
def find_rule_def(name):
    if name in RULE_DEFS: return RULE_DEFS[name]
    import subprocess
    r = subprocess.run(["grep", "-rn", "--include=*.java", "-E",
                        f'(\\.name\\("{name}"\\)|setName\\("{name}"\\)|"{name}"\\)\\s*$|RuleDefinition.Metadata.builder\\(\\)\\s*$)', SRCJ],
                       capture_output=True, text=True).stdout.splitlines()
    for l in r:
        if f'"{name}"' in l:
            f, ln, _ = l.split(":", 2)
            return f"{os.path.relpath(f, SRCJ)}:{ln}"
    return ""

agg = {}   # key (receiver, symbol, kind) -> dict
cls_counts = {p: collections.Counter() for p in PROJECTS}
amb = collections.Counter(); unknown_globals = collections.Counter()
file_stats = {}
def add(key, p, use, java, note=""):
    a = agg.setdefault(key, {"uses": collections.Counter(), "files": {}, "java": java, "notes": set(),
                             "kwargs": collections.Counter(), "kinds": set(), "first": None, "sites": set()})
    a["uses"][p] += 1
    site = (use["file"], use["line"], use.get("recv", ""))
    if site in a["sites"]: return
    a["sites"].add(site)
    if a["first"] is None or (use["file"], use["line"]) < a["first"]: a["first"] = (use["file"], use["line"])
    a["kinds"].add(use["kind"])
    for k in use.get("kwargs", []): a["kwargs"][k] += 1
    if note: a["notes"].add(note)
    if not a["java"] and java: a["java"] = java

SEED_TYPES = {"ctx", "actions", "File", "Target", "Label", "runfiles", "depset", "Args", "repository_ctx",
              "module_ctx", "path", "exec_result", "repository_os", "DefaultInfo", "OutputGroupInfo", "attr",
              "native", "config", "config_common", "platform_common", "testing", "coverage_common", "json",
              "proto", "fragments", "configuration", "root", "FilesToRunProvider", "rule_attributes",
              "cc_internal", "cc_common", "py_builtins", "apple_common", "java_common", "bazel_module",
              "bazel_module_tags", "Provider", "RunEnvironmentInfo", "InstrumentedFilesInfo", "subrule_ctx",
              "ToolchainContext", "ExecGroupCollection", "ExecGroupContext", "TemplateDict", "Attribute"}
all_uses = {}
for p in PROJECTS:
    rows = [l.rstrip("\n").split("\t") for l in open(f"{OUT}/{p}/files.tsv")]
    seen_paths = {}
    for label, kind, path, n in rows:
        seen_paths.setdefault(path, (label, kind, int(n)))
    file_stats[p] = seen_paths
    first_label = {v[0]: k for k, v in seen_paths.items()}
    lst = all_uses[p] = []
    for line in open(f"{OUT}/{p}/uses.jsonl"):
        u = json.loads(line)
        path = first_label.get(u["file"])
        if path is None or seen_paths[path][1] != u["kind"]:
            continue  # same file already counted under another key kind
        lst.append(u)

def classify(u, reachable):
    """Return (category, key, java, note)."""
    kind = u["kind"]
    if u["cat"] == "global":
        n = u["name"]
        if n in CORE_CONST or n in core_fns: return ("core", None, None, "")
        if kind == "module" and n in module_fns: return ("native", ("MODULE.bazel", n, "global"), impl_of(module_fns[n]), "")
        if kind == "repo-file" and n in repo_fns: return ("native", ("REPO.bazel", n, "global"), impl_of(repo_fns[n]), "")
        if n in global_fns: return ("native", ("(global)", n, "global"), impl_of(global_fns[n]), "")
        if kind == "build" and n in native_mod_fns: return ("native", ("(BUILD)", n, "global"), impl_of(native_mod_fns[n]), "")
        if kind == "build" and n in NATIVE_RULES: return ("native", ("(BUILD)", n, "rule"), None, "")
        if n in OBJ_GLOBALS:
            loc, cname = builtin_loc(OBJ_GLOBALS[n])
            return ("native", ("(global)", n, "global"), f"{loc} {cname}".strip(), OBJ_NOTES.get(n, ""))
        return ("unknown", ("(global)", n, "unknown-global"), None, "")
    t, mem = u["recv_type"], u["member"]
    if t == "attrs" or (t or "").startswith("struct_of"): return ("dynamic", None, None, "")
    if t and t.startswith("sprovider:"): return ("starlark", None, None, "")
    if t == "native" and mem not in members["native"]:
        if mem in NATIVE_RULES: return ("native", ("native", mem, "rule"), None, "")
        return ("starlark", None, None, "")
    if t in ("_builtins.internal", "_builtins.toplevel", "_builtins"):
        return ("native", (t, mem, "field"), "packages/BuiltinsInternalModule.java", "")
    if t == "struct" and mem not in ("to_json", "to_proto"): return ("starlark", None, None, "")
    if t and t.startswith("provider:"): t = t[len("provider:"):]
    if t == "cc_common.internal_DO_NOT_USE": t = "cc_internal"
    if t == "ctx" and mem not in members["ctx"]:
        for alt in ("repository_ctx", "module_ctx", "subrule_ctx"):
            if mem in members[alt]: t = alt; break
    if t in WRAPPERS and u["file"].startswith("@@_builtins") is False and mem not in WRAPPERS[t]:
        return ("starlark", None, None, "")
    if t and mem in members.get(t, {}):
        m = members[t][mem]
        return ("native", (t, mem, "field" if m["structField"] else "method"), impl_of(m), "")
    if t and t in members:
        if t in ("cc_common", "py_builtins", "apple_common", "java_common"):
            return ("native", (t, mem, "method"), "", "builtins_bzl Starlark wrapper member (no same-named native method)")
        if mem in core_members: return ("core", None, None, "")
        return ("starlark", None, None, "")
    if t and t.startswith("internal:"): return ("native", (t, mem, "method"), "", "")
    cands = {c for c in by_member.get(mem, set()) if c not in CORE_TYPES and c != "struct" and c in reachable}
    if mem in core_members: return ("core", None, None, "")
    if not cands: return ("starlark", None, None, "")
    if len(cands) == 1:
        c = next(iter(cands)); m = members[c][mem]
        return ("native-inferred", (c, mem, "field" if m["structField"] else "method"), impl_of(m), "receiver inferred from member name")
    return ("ambiguous", ("?", mem, "member"), "", "receiver not resolved; candidates: " + "|".join(sorted(cands)))

reachable = set(SEED_TYPES)
for it in range(4):
    before = len(reachable)
    for p in PROJECTS:
        for u in all_uses[p]:
            c, key, _, _ = classify(u, reachable)
            if c == "native" and key[2] in ("method", "field"):
                m = members.get(key[0], {}).get(key[1])
                if m and ret_type(m): reachable.add(ret_type(m))
    if len(reachable) == before: break
for p in PROJECTS:
    for u in all_uses[p]:
        c, key, java, note = classify(u, reachable)
        cls_counts[p][c] += 1
        if c == "ambiguous": amb[(key[1], note)] += 1
        if c == "unknown": unknown_globals[(key[1], u["kind"])] += 1
        if key and c != "unknown": add(key, p, u, java, note)

# native rules from the configured-target dump
dump = {}
for p in PROJECTS:
    for l in open(f"{OUT}/{p}/dump-rules.txt"):
        m = re.match(r'^(\S+)\s+([\d,]+)\s+([\d,]+)\s*$', l)
        if m and m.group(1) != "RULE":
            dump.setdefault(m.group(1), collections.Counter())[p] += int(m.group(2).replace(",", ""))
# repo rules
repo_rules = {}
for p in PROJECTS:
    txt = open(f"{OUT}/{p}/show-repo.txt").read()
    for m in re.finditer(r'^## (@@\S*):\nload\("([^"]+)", "([^"]+)"\)', txt, re.M):
        repo_rules.setdefault((m.group(2), m.group(3)), collections.Counter())[p] += 1
exts = {}
for p in PROJECTS:
    for l in open(f"{OUT}/{p}/skyframe-deps.txt"):
        if l.startswith("SINGLE_EXTENSION_EVAL:"):
            exts.setdefault(l.split(":", 1)[1].strip(), set()).add(p)

def needed(c):
    ps = [p for p in PROJECTS if c[p]]
    return "both" if len(ps) == 2 else ps[0]

out = open(sys.argv[1], "w")
out.write("symbol\treceiver\tkind\tuses\tfirst_seen_file\tjava_impl\tneeded_by\tnotes\n")
rows = []
for (recv, sym, kind), a in agg.items():
    if kind == "rule":
        continue
    kw = ",".join(k for k, _ in a["kwargs"].most_common(8))
    notes = sorted(a["notes"])
    notes.append("in:" + "/".join(sorted(a["kinds"])))
    def _repo(f):
        if f.startswith("//"): return "main"
        if f.startswith("module:"): return "MODULE.bazel"
        return re.sub(r'^@@([^/]*)//.*', r'\1', f) or "main"
    repos = collections.Counter(_repo(f) for f, _, _ in a["sites"])
    notes.append("repos:" + ",".join(f"{(r or 'main').split('+')[0] if r != '_builtins' else '_builtins'}" for r, _ in repos.most_common(6)))
    if kw: notes.append("kwargs:" + kw)
    total = len(a["sites"])
    rows.append((sym, recv, kind, total, f"{a['first'][0]}:{a['first'][1]}", a["java"] or "", needed(a["uses"]), "; ".join(notes)))
# native rules: union static calls + configured counts
# Only names called as native rules; a dump-only name (e.g. cc_toolchain_alias) is a same-named Starlark rule.
rule_names = {k[1] for k in agg if k[2] == "rule"}
for r in sorted(rule_names):
    st = collections.Counter()
    first = None
    for key in (("(BUILD)", r, "rule"), ("native", r, "rule")):
        if key in agg:
            st.update(agg[key]["uses"])
            f = agg[key]["first"]
            if first is None or f < first: first = f
    conf = dump.get(r, collections.Counter())
    tot = st + conf
    nst = len(set().union(*[agg[k]["sites"] for k in (("(BUILD)", r, "rule"), ("native", r, "rule")) if k in agg]))
    rows.append((r, "(BUILD)/native", "rule", nst, f"{first[0]}:{first[1]}" if first else "",
                 find_rule_def(r), needed(tot),
                 f"static calls brotli={st['brotli']} re2={st['re2']}; configured targets brotli={conf['brotli']} re2={conf['re2']}" + ("; alias targets are counted under the actual rule in dump --rules" if r == "alias" else "")))
for (bzl, rr), c in sorted(repo_rules.items()):
    rows.append((rr, bzl, "repo_rule", sum(c.values()), bzl, "Starlark (repository_ctx is native: bazel/repository/starlark/StarlarkRepositoryContext.java)", needed(c),
                 f"repos fetched brotli={c['brotli']} re2={c['re2']}"))
for e, ps in sorted(exts.items()):
    c = collections.Counter({p: 1 for p in ps})
    rows.append((e.split("%")[-1], e.split("%")[0], "extension", len(ps), e.split("%")[0],
                 "Starlark (module_ctx is native: bazel/bzlmod/ModuleExtensionContext.java)", needed(c), ""))
order = {"global": 0, "rule": 1, "method": 2, "field": 3, "repo_rule": 4, "extension": 5}
rows.sort(key=lambda r: (order.get(r[2], 9), r[1], -r[3], r[0]))
for r in rows: out.write("\t".join(str(x) for x in r) + "\n")
out.close()

stats = {"classes": {p: dict(c) for p, c in cls_counts.items()},
         "ambiguous_top": [(f"{k[0]} [{k[1]}]", v) for k, v in amb.most_common(60)],
         "unknown_globals": [(f"{k[0]} ({k[1]})", v) for k, v in unknown_globals.most_common(60)],
         "rows": len(rows), "reachable_types": sorted(reachable),
         "dump_rules": {k: dict(v) for k, v in dump.items()},
         }
json.dump(stats, open(sys.argv[2] if len(sys.argv) > 2 else f"{OUT}/stats.json", "w"), indent=1)
print(json.dumps(stats["classes"]), len(rows))
