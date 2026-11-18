#!/usr/bin/env python3
"""Index every @StarlarkMethod / @StarlarkBuiltin in the pinned Bazel Java source.

Usage: java_index.py <bazel-src-root> <out.json>      java_index.py help

For each annotated method records: Starlark name, structField/selfCall flags, the declaring
Java class, its @StarlarkBuiltin name, file:line of the annotation, the Java method name, and
the file:line of concrete implementations in classes that implement/extend the declaring type.
"""
import json, os, re, sys

if len(sys.argv) < 3 or sys.argv[1] in ("help", "-h", "--help"):
    print(__doc__); sys.exit(0)
root, out = sys.argv[1], sys.argv[2]
jroot = os.path.join(root, "src/main/java")

CLASS_RE = re.compile(r'\b(class|interface|enum|record)\s+([A-Z]\w*)(\s*<[^{]*?>)?([^{;]*)\{', re.S)

TOK_RE = re.compile(r'//[^\n]*|/\*.*?\*/|"""(?:.|\n)*?"""|"(?:\\.|[^"\\\n])*"|\'(?:\\.|[^\'\\\n])*\'', re.S)
def strip_comments_strings(src):
    # Replace comments and string contents with spaces, keeping offsets and newlines.
    return TOK_RE.sub(lambda m: re.sub(r'[^\n]', ' ', m.group(0)), src)

def matching_paren(s, i):
    d = 0
    for j in range(i, len(s)):
        if s[j] == "(": d += 1
        elif s[j] == ")":
            d -= 1
            if d == 0: return j
    return len(s) - 1

METH_RE = re.compile(r'[\w>\]]\s+(\w+)\s*\([^;{)(]*(?:\([^;{)(]*\)[^;{)(]*)*\)\s*(?:throws\s+[\w.,\s]+)?\{')
KW = {"if", "for", "while", "switch", "catch", "synchronized", "return", "new", "else"}
classes = {}   # simple name -> list of {file, line, supers, builtin}
methods = []   # annotated methods
impl_defs = {} # (class, javaMethod) -> file:line

for dp, dn, fn in os.walk(jroot):
    for f in fn:
        if not f.endswith(".java"): continue
        p = os.path.join(dp, f); rel = os.path.relpath(p, root)
        src = open(p, encoding="utf-8", errors="replace").read()
        clean = strip_comments_strings(src)
        lineof = lambda off: src.count("\n", 0, off) + 1
        # class spans via brace depth
        decls = []
        for m in CLASS_RE.finditer(clean):
            start = m.end() - 1; d = 0; end = len(clean)
            for j in range(start, len(clean)):
                if clean[j] == "{": d += 1
                elif clean[j] == "}":
                    d -= 1
                    if d == 0: end = j; break
            hdr = m.group(4)
            supers = re.findall(r'\b([A-Z]\w*)\b', re.sub(r'<[^<>]*>', '', re.sub(r'<[^<>]*>', '', hdr)))
            decls.append((m.start(), start, end, m.group(2), supers))
        def enclosing(off):
            best = None
            for d_ in decls:
                if d_[1] < off < d_[2] and (best is None or d_[1] > best[1]): best = d_
            return best
        # StarlarkBuiltin on classes
        for m in re.finditer(r'@StarlarkBuiltin\s*\(', src):
            pe = matching_paren(clean, m.end() - 1)
            ann = src[m.start():pe + 1]
            nm = re.search(r'name\s*=\s*"([^"]*)"', ann)
            nxt = CLASS_RE.search(clean, pe)
            if nxt and nm:
                classes.setdefault(nxt.group(2), []).append({"file": rel, "line": lineof(m.start()), "builtin": nm.group(1)})
        for (s, b, e, name, sup) in decls:
            ent = {"file": rel, "line": lineof(s), "supers": sup}
            lst = classes.setdefault(name, [])
            merged = False
            for x in lst:
                if x.get("file") == rel and "supers" not in x:
                    x.update(ent); merged = True
            if not merged and not any(x.get("file") == rel and x.get("line") == ent["line"] for x in lst):
                lst.append(ent)
        # method definitions (for impl lookup)
        if not decls: continue
        for m in METH_RE.finditer(clean):
            if m.group(1) in KW: continue
            enc = enclosing(m.start(1))
            if enc: impl_defs.setdefault((enc[3], m.group(1)), f"{rel}:{lineof(m.start(1))}")
        for m in re.finditer(r'@StarlarkMethod\s*\(', src):
            pe = matching_paren(clean, m.end() - 1)
            ann = src[m.start():pe + 1]
            nm = re.search(r'\bname\s*=\s*"([^"]*)"', ann)
            if not nm: continue
            tail = clean[pe + 1:pe + 600]
            sig = re.search(r'(\w+)\s*\(', tail)
            pre = tail[:sig.start()] if sig else ""
            pre = re.sub(r'@\w+(\([^)]*\))?', ' ', pre)
            pre = re.sub(r'\b(public|protected|private|static|final|abstract|default|synchronized)\b', ' ', pre)
            pre = re.sub(r'^\s*<[^>]*(<[^>]*>[^>]*)*>', ' ', pre.strip())
            rt = re.match(r'\s*([\w.]+)', pre)
            ret = rt.group(1).split(".")[-1] if rt else None
            enc = enclosing(m.start())
            methods.append({
                "name": nm.group(1),
                "structField": bool(re.search(r'structField\s*=\s*true', ann)),
                "selfCall": bool(re.search(r'selfCall\s*=\s*true', ann)),
                "params": re.findall(r'@Param\(\s*name\s*=\s*"([^"]*)"', ann),
                "class": enc[3] if enc else None,
                "file": rel, "line": lineof(m.start()),
                "java_method": sig.group(1) if sig else None,
                "ret": ret,
            })

# subclass map
subs = {}
for name, lst in classes.items():
    for x in lst:
        for s in x.get("supers", []):
            subs.setdefault(s, set()).add(name)
def all_subs(c, seen=None):
    seen = seen or set()
    for s in subs.get(c, ()):
        if s not in seen:
            seen.add(s); all_subs(s, seen)
    return seen
def builtin_name(c, seen=None):
    seen = seen or set()
    for x in classes.get(c, []):
        if "builtin" in x: return x["builtin"]
    for x in classes.get(c, []):
        for s in x.get("supers", []):
            if s not in seen:
                seen.add(s); b = builtin_name(s, seen)
                if b: return b
    return None
for m in methods:
    c = m["class"]
    m["builtin"] = builtin_name(c) if c else None
    impls = []
    for s in sorted(all_subs(c)) if c else []:
        d = impl_defs.get((s, m["java_method"]))
        if d: impls.append(f"{s}:{d}")
    m["impls"] = impls
    m["ret_builtin"] = builtin_name(m["ret"]) if m.get("ret") in classes else None
    m["subclasses"] = sorted(all_subs(c))[:50] if c else []
json.dump({"methods": methods, "classes": {k: v for k, v in classes.items()}}, open(out, "w"), indent=1, default=list)
print(f"{len(methods)} @StarlarkMethod, {sum(1 for v in classes.values() for x in v if 'builtin' in x)} @StarlarkBuiltin")
