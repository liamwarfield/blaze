#!/usr/bin/env python3
"""Statically extract native Starlark API uses from the files real Bazel loaded.

Usage:
  extract.py files <project>          list loaded files (from the oracle skyframe dump) -> out/<p>/files.tsv
  extract.py uses <project>           parse every loaded file, write out/<p>/uses.jsonl
  extract.py help

Inputs come from scripts/inventory/oracle-run (~/.cache/blaze/work/inventory/out/<project>/):
skyframe-deps.txt, output_base.txt, MODULE.bazel.lock. Java index: out/java-index.json
(scripts/inventory/java_index.py). Files are parsed with Python's ast (Starlark is a syntactic
subset); receivers of attribute accesses are typed with a small flow-insensitive inference
(ctx.* return types from the Java signatures, local assignments, load() aliases, parameter names).
"""
import ast, json, os, re, sys, collections

HOME = os.path.expanduser("~")
WORK = f"{HOME}/.cache/blaze/work/inventory"
OUT = f"{WORK}/out"
SRC = f"{HOME}/.cache/blaze/src/bazel-9.3.2"
BUILTINS = f"{SRC}/src/main/starlark/builtins_bzl"
REPO_CACHE = f"{HOME}/.cache/blaze/repo-cache/content_addressable/sha256"
EMBEDDED = f"{HOME}/.cache/blaze/oracle-out/ur/install/58cf1ecbff441bac762b0050d6da21e9/embedded_tools"

def label_parts(lbl):
    if lbl.startswith("//"): lbl = "@@" + lbl
    elif not lbl.startswith("@"): lbl = "@@//" + lbl
    m = re.match(r'@@([^/]*)//([^:]*)(?::(.*))?$', lbl)
    return m.group(1), m.group(2), m.group(3)

def resolve_path(repo, pkg, name, ob, ws):
    if repo == "_builtins": base = BUILTINS
    elif repo == "": base = ws
    else: base = f"{ob}/external/{repo}"
    return os.path.join(base, pkg, name) if name else os.path.join(base, pkg)

def cmd_files(p):
    d = f"{OUT}/{p}"
    ob = open(f"{d}/output_base.txt").read().strip()
    ws = f"{WORK}/{p}"
    rows = []
    keys = [l.rstrip("\n") for l in open(f"{d}/skyframe-deps.txt") if re.match(r'^[A-Z_]+:', l)]
    for k in keys:
        if k.startswith("BZL_LOAD:"):
            m = re.match(r'BZL_LOAD:(\w+)\{label=([^,]+), isBuildPrelude=(\w+)\}', k)
            kind = {"KeyForBuild": "bzl", "KeyForBzlmod": "bzl-bzlmod", "KeyForBuiltins": "bzl-builtins",
                    "KeyForBzlmodBootstrap": "bzl-bootstrap"}.get(m.group(1), m.group(1))
            if m.group(3) == "true": kind = "prelude"
            repo, pkg, name = label_parts(m.group(2))
            rows.append((m.group(2), kind, resolve_path(repo, pkg, name, ob, ws)))
        elif k.startswith("PACKAGE:"):
            lbl = k[len("PACKAGE:"):]
            repo, pkg, _ = label_parts(lbl)
            dd = resolve_path(repo, pkg, None, ob, ws)
            path = next((os.path.join(dd, b) for b in ("BUILD.bazel", "BUILD") if os.path.isfile(os.path.join(dd, b))), dd + "/BUILD?")
            rows.append((f"@@{repo}//{pkg}:{os.path.basename(path)}" if repo else f"//{pkg}:{os.path.basename(path)}", "build", path))
        elif k.startswith("REPOSITORY_DIRECTORY:"):
            repo = k[len("REPOSITORY_DIRECTORY:@@"):]
            for f in ("REPO.bazel",):
                pth = f"{ob}/external/{repo}/{f}"
                if os.path.isfile(pth): rows.append((f"@@{repo}//:{f}", "repo-file", pth))
        elif k.startswith("MODULE_FILE:"):
            m = re.search(r'moduleKey=([^\]]+)\]', k)
            mk = m.group(1)
            rows.append((f"module:{mk}", "module", module_file_path(mk, d, ws, ob)))
    if os.path.isfile(f"{ws}/REPO.bazel"): rows.append(("@@//:REPO.bazel", "repo-file", f"{ws}/REPO.bazel"))
    # The build prelude is only probed; it does not exist in any repo here.
    rows = sorted(r for r in set(rows) if not (r[1] == "prelude" and not os.path.isfile(r[2])))
    with open(f"{d}/files.tsv", "w") as f:
        for r in rows:
            n = sum(1 for _ in open(r[2], errors="replace")) if os.path.isfile(r[2]) else -1
            f.write("\t".join(r + (str(n),)) + "\n")
    c = collections.Counter(r[1] for r in rows)
    missing = [r for r in rows if not os.path.isfile(r[2])]
    print(p, dict(c), "missing:", len(missing))
    for r in missing[:20]: print("  missing", r)

