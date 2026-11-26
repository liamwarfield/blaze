#!/usr/bin/python3 -I
"""Shared guards of the oracle-against-Blaze comparison tools (audit rounds 3 and 4).

Standard library only. Tools import it with

    sys.dont_write_bytecode = True
    sys.path.insert(0, os.path.join(<repo>, "scripts", "lib"))
    import compare_common as cc

What lives here (and nowhere else):

  run()            runs one binary in a memory-capped systemd unit (`--scope`: a server the
                   binary daemonizes keeps living between commands; or a `--wait` service: the
                   unit ends with the binary) with a clean environment (env -i HOME
                   PATH=/usr/bin:/bin plus what the tool names), under a supervisor that records
                   inside the unit how the command ended: `exit N`, `signal NAME`, `oom-kill`
                   (the unit cgroup's oom_kill counter), `timeout` (the whole unit is SIGKILLed),
                   or `unrecorded`. The supervisor runs from a private copy of this file taken
                   when the module is imported, so an edit during a run cannot change it; a
                   Report refuses to finish (exit 2) when this file or the tool changed on disk
                   during the run.
  require_oracle() refuses an oracle whose sha256 is not the pin in UPSTREAM.json;
                   blaze_identity() refuses the oracle as the binary under test.
  Rule/Normaliser  stdout and stderr are compared in full after an EXCLUDE normalisation: every
                   exclusion is a named rule with a reason and a kind, and the report lists for
                   each rule how many lines it changed or removed on each side.
                     volatile  a path, a time, a random name, a timer-driven progress line:
                               removed or masked, never affects the class;
                     gap       a line that depends on state outside the behaviour under test,
                               chiefly the oracle's warm server against Blaze's cold process
                               (audit round 3, B1): removed from the comparison, but when the
                               removed text differs between the sides the case is KNOWN-GAP,
                               never IDENTICAL.
  keys             write_key() is called after the outputs it anchors are complete and stores
                   the digest of those outputs with the key (oracle sha256, exact argv, the
                   environment, the digest of the input, the case sequence); check_key() refuses
                   a key that differs or whose outputs changed or are missing; invalidate_key()
                   is called before outputs are rewritten.
  captures         write_capture()/check_capture(): a stored Blaze capture carries the binary's
                   sha256 and the digest of its files; it is judged only when the caller names
                   that binary and its sha256 matches.
  judge()          the classes:
                     IDENTICAL          termination, stdout and the full stderr equal after the
                                        volatile rules; the oracle exited 0, or the case declares
                                        its exit code as the failure under test
                     KNOWN-GAP          equal except in lines a gap rule removed
                     IDENTICAL-REDUCED  equal in what was compared, but the comparison of some
                                        field was reduced (the detail names it)
                     DIFFERENT, BOTH-FAILED (equal failures not declared), NOT-PORTED,
                     ORACLE-UNEXPECTED, SKIPPED (not compared, reason given), DECLARED-GAP (not
                     compared; the case list names the gap)
  Report           header (tool, date, git HEAD, dirty flag incl. untracked files, both binaries'
                   path and sha256, what is compared), one row per selected case, the exclusion
                   table, the terminations of auxiliary commands (shutdown, clean, info,
                   warm-up), the footer with the denominator. Exit rule: 0 only when at least one
                   case was compared and every selected case is IDENTICAL or DECLARED-GAP, no
                   auxiliary command died abnormally, and the run judged something new
                   (Report.reclassify_only makes 0 impossible); 1 otherwise; 2 for a refusal.
  selftest_tool()  drives a tool's real command line (python3 <tool> <argv>) under a fake HOME
                   whose oracle is a fake script, with the pin overridden for that fake only, and
                   a tripwire: the fakes refuse to run (and leave a mark) unless started through
                   run(), so any launch that bypasses the module fails the selftest.

Command line:

  compare_common.py run [--mode scope|service] [--cwd DIR] [--timeout S] [--env K=V]...
                        [--stdout FILE] [--stderr FILE] [--termination FILE] -- ARGV...
      runs ARGV as run() does; prints the termination to stderr, writes it to --termination,
      exits 0 after `exit 0`, the code after another exit, 128+N after a signal, 137 after an
      OOM kill or timeout, 125 when unrecorded.
  compare_common.py check-oracle [PATH]   exit 0 when PATH (default the pinned path) is the pin
  compare_common.py supervise STATUS ARGV...   (internal: the unit's main process)
  compare_common.py selftest              the module's own guards
  compare_common.py help
"""

import atexit
import datetime
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid

def _proc_start(pid):
    """The start time (clock ticks since boot) of process pid, or None: with the pid it names
    one process, whatever reuses the number later."""
    try:
        with open("/proc/%d/stat" % pid) as f:
            return f.read().rsplit(")", 1)[1].split()[19]
    except (OSError, IndexError):
        return None


def _keep_scope(cg, tool, release):
    """Forks a keeper that stays in the scope's cgroup after the supervisor exits, while a process
    the command left behind (a daemonized server) lives there, so the scope and its oom_kill
    counter outlive an OOM kill of that server until the tool has read them (audit round 4, M1).
    It exits when `release` exists or the tool process (`tool`: "PID:STARTTIME") is gone."""
    tool_pid, tool_start = tool.split(":", 1)
    tool_pid = int(tool_pid)
    try:
        with open("/sys/fs/cgroup" + cg + "/cgroup.procs") as f:
            others = [int(x) for x in f.read().split() if int(x) != os.getpid()]
    except (OSError, ValueError):
        return
    if not others or os.fork() != 0:
        return
    os.setsid()
    devnull = os.open(os.devnull, os.O_RDWR)
    for fd in (0, 1, 2):
        os.dup2(devnull, fd)
    for s in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT):
        signal.signal(s, signal.SIG_DFL)
    while not os.path.exists(release) and _proc_start(tool_pid) == tool_start:
        time.sleep(0.5)
    os._exit(0)


def supervise(status_file, cmd, keep=None):
    """The unit's main process: runs cmd, writes `exit N` or `signal N NAME`, then
    `oom_kill K` (the unit cgroup's counter), to status_file. keep (scope mode): (tool pid,
    release file) for _keep_scope."""
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
    cg = None
    try:
        cg = open("/proc/self/cgroup").read().strip().split("::", 1)[1]
        for ln in open("/sys/fs/cgroup" + cg + "/memory.events"):
            k, v = ln.split()
            if k == "oom_kill":
                oom = int(v)
    except (OSError, IndexError, ValueError):
        pass
    with open(status_file, "w") as f:
        f.write("%s\noom_kill %d\ncgroup %s\n" % (line, oom, cg or "-"))
    if keep and cg:
        _keep_scope(cg, keep[0], keep[1])
    return code


# The unit's main process runs a private copy of this file: it needs nothing below.
if __name__ == "__main__" and len(sys.argv) > 2 and sys.argv[1] == "supervise":
    if sys.argv[3:4] == ["--keep"]:
        sys.exit(supervise(sys.argv[2], sys.argv[6:], keep=(sys.argv[4], sys.argv[5])))
    sys.exit(supervise(sys.argv[2], sys.argv[3:]))

HOME = os.path.expanduser("~")
LIB = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(LIB))
CACHE = os.path.join(HOME, ".cache", "blaze")
WORK = os.path.join(CACHE, "work", "compare-common")
STATUS_DIR = os.path.join(WORK, "status")
THIS = os.path.abspath(__file__)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


MODULE_SHA256 = sha256_file(THIS)


def _supervisor_copy():
    """A private, content-addressed copy of this file for the units' supervisor."""
    path = os.path.join(STATUS_DIR, "compare_common-%s.py" % MODULE_SHA256)
    if not os.path.exists(path):
        os.makedirs(STATUS_DIR, exist_ok=True)
        tmp = "%s.%d.tmp" % (path, os.getpid())
        shutil.copyfile(THIS, tmp)
        os.replace(tmp, path)
    return path

