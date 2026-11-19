#!/usr/bin/python3 -I
"""Shared guards of the oracle-against-Blaze comparison tools (audit round 3, batch B).

Standard library only. The tools under scripts/{run,cmds,textproto,starlark,proxy,query,config,
macros,tests,java,exec2,repo-gaps,mod,bytes}/ import it with

    sys.path.insert(0, os.path.join(<repo>, "scripts", "lib"))
    import compare_common as cc

What lives here (and nowhere else):

  run()            runs one binary in a memory-capped systemd unit (`--scope`, so a server the
                   binary daemonizes keeps living between commands, or a `--wait` service, where
                   the unit ends with the binary) with a clean environment (env -i HOME
                   PATH=/usr/bin:/bin plus what the tool names), under a supervisor that records
                   inside the unit how the command really ended: `exit N`, `signal NAME`, or
                   `oom-kill` (the unit's cgroup counted an OOM kill, read from memory.events),
                   or `timeout` (the tool's limit; the whole unit is SIGKILLed). Without a
                   record (the unit died before the supervisor wrote one) the result is
                   `unrecorded`, never a normal exit.
  require_oracle() refuses an oracle whose sha256 is not the pinned one (UPSTREAM.json).
  cache keys       an oracle output reused from an earlier run is accepted only when its key
                   (oracle sha256, tool sha256, exact argv, the environment the tool sets, the
                   digest of the input workspace, the case sequence) equals the key the current
                   run would use.
  judge()          the result classes: IDENTICAL, DIFFERENT, BOTH-FAILED, NOT-PORTED, SKIPPED,
                   DECLARED-GAP, ORACLE-UNEXPECTED.
  Report           header (tool, date, git HEAD and dirty flag, Blaze path and sha256, oracle
                   path and sha256), one row per selected case, footer with the denominator, the
                   exit-code rule: 0 only when at least one case was compared and every selected
                   case is IDENTICAL or DECLARED-GAP; 1 otherwise; 2 for a refusal or usage error.
  selftest()       fake "oracle" and "blaze" scripts that prove the guards: equal output,
                   different output, equal failures, death by SIGPIPE and SIGTERM, an OOM kill,
                   an empty selection, a wrong-oracle hash, a stale cache entry.

Command line (for shell tools and for the unit itself):

  compare_common.py run [--mode scope|service] [--cwd DIR] [--timeout S] [--env K=V]...
                        [--stdout FILE] [--stderr FILE] [--termination FILE] -- ARGV...
      runs ARGV as run() does; prints the termination to stderr (`termination: signal SIGPIPE`),
      writes it (one line) to --termination, exits 0 after `exit 0`, the exit code after another
      exit, 128+N after a signal, 137 after an OOM kill or timeout, 125 when unrecorded.
  compare_common.py check-oracle [PATH]   exit 0 when PATH (default: the pinned path) is the pin
  compare_common.py supervise STATUS ARGV...   (internal: the unit's main process)
  compare_common.py selftest              the module's own guards
  compare_common.py help
"""

import datetime
import hashlib
import json
import os
import shlex
import signal
import subprocess
import sys
import tempfile
import time
import uuid

HOME = os.path.expanduser("~")
LIB = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(LIB))
CACHE = os.path.join(HOME, ".cache", "blaze")
STATUS_DIR = os.path.join(CACHE, "work", "compare-common", "status")
THIS = os.path.abspath(__file__)

# ---------------------------------------------------------------------------------------------
# The pin.


class Refusal(Exception):
    """A guard refused the run (wrong oracle, stale cache entry, ...). Tools exit 2."""


def _load_pin():
    with open(os.path.join(REPO, "UPSTREAM.json")) as f:
        up = json.load(f)
    pin = up["pins"][up["current"]]
    return os.path.expanduser(pin["oracle_binary"]), pin["oracle_sha256"]


PINNED_ORACLE, PINNED_SHA256 = _load_pin()
# Replaced only by selftest(), in-process, to let a fake oracle through; never from outside.
_pin_override = None


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


_sha_memo = {}


def _sha_cached(path):
    st = os.stat(path)
    k = (os.path.realpath(path), st.st_size, st.st_mtime_ns)
    if k not in _sha_memo:
        _sha_memo[k] = sha256_file(path)
    return _sha_memo[k]


def pinned_sha256():
    return _pin_override or PINNED_SHA256