_lock_cache = {}
def module_file_path(mk, d, ws, ob):
    name, _, ver = mk.partition("@")
    if name == "<root>" or mk == "<root>": return f"{ws}/MODULE.bazel"
    if name == "bazel_tools": return f"{EMBEDDED}/MODULE.bazel"
    if d not in _lock_cache:
        _lock_cache[d] = json.load(open(f"{d}/MODULE.bazel.lock")).get("registryFileHashes", {})
    hashes = _lock_cache[d]
    for url, h in hashes.items():
        if url.endswith(f"/modules/{name}/{ver}/MODULE.bazel") and h:
            pth = f"{REPO_CACHE}/{h}/file"
            if os.path.isfile(pth): return pth
    return f"?module:{mk}"

# ---------------------------------------------------------------------------------------------
# Type inference

ATTR_STRUCT_MEMBERS = {  # (receiver type, member) -> resulting synthetic type
    ("ctx", "attr"): "attrs", ("ctx", "split_attr"): "attrs", ("ctx", "file"): "struct_of:File",
    ("ctx", "executable"): "struct_of:File", ("ctx", "outputs"): "struct_of:File",
    ("ctx", "files"): "struct_of:list:File", ("repository_ctx", "attr"): "attrs",
    ("ctx", "var"): "dict", ("ctx", "toolchains"): "ToolchainContext",
    ("ctx", "exec_groups"): "ExecGroupCollection", ("subrule_ctx", "toolchains"): "ToolchainContext",
}
NATIVE_PROVIDERS = {"DefaultInfo": "DefaultInfo", "OutputGroupInfo": "OutputGroupInfo",
                    "RunEnvironmentInfo": "RunEnvironmentInfo", "InstrumentedFilesInfo": "InstrumentedFilesInfo",
                    "AnalysisTestResultInfo": "AnalysisTestResultInfo", "PackageSpecificationInfo": "PackageSpecificationInfo",
                    "MaterializedDepsInfo": "MaterializedDepsInfo"}
GLOBAL_OBJECTS = {"attr": "attr", "native": "native", "config": "config", "config_common": "config_common",
                  "platform_common": "platform_common", "testing": "testing", "coverage_common": "coverage_common",
                  "json": "json", "proto": "proto", "cc_common": "cc_common", "apple_common": "apple_common",
                  "java_common": "java_common", "py_internal": "py_builtins", "_builtins": "_builtins",
                  "proto_common_do_not_use": "proto_common", "cc_internal": "cc_internal"}
BUILTINS_INTERNAL = {"cc_common": "cc_common", "cc_internal": "cc_internal", "apple_common": "apple_common",
                     "java_common_internal_do_not_use": "java_common", "py_builtins": "py_builtins"}