# ---------------------------------------------------------------------------------------------
# Selftest mode (only under a fake HOME prepared by selftest_tool()).

SELFTEST_TOKEN_FILE = ".compare-common-selftest"


def selftest_active():
    tok = os.environ.get("COMPARE_COMMON_SELFTEST")
    if not tok:
        return False
    try:
        with open(os.path.join(HOME, SELFTEST_TOKEN_FILE)) as f:
            return f.read().strip() == tok
    except OSError:
        return False

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
_sha_memo = {}


def _sha_cached(path):
    st = os.stat(path)
    k = (os.path.realpath(path), st.st_size, st.st_mtime_ns)
    if k not in _sha_memo:
        _sha_memo[k] = sha256_file(path)
    return _sha_memo[k]


def pinned_sha256():
    if selftest_active() and os.environ.get("COMPARE_COMMON_SELFTEST_PIN"):
        return os.environ["COMPARE_COMMON_SELFTEST_PIN"]
    return PINNED_SHA256


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
    the oracle itself."""
    path = os.path.abspath(os.path.expanduser(path))
    if not os.path.isfile(path):
        raise Refusal("binary under test %s does not exist" % path)
    sha = _sha_cached(path)
    if sha in (pinned_sha256(), PINNED_SHA256):
        raise Refusal("the binary under test %s is the oracle (sha256 %s)" % (path, sha))
    return {"path": path, "realpath": os.path.realpath(path), "sha256": sha}


def repo_state():
    def git(*args):
        p = subprocess.run(["git", "--no-optional-locks", "-C", REPO] + list(args),
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        return p.stdout.decode() if p.returncode == 0 else None
    head = git("rev-parse", "HEAD")
    status = git("status", "--porcelain=v1", "--untracked-files=normal")
    return {"commit": head.strip() if head else None,
            "dirty": (bool(status.strip()) if status is not None else None)}


def tool_sha256(*paths):
    """sha256 over the given tool files."""
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
        return (self.kind, self.code, self.sig)

    def exit_code_like(self):
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
    full_env = clean_env(env)
    if selftest_active():
        full_env["COMPARE_COMMON_TRIPWIRE"] = os.environ["COMPARE_COMMON_SELFTEST"]
    keep = (["--keep", "%d:%s" % (os.getpid(), _proc_start(os.getpid())),
             status_path + ".release"] if mode == "scope" else [])
    cmd += ["-p", "MemoryMax=" + memory_max, "-p", "MemorySwapMax=0", "-p", "OOMPolicy=continue",
            "--", "/usr/bin/python3", "-I", _supervisor_copy(), "supervise", status_path] + keep + [
            "env", "-i"] + ["%s=%s" % kv for kv in full_env.items()] + list(argv)
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
    if "cgroup" in words:
        i = words.index("cgroup")
        _parse_status.cgroup = words[i + 1] if words[i + 1] != "-" else None
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


# Scope units of earlier commands of this process that still hold a process (a server the
# command daemonized stays in the scope of the command that started it): unit -> the oom_kill
# count last read. An OOM kill in one of them while a later command runs belongs to that later
# command (audit round 4, M1).
_LIVE_SCOPES = {}


def _scope_state(cg):
    """(oom_kill count, number of processes) of the scope cgroup `cg`, or None when it is gone."""
    try:
        with open("/sys/fs/cgroup" + cg + "/memory.events") as f:
            oom = 0
            for ln in f:
                k, v = ln.split()
                if k == "oom_kill":
                    oom = int(v)
        with open("/sys/fs/cgroup" + cg + "/cgroup.procs") as f:
            procs = len(f.read().split())
    except (OSError, ValueError):
        return None
    return oom, procs


def _release_scope(unit):
    """Lets the scope's keeper exit (it polls for the release file every 0.5 s; the file is
    removed by the next run() of this process or at exit, after the keeper had time to see it)."""
    info = _LIVE_SCOPES.pop(unit, None)
    if info:
        try:
            open(info["release"], "w").close()
            _RELEASED.append(info["release"])
        except OSError:
            pass


_RELEASED = []


def _at_exit():
    """At exit the keepers see this process gone and exit by themselves; the release files
    written so far are removed."""
    _remove_released(min_age=0)


atexit.register(_at_exit)


def _remove_released(min_age=2.0):
    now = time.time()
    for path in list(_RELEASED):
        try:
            if now - os.stat(path).st_mtime >= min_age:
                os.remove(path)
                _RELEASED.remove(path)
        except OSError:
            _RELEASED.remove(path)


def release_scopes():
    """Lets the keepers of every scope this process still tracks exit (run at exit)."""
    for unit in list(_LIVE_SCOPES):
        _release_scope(unit)


def _live_scope_ooms():
    """{unit: oom_kill count} of the earlier scopes still tracked. A scope in which only its
    keeper is left (the server ended) is released after this reading."""
    now = {}
    for unit, info in list(_LIVE_SCOPES.items()):
        st = _scope_state(info["cg"])
        if st is None:
            _LIVE_SCOPES.pop(unit, None)
            continue
        now[unit] = st[0]
        if st[1] <= 1:
            _release_scope(unit)
    return now


def run(argv, cwd, env=None, timeout=None, mode="scope", memory_max="12G", stdin=None,
        stdout_path=None, stderr_path=None, tag="cmp"):
    """Runs argv in a unit (see the module doc). Returns a RunResult whose out/err are the bytes
    (also written to stdout_path/stderr_path when given). An OOM kill during the command in the
    scope of an earlier command of this process (a server it daemonized) makes the termination
    `oom-kill` too."""
    os.makedirs(STATUS_DIR, exist_ok=True)
    _remove_released()
    before = _live_scope_ooms()
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
        after = _live_scope_ooms()
        extra = sum(max(0, after.get(u, n) - n) for u, n in before.items())
        if extra and term.kind not in ("oom-kill", "timeout"):
            term = Termination("oom-kill", detail="%d OOM kill(s) during this command in the "
                               "scope of an earlier command (a server it started); the command "
                               "then ended with %s" % (extra, term))
        cg = getattr(_parse_status, "cgroup", None)
        _parse_status.cgroup = None
        if mode == "scope" and cg and term.kind != "timeout":
            st = _scope_state(cg)
            if st is not None and st[1] >= 1:
                _LIVE_SCOPES[unit] = {"cg": cg, "release": status + ".release"}
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


def server_owned_by_root(pid, root):
    """pid is alive (not a zombie) and its command line names the output user root `root`: a
    server of that root, not an unrelated process that reused the pid (as
    scripts/server/common.py owned_by_root)."""
    try:
        with open("/proc/%d/stat" % pid) as f:
            if f.read().rsplit(")", 1)[1].split()[0] == "Z":
                return False
        with open("/proc/%d/cmdline" % pid, "rb") as f:
            cmdline = f.read()
    except (OSError, IndexError):
        return False
    return os.fsencode(os.path.realpath(root)) in cmdline or os.fsencode(root) in cmdline


def kill_root_servers(root, wait=10.0):
    """SIGTERMs (then, after `wait` seconds, SIGKILLs) every server named by
    <root>/*/server/server.pid.txt whose process is a server of `root`; returns the list of
    (pid, what happened). A pid whose command line does not name the root is left alone."""
    done = []
    if not os.path.isdir(root):
        return done
    for name in sorted(os.listdir(root)):
        pidfile = os.path.join(root, name, "server", "server.pid.txt")
        try:
            pid = int(open(pidfile).read().strip())
        except (OSError, ValueError):
            continue
        if not server_owned_by_root(pid, root):
            done.append((pid, "not a server of %s; left alone" % root))
            continue
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                break
            t0 = time.monotonic()
            while time.monotonic() - t0 < wait and server_owned_by_root(pid, root):
                time.sleep(0.1)
            if not server_owned_by_root(pid, root):
                done.append((pid, "ended after %s" % signal.Signals(sig).name))
                break
        else:
            done.append((pid, "STILL ALIVE after SIGKILL"))
    return done

# ---------------------------------------------------------------------------------------------
# Normalisation: named exclusion rules with counts.


class Rule:
    """A named exclusion. pattern is a regular expression applied to one line. repl None: a
    matching line is removed; otherwise every match in the line is replaced by repl. kind:
    "volatile" (never affects the class) or "gap" (the removed text is compared separately;
    a difference makes the case KNOWN-GAP)."""

    def __init__(self, name, pattern, reason, repl=None, kind="volatile",
                 streams=("stderr",)):
        self.name, self.pattern, self.reason, self.repl = name, pattern, reason, repl
        self.kind, self.streams = kind, tuple(streams)
        self.rx = re.compile(pattern)

    def describe(self):
        return {"name": self.name, "kind": self.kind, "pattern": self.pattern,
                "replacement": self.repl, "reason": self.reason, "streams": list(self.streams)}


# Lines Bazel prints on a timer or with wall-clock values (as scripts/cmds/compare has done since
# round 3), and values that are random by construction.
VOLATILE_RULES = [
    Rule("progress",
         r"^(Computing main repo mapping: |Loading: |    currently loading: |Analyzing: |Fetching "
         r"|\[[\d,]+ / [\d,]+\] )",
         "UI progress lines printed on a timer (--show_progress_rate_limit); their number "
         "depends on wall-clock time"),
    Rule("elapsed", r"Elapsed time: \d+\.\d+s", "wall-clock time", repl="Elapsed time: <T>s",
         streams=("stdout", "stderr")),
    Rule("critical_path", r"Critical Path: \d+\.\d+s", "wall-clock time",
         repl="Critical Path: <T>s", streams=("stdout", "stderr")),
    Rule("blank", r"^$", "blank lines the UI writes around its timer-driven progress messages: "
         "their number differs between two runs of the oracle (scripts/server interop-client "
         "--control; scripts/w4/probe drops them for the same reason)"),
    Rule("no_actions_running", r"^ no actions running$", "printed by the UI's progress timer "
         "while work runs longer than its interval"),
    Rule("jvm_warning", r"^OpenJDK 64-Bit Server VM warning: ",
         "printed by the JVM, not by Bazel code; Blaze has no JVM"),
    Rule("download_temp", r"/temp\d+/", "Files.createTempDirectory picks a random number",
         repl="/temp<N>/", streams=("stdout", "stderr")),
    Rule("install_md5", r"/install/[0-9a-f]{32}", "the install base's name is a checksum of "
         "the binary's embedded files", repl="/install/<MD5>", streams=("stdout", "stderr")),
    Rule("uuid", r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
         "invocation and build ids are random", repl="<UUID>", streams=("stdout", "stderr")),
    Rule("test_ms", r"\(\d+ ms( total)?\)", "googletest timings", repl="(<N> ms)",
         streams=("stdout", "stderr")),
    Rule("test_secs", r" in \d+\.\d+s\b", "test durations", repl=" in <T>s",
         streams=("stdout", "stderr")),
]

# Lines that depend on the server's state: the oracle keeps a warm server between commands where
# Blaze, without a persistent server, starts a cold process (audit round 3, B1). For tools whose
# oracle runs on a warm server only.
B1_GAP_RULES = [
    Rule("b1_server_start", r"^Starting local Bazel server",
         "B1: printed when the client starts a server; the oracle's server is warm", kind="gap"),
    Rule("b1_packages_loaded", r"\(\d+ packages? loaded(, \d+ targets? configured)?\)",
         "B1: packages loaded and targets configured in this command depend on what the warm "
         "server already holds", repl="(<B1 loaded>)", kind="gap"),
    Rule("b1_processes", r"^INFO: \d+ process(es)?\b.*$",
         "B1: action cache hits against in-memory results depend on the server's state",
         repl="INFO: <B1 processes>", kind="gap"),
    # No rule for `INFO: Found N targets...` (audit 4r2, B3): with merged analysis and execution
    # Bazel prints it after runAnalysisAndExecutionPhase returns
    # (buildtool/AnalysisAndExecutionPhaseRunner.java:123-141), and that call throws
    # BuildFailedException when an action fails (:170), so a failed build prints no Found line
    # whatever the server holds. The line is compared in place, position included.
    Rule("b1_discarding", r"discarding analysis cache",
         "B1: only a warm server discards an earlier analysis cache", kind="gap"),
]


class Normalised:
    def __init__(self, text, gaps):
        self.text, self.gaps = text, gaps


class Normaliser:
    """Applies rules (and per-side path replacements, counted as rule `paths`) to stdout and
    stderr and counts, per rule and side, the lines each rule removed or changed."""

    def __init__(self, rules, path_reason="each side's own workspace copy, output base, "
                 "output user root and install base"):
        self.rules = list(rules)
        self.path_reason = path_reason
        self.counts = {}

    def _count(self, name, side, n=1):
        self.counts.setdefault(name, {"oracle": 0, "blaze": 0})
        self.counts[name][side] += n

    def apply(self, text, side, stream="stderr", paths=()):
        if isinstance(text, bytes):
            text = text.decode("latin-1")
        gaps = {}
        kept = []
        pairs = sorted([(f, t) for f, t in paths if f], key=lambda p: -len(p[0]))
        for line in text.split("\n"):
            orig = line
            for frm, to in pairs:
                line = line.replace(frm, to)
            if line != orig:
                self._count("paths", side)
            dropped = False
            for r in self.rules:
                if stream not in r.streams:
                    continue
                if r.repl is None:
                    if r.rx.search(line):
                        self._count(r.name, side)
                        if r.kind == "gap":
                            gaps.setdefault(r.name, []).append(line)
                        dropped = True
                        break
                else:
                    found = [m.group(0) for m in r.rx.finditer(line)]
                    if found:
                        if r.kind == "gap":
                            gaps.setdefault(r.name, []).extend(found)
                        new = r.rx.sub(r.repl, line)
                        if new != line:
                            self._count(r.name, side)
                        line = new
            if not dropped:
                kept.append(line)
        return Normalised("\n".join(kept), gaps)

    def compare(self, oracle_text, blaze_text, stream, oracle_paths=(), blaze_paths=()):
        """(equal, differing gap rule names) of one stream."""
        o = self.apply(oracle_text, "oracle", stream, oracle_paths)
        b = self.apply(blaze_text, "blaze", stream, blaze_paths)
        gap_diff = sorted(n for n in set(o.gaps) | set(b.gaps) if o.gaps.get(n) != b.gaps.get(n))
        return o.text == b.text, gap_diff

    def table(self):
        rows = []
        if "paths" in self.counts:
            rows.append({"name": "paths", "kind": "volatile", "reason": self.path_reason,
                         "oracle": self.counts["paths"]["oracle"],
                         "blaze": self.counts["paths"]["blaze"]})
        for r in self.rules:
            c = self.counts.get(r.name, {"oracle": 0, "blaze": 0})
            rows.append(dict(r.describe(), oracle=c["oracle"], blaze=c["blaze"]))
        return rows

# ---------------------------------------------------------------------------------------------
# Anchored cache of oracle outputs; provenance of stored Blaze captures.


def tree_digest(path, skip_top=("bazel-", ".git")):
    """sha256 over every file, symlink and empty directory below path (relative path, mode,
    then content or link target), skipping top-level entries named `.git` or starting with
    `bazel-` (or the given prefixes)."""
    h = hashlib.sha256()
    path = os.path.abspath(path)
    if os.path.isfile(path):
        h.update(b"F" + sha256_file(path).encode())
        return h.hexdigest()
    if not os.path.isdir(path):
        return None

    def skipped(name):
        return any(name == p or (p.endswith("-") and name.startswith(p)) for p in skip_top)
    for dirpath, dirnames, filenames in os.walk(path):
        rel_dir = os.path.relpath(dirpath, path)
        if rel_dir == ".":
            dirnames[:] = [d for d in dirnames if not skipped(d)]
            filenames = [f for f in filenames if not skipped(f)]
        dirnames.sort()
        if not dirnames and not filenames:
            h.update(b"D" + rel_dir.encode() + b"\n")
        for name in sorted(filenames) + [d for d in dirnames
                                         if os.path.islink(os.path.join(dirpath, d))]:
            full = os.path.join(dirpath, name)
            rel = os.path.normpath(os.path.join(rel_dir, name))
            if os.path.islink(full):
                h.update(b"L" + rel.encode() + b"\0" + os.readlink(full).encode() + b"\n")
            elif os.path.isfile(full):
                mode = os.stat(full).st_mode & 0o777
                h.update(b"F" + rel.encode() + b"\0" + ("%o" % mode).encode() + b"\0"
                         + sha256_file(full).encode() + b"\n")
    return h.hexdigest()


def outputs_digest(paths):
    """{path: sha256 or tree digest or None} of the given outputs."""
    out = {}
    for p in sorted(set(paths)):
        if os.path.isdir(p):
            out[p] = "tree:" + tree_digest(p, skip_top=())
        elif os.path.isfile(p):
            out[p] = sha256_file(p)
        else:
            out[p] = None
    return out


def cache_key(tool_sha, oracle_sha, argv, env, inputs, extra=None):
    """The key of one cached oracle output. tool_sha: the tool's sha256 when it stores
    normalised outputs, None when it stores raw outputs and normalises on reading."""
    return {"tool_sha256": tool_sha, "oracle_sha256": oracle_sha, "argv": argv, "env": env,
            "inputs": inputs, "extra": extra}


def key_digest(key):
    return sha256_bytes(json.dumps(key, sort_keys=True, separators=(",", ":")).encode())


def invalidate_key(path):
    """Removes a key before the outputs it anchored are rewritten."""
    if os.path.lexists(path):
        os.remove(path)


def write_key(path, key, outputs):
    """Writes the key AFTER the outputs are complete, with the digest of every output."""
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump({"digest": key_digest(key), "key": key, "outputs": outputs_digest(outputs)},
                  f, indent=1, sort_keys=True)
    os.replace(tmp, path)


def check_key(path, key):
    """None when the stored key at path equals key and every output it anchors is unchanged;
    otherwise the reason it is stale."""
    try:
        with open(path) as f:
            stored = json.load(f)
    except (OSError, ValueError):
        return "no cache key at %s (written by an older tool, or the run did not finish)" % path
    if "outputs" not in stored or not stored["outputs"]:
        return "cache key at %s anchors no outputs (written by an older tool)" % path
    if stored.get("digest") != key_digest(key):
        old = stored.get("key") or {}
        diffs = [k for k in sorted(set(old) | set(key)) if old.get(k) != key.get(k)]
        return "cached oracle output at %s is stale (differs in: %s)" % (path, ", ".join(diffs))
    now = outputs_digest(list(stored["outputs"]))
    changed = [p for p, d in stored["outputs"].items() if now.get(p) != d]
    if changed:
        return "the outputs anchored by %s changed or are missing: %s" % (
            path, ", ".join(changed[:3]))
    return None


def require_fresh(path, key):
    reason = check_key(path, key)
    if reason:
        raise Refusal(reason + "; rerun without reuse of the oracle outputs")


def write_capture(path, blaze, outputs, extra=None):
    """Provenance of a stored Blaze capture: the binary and the digest of its files."""
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump({"binary": blaze, "outputs": outputs_digest(outputs), "extra": extra,
                   "date": datetime.datetime.now().astimezone().isoformat(timespec="seconds")},
                  f, indent=1, sort_keys=True)
    os.replace(tmp, path)


def check_capture(path, blaze):
    """None when the capture at path was made by the binary `blaze` names (sha256) and its files
    are unchanged; otherwise the reason it may not be judged."""
    if blaze is None or not blaze.get("sha256"):
        return "no binary named by the caller: a stored Blaze capture is judged only against " \
               "the binary that made it"
    try:
        with open(path) as f:
            stored = json.load(f)
    except (OSError, ValueError):
        return "stored Blaze capture has no provenance (%s)" % path
    if not stored.get("outputs"):
        return "stored Blaze capture at %s anchors no outputs" % path
    sha = (stored.get("binary") or {}).get("sha256")
    if sha != blaze["sha256"]:
        return "stored Blaze capture was made by sha256 %s, not by the named %s" % (
            sha, blaze["sha256"])
    now = outputs_digest(list(stored.get("outputs") or {}))
    changed = [p for p, d in (stored.get("outputs") or {}).items() if now.get(p) != d]
    if changed:
        return "stored Blaze capture changed: %s" % ", ".join(changed[:3])
    return None

# ---------------------------------------------------------------------------------------------
# Result classes and the report.

IDENTICAL = "IDENTICAL"
KNOWN_GAP = "KNOWN-GAP"
REDUCED = "IDENTICAL-REDUCED"
DIFFERENT = "DIFFERENT"
BOTH_FAILED = "BOTH-FAILED"
NOT_PORTED = "NOT-PORTED"
SKIPPED = "SKIPPED"
DECLARED_GAP = "DECLARED-GAP"
ORACLE_UNEXPECTED = "ORACLE-UNEXPECTED"
CLASSES = [IDENTICAL, KNOWN_GAP, REDUCED, DIFFERENT, BOTH_FAILED, NOT_PORTED, ORACLE_UNEXPECTED,
           SKIPPED, DECLARED_GAP]
PASSING = {IDENTICAL, DECLARED_GAP}
CLASS_HELP = {
    IDENTICAL: "termination, stdout and the full stderr equal after the volatile rules (listed "
               "with their counts; among them `blank`: blank stderr lines are never compared); "
               "the oracle exited 0, or the case declares its exit code as the failure under test",
    KNOWN_GAP: "equal except in lines a gap rule removed (named in the detail); not identical",
    REDUCED: "equal in what was compared, but the comparison of some field was reduced (named "
             "in the detail); not identical",
    DIFFERENT: "termination or a compared field differs",
    BOTH_FAILED: "both sides failed the same way (a non-zero exit not declared by the case, or a "
                 "signal, OOM kill, timeout, unrecorded end on both): listed, not identical",
    NOT_PORTED: "Blaze stopped at unported!(...)",
    ORACLE_UNEXPECTED: "the oracle did not end as the case declares",
    SKIPPED: "not compared (reason given)",
    DECLARED_GAP: "not compared; the case list names this gap",
}


def judge(oracle_term, blaze_term, differing, declared_exit=None, blaze_unported=None,
          gaps=(), reduced=None):
    """(class, detail). differing: compared fields that differ (not the termination). gaps: gap
    rules whose removed text differs. reduced: text naming a reduced comparison, or None."""
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
    failed = not oracle_term.ok and declared_exit is None
    if failed:
        return BOTH_FAILED, "both %s%s" % (oracle_term, "; gaps " + ", ".join(gaps) if gaps
                                           else "")
    note = "declared failure: %s" % oracle_term if not oracle_term.ok else ""
    if gaps:
        return KNOWN_GAP, "differs only in gap rules: %s%s" % (", ".join(gaps),
                                                               "; " + note if note else "")
    if reduced:
        return REDUCED, reduced + ("; " + note if note else "")
    return IDENTICAL, note


def unported_message(err):
    """The `blaze: not ported yet: X` text of Blaze's stderr (bytes or str), or None."""
    if isinstance(err, bytes):
        err = err.decode("latin-1")
    i = err.find("not ported yet")
    if i < 0:
        return None
    return err[i:].split("\n", 1)[0][:160]


class Report:
    """Header, rows, exclusions, auxiliary terminations and footer of one comparison."""

    def __init__(self, tool, blaze=None, oracle=None, selection=None, tool_files=(),
                 compared=None):
        self.tool = tool
        self.rows = []
        self.aux = []
        self.notes = []
        self.problems = []
        self.normalisers = []
        self.reclassify_only = False
        self.tool_files = [os.path.abspath(p) for p in tool_files]
        self._tool_shas = {p: sha256_file(p) for p in self.tool_files}
        self.prov = {
            "tool": tool,
            "tool_sha256": tool_sha256(*self.tool_files) if self.tool_files else None,
            "compare_common_sha256": MODULE_SHA256,
            "date": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
            "repo": repo_state(),
            "blaze": blaze,
            "oracle": oracle,
            "selection": selection,
            "compared": compared,
            "selftest": selftest_active(),
        }
        self._changed = None

    def add(self, case, cls, detail="", **extra):
        if cls not in CLASSES:
            raise ValueError(cls)
        row = {"case": case, "class": cls, "detail": detail}
        row.update(extra)
        self.rows.append(row)
        return row

    def add_aux(self, what, term):
        """The termination of an auxiliary command (shutdown, clean, info, warm-up)."""
        self.aux.append({"what": what, "termination": str(term),
                         "abnormal": not term.normal})

    def note(self, text):
        self.notes.append(text)

    def problem(self, text):
        """Something outside the rows that makes the run fail (a side runner that died, a
        server left alive, a tool error inside a case)."""
        self.problems.append(text)

    def add_normaliser(self, norm):
        if norm not in self.normalisers:
            self.normalisers.append(norm)

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
                "identical": c[IDENTICAL], "known_gap": c[KNOWN_GAP],
                "identical_reduced": c[REDUCED], "both_failed": c[BOTH_FAILED],
                "different": c[DIFFERENT], "not_ported": c[NOT_PORTED],
                "oracle_unexpected": c[ORACLE_UNEXPECTED]}

    def changed_files(self):
        if self._changed is None:
            changed = []
            if sha256_file(THIS) != MODULE_SHA256:
                changed.append(THIS)
            for p, s in self._tool_shas.items():
                if not os.path.exists(p) or sha256_file(p) != s:
                    changed.append(p)
            self._changed = changed
        return self._changed

    def header_lines(self):
        p = self.prov
        b, o, r = p["blaze"] or {}, p["oracle"] or {}, p["repo"]
        lines = ["%s: %s" % (p["tool"], p["date"]),
                 "  repository: HEAD %s, tree %s" % (
                     r.get("commit"), {True: "dirty", False: "clean"}.get(r.get("dirty"), "?")),
                 "  tool sha256 %s, compare_common sha256 %s" % (p["tool_sha256"],
                                                                 p["compare_common_sha256"]),
                 "  blaze:  %s sha256 %s" % (b.get("path", "-"), b.get("sha256", "-")),
                 "  oracle: %s sha256 %s" % (o.get("path", "-"), o.get("sha256", "-"))]
        if p.get("selection"):
            lines.append("  selection: %s" % p["selection"])
        if p.get("compared"):
            lines.append("  compared: %s" % p["compared"])
        if p.get("selftest"):
            lines.append("  SELFTEST: pin overridden for a fake oracle under a fake HOME")
        return lines

    def print_header(self, file=None):
        print("\n".join(self.header_lines()), file=file or sys.stdout, flush=True)

    def exclusion_rows(self):
        out = []
        for n in self.normalisers:
            out.extend(n.table())
        return out

    def footer_lines(self):
        d = self.denominator()
        lines = []
        ex = self.exclusion_rows()
        if ex:
            lines.append("exclusions (rule, kind, lines changed or removed oracle/blaze, reason):")
            for e in ex:
                lines.append("  %-20s %-8s %5d/%-5d %s" % (e["name"], e["kind"], e["oracle"],
                                                           e["blaze"], e["reason"]))
        abnormal = [a for a in self.aux if a["abnormal"]]
        failed_aux = [a for a in self.aux if a["termination"] != "exit 0"]
        if failed_aux:
            lines.append("auxiliary commands not ending with exit 0:")
            for a in failed_aux:
                lines.append("  %s: %s" % (a["what"], a["termination"]))
        for n in self.notes:
            lines.append("note: %s" % n)
        blank = sum(e["oracle"] + e["blaze"] for e in ex if e["name"] == "blank")
        if blank:
            lines.append("note: %d blank stderr lines were removed on the two sides (volatile rule "
                         "`blank`): no class here says anything about blank lines" % blank)
        lines.append("denominator: %d selected, %d compared, %d skipped, %d declared gaps, "
                     "%d identical, %d known gap, %d identical-reduced, %d both failed, "
                     "%d different, %d not ported, %d oracle unexpected" % (
                         d["selected"], d["compared"], d["skipped"], d["declared_gap"],
                         d["identical"], d["known_gap"], d["identical_reduced"],
                         d["both_failed"], d["different"], d["not_ported"],
                         d["oracle_unexpected"]))
        if not self.rows:
            lines.append("error: the selection is empty: nothing was compared")
        elif d["compared"] == 0:
            lines.append("error: nothing was compared")
        if abnormal:
            lines.append("error: an auxiliary command died abnormally")
        for p in self.problems:
            lines.append("error: %s" % p)
        if self.reclassify_only:
            lines.append("error: re-classification of stored outputs only; nothing new was "
                         "judged, so this run cannot pass")
        if self.changed_files():
            lines.append("error: changed on disk during the run, so the run names a version it "
                         "did not use: %s" % ", ".join(self.changed_files()))
        code = self.exit_code()
        lines.append("result: %s (exit %d)" % ({0: "PASS", 1: "FAIL", 2: "REFUSED"}[code], code))
        return lines

    def print_rows(self, file=None, width=None):
        width = width or max([len(r["case"]) for r in self.rows] + [10])
        for r in self.rows:
            print("%-*s  %-17s %s" % (width, r["case"], r["class"], r["detail"]),
                  file=file or sys.stdout)

    def print_footer(self, file=None):
        print("\n".join(self.footer_lines()), file=file or sys.stdout, flush=True)
        path = os.environ.get("COMPARE_COMMON_SELFTEST_REPORT")
        if path and selftest_active():
            self.write_json(path)

    def exit_code(self):
        if self.changed_files():
            return 2
        d = self.denominator()
        if not self.rows or d["compared"] == 0 or self.reclassify_only:
            return 1
        if any(a["abnormal"] for a in self.aux) or self.problems:
            return 1
        return 0 if all(r["class"] in PASSING for r in self.rows) else 1

    def to_json(self):
        return {"provenance": self.prov, "denominator": self.denominator(),
                "exit_code": self.exit_code(), "classes": CLASS_HELP, "cases": self.rows,
                "exclusions": self.exclusion_rows(), "auxiliary": self.aux,
                "notes": self.notes, "problems": self.problems,
                "reclassify_only": self.reclassify_only,
                "footer": self.footer_lines()}

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
    print("%s: error: the selection matches no %s; nothing compared" % (tool, what),
          file=sys.stderr)
    return 1


def refuse(tool, e):
    print("%s: refused: %s" % (tool, e), file=sys.stderr)
    return 2

# ---------------------------------------------------------------------------------------------
# Selftest: a tool's real command line against fake binaries.


def fake_script(out=b"", err=b"", code=0, sig=None, alloc_mb=0, token=None, tripfile=None,
                info=None, role="fake", when=()):
    """Text of a fake binary. It ignores its arguments except the command (the first argument
    that does not start with '-'): `info` prints `info` (default nothing), `shutdown`, `clean`
    and `version` print nothing and exit 0; any other command prints out and err, then exits
    with code or kills itself with sig. With token, it refuses to run (exit 99, appends its
    argv to tripfile) unless COMPARE_COMMON_TRIPWIRE equals token, i.e. unless run() started
    it."""
    def q(b):
        return shlex.quote(b.decode("latin-1") if isinstance(b, bytes) else b)
    lines = ["#!/bin/bash", "# compare_common fake binary: %s" % role]
    if token:
        lines.append('[ "$COMPARE_COMMON_TRIPWIRE" = %s ] || { echo "$0 $*" >> %s; '
                     'echo "TRIPWIRE: started outside compare_common.run" >&2; exit 99; }'
                     % (q(token), q(tripfile)))
    lines += ['cmd=""', 'for a in "$@"; do case "$a" in -*) ;; *) cmd="$a"; break;; esac; done',
              'case "$cmd" in info) printf %%s %s; exit 0;; shutdown|clean|version) exit 0;; esac'
              % q(info or b"")]
    lines += ["printf %%s %s" % q(out), "printf %%s %s >&2" % q(err)]
    for needle, wcode in when:
        # A tool's declared failure case: exit wcode when an argument contains needle.
        lines.append('case " $* " in *%s*) exit %d;; esac' % (shlex.quote(needle)[1:-1]
                                                             if needle.isprintable() else needle,
                                                             wcode))
    if alloc_mb:
        lines.append("/usr/bin/python3 -c 'x = bytearray(%d * 1024 * 1024); x[::4096] = "
                     "b\"1\" * len(x[::4096])'" % alloc_mb)
    if sig:
        lines.append("kill -%s $$" % sig)
    lines.append("exit %d" % code)
    return "\n".join(lines) + "\n"


def make_fake(path, **kw):
    with open(path, "w") as f:
        f.write(fake_script(**kw))
    os.chmod(path, 0o755)
    return path


SAME_ERR = b"INFO: Analyzed 1 target.\nINFO: Elapsed time: 1.234s\n"
# name, oracle fake, blaze fake, check(exit code, classes) -> bool, what must hold
SCENARIOS = [
    ("equal", dict(out=b"same\n", err=SAME_ERR), dict(out=b"same\n", err=SAME_ERR),
     lambda rc, cl: rc == 0 and cl and set(cl) == {IDENTICAL}, "exit 0, every row IDENTICAL"),
    ("timing-only", dict(out=b"same\n", err=SAME_ERR),
     dict(out=b"same\n", err=SAME_ERR.replace(b"1.234s", b"9.876s")),
     lambda rc, cl: rc == 0 and cl and set(cl) == {IDENTICAL},
     "an `Elapsed time` difference is a volatile rule: exit 0"),
    ("stdout-differs", dict(out=b"same\n", err=SAME_ERR), dict(out=b"other\n", err=SAME_ERR),
     lambda rc, cl: rc == 1 and DIFFERENT in cl, "exit 1, a row DIFFERENT"),
    ("stderr-extra-line", dict(out=b"same\n", err=SAME_ERR),
     dict(out=b"same\n", err=SAME_ERR + b"INFO: one more line\n"),
     lambda rc, cl: rc == 1 and DIFFERENT in cl,
     "an unfiltered stderr line: exit 1, a row DIFFERENT"),
    ("equal-failures", dict(out=b"x\n", err=b"ERROR: boom\n", code=1),
     dict(out=b"x\n", err=b"ERROR: boom\n", code=1),
     lambda rc, cl: rc == 1 and BOTH_FAILED in cl and IDENTICAL not in cl,
     "exit 1, BOTH-FAILED, nothing IDENTICAL"),
    ("blaze-sigpipe", dict(out=b"same\n", err=SAME_ERR), dict(out=b"same\n", err=SAME_ERR,
                                                              sig="PIPE"),
     lambda rc, cl: rc == 1 and DIFFERENT in cl, "exit 1, DIFFERENT"),
    ("blaze-sigterm", dict(out=b"same\n", err=SAME_ERR), dict(out=b"same\n", err=SAME_ERR,
                                                              sig="TERM"),
     lambda rc, cl: rc == 1 and DIFFERENT in cl, "exit 1, DIFFERENT"),
    ("both-sigpipe", dict(out=b"same\n", sig="PIPE"), dict(out=b"same\n", sig="PIPE"),
     lambda rc, cl: rc == 1 and BOTH_FAILED in cl and IDENTICAL not in cl,
     "exit 1, BOTH-FAILED"),
    ("blaze-unported", dict(out=b"same\n", err=SAME_ERR),
     dict(out=b"same\n", err=b"blaze: not ported yet: Foo.bar\n", code=37),
     lambda rc, cl: rc == 1 and (NOT_PORTED in cl or DIFFERENT in cl),
     "exit 1, NOT-PORTED or DIFFERENT"),
]


class Selftest:
    def __init__(self, tool):
        self.tool = tool
        self.failures = 0
        self.n = 0

    def check(self, name, ok, detail=""):
        self.n += 1
        if not ok:
            self.failures += 1
        print("  %-4s %-48s %s" % ("ok" if ok else "FAIL", name, detail), flush=True)

    def finish(self):
        print("%s selftest: %d checks, %d failed: %s" % (
            self.tool, self.n, self.failures, "PASS" if not self.failures else "FAIL"))
        return 0 if not self.failures else 1


class FakeHome:
    """A fake HOME for a tool's real command line: ~/.cache/blaze/{oracle,corpus,repo-cache,
    work/repos} with the read-only corpus and caches linked in, the token file, a tripwire."""

    def __init__(self, tool, links=("corpus", "repo-cache", "work/repos"), setup=None):
        os.makedirs(WORK, exist_ok=True)
        self.root = tempfile.mkdtemp(prefix="selftest-%s-" % tool.replace("/", "-"), dir=WORK)
        self.home = os.path.join(self.root, "home")
        self.cache = os.path.join(self.home, ".cache", "blaze")
        os.makedirs(os.path.join(self.cache, "oracle"))
        os.makedirs(os.path.join(self.cache, "work"))
        os.makedirs(os.path.join(self.home, "bin"))
        for rel in links:
            src = os.path.join(CACHE, rel)
            if os.path.exists(src):
                dst = os.path.join(self.cache, rel)
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                os.symlink(src, dst)
        self.token = uuid.uuid4().hex
        with open(os.path.join(self.home, SELFTEST_TOKEN_FILE), "w") as f:
            f.write(self.token)
        self.tripfile = os.path.join(self.root, "TRIPPED")
        self.oracle = os.path.join(self.cache, "oracle", os.path.basename(PINNED_ORACLE))
        self.n = 0
        if setup:
            setup(self)

    def fakes(self, ospec, bspec, when=()):
        self.n += 1
        if when:
            ospec, bspec = dict(ospec, when=when), dict(bspec, when=when)
        make_fake(self.oracle, token=self.token, tripfile=self.tripfile, role="oracle", **ospec)
        blaze = os.path.join(self.home, "bin", "blaze-%d" % self.n)
        make_fake(blaze, token=self.token, tripfile=self.tripfile, role="blaze-%d" % self.n,
                  **bspec)
        return self.oracle, blaze

    def run_tool(self, tool_path, argv, pin=None, timeout=900):
        """(exit code, report json or None, log path) of python3 <tool> <argv>."""
        report = os.path.join(self.root, "report-%d.json" % self.n)
        if os.path.exists(report):
            os.remove(report)
        env = dict(os.environ)
        env.update({"HOME": self.home, "COMPARE_COMMON_SELFTEST": self.token,
                    "COMPARE_COMMON_SELFTEST_REPORT": report,
                    "COMPARE_COMMON_SELFTEST_PIN": pin or sha256_file(self.oracle)})
        env.pop("COMPARE_COMMON_TRIPWIRE", None)
        log = os.path.join(self.root, "log-%d-%d.txt" % (self.n, int(time.time() * 1000)))
        cmd = [tool_path]  # through its own shebang, as from the command line
        with open(log, "wb") as f:
            try:
                rc = subprocess.run(cmd + list(argv), cwd=self.root, env=env, stdout=f,
                                    stderr=subprocess.STDOUT, timeout=timeout,
                                    stdin=subprocess.DEVNULL).returncode
            except subprocess.TimeoutExpired:
                rc = "timeout"
        rep = None
        if os.path.exists(report):
            with open(report) as f:
                rep = json.load(f)
        return rc, rep, log

    def tripped(self):
        return os.path.exists(self.tripfile)

    def remove(self):
        subprocess.run(["chmod", "-R", "u+w", self.root], stderr=subprocess.DEVNULL)
        shutil.rmtree(self.root, ignore_errors=True)


def _classes(rep):
    return [r["class"] for r in (rep or {}).get("cases", [])]


def itemised(scenarios=None):
    """The scenarios for bytes/compare only, whose rows are files both runs wrote (equal content
    items stay IDENTICAL when both runs fail): failure scenarios require exit 1 and a BOTH-FAILED
    row. Not for a tool whose rows are derived from a stream a killed command may have cut short
    (audit 4r2, B5: interp-diff uses the strict SCENARIOS)."""
    out = []
    for name, o, b, ok, what in scenarios or SCENARIOS:
        if name in ("equal-failures", "both-sigpipe"):
            ok, what = (lambda rc, cl: rc == 1 and BOTH_FAILED in cl), "exit 1, a BOTH-FAILED row"
        out.append((name, o, b, ok, what))
    return out


def selftest_tool(tool, tool_path, argv, empty=None, reuse=None, oracle_outputs=None,
                  stored=None, setup=None, scenarios=None, extra=None, keep=False, when=(),
                  fake_extra=None):
    """Drives the tool's real command line with fake binaries.

    argv(blaze, label) -> argv of one normal run (oracle at the default pinned path).
    empty(blaze, label) -> argv whose selection matches nothing.
    reuse(blaze, label) -> argv that reuses the oracle outputs of the run with the same label.
    oracle_outputs(home, label) -> stored oracle output files of that run (to tamper with).
    stored(blaze, label) -> argv that judges stored Blaze captures, run with a binary that did
                            not make them.
    extra(st, fh, run) -> more checks; run(argv, pin=None) -> (rc, report, log).
    """
    st = Selftest(tool)
    fh = FakeHome(tool, setup=setup)
    print("%s selftest: the real command line against fake binaries (fake HOME %s)" % (
        tool, fh.home))

    def run(a, pin=None):
        return fh.run_tool(tool_path, a, pin=pin)

    def show(rc, rep, log):
        if rep is None:
            return "exit %s, no report (log %s)" % (rc, log)
        return "exit %s, %s" % (rc, ", ".join("%s %d" % (k, v) for k, v in
                                              rep["denominator"].items() if v))
    try:
        # Tripwire positive control: a fake started without the launcher trips.
        o, b = fh.fakes(dict(out=b"x"), dict(out=b"x"))
        subprocess.run([b, "build"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        st.check("tripwire trips on a direct launch", fh.tripped())
        if fh.tripped():
            os.remove(fh.tripfile)
        for i, (name, ospec, bspec, ok, what) in enumerate(scenarios or SCENARIOS):
            if fake_extra:
                fx = fake_extra() if callable(fake_extra) else fake_extra
                ospec, bspec = dict(ospec, **fx), dict(bspec, **fx)
            o, b = fh.fakes(ospec, bspec, when)
            label = "selftest-%s" % name
            rc, rep, log = run(argv(b, label))
            st.check("scenario %s" % name, ok(rc, _classes(rep)), "%s; want %s" % (
                show(rc, rep, log), what))
            if name == "timing-only" and rep is not None:
                ex = {e["name"]: e for e in rep.get("exclusions", [])}
                st.check("timing-only: the elapsed rule is counted",
                         ex.get("elapsed", {}).get("blaze", 0) > 0,
                         "elapsed %s" % ex.get("elapsed"))
            if name == "equal" and reuse is not None:
                rc2, rep2, log2 = run(reuse(b, label))
                st.check("reuse of fresh oracle outputs accepted", rc2 == 0,
                         show(rc2, rep2, log2))
                files = [p for p in (oracle_outputs(fh.home, label) if oracle_outputs else [])
                         if os.path.isfile(p)]
                if files:
                    with open(files[0], "ab") as f:
                        f.write(b"tampered\n")
                    rc3, rep3, log3 = run(reuse(b, label))
                    st.check("reuse refused after an oracle output changed", rc3 not in (0,),
                             "%s (tampered %s)" % (show(rc3, rep3, log3),
                                                   os.path.relpath(files[0], fh.home)))
                else:
                    st.check("reuse: oracle outputs found to tamper with", False,
                             "none listed")
                rc4, rep4, log4 = run(reuse(b, "selftest-never-recorded"))
                st.check("reuse refused without a recorded key", rc4 not in (0,),
                         show(rc4, rep4, log4))
            if name == "equal" and stored is not None:
                other = os.path.join(fh.home, "bin", "blaze-other")
                make_fake(other, token=fh.token, tripfile=fh.tripfile, out=b"same\n",
                          err=SAME_ERR, role="another blaze")
                rc5, rep5, log5 = run(stored(other, label))
                st.check("stored Blaze capture not judged for another binary", rc5 != 0,
                         show(rc5, rep5, log5))
        o, b = fh.fakes(dict(out=b"same\n"), dict(out=b"same\n"))
        rc, rep, log = run(argv(b, "selftest-wrong-oracle"), pin="0" * 64)
        st.check("wrong oracle hash refused (exit 2)", rc == 2, show(rc, rep, log))
        rc, rep, log = run(argv(o, "selftest-oracle-as-blaze"))
        st.check("the oracle as the binary under test refused (exit 2)", rc == 2,
                 show(rc, rep, log))
        if empty is not None:
            rc, rep, log = run(empty(b, "selftest-empty"))
            st.check("empty selection fails", rc not in (0, None), show(rc, rep, log))
        if extra is not None:
            extra(st, fh, run)
        st.check("no process started outside the launcher", not fh.tripped(),
                 open(fh.tripfile).read()[:200] if fh.tripped() else "")
    except Exception as e:  # noqa: BLE001 - a crash of the selftest is a failure
        import traceback
        traceback.print_exc()
        st.check("selftest ran to the end", False, "%s: %s" % (type(e).__name__, e))
    finally:
        if keep or st.failures:
            print("  (fake HOME kept for inspection: %s)" % fh.root)
        else:
            fh.remove()
    return st.finish()


def selftest_module():
    """The module's own guards: launcher, rules, keys, captures, classes, exit rule."""
    st = Selftest("compare_common")
    os.makedirs(WORK, exist_ok=True)
    d = tempfile.mkdtemp(prefix="selftest-compare_common-", dir=WORK)
    try:
        for mode in ("scope", "service"):
            for sig in ("PIPE", "TERM"):
                f = make_fake(os.path.join(d, "sig" + sig), out=b"x", sig=sig)
                r = run([f, "build"], d, mode=mode, tag="selftest")
                st.check("launcher records SIG%s (%s)" % (sig, mode),
                         r.term.kind == "signal" and r.term.sig == "SIG" + sig, str(r.term))
            f = make_fake(os.path.join(d, "exit3"), out=b"o", err=b"e", code=3)
            r = run([f, "build"], d, mode=mode, tag="selftest")
            st.check("launcher records exit 3 and output (%s)" % mode,
                     r.term == Termination("exit", 3) and r.out == b"o" and r.err == b"e",
                     "%s %r %r" % (r.term, r.out, r.err))
            f = make_fake(os.path.join(d, "oom"), alloc_mb=256)
            r = run([f, "build"], d, mode=mode, memory_max="64M", tag="selftest")
            st.check("launcher records an OOM kill (%s)" % mode, r.term.kind == "oom-kill",
                     str(r.term))
            f = os.path.join(d, "sleep")
            with open(f, "w") as fh:
                fh.write("#!/bin/bash\nsleep 30\n")
            os.chmod(f, 0o755)
            r = run([f], d, mode=mode, timeout=2, tag="selftest")
            st.check("launcher records a timeout (%s)" % mode, r.term.kind == "timeout",
                     str(r.term))
            f = os.path.join(d, "env")
            with open(f, "w") as fh:
                fh.write("#!/bin/bash\nenv | sort\n")
            os.chmod(f, 0o755)
            r = run([f], d, mode=mode, env={"X": "1"}, tag="selftest")
            names = sorted(l.split("=", 1)[0] for l in r.out.decode().splitlines())
            st.check("clean environment (%s)" % mode,
                     set(names) <= {"HOME", "PATH", "X", "PWD", "SHLVL", "_", "OLDPWD"},
                     " ".join(names))
        try:
            require_oracle(make_fake(os.path.join(d, "wrong-oracle"), out=b"x"))
            st.check("wrong oracle hash refused", False, "accepted")
        except Refusal:
            st.check("wrong oracle hash refused", True)
        # Rules.
        n = Normaliser(VOLATILE_RULES + B1_GAP_RULES)
        eq, gaps = n.compare("INFO: Elapsed time: 1.0s\nx\n", "INFO: Elapsed time: 2.0s\nx\n",
                             "stderr")
        st.check("volatile rule masks a time", eq and not gaps)
        eq, gaps = n.compare("x\n", "x\nINFO: extra\n", "stderr")
        st.check("an extra unfiltered line differs", not eq)
        eq, gaps = n.compare("INFO: 1 process: 1 internal.\n",
                             "Starting local Bazel server\nINFO: 1 process: 6 action cache hit, "
                             "1 internal.\n", "stderr")
        st.check("gap rules: equal main text, gaps named", eq and gaps == [
            "b1_processes", "b1_server_start"], str(gaps))
        st.check("rule counts per side", n.counts["elapsed"] == {"oracle": 1, "blaze": 1},
                 str(n.counts.get("elapsed")))
        # B3 (audit 4r2): the Found line is compared in place, by no gap rule.
        found = "INFO: Found 1 target...\n"
        eq, gaps = n.compare("INFO: Analyzed 1 target.\nERROR: build failed\n",
                             "INFO: Analyzed 1 target.\n" + found + "ERROR: build failed\n",
                             "stderr")
        st.check("B3: a Found line only Blaze prints differs (no gap)", not eq and not gaps,
                 "equal %s, gaps %s" % (eq, gaps))
        eq, gaps = n.compare("INFO: Analyzed 1 target.\n" + found + "x\n",
                             found + "INFO: Analyzed 1 target.\nx\n", "stderr")
        st.check("B3: a Found line at another position differs", not eq, "equal %s" % eq)
        # Keys.
        out1 = os.path.join(d, "out1")
        with open(out1, "w") as f:
            f.write("a")
        key = cache_key(None, "o", ["a"], {}, "in1")
        kp = os.path.join(d, "key.json")
        write_key(kp, key, [out1])
        st.check("cache: same key and outputs accepted", check_key(kp, key) is None)
        st.check("cache: other input refused",
                 check_key(kp, cache_key(None, "o", ["a"], {}, "in2")) is not None)
        with open(out1, "a") as f:
            f.write("b")
        st.check("cache: changed output refused", check_key(kp, key) is not None)
        write_key(kp, key, [])
        st.check("cache: a key anchoring no outputs refused", check_key(kp, key) is not None)
        st.check("cache: missing key refused", check_key(kp + ".none", key) is not None)
        cp = os.path.join(d, "capture.json")
        write_capture(cp, {"path": "/b", "sha256": "1" * 64}, [out1])
        st.check("capture: the named binary accepted",
                 check_capture(cp, {"sha256": "1" * 64}) is None)
        st.check("capture: another binary refused",
                 check_capture(cp, {"sha256": "2" * 64}) is not None)
        st.check("capture: no binary named refused", check_capture(cp, None) is not None)
        write_capture(cp, {"path": "/b", "sha256": "1" * 64}, [])
        st.check("M5: a capture anchoring no outputs refused",
                 check_capture(cp, {"sha256": "1" * 64}) is not None)
        # M1: an OOM kill of a server left in an earlier command's scope, during a later
        # command, is that later command's termination.
        f = os.path.join(d, "daemonize")
        with open(f, "w") as fh:
            fh.write("#!/bin/bash\n( sleep 3; /usr/bin/python3 -c 'x = bytearray(256 * 1024 * "
                     "1024); x[::4096] = b\"1\" * len(x[::4096])' ) </dev/null >/dev/null "
                     "2>&1 &\nexit 0\n")
        os.chmod(f, 0o755)
        r1 = run([f], d, mode="scope", memory_max="64M", tag="selftest")
        g = os.path.join(d, "later")
        with open(g, "w") as fh:
            fh.write("#!/bin/bash\nsleep 8\nexit 0\n")
        os.chmod(g, 0o755)
        r2 = run([g], d, mode="scope", tag="selftest")
        st.check("M1: the first command itself ended normally", r1.term == Termination("exit", 0),
                 str(r1.term))
        st.check("M1: an OOM kill in an earlier command's scope is the later command's",
                 r2.term.kind == "oom-kill", str(r2.term))
        # T4: a pid file naming a process that is not a server of the root is left alone.
        root = os.path.join(d, "ur")
        os.makedirs(os.path.join(root, "ob", "server"))
        sleeper = subprocess.Popen(["sleep", "30"])
        with open(os.path.join(root, "ob", "server", "server.pid.txt"), "w") as fh:
            fh.write("%d\n" % sleeper.pid)
        res = kill_root_servers(root, wait=1)
        st.check("T4: a pid that is not a server of the root is not signalled",
                 sleeper.poll() is None and "left alone" in str(res), str(res))
        sleeper.kill()
        sleeper.wait()
        owned = subprocess.Popen(["/bin/bash", "-c", "exec -a server-of-%s sleep 30" % root])
        time.sleep(0.3)
        with open(os.path.join(root, "ob", "server", "server.pid.txt"), "w") as fh:
            fh.write("%d\n" % owned.pid)
        res = kill_root_servers(root, wait=2)
        owned.wait()
        st.check("T4: a server of the root is ended", "ended after" in str(res), str(res))
        # Classes and exit rule.
        st.check("empty report fails", Report("selftest").exit_code() != 0)
        for cls in CLASSES:
            rep = Report("selftest")
            rep.add("a", IDENTICAL)
            rep.add("b", cls)
            st.check("report with one %s %s" % (cls, "passes" if cls in PASSING else "fails"),
                     (rep.exit_code() == 0) == (cls in PASSING))
        rep = Report("selftest")
        rep.add("a", IDENTICAL)
        rep.problem("a side runner died")
        st.check("a problem outside the rows fails the run", rep.exit_code() == 1)
        rep = Report("selftest")
        rep.add("a", IDENTICAL)
        rep.reclassify_only = True
        st.check("re-classification only never passes", rep.exit_code() != 0)
        rep = Report("selftest")
        rep.add("a", IDENTICAL)
        rep.add_aux("shutdown", Termination("signal", 141, "SIGPIPE"))
        st.check("an auxiliary command killed by a signal fails the run", rep.exit_code() != 0)
        st.check("judge: gap differences are KNOWN-GAP",
                 judge(Termination("exit", 0), Termination("exit", 0), [],
                       gaps=["b1_server_start"])[0] == KNOWN_GAP)
        st.check("judge: a reduced comparison is IDENTICAL-REDUCED",
                 judge(Termination("exit", 0), Termination("exit", 0), [],
                       reduced="stderr not compared")[0] == REDUCED)
        st.check("judge: declared failure matches",
                 judge(Termination("exit", 2), Termination("exit", 2), [], declared_exit=2)[0]
                 == IDENTICAL)
        st.check("judge: declaration the oracle contradicts",
                 judge(Termination("exit", 1), Termination("exit", 1), [], declared_exit=2)[0]
                 == ORACLE_UNEXPECTED)
    finally:
        shutil.rmtree(d, ignore_errors=True)
    return st.finish()

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
    if cmd == "check-blaze":
        try:
            b = blaze_identity(rest[0])
        except (Refusal, IndexError) as e:
            print("refused: %s" % e, file=sys.stderr)
            return 2
        print("%s sha256 %s" % (b["path"], b["sha256"]))
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
        return selftest_module()
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(_cli(sys.argv[1:]))