def require_oracle(path=None):
    """(absolute path, sha256) of the oracle; Refusal unless its sha256 is the pinned one."""
    path = os.path.abspath(os.path.expanduser(path or PINNED_ORACLE))
    if not os.path.isfile(path):
        raise Refusal("oracle %s does not exist" % path)
    sha = _sha_cached(path)
    if sha != pinned_sha256():
        raise Refusal("oracle %s has sha256 %s, not the pinned %s (UPSTREAM.json); refusing to "
                      "use it as the reference" % (path, sha, pinned_sha256()))
    return path, sha


def blaze_identity(path):
    """{"path", "realpath", "sha256"} of the binary under test; Refusal if it is missing or is
    the oracle itself (the oracle on both sides proves nothing)."""
    path = os.path.abspath(os.path.expanduser(path))
    if not os.path.isfile(path):
        raise Refusal("binary under test %s does not exist" % path)
    sha = _sha_cached(path)
    if sha == pinned_sha256():
        raise Refusal("the binary under test %s is the oracle (sha256 %s)" % (path, sha))
    return {"path": path, "realpath": os.path.realpath(path), "sha256": sha}


def repo_state():
    def git(*args):
        p = subprocess.run(["git", "--no-optional-locks", "-C", REPO] + list(args),
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        return p.stdout.decode() if p.returncode == 0 else None
    head = git("rev-parse", "HEAD")
    status = git("status", "--porcelain=v1", "--untracked-files=no")
    return {"commit": head.strip() if head else None,
            "dirty": (bool(status.strip()) if status is not None else None)}


def tool_sha256(*paths):
    """sha256 over the given tool files (their normalisers decide what a stored normalised
    output is). Tools that store raw outputs pass None as the key's tool_sha instead."""
    h = hashlib.sha256()
    for p in paths:
        h.update(os.path.basename(p).encode() + b"\0" + sha256_file(p).encode() + b"\n")
    return h.hexdigest()


# ---------------------------------------------------------------------------------------------
# Running a binary.


class Termination:
    """How a command ended. kind: exit | signal | oom-kill | timeout | unrecorded | not-run."""

    def __init__(self, kind, code=None, sig=None, detail=None):
        self.kind, self.code, self.sig, self.detail = kind, code, sig, detail

    @property
    def ok(self):
        return self.kind == "exit" and self.code == 0

    @property
    def normal(self):
        return self.kind == "exit"

    def __str__(self):
        if self.kind == "exit":
            return "exit %d" % self.code
        if self.kind == "signal":
            return "signal %s" % self.sig
        if self.kind == "oom-kill":
            return "oom-kill (%s)" % self.detail if self.detail else "oom-kill"
        if self.kind == "timeout":
            return "timeout (%s)" % self.detail if self.detail else "timeout"
        return "%s (%s)" % (self.kind, self.detail) if self.detail else self.kind

    def __eq__(self, other):
        return isinstance(other, Termination) and self.key() == other.key()

    def __hash__(self):
        return hash(self.key())

    def key(self):
        # The detail of an oom-kill/timeout (what the child did when killed) is not part of it.
        return (self.kind, self.code, self.sig)

    def exit_code_like(self):
        """A shell-style number: the code, 128+N after a signal, 137 after oom/timeout, 125."""
        if self.kind == "exit":
            return self.code
        if self.kind == "signal":
            return 128 + int(getattr(signal, self.sig, 0) or 0)
        if self.kind in ("oom-kill", "timeout"):
            return 137
        return 125

    def to_dict(self):
        return {"kind": self.kind, "code": self.code, "signal": self.sig, "detail": self.detail,
                "text": str(self)}

    @staticmethod
    def from_dict(d):
        if not isinstance(d, dict) or "kind" not in d:
            return None
        return Termination(d["kind"], d.get("code"), d.get("signal"), d.get("detail"))


class RunResult:
    def __init__(self, term, out, err, duration, argv, unit):
        self.term, self.out, self.err, self.duration = term, out, err, duration
        self.argv, self.unit = argv, unit

    @property
    def rc(self):
        """The shell-style number (for tables only; compare `term`)."""
        return self.term.exit_code_like()


def clean_env(extra=None):
    env = {"HOME": HOME, "PATH": "/usr/bin:/bin"}
    env.update(extra or {})
    return env


def unit_argv(argv, status_path, unit, mode="scope", cwd=None, env=None, memory_max="12G"):
    """The systemd-run argv that runs `argv` under the supervisor with a clean environment."""
    if mode not in ("scope", "service"):
        raise ValueError(mode)
    cmd = ["systemd-run", "--user", "--collect", "--quiet", "--unit=" + unit]
    if mode == "scope":
        cmd.append("--scope")
    else:
        cmd += ["--wait", "--pipe"]
        if cwd:
            cmd.append("--working-directory=" + cwd)
    cmd += ["-p", "MemoryMax=" + memory_max, "-p", "MemorySwapMax=0", "-p", "OOMPolicy=continue",
            "--", "/usr/bin/python3", "-I", THIS, "supervise", status_path,
            "env", "-i"] + ["%s=%s" % kv for kv in clean_env(env).items()] + list(argv)
    return cmd


def _parse_status(path):
    try:
        with open(path) as f:
            words = f.read().split()
    except OSError:
        return None
    if not words:
        return None
    oom = 0
    if "oom_kill" in words:
        i = words.index("oom_kill")
        oom = int(words[i + 1])
    if words[0] == "exit":
        base = Termination("exit", int(words[1]))
    elif words[0] == "signal":
        base = Termination("signal", 128 + int(words[1]), words[2])
    else:
        return None
    if oom:
        return Termination("oom-kill", detail="%d OOM kill(s) in the unit; the command then ended "
                           "with %s" % (oom, base))
    return base


def _kill_unit(unit, mode):
    name = unit + (".scope" if mode == "scope" else ".service")
    subprocess.run(["systemctl", "--user", "kill", "--signal=SIGKILL", name],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def run(argv, cwd, env=None, timeout=None, mode="scope", memory_max="12G", stdin=None,
        stdout_path=None, stderr_path=None, tag="cmp"):
    """Runs argv in a unit (see the module doc). Returns a RunResult whose out/err are the bytes
    (also written to stdout_path/stderr_path when given)."""
    os.makedirs(STATUS_DIR, exist_ok=True)
    unit = "blaze-%s-%d-%s" % ("".join(c if c.isalnum() else "-" for c in tag)[:24], os.getpid(),
                               uuid.uuid4().hex[:8])
    status = os.path.join(STATUS_DIR, unit + ".status")
    cmd = unit_argv(argv, status, unit, mode, cwd, env, memory_max)
    t0 = time.monotonic()
    out_f = open(stdout_path, "w+b") if stdout_path else tempfile.TemporaryFile()
    err_f = open(stderr_path, "w+b") if stderr_path else tempfile.TemporaryFile()
    term = None
    try:
        p = subprocess.Popen(cmd, cwd=cwd, stdout=out_f, stderr=err_f,
                             stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL)
        try:
            p.communicate(input=stdin, timeout=timeout)
            wrapper_rc = p.returncode
        except subprocess.TimeoutExpired:
            _kill_unit(unit, mode)
            p.kill()
            p.wait()
            wrapper_rc = p.returncode
            term = Termination("timeout", detail="%ss limit; unit SIGKILLed" % timeout)
        recorded = _parse_status(status)
        if term is None:
            term = recorded or Termination(
                "unrecorded", detail="no status written inside the unit; systemd-run returned %d"
                % wrapper_rc)
        out_f.seek(0)
        out = out_f.read()
        err_f.seek(0)
        err = err_f.read()
    finally:
        out_f.close()
        err_f.close()
        if os.path.lexists(status):
            os.remove(status)
    return RunResult(term, out, err, round(time.monotonic() - t0, 1), argv, unit)


def supervise(status_file, cmd):
    """The unit's main process: runs cmd, writes `exit N` or `signal N NAME`, then
    `oom_kill K` (the unit cgroup's counter), to status_file."""
    child = []
    pending = []

    def forward(signum, _frame):
        if child:
            try:
                os.kill(child[0], signum)
            except ProcessLookupError:
                pass
        else:
            pending.append(signum)

    forwarded = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT)
    for s in forwarded:
        signal.signal(s, forward)
    pid = os.fork()
    if pid == 0:
        for s in forwarded + (signal.SIGPIPE, signal.SIGXFSZ):
            signal.signal(s, signal.SIG_DFL)
        try:
            os.execvp(cmd[0], cmd)
        except OSError as e:
            os.write(2, ("compare_common supervise: cannot run %s: %s\n" % (cmd[0], e)).encode())
            os._exit(127)
    child.append(pid)
    for s in pending:
        os.kill(pid, s)
    while True:
        try:
            _, st = os.waitpid(pid, 0)
            break
        except InterruptedError:
            continue
    if os.WIFSIGNALED(st):
        n = os.WTERMSIG(st)
        line = "signal %d %s" % (n, signal.Signals(n).name)
        code = 128 + n
    else:
        code = os.WEXITSTATUS(st)
        line = "exit %d" % code
    oom = 0
    try:
        cg = open("/proc/self/cgroup").read().strip().split("::", 1)[1]
        for ln in open("/sys/fs/cgroup" + cg + "/memory.events"):
            k, v = ln.split()
            if k == "oom_kill":
                oom = int(v)
    except (OSError, IndexError, ValueError):
        pass
    with open(status_file, "w") as f:
        f.write("%s\noom_kill %d\n" % (line, oom))
    return code


# ---------------------------------------------------------------------------------------------
# Anchored cache of oracle outputs.


def tree_digest(path, skip_top=("bazel-", ".git")):
    """sha256 over every file and symlink below path (relative path, then content or link
    target), skipping top-level entries whose name starts with one of skip_top."""
    h = hashlib.sha256()
    path = os.path.abspath(path)
    if os.path.isfile(path):
        h.update(b"F" + sha256_file(path).encode())
        return h.hexdigest()
    for dirpath, dirnames, filenames in os.walk(path):
        rel_dir = os.path.relpath(dirpath, path)
        if rel_dir == ".":
            dirnames[:] = [d for d in dirnames if not d.startswith(skip_top)]
            filenames = [f for f in filenames if not f.startswith(skip_top)]
        dirnames.sort()
        for name in sorted(filenames) + [d for d in dirnames if os.path.islink(os.path.join(dirpath, d))]:
            full = os.path.join(dirpath, name)
            rel = os.path.normpath(os.path.join(rel_dir, name))
            if os.path.islink(full):
                h.update(b"L" + rel.encode() + b"\0" + os.readlink(full).encode() + b"\n")
            elif os.path.isfile(full):
                h.update(b"F" + rel.encode() + b"\0" + sha256_file(full).encode() + b"\n")
    return h.hexdigest()


def cache_key(tool_sha, oracle_sha, argv, env, inputs, extra=None):
    """The key of one cached oracle output. argv: the exact command (list or list of lists);
    env: the variables the tool sets; inputs: digest(s) of the input workspace(s)."""
    return {"tool_sha256": tool_sha, "oracle_sha256": oracle_sha, "argv": argv, "env": env,
            "inputs": inputs, "extra": extra}


def key_digest(key):
    return sha256_bytes(json.dumps(key, sort_keys=True, separators=(",", ":")).encode())


def write_key(path, key):
    with open(path, "w") as f:
        json.dump({"digest": key_digest(key), "key": key}, f, indent=1, sort_keys=True)


def check_key(path, key):
    """None when the stored key at path equals key; otherwise the reason it is stale."""
    try:
        with open(path) as f:
            stored = json.load(f)
    except (OSError, ValueError):
        return "no cache key at %s (written by an older tool or never)" % path
    if stored.get("digest") == key_digest(key):
        return None
    old = stored.get("key") or {}
    diffs = [k for k in sorted(set(old) | set(key)) if old.get(k) != key.get(k)]
    return "cached oracle output at %s is stale (differs in: %s)" % (path, ", ".join(diffs))


def require_fresh(path, key):
    reason = check_key(path, key)
    if reason:
        raise Refusal(reason + "; rerun without reuse of the oracle outputs")


# ---------------------------------------------------------------------------------------------
# Result classes and the report.

IDENTICAL = "IDENTICAL"
DIFFERENT = "DIFFERENT"
BOTH_FAILED = "BOTH-FAILED"
NOT_PORTED = "NOT-PORTED"
SKIPPED = "SKIPPED"
DECLARED_GAP = "DECLARED-GAP"
ORACLE_UNEXPECTED = "ORACLE-UNEXPECTED"
CLASSES = [IDENTICAL, DIFFERENT, BOTH_FAILED, NOT_PORTED, ORACLE_UNEXPECTED, SKIPPED, DECLARED_GAP]
PASSING = {IDENTICAL, DECLARED_GAP}
CLASS_HELP = {
    IDENTICAL: "same termination and every compared field equal; the oracle exited 0, or the "
               "case declares its non-zero exit code as the behaviour under test",
    DIFFERENT: "termination or a compared field differs",
    BOTH_FAILED: "both sides failed the same way (a non-zero exit not declared by the case, or a "
                 "signal, OOM kill, timeout, unrecorded end on both): listed, not identical",
    NOT_PORTED: "Blaze stopped at unported!(...)",
    ORACLE_UNEXPECTED: "the oracle did not end as the case declares",
    SKIPPED: "not compared (reason given)",
    DECLARED_GAP: "not compared; the case list names this gap",
}


def judge(oracle_term, blaze_term, differing, declared_exit=None, blaze_unported=None):
    """(class, detail). differing: names of the compared fields that differ (termination is
    compared here, do not include it). declared_exit: the exit code the case list declares as the
    behaviour under test (None: none declared, i.e. 0 expected)."""
    if oracle_term is None or blaze_term is None:
        return SKIPPED, "no termination recorded for the %s" % (
            "oracle" if oracle_term is None else "blaze side")
    if declared_exit is not None and not (oracle_term.kind == "exit"
                                          and oracle_term.code == declared_exit):
        return ORACLE_UNEXPECTED, "declared exit %d, oracle %s" % (declared_exit, oracle_term)
    if blaze_unported:
        return NOT_PORTED, blaze_unported
    diff = list(differing)
    if oracle_term != blaze_term:
        diff.insert(0, "termination (oracle %s, blaze %s)" % (oracle_term, blaze_term))
    if diff:
        return DIFFERENT, ", ".join(diff)
    if oracle_term.ok:
        return IDENTICAL, ""
    if declared_exit is not None:
        return IDENTICAL, "declared failure: %s" % oracle_term
    return BOTH_FAILED, "both %s" % oracle_term


def unported_message(err):
    """The `blaze: not ported yet: X` text of Blaze's stderr (bytes or str), or None."""
    if isinstance(err, bytes):
        err = err.decode("latin-1")
    i = err.find("not ported yet")
    if i < 0:
        return None
    return err[i:].split("\n", 1)[0][:160]


class Report:
    """Header, rows and footer of one comparison; the exit-code rule."""

    def __init__(self, tool, blaze=None, oracle=None, selection=None, tool_files=()):
        self.tool = tool
        self.rows = []
        self.prov = {
            "tool": tool,
            "tool_sha256": tool_sha256(*tool_files) if tool_files else None,
            "compare_common_sha256": sha256_file(THIS),
            "date": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
            "repo": repo_state(),
            "blaze": blaze,
            "oracle": oracle,
            "selection": selection,
        }

    def add(self, case, cls, detail="", **extra):
        if cls not in CLASSES:
            raise ValueError(cls)
        row = {"case": case, "class": cls, "detail": detail}
        row.update(extra)
        self.rows.append(row)
        return row

    def counts(self):
        c = {k: 0 for k in CLASSES}
        for r in self.rows:
            c[r["class"]] += 1
        return c

    def denominator(self):
        c = self.counts()
        return {"selected": len(self.rows),
                "compared": len(self.rows) - c[SKIPPED] - c[DECLARED_GAP],
                "skipped": c[SKIPPED], "declared_gap": c[DECLARED_GAP],
                "both_failed": c[BOTH_FAILED], "identical": c[IDENTICAL],
                "different": c[DIFFERENT], "not_ported": c[NOT_PORTED],
                "oracle_unexpected": c[ORACLE_UNEXPECTED]}

    def header_lines(self):
        p = self.prov
        b, o, r = p["blaze"] or {}, p["oracle"] or {}, p["repo"]
        lines = ["%s: %s" % (p["tool"], p["date"]),
                 "  repository: HEAD %s, tree %s" % (
                     r.get("commit"), {True: "dirty", False: "clean"}.get(r.get("dirty"), "?")),
                 "  blaze:  %s sha256 %s" % (b.get("path", "-"), b.get("sha256", "-")),
                 "  oracle: %s sha256 %s" % (o.get("path", "-"), o.get("sha256", "-"))]
        if p.get("selection"):
            lines.append("  selection: %s" % p["selection"])
        return lines

    def print_header(self, file=None):
        print("\n".join(self.header_lines()), file=file or sys.stdout, flush=True)

    def footer_lines(self):
        d = self.denominator()
        lines = ["denominator: %d selected, %d compared, %d skipped, %d declared gaps, "
                 "%d both failed, %d identical, %d different, %d not ported, "
                 "%d oracle unexpected" % (
                     d["selected"], d["compared"], d["skipped"], d["declared_gap"],
                     d["both_failed"], d["identical"], d["different"], d["not_ported"],
                     d["oracle_unexpected"])]
        if not self.rows:
            lines.append("error: the selection is empty: nothing was compared")
        elif d["compared"] == 0:
            lines.append("error: nothing was compared")
        lines.append("result: %s (exit %d)" % ("PASS" if self.exit_code() == 0 else "FAIL",
                                               self.exit_code()))
        return lines

    def print_rows(self, file=None, width=None):
        width = width or max([len(r["case"]) for r in self.rows] + [10])
        for r in self.rows:
            print("%-*s  %-17s %s" % (width, r["case"], r["class"], r["detail"]),
                  file=file or sys.stdout)

    def print_footer(self, file=None):
        print("\n".join(self.footer_lines()), file=file or sys.stdout, flush=True)

    def exit_code(self):
        d = self.denominator()
        if not self.rows or d["compared"] == 0:
            return 1
        return 0 if all(r["class"] in PASSING for r in self.rows) else 1

    def to_json(self):
        return {"provenance": self.prov, "denominator": self.denominator(),
                "exit_code": self.exit_code(), "classes": CLASS_HELP, "cases": self.rows}

    def write_json(self, path, extra=None):
        obj = self.to_json()
        if extra:
            obj.update(extra)
        tmp = path + ".tmp"
        with open(tmp, "w") as f:
            json.dump(obj, f, indent=1, sort_keys=True, default=str)
            f.write("\n")
        os.replace(tmp, path)


def empty_selection_error(tool, what="cases"):
    """Message and exit status for a selection that matches nothing."""
    print("%s: error: the selection matches no %s; nothing compared" % (tool, what),
          file=sys.stderr)
    return 1


def refuse(tool, e):
    print("%s: refused: %s" % (tool, e), file=sys.stderr)
    return 2


# ---------------------------------------------------------------------------------------------
# Selftest.


def make_fake(path, out=b"", err=b"", code=0, sig=None, alloc_mb=0, write_files=None):
    """A fake binary (a shell script) that ignores its arguments, prints out/err, writes the
    given files (relative to its cwd), allocates alloc_mb MiB, then exits with code or kills
    itself with sig."""
    def q(b):
        return shlex.quote(b.decode("latin-1") if isinstance(b, bytes) else b)
    lines = ["#!/bin/bash", "printf %%s %s" % q(out), "printf %%s %s >&2" % q(err)]
    for rel, data in (write_files or {}).items():
        lines.append("mkdir -p \"$(dirname %s)\"; printf %%s %s > %s" % (q(rel), q(data), q(rel)))
    if alloc_mb:
        lines.append("/usr/bin/python3 -c 'x = bytearray(%d * 1024 * 1024); x[::4096] = "
                     "b\"1\" * len(x[::4096])'" % alloc_mb)
    if sig:
        lines.append("kill -%s $$" % sig)
    lines.append("exit %d" % code)
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")
    os.chmod(path, 0o755)
    return path


SCENARIOS = [
    # name, oracle fake, blaze fake, classes that are acceptable (none of them passing except
    # for "equal")
    ("equal", dict(out=b"same\n", err=b"INFO: x\n"), dict(out=b"same\n", err=b"INFO: x\n"),
     {IDENTICAL}),
    ("different", dict(out=b"same\n"), dict(out=b"other\n"), {DIFFERENT}),
    ("equal-failures", dict(out=b"x\n", err=b"ERROR: boom\n", code=1),
     dict(out=b"x\n", err=b"ERROR: boom\n", code=1), {BOTH_FAILED}),
    ("blaze-sigpipe", dict(out=b"same\n"), dict(out=b"same\n", sig="PIPE"), {DIFFERENT}),
    ("blaze-sigterm", dict(out=b"same\n"), dict(out=b"same\n", sig="TERM"), {DIFFERENT}),
    ("both-sigpipe", dict(out=b"same\n", sig="PIPE"), dict(out=b"same\n", sig="PIPE"),
     {BOTH_FAILED}),
    ("blaze-unported", dict(out=b"same\n"),
     dict(out=b"same\n", err=b"blaze: not ported yet: Foo.bar\n", code=37),
     {NOT_PORTED, DIFFERENT}),  # DIFFERENT for a tool without a not-ported class
]


class SelftestFailure(Exception):
    pass


class Selftest:
    """Collects checks; tools add their own with check()."""

    def __init__(self, tool):
        self.tool = tool
        self.failures = 0
        self.n = 0
        self.dir = tempfile.mkdtemp(prefix="selftest-%s-" % tool.replace("/", "-"),
                                    dir=os.path.join(CACHE, "work", "compare-common"))

    def check(self, name, ok, detail=""):
        self.n += 1
        if not ok:
            self.failures += 1
        print("  %-4s %-40s %s" % ("ok" if ok else "FAIL", name, detail), flush=True)

    def fakes(self, name, ospec, bspec):
        d = os.path.join(self.dir, name)
        os.makedirs(d, exist_ok=True)
        return (make_fake(os.path.join(d, "oracle"), **ospec),
                make_fake(os.path.join(d, "blaze"), **bspec), d)

    def finish(self):
        import shutil
        shutil.rmtree(self.dir, ignore_errors=True)
        print("%s selftest: %d checks, %d failed: %s" % (
            self.tool, self.n, self.failures, "PASS" if not self.failures else "FAIL"))
        return 0 if not self.failures else 1


def selftest(tool, run_pair=None, empty_selection=None, extra=None, mode="scope", modes=None):
    """The common part of every tool's `selftest`.

    run_pair(oracle_fake, blaze_fake, workdir) -> (class, detail): the tool's own code path for
      one synthetic case with the given fake binaries (they ignore arguments). Each scenario in
      SCENARIOS must come out in its expected class.
    empty_selection() -> exit status of the tool's main() with a selection matching nothing
      (must not be 0).
    extra(st): tool-specific checks (st.check(name, ok, detail)).
    """
    global _pin_override
    os.makedirs(os.path.join(CACHE, "work", "compare-common"), exist_ok=True)
    st = Selftest(tool)
    print("%s selftest (fake binaries in %s)" % (tool, st.dir))
    try:
        # 1. The launcher records the real end of a command, in the tool's mode.
        d = os.path.join(st.dir, "launch")
        os.makedirs(d)
        for mode in modes or (mode,):
            for sig in ("PIPE", "TERM"):
                f = make_fake(os.path.join(d, "sig" + sig), out=b"x", sig=sig)
                r = run([f], d, mode=mode, tag="selftest")
                st.check("launcher records SIG%s (%s)" % (sig, mode),
                         r.term.kind == "signal" and r.term.sig == "SIG" + sig, str(r.term))
            f = make_fake(os.path.join(d, "exit3"), out=b"o", err=b"e", code=3)
            r = run([f], d, mode=mode, tag="selftest")
            st.check("launcher records exit 3 and output (%s)" % mode, r.term == Termination("exit", 3)
                     and r.out == b"o" and r.err == b"e", "%s %r %r" % (r.term, r.out, r.err))
            f = make_fake(os.path.join(d, "oom"), alloc_mb=256)
            r = run([f], d, mode=mode, memory_max="64M", tag="selftest")
            st.check("launcher records an OOM kill (%s)" % mode, r.term.kind == "oom-kill", str(r.term))
            f = os.path.join(d, "sleep")
            with open(f, "w") as fh:
                fh.write("#!/bin/bash\nsleep 30\n")
            os.chmod(f, 0o755)
            r = run([f], d, mode=mode, timeout=2, tag="selftest")
            st.check("launcher records a timeout (%s)" % mode, r.term.kind == "timeout", str(r.term))
            f = make_fake(os.path.join(d, "env"), out=b"")
            with open(f, "w") as fh:
                fh.write("#!/bin/bash\nenv | sort\n")
            r = run([f], d, mode=mode, env={"X": "1"}, tag="selftest")
            names = sorted(l.split("=", 1)[0] for l in r.out.decode().splitlines())
            st.check("clean environment (%s)" % mode, set(names) <= {"HOME", "PATH", "X", "PWD", "SHLVL", "_",
                                                         "OLDPWD"}, " ".join(names))

        # 2. The pin.
        try:
            require_oracle(make_fake(os.path.join(d, "wrong-oracle"), out=b"x"))
            st.check("wrong oracle hash refused", False, "accepted")
        except Refusal as e:
            st.check("wrong oracle hash refused", True, str(e)[:80])

        # 3. The cache key.
        key = cache_key("t", "o", ["a"], {}, "in1")
        kp = os.path.join(d, "key.json")
        write_key(kp, key)
        st.check("cache: same key accepted", check_key(kp, key) is None)
        st.check("cache: other input refused",
                 check_key(kp, cache_key("t", "o", ["a"], {}, "in2")) is not None)
        st.check("cache: other oracle refused",
                 check_key(kp, cache_key("t", "o2", ["a"], {}, "in1")) is not None)
        st.check("cache: missing key refused", check_key(kp + ".none", key) is not None)

        # 4. The report's exit rule.
        rep = Report("selftest")
        st.check("empty report fails", rep.exit_code() != 0)
        for cls in CLASSES:
            rep = Report("selftest")
            rep.add("a", IDENTICAL)
            rep.add("b", cls)
            st.check("report with one %s %s" % (cls, "passes" if cls in PASSING else "fails"),
                     (rep.exit_code() == 0) == (cls in PASSING))
        rep = Report("selftest")
        rep.add("a", DECLARED_GAP)
        st.check("report with only declared gaps fails", rep.exit_code() != 0)
        st.check("judge: declared failure matches",
                 judge(Termination("exit", 2), Termination("exit", 2), [], declared_exit=2)[0]
                 == IDENTICAL)
        st.check("judge: declaration the oracle contradicts",
                 judge(Termination("exit", 1), Termination("exit", 1), [], declared_exit=2)[0]
                 == ORACLE_UNEXPECTED)

        # 5. The tool's own path, scenario by scenario.
        if run_pair is not None:
            for name, ospec, bspec, want in SCENARIOS:
                ofake, bfake, wd = st.fakes(name, ospec, bspec)
                _pin_override = sha256_file(ofake)
                try:
                    cls, detail = run_pair(ofake, bfake, wd)
                except Refusal as e:
                    cls, detail = "REFUSED", str(e)
                except Exception as e:  # noqa: BLE001 - a crash of the tool's path is a failure
                    cls, detail = "CRASHED", "%s: %s" % (type(e).__name__, e)
                finally:
                    _pin_override = None
                st.check("tool: %s" % name, cls in want,
                         "%s %s" % (cls, (detail or "")[:90]))
            # The tool must refuse the fake oracle once the real pin is back.
            ofake, bfake, wd = st.fakes("wrong-oracle", dict(out=b"same\n"), dict(out=b"same\n"))
            try:
                cls, detail = run_pair(ofake, bfake, wd)
                st.check("tool: wrong oracle refused", False, "got %s" % cls)
            except Refusal as e:
                st.check("tool: wrong oracle refused", True, str(e)[:80])
            except Exception as e:  # noqa: BLE001
                st.check("tool: wrong oracle refused", False, "crashed: %s" % e)
        if empty_selection is not None:
            rc = empty_selection()
            st.check("tool: empty selection fails", rc not in (0, None), "exit %s" % rc)
        if extra is not None:
            try:
                extra(st)
            except Exception as e:  # noqa: BLE001
                import traceback
                traceback.print_exc()
                st.check("tool-specific checks ran to the end", False, "%s: %s" % (
                    type(e).__name__, e))
    finally:
        _pin_override = None
    return st.finish()


def pin_override_for_selftest(sha):
    """Context for tool selftests that drive their own main(): lets one fake oracle through."""
    class _Ctx:
        def __enter__(self):
            global _pin_override
            _pin_override = sha

        def __exit__(self, *a):
            global _pin_override
            _pin_override = None
    return _Ctx()


# ---------------------------------------------------------------------------------------------
# Command line.


def _cli(argv):
    if not argv or argv[0] in ("help", "--help", "-h"):
        print(__doc__)
        return 0
    cmd, rest = argv[0], argv[1:]
    if cmd == "supervise":
        return supervise(rest[0], rest[1:])
    if cmd == "check-oracle":
        try:
            path, sha = require_oracle(rest[0] if rest else None)
        except Refusal as e:
            print("refused: %s" % e, file=sys.stderr)
            return 2
        print("%s sha256 %s (pinned)" % (path, sha))
        return 0
    if cmd == "run":
        mode, cwd, timeout, env, so, se, tf = "scope", os.getcwd(), None, {}, None, None, None
        i = 0
        while i < len(rest) and rest[i] != "--":
            a = rest[i]
            if a == "--mode":
                mode = rest[i + 1]
            elif a == "--cwd":
                cwd = rest[i + 1]
            elif a == "--timeout":
                timeout = float(rest[i + 1])
            elif a == "--env":
                k, _, v = rest[i + 1].partition("=")
                env[k] = v
            elif a == "--stdout":
                so = rest[i + 1]
            elif a == "--stderr":
                se = rest[i + 1]
            elif a == "--termination":
                tf = rest[i + 1]
            else:
                print("unknown option %s" % a, file=sys.stderr)
                return 2
            i += 2
        argv_ = rest[i + 1:]
        if not argv_:
            print(__doc__, file=sys.stderr)
            return 2
        r = run(argv_, cwd, env=env, timeout=timeout, mode=mode, stdout_path=so, stderr_path=se,
                tag="cli")
        if so is None:
            sys.stdout.buffer.write(r.out)
        if se is None:
            sys.stderr.buffer.write(r.err)
        if tf:
            with open(tf, "w") as f:
                f.write(str(r.term) + "\n")
        print("termination: %s" % r.term, file=sys.stderr)
        return r.term.exit_code_like()
    if cmd == "selftest":
        return selftest("compare_common", modes=("scope", "service"))
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(_cli(sys.argv[1:]))