NAME_HEUR = [
    (r'^(rctx|repository_ctx|repo_ctx|repository_context)$', "repository_ctx"),
    (r'^(mctx|module_ctx|module_context)$', "module_ctx"),
    (r'^actions$', "actions"),
    (r'^(args|.*_args)$', "Args"),
    (r'^feature_configuration$', "FeatureConfiguration"),
    (r'^(label|.*_label|lbl)$', "Label"),
    (r'^(runfiles|.*_runfiles)$', "runfiles"),
    (r'^(f|file|src|hdr|out|output|.*_file|artifact|template|header|source)$', "File"),
    (r'^(target|dep|.*_target|tgt|d)$', "Target"),
    (r'^(depset|.*_depset)$', "depset"),
]

def name_heuristic(n):
    for rx, t in NAME_HEUR:
        if re.match(rx, n): return t
    return None

class Index:
    def __init__(self):
        d = json.load(open(f"{OUT}/java-index.json"))
        self.members = collections.defaultdict(dict)
        for m in d["methods"]:
            b = m["builtin"] or m["class"]
            if m["class"] == "StarlarkBaseExternalContext":
                for t in ("repository_ctx", "module_ctx"):
                    self.members[t].setdefault(m["name"], m)
            elif b:
                self.members[b].setdefault(m["name"], m)
        self.by_member = collections.defaultdict(set)
        for t, ms in self.members.items():
            for n in ms: self.by_member[n].add(t)

CORE_TYPES = {"string", "list", "dict", "set", "tuple", "int", "float", "bool", "range", "bytes", "NoneType", "function", "builtin_function_or_method"}

def collect_bound(nodes):
    """Names bound (Store) in a list of statements, not descending into nested defs/lambdas."""
    out = set()
    class V(ast.NodeVisitor):
        def visit_FunctionDef(self, n): out.add(n.name)
        def visit_Lambda(self, n): pass
        def visit_Name(self, n):
            if isinstance(n.ctx, (ast.Store, ast.Del)): out.add(n.id)
    for s in nodes: V().visit(s)
    return out

def load_bindings(tree):
    res = {}
    for s in tree.body:
        if isinstance(s, ast.Expr) and isinstance(s.value, ast.Call) and isinstance(s.value.func, ast.Name) and s.value.func.id == "load":
            c = s.value
            if not c.args or not isinstance(c.args[0], ast.Constant): continue
            mod = c.args[0].value
            for a in c.args[1:]:
                if isinstance(a, ast.Constant): res[a.value] = (mod, a.value)
            for k in c.keywords:
                if isinstance(k.value, ast.Constant): res[k.arg] = (mod, k.value.value)
    return res

def dotted(n):
    if isinstance(n, ast.Name): return n.id
    if isinstance(n, ast.Attribute):
        b = dotted(n.value)
        return b + "." + n.attr if b else None
    return None

class FileExtractor:
    def __init__(self, idx, label, kind, path, export_types):
        self.idx, self.label, self.kind, self.path = idx, label, kind, path
        self.export_types = export_types  # resolved load bindings: name -> type
        self.uses = []

    def run(self):
        src = open(self.path, encoding="utf-8", errors="replace").read()
        tree = ast.parse(src, self.path)
        self.tree = tree
        self.loads = load_bindings(tree)
        self.module_bound = collect_bound(tree.body) | set(self.loads)
        # roles of functions
        self.roles = {}
        file_has = collections.Counter()
        for n in ast.walk(tree):
            if isinstance(n, ast.Call) and isinstance(n.func, ast.Name):
                fn = n.func.id
                if fn in ("rule", "aspect", "repository_rule", "module_extension", "subrule", "transition", "macro", "materializer_rule"):
                    file_has[fn] += 1
                    impl = next((k.value for k in n.keywords if k.arg == "implementation"), n.args[0] if n.args else None)
                    if isinstance(impl, ast.Name):
                        self.roles[impl.id] = {"rule": "ctx", "aspect": "ctx", "repository_rule": "repository_ctx",
                                               "module_extension": "module_ctx", "subrule": "subrule_ctx",
                                               "materializer_rule": "ctx"}.get(fn)
        if file_has["repository_rule"] and not (file_has["rule"] or file_has["aspect"]): self.default_ctx = "repository_ctx"
        elif file_has["module_extension"] and not (file_has["rule"] or file_has["aspect"]): self.default_ctx = "module_ctx"
        elif self.kind == "bzl-bzlmod" and not (file_has["rule"] or file_has["aspect"]): self.default_ctx = "repository_ctx"
        else: self.default_ctx = "ctx"
        # module-level types
        self.mod_types = dict(self.export_types)
        for _ in range(2):
            for s in tree.body:
                if isinstance(s, ast.Assign) and len(s.targets) == 1 and isinstance(s.targets[0], ast.Name):
                    t = self.type_of(s.value, [self.mod_types])
                    if t: self.mod_types[s.targets[0].id] = t
        self.walk_scope(tree.body, [self.mod_types], [self.module_bound], None)
        return self.uses

    # --- scopes
    def walk_scope(self, body, types_stack, bound_stack, fname):
        for s in body: self.visit(s, types_stack, bound_stack)

    def func_scope(self, node, types_stack, bound_stack):
        a = node.args
        params = [x.arg for x in a.posonlyargs + a.args + a.kwonlyargs]
        extra = [x.arg for x in (a.vararg, a.kwarg) if x]
        body = node.body if isinstance(node.body, list) else [node.body]
        bound = set(params) | set(extra) | collect_bound(body)
        for c in ast.walk(node):
            if isinstance(c, (ast.ListComp, ast.DictComp, ast.SetComp, ast.GeneratorExp)):
                for g in c.generators: bound |= collect_bound([ast.Expr(g.target)]) if False else {x.id for x in ast.walk(g.target) if isinstance(x, ast.Name)}
        types = {}
        role = self.roles.get(getattr(node, "name", None))
        for p in params:
            if p == "ctx": types[p] = role or self.default_ctx
            else:
                t = name_heuristic(p)
                if t: types[p] = t
        ts = types_stack + [types]
        bs = bound_stack + [bound]
        # local assignment inference (two passes)
        for _ in range(2):
            for n in ast.walk(node) if isinstance(node.body, list) else []:
                if isinstance(n, ast.Assign) and len(n.targets) == 1 and isinstance(n.targets[0], ast.Name):
                    t = self.type_of(n.value, ts)
                    if t and n.targets[0].id not in [x.arg for x in a.args]: types[n.targets[0].id] = t
                elif isinstance(n, (ast.For, ast.comprehension)):
                    it = self.type_of(n.iter, ts)
                    if isinstance(n.target, ast.Name):
                        if it and it.startswith("list:"): types[n.target.id] = it[5:]
                        elif it == "depset": types.setdefault(n.target.id, name_heuristic(n.target.id) or "?")
                        elif name_heuristic(n.target.id): types.setdefault(n.target.id, name_heuristic(n.target.id))
        for k in list(types):
            if types[k] == "?": del types[k]
        if isinstance(node.body, list):
            for s in node.body: self.visit(s, ts, bs)
        else:
            self.visit(node.body, ts, bs)
        for d in node.args.defaults + node.args.kw_defaults:
            if d is not None: self.visit(d, types_stack, bound_stack)

    def lookup(self, name, types_stack):
        for t in reversed(types_stack):
            if name in t: return t[name]
        return None

    def is_bound(self, name, bound_stack):
        return any(name in b for b in bound_stack)

    def type_of(self, e, ts):
        if isinstance(e, ast.Name):
            t = self.lookup(e.id, ts)
            if t: return t
            if e.id in GLOBAL_OBJECTS and e.id not in self.module_bound: return GLOBAL_OBJECTS[e.id]
            if e.id in NATIVE_PROVIDERS and e.id not in self.module_bound: return "provider:" + e.id
            return None
        if isinstance(e, ast.Attribute):
            tv = self.type_of(e.value, ts)
            if not tv: return None
            if (tv, e.attr) in ATTR_STRUCT_MEMBERS: return ATTR_STRUCT_MEMBERS[(tv, e.attr)]
            if tv.startswith("struct_of:"): return tv[len("struct_of:"):]
            if tv == "_builtins":
                return {"internal": "_builtins.internal", "toplevel": "_builtins.toplevel", "native": "native"}.get(e.attr)
            if tv == "_builtins.internal": return BUILTINS_INTERNAL.get(e.attr, "internal:" + e.attr)
            if tv == "_builtins.toplevel":
                return GLOBAL_OBJECTS.get(e.attr) or ("provider:" + e.attr if e.attr in NATIVE_PROVIDERS else None)
            if tv == "ToolchainContext": return None
            if tv == "fragments": return e.attr  # ctx.fragments.cpp -> fragment builtin "cpp"
            m = self.idx.members.get(tv, {}).get(e.attr)
            if m and m.get("structField"): return m.get("ret_builtin") or (m.get("ret") if m.get("ret") in self.idx.members else None)
            if tv == "cc_common" and e.attr == "internal_DO_NOT_USE": return "cc_common.internal_DO_NOT_USE"
            return None
        if isinstance(e, ast.Call):
            f = e.func
            if isinstance(f, ast.Name) and not self.lookup(f.id, ts) and f.id not in self.module_bound:
                return {"depset": "depset", "struct": "struct", "Label": "Label", "select": "select"}.get(f.id)
            if isinstance(f, ast.Attribute):
                tv = self.type_of(f.value, ts)
                if tv == "cc_common.internal_DO_NOT_USE" or (tv == "cc_common" and f.attr == "internal_DO_NOT_USE"): return "cc_internal"
                if tv:
                    m = self.idx.members.get(tv, {}).get(f.attr)
                    if m and not m.get("structField"):
                        if f.attr == "to_list": return "list:?"
                        return m.get("ret_builtin") or (m.get("ret") if m.get("ret") in self.idx.members else None)
            return None
        if isinstance(e, ast.Subscript):
            tv = self.type_of(e.value, ts)
            sl = e.slice
            if isinstance(sl, ast.Name):
                pt = self.type_of(sl, ts)
                if pt and pt.startswith("provider:"): return pt[len("provider:"):]
                if sl.id.endswith("Info") and tv in (None, "Target"): return "sprovider:" + sl.id
            if isinstance(sl, ast.Attribute) and isinstance(sl.attr, str) and sl.attr.endswith("Info"):
                pt = self.type_of(sl, ts)
                if pt and pt.startswith("provider:"): return pt[len("provider:"):]
                return "sprovider:" + sl.attr
            if tv and tv.startswith("list:"): return tv[5:]
            return None
        if isinstance(e, ast.IfExp):
            return self.type_of(e.body, ts) or self.type_of(e.orelse, ts)
        if isinstance(e, ast.BinOp) and isinstance(e.op, ast.Add):
            return self.type_of(e.left, ts)
        return None

    def rec(self, cat, node, **kw):
        kw.update(cat=cat, line=getattr(node, "lineno", 0))
        self.uses.append(kw)

    def visit(self, n, ts, bs):
        if isinstance(n, (ast.FunctionDef, ast.Lambda)):
            for d in getattr(n, "decorator_list", []): self.visit(d, ts, bs)
            self.func_scope(n, ts, bs)
            return
        if isinstance(n, ast.Expr) and isinstance(n.value, ast.Call) and isinstance(n.value.func, ast.Name) and n.value.func.id == "load" and n in self.tree.body:
            return
        if isinstance(n, ast.Call):
            kws = [k.arg for k in n.keywords if k.arg]
            f = n.func
            if isinstance(f, ast.Name) and not self.is_bound(f.id, bs):
                self.rec("global", f, name=f.id, called=True, kwargs=kws)
                for a in n.args: self.visit(a, ts, bs)
                for k in n.keywords: self.visit(k.value, ts, bs)
                return
            if isinstance(f, ast.Attribute):
                self.member(f, ts, bs, called=True, kwargs=kws)
                for a in n.args: self.visit(a, ts, bs)
                for k in n.keywords: self.visit(k.value, ts, bs)
                return
        if isinstance(n, ast.Name) and isinstance(n.ctx, ast.Load) and not self.is_bound(n.id, bs):
            self.rec("global", n, name=n.id, called=False, kwargs=[])
            return
        if isinstance(n, ast.Attribute) and isinstance(n.ctx, ast.Load):
            self.member(n, ts, bs, called=False, kwargs=[])
            return
        if isinstance(n, (ast.ListComp, ast.SetComp, ast.GeneratorExp, ast.DictComp)):
            types = {}
            for g in n.generators:
                it = self.type_of(g.iter, ts + [types])
                if isinstance(g.target, ast.Name):
                    if it and it.startswith("list:") and it != "list:?": types[g.target.id] = it[5:]
                    elif name_heuristic(g.target.id): types[g.target.id] = name_heuristic(g.target.id)
            bound = {x.id for g in n.generators for x in ast.walk(g.target) if isinstance(x, ast.Name)}
            for c in ast.iter_child_nodes(n): self.visit(c, ts + [types], bs + [bound])
            return
        for c in ast.iter_child_nodes(n): self.visit(c, ts, bs)

    def member(self, f, ts, bs, called, kwargs):
        tv = self.type_of(f.value, ts)
        recv = dotted(f.value) or type(f.value).__name__
        self.rec("member", f, member=f.attr, recv_type=tv, recv=recv[:60], called=called, kwargs=kwargs)
        self.visit(f.value, ts, bs)

def resolve_load(mod, cur_label):
    """Return canonical label of a load() module string relative to cur_label (best-effort)."""
    repo, pkg, _ = label_parts(cur_label) if cur_label.startswith("@@") else ("", "", None)
    if mod.startswith(":"): return f"@@{repo}//{pkg}:{mod[1:]}"
    if mod.startswith("//"): return f"@@{repo}{mod}"
    return mod  # apparent repo name; not resolved

def cmd_uses(p):
    idx = Index()
    d = f"{OUT}/{p}"
    rows = [l.rstrip("\n").split("\t") for l in open(f"{d}/files.tsv")]
    exports = {}  # label -> {name: type}
    out = open(f"{d}/uses.jsonl", "w")
    errs = 0
    # Two passes so load()ed aliases (e.g. cc_internal) get their types.
    for pas in (0, 1):
        for label, kind, path, n in rows:
            if not os.path.isfile(path): continue
            et = {}
            if pas == 1:
                try:
                    tree = ast.parse(open(path, errors="replace").read())
                except SyntaxError:
                    tree = None
                if tree:
                    for local, (mod, sym) in load_bindings(tree).items():
                        lab = resolve_load(mod, label)
                        cand = exports.get(lab)
                        if cand is None:
                            # apparent repo: match by package:file suffix
                            suf = mod.split("//", 1)[-1]
                            cand = next((v for k, v in exports.items() if k.endswith("//" + suf)), None)
                        if cand and sym in cand: et[local] = cand[sym]
            try:
                fx = FileExtractor(idx, label, kind, path, et)
                uses = fx.run()
            except SyntaxError as e:
                if pas == 1: print("syntax error", path, e); errs += 1
                continue
            exports[label] = {k: v for k, v in fx.mod_types.items() if v}
            if pas == 1:
                for u in uses:
                    u.update(file=label, kind=kind)
                    out.write(json.dumps(u) + "\n")
    print(p, "parse errors:", errs)

if __name__ == "__main__":
    if len(sys.argv) < 3 or sys.argv[1] in ("help", "-h", "--help"):
        print(__doc__); sys.exit(0)
    {"files": cmd_files, "uses": cmd_uses}[sys.argv[1]](sys.argv[2])
