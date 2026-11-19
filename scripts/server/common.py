"""Shared by the tools of scripts/server/ (interop-client, interop-server, seq).

Standard library only. Not a tool: `python3 scripts/server/common.py help` prints this text.

What lives here, so the tools agree on it (audit 3, "Batch B", rules for comparison tools):
  * The oracle is accepted only by its pinned sha256 (UPSTREAM.json "oracle_sha256");
    `verified_oracle()` refuses any other file.
  * `header()`: every report starts with the sha256 of each binary involved, the Blaze checkout's
    commit and dirty state (scripts/oracle/oracle_common.repo_state) and whether the Blaze
    binary can come from that commit (oracle_common.binary_origin).
  * `Status`: the true termination of a process: `exit N`, `signal N NAME`, plus a separate
    `timed out` flag (never a sentinel exit code).
  * `in_unit()`: the tool re-executes itself as the main process of
    `systemd-run --user --collect --wait --pipe -p MemoryMax=12G -p MemorySwapMax=0` under
    scripts/remote/unit-supervisor, which records the tool's own status inside the unit
    (a bare `systemd-run --wait` reports 0 for a death by SIGTERM, SIGINT, SIGHUP, SIGPIPE).
  * `oom_kills()`: the oom_kill counter of the unit's cgroup, read before and after each step.
  * `Norm`: the transcript normaliser of the interop tools (rules in Norm.RULES).
  * Server cleanup through <output_user_root>/*/server/server.pid.txt (`server_pids`,
    `kill_root_servers`): a pid is only ever signalled when it is alive and its command line
    names that output user root.
  * `safe_rmtree(path, root)`: deletes `path` only when it lies strictly inside `root` and
    `root` lies inside ~/.cache/blaze/work/server/.
  * Step classes: IDENTICAL (equal, and every exit was 0), SAME-FAILURE (equal, and some exit
    was not 0, or a side recorded a failure: never counted as a match), DIFFERENT, NOT-RUN.
"""

import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time

sys.dont_write_bytecode = True  # never write .pyc files into scripts/ (scripts/oracle is protected)

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
HOME = os.path.expanduser("~")
CACHE = os.path.join(HOME, ".cache", "blaze")
SERVER_WORK = os.path.join(CACHE, "work", "server")
TOOLS_WORK = os.path.join(SERVER_WORK, "tools")
REPO_CACHE = os.path.join(CACHE, "repo-cache")
ORACLE_ROOT = os.path.realpath(os.path.join(CACHE, "oracle-out", "ur"))
SUPERVISOR = os.path.join(REPO, "scripts", "remote", "unit-supervisor")

sys.path.insert(0, os.path.join(REPO, "scripts", "oracle"))
import oracle_common as oc  # noqa: E402  (imported, not edited)
import normalise as onorm  # noqa: E402
import surfaces as osurf  # noqa: E402
sys.path.pop(0)

with open(os.path.join(REPO, "UPSTREAM.json")) as _f:
    _UPSTREAM = json.load(_f)
_PIN = _UPSTREAM["pins"][_UPSTREAM["current"]]
ORACLE = os.path.expanduser(_PIN["oracle_binary"])
ORACLE_SHA256 = _PIN["oracle_sha256"]
ORACLE_INSTALL_KEY = _PIN["install_base_key"]
ORACLE_INSTALL_BASE = os.path.join(ORACLE_ROOT, "install", ORACLE_INSTALL_KEY)


def die(tool, msg, code=2):
    sys.stderr.write("%s: %s\n" % (tool, msg))
    sys.exit(code)


_SHA_CACHE = {}


def sha256_file(path):
    real = os.path.realpath(path)
    st = os.stat(real)
    key = (real, st.st_size, st.st_mtime_ns)
    if key not in _SHA_CACHE:
        _SHA_CACHE[key] = oc.sha256_file(real)
    return _SHA_CACHE[key]


def verified_oracle(tool, path=None):
    """The oracle binary, after checking its sha256 against the pin; exits otherwise."""
    path = os.path.abspath(os.path.expanduser(path or ORACLE))
    if not os.path.isfile(path):
        die(tool, "no oracle binary at %s" % path)
    got = sha256_file(path)
    if got != ORACLE_SHA256:
        die(tool, "refused: %s has sha256 %s, the pinned oracle (UPSTREAM.json) is %s"
            % (path, got, ORACLE_SHA256))
    return path


def blaze_binary(path=None):
    return os.path.abspath(path or os.environ.get("BLAZE") or os.path.join(REPO, "target/debug/blaze"))


def stage_binary(tool, path):
    """A private copy of a Blaze binary, TOOLS_WORK/bin/<sha256 prefix>-<pid>/blaze, so a rebuild
    of `path` by another agent cannot change a running comparison (and no other run deletes
    it). The copy's sha256 is checked against the original's. Returns (staged path, sha256)."""
    if not os.access(path, os.X_OK):
        die(tool, "not an executable: %s" % path)
    sha = sha256_file(path)
    d = os.path.join(TOOLS_WORK, "bin", "%s-%d" % (sha[:16], os.getpid()))
    dst = os.path.join(d, "blaze")
    os.makedirs(d, exist_ok=True)
    shutil.copy2(path, dst)
    if oc.sha256_file(dst) != sha:
        unstage_binary(dst)
        die(tool, "%s changed while it was being copied; rerun" % path)
    return dst, sha


def unstage_binary(staged):
    """Removes a staged copy made by stage_binary (only inside TOOLS_WORK/bin)."""
    d = os.path.dirname(staged)
    if os.path.dirname(d) == os.path.join(TOOLS_WORK, "bin") and os.path.isdir(d):
        safe_rmtree(d, os.path.join(TOOLS_WORK, "bin"))


def header(tool, binaries, blaze=None):
    """Lines that name what a report is about: each binary's path and sha256, the checkout's
    commit and dirty state, and (for a Blaze binary) whether it can come from that commit."""
    lines = []
    for role, path, sha in binaries:
        lines.append("%s: %s binary %s sha256 %s" % (tool, role, path, sha))
    st = oc.repo_state(REPO)
    lines.append("%s: checkout %s commit %s dirty %s%s" % (
        tool, REPO, st["commit"], st["dirty"],
        (" (%d paths, diff sha256 %s)" % (st["dirty_paths"], st["dirty_diff_sha256"]))
        if st["dirty"] else ""))
    if blaze:
        origin = oc.binary_origin(blaze, REPO)
        lines.append("%s: blaze binary commit_verified %s%s" % (
            tool, origin["commit_verified"],
            ("" if origin["commit_verified"] else " (" + "; ".join(origin["reasons"]) + ")")))
    lines.append("%s: tool sha256 %s (scripts/server/*: %s)" % (tool, tools_sha256(), ", ".join(
        sorted(f for f in os.listdir(HERE) if not f.startswith(".") and f != "__pycache__"))))
    return lines


def tools_sha256(files=None):
    """sha256 over tool files (default: every file of scripts/server) plus the scripts/oracle
    files these tools import and unit-supervisor."""
    h = hashlib.sha256()
    if files is None:
        files = [f for f in os.listdir(HERE)
                 if os.path.isfile(os.path.join(HERE, f)) and not f.endswith(".pyc")]
    paths = sorted(os.path.join(HERE, f) for f in files)
    paths += [os.path.join(REPO, "scripts/oracle", f) for f in
              ("oracle_common.py", "normalise.py", "surfaces.py", "record")]
    paths += [SUPERVISOR]
    for f in paths:
        h.update(os.path.relpath(f, REPO).encode() + b"\0" + oc.sha256_file(f).encode() + b"\n")
    return h.hexdigest()


# ---------------------------------------------------------------------------------------------
# Process status.

class Status:
    """How a process ended: exit code, or signal; `timed_out` is a separate fact."""

    def __init__(self, returncode, timed_out=False):
        self.returncode = returncode
        self.timed_out = timed_out

    @property
    def ok(self):
        return self.returncode == 0 and not self.timed_out

    def __str__(self):
        rc = self.returncode
        if rc is None:
            s = "not run"
        elif rc < 0:
            try:
                name = signal.Signals(-rc).name
            except ValueError:
                name = "?"
            s = "signal %d %s" % (-rc, name)
        else:
            s = "exit %d" % rc
        if self.timed_out:
            s += " (timed out; killed by the tool)"
        return s


def oom_kills():
    """oom_kill count of this process's cgroup (the systemd unit), or None."""
    try:
        with open("/proc/self/cgroup") as f:
            rel = [l.split("::", 1)[1].strip() for l in f if l.startswith("0::")][0]
        with open("/sys/fs/cgroup%s/memory.events" % rel) as f:
            for line in f:
                k, v = line.split()
                if k == "oom_kill":
                    return int(v)
    except (OSError, IndexError, ValueError):
        return None
    return None


UNIT_ENV = "BLAZE_SERVER_TOOL_UNIT"


def in_unit(tool, argv, status_dir, memory_max="12G"):
    """Re-executes `argv` (this tool) as the main process of a systemd unit under
    unit-supervisor, unless already inside one; returns the exit code to use, or None when the
    caller is already in its unit and should do the work."""
    if os.environ.get(UNIT_ENV) == "1":
        return None
    os.makedirs(status_dir, exist_ok=True)
    status = os.path.join(status_dir, "unit-status-%d" % os.getpid())
    cmd = ["systemd-run", "--user", "--collect", "--wait", "--pipe", "--quiet", "--same-dir",
           "-p", "MemoryMax=" + memory_max, "-p", "MemorySwapMax=0",
           "/usr/bin/python3", "-I", SUPERVISOR, status,
           "env", "-i", "HOME=" + HOME, "PATH=/usr/bin:/bin", UNIT_ENV + "=1"]
    if "BLAZE" in os.environ:
        cmd.append("BLAZE=" + os.environ["BLAZE"])
    cmd += ["/usr/bin/python3", "-I"] + list(argv)
    rc = subprocess.call(cmd, stdin=subprocess.DEVNULL)
    try:
        words = open(status).read().split()
        os.remove(status)
    except OSError:
        words = []
    if words[:1] == ["exit"]:
        code = int(words[1])
    elif words[:1] == ["signal"]:
        sys.stderr.write("%s: the tool was killed by %s inside its unit\n" % (tool, words[2]))
        code = 128 + int(words[1])
    else:
        sys.stderr.write("%s: no status recorded inside the unit (unit killed? memory limit?); "
                         "systemd-run returned %d, reported as 3\n" % (tool, rc))
        code = 3
    return code


# ---------------------------------------------------------------------------------------------
# Servers of an output user root.

def server_pid_files(ur):
    out = []
    if os.path.isdir(ur):
        for d in sorted(os.listdir(ur)):
            f = os.path.join(ur, d, "server", "server.pid.txt")
            if os.path.exists(f):
                out.append(f)
    return out


def read_pid(pidfile):
    try:
        with open(pidfile) as f:
            return int(f.read().strip())
    except (OSError, ValueError):
        return None


def server_pid(ur):
    for f in server_pid_files(ur):
        pid = read_pid(f)
        if pid is not None:
            return pid
    return None


def alive(pid):
    if not pid:
        return False
    try:
        with open("/proc/%d/stat" % pid) as fh:
            state = fh.read().rsplit(")", 1)[1].split()[0]
        return state != "Z"
    except (OSError, IndexError):
        return False


def owned_by_root(pid, ur):
    """pid is alive and its command line names the output user root `ur` (so it is a server of
    that root, not an unrelated process that reused the pid)."""
    if not alive(pid):
        return False
    try:
        with open("/proc/%d/cmdline" % pid, "rb") as f:
            cmdline = f.read()
    except OSError:
        return False
    return os.fsencode(os.path.realpath(ur)) in cmdline or os.fsencode(ur) in cmdline


def wait_gone(pid, seconds):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if not alive(pid):
            return True
        time.sleep(0.1)
    return not alive(pid)


def kill_root_servers(ur, shutdown=None, log=None):
    """Shuts down (with `shutdown(output_base)`, a callable running the side's own client) and,
    failing that, SIGKILLs every server of output user root `ur`. Returns the pids still alive."""
    left = []
    for f in server_pid_files(ur):
        pid = read_pid(f)
        if pid is None or not owned_by_root(pid, ur):
            continue
        if shutdown is not None:
            try:
                shutdown(os.path.dirname(os.path.dirname(f)))
            except Exception as e:  # noqa: BLE001 - fall back to the kill
                if log:
                    log("shutdown of %d failed: %s" % (pid, e))
            wait_gone(pid, 10)
        if owned_by_root(pid, ur):
            for kill in (lambda: os.killpg(pid, signal.SIGKILL), lambda: os.kill(pid, signal.SIGKILL)):
                try:
                    kill()
                except OSError:
                    pass
            wait_gone(pid, 10)
        if alive(pid):
            left.append(pid)
        if log:
            log("server %d of %s: %s" % (pid, ur, "STILL ALIVE" if alive(pid) else "gone"))
    return left


# ---------------------------------------------------------------------------------------------
# Deleting.

def safe_rmtree(path, root):
    """Deletes `path` (a directory or file) only if it lies strictly inside `root` and `root`
    lies inside ~/.cache/blaze/work/server/; refuses (raises) otherwise."""
    real_root = os.path.realpath(root)
    base = os.path.realpath(SERVER_WORK)
    if not (real_root + os.sep).startswith(base + os.sep):
        raise RuntimeError("refusing to delete under %s: not inside %s" % (root, base))
    if real_root.startswith(ORACLE_ROOT):
        raise RuntimeError("refusing to touch the oracle's output_user_root")
    parent = os.path.realpath(os.path.dirname(os.path.abspath(path)))
    target = os.path.join(parent, os.path.basename(os.path.abspath(path)))
    if not (target + os.sep).startswith(real_root + os.sep) or target == real_root:
        raise RuntimeError("refusing to delete %s: not strictly inside %s" % (path, root))
    if not os.path.lexists(target):
        return
    if os.path.isdir(target) and not os.path.islink(target):
        subprocess.run(["chmod", "-R", "u+w", target], check=False, stderr=subprocess.DEVNULL)
        shutil.rmtree(target, ignore_errors=True)
        if os.path.lexists(target):
            subprocess.run(["rm", "-rf", "--one-file-system", target], check=False)
    else:
        os.remove(target)


# ---------------------------------------------------------------------------------------------
# Transcript normalisation of the interop tools.

PROGRESS = re.compile(
    r"^(Computing main repo mapping:|Loading:|Analyzing:|Fetching |Building:|Testing:|"
    r"\[[\d,]+ / [\d,]+\])")  # the classes scripts/w4/probe drops
# Also dropped from stderr: lines whose presence depends only on timing, seen to differ between
# two oracle runs (interop-client --control, 2026-11-19): empty lines (the client's progress
# output leaves a varying number) and the client's "... still trying to connect to local Bazel
# server (N) after 10 seconds ..." under load.
TIMING_LINES = re.compile(r"^$|^\.\.\. still trying to connect to local ")
UUID = re.compile(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
DUR = re.compile(r"\b\d+(\.\d+)?m?s\b")
PIDS = [
    (re.compile(r"pid=\d+"), "pid=<PID>"),
    (re.compile(r"server \(\d+\)"), "server (<PID>)"),
    (re.compile(r"JVM process \d+"), "JVM process <PID>"),
    (re.compile(r"process \(pid=\d+\)"), "process (pid=<PID>)"),
    (re.compile(r"^server_pid: \d+$", re.M), "server_pid: <PID>"),
    # The server log's name: java.log.<host>.<user>.log.java.<YYYYMMDD-HHMMSS>.<pid>.
    (re.compile(r"(\.log\.java)\.\d{8}-\d{6}\.\d+\b"), r"\1.<TIMESTAMP>.<PID>"),
]
SANDBOX = re.compile(r"/sandbox/([a-z-]+)/\d+/")
LOG_TIME = re.compile(r"^\[([A-Z]+) \d\d:\d\d:\d\d\.\d{3} ", re.M)


class Norm:
    """The interop tools' normaliser for one side: only what must differ between two runs."""

    RULES = """\
Normalisation (only what must differ between two runs; applied to stdout, stderr and file
contents alike):
  1. the side's output user root -> <OUTPUT_USER_ROOT>; <OUTPUT_USER_ROOT>/<32 hex> (an output
     base) -> <OUTPUT_BASE>; an install base (<root>/install/<32 hex>, or the path given with
     --install_base) -> <INSTALL_BASE>; other side-specific directories the tool names
     (e.g. the server_javabase directory) -> <NAME>;
  2. process ids: `pid=N`, `server (N)`, `JVM process N`, `process (pid=N)`, the
     `server_pid: N` line of `info` -> <PID>; the time and pid in the server log's name
     (`java.log.<host>.<user>.log.java.<YYYYMMDD-HHMMSS>.<pid>`) -> <TIMESTAMP>.<PID>;
  3. durations: a number followed by s or ms (`1.23s`, `45ms`) -> <DUR>;
  4. UUIDs -> <UUID>; the time of day in the client's log prefix `[LEVEL HH:MM:SS.mmm` ->
     `[LEVEL <TIME>`;
  5. sandbox directory numbers `/sandbox/<strategy>/N/` -> `/sandbox/<strategy>/<N>/`;
  6. stderr only: the rate-limited progress lines whose number and content depend on timing
     (lines starting `Computing main repo mapping:`, `Loading:`, `Analyzing:`, `Fetching `,
     `Building:`, `Testing:`, `[n / m]`; the classes scripts/w4/probe drops) are dropped, and
     so are empty lines and `... still trying to connect to local Bazel server ...` (both
     differ between two oracle runs: interop-client --control).
Nothing else is changed or dropped: "Starting local Bazel server", "Extracting Bazel
installation", "INFO: N processes" and "INFO: Analyzed ..." lines are compared verbatim.
"""

    def __init__(self, ur, paths=()):
        self.ur = ur
        self.paths = sorted(paths, key=lambda p: -len(p[0]))  # (path, placeholder)

    def text(self, data):
        text = data.decode("latin-1") if isinstance(data, bytes) else data
        for path, ph in self.paths:
            text = text.replace(path, ph)
        text = re.sub(re.escape(self.ur) + r"/install/[0-9a-f]{32}\b", "<INSTALL_BASE>", text)
        text = re.sub(re.escape(self.ur) + r"/[0-9a-f]{32}", "<OUTPUT_BASE>", text)
        text = text.replace(self.ur, "<OUTPUT_USER_ROOT>")
        for pattern, repl in PIDS:
            text = pattern.sub(repl, text)
        text = DUR.sub("<DUR>", text)
        text = UUID.sub("<UUID>", text)
        text = LOG_TIME.sub(r"[\1 <TIME> ", text)
        text = SANDBOX.sub(r"/sandbox/\1/<N>/", text)
        if text and not text.endswith("\n"):
            text += "<no newline at end>\n"
        return text

    def stderr(self, data):
        lines = data.decode("latin-1").split("\n")
        last = lines.pop()  # "" when the data ends with a newline
        kept = [l + "\n" for l in lines if not PROGRESS.match(l) and not TIMING_LINES.match(l)]
        if last and not PROGRESS.match(last):
            kept.append(last)
        return self.text("".join(kept))


# ---------------------------------------------------------------------------------------------
# The small workspace of the interop tools.

INTEROP_TEMPLATE = {
    "MODULE.bazel": 'module(name = "interop")\n',
    "hello.sh.in": "#!/bin/sh\necho \"hi from run $*\"\necho to stderr >&2\nexit 3\n",
    "BUILD.bazel": """\
genrule(
    name = "ok",
    outs = ["ok.txt"],
    cmd = "echo hello > $@",
)

genrule(
    name = "fail",
    outs = ["fail.txt"],
    cmd = "echo failing >&2; exit 1",
)

genrule(
    name = "sleep",
    outs = ["sleep.txt"],
    cmd = "sleep 60; touch $@",
)

genrule(
    name = "sleep20",
    outs = ["sleep20.txt"],
    cmd = "sleep 20; touch $@",
)

genrule(
    name = "hello_bin",
    srcs = ["hello.sh.in"],
    outs = ["hello.sh"],
    cmd = "cp $< $@",
    executable = True,
)

genrule(
    name = "noisy",
    outs = ["noisy.txt"],
    cmd = "for i in $$(seq 1 3000); do echo \\"noisy stdout line $$i\\"; echo \\"noisy stderr line $$i\\" >&2; done; touch $@",
)
""",
}


def write_template(dest):
    os.makedirs(dest)
    for name, content in INTEROP_TEMPLATE.items():
        with open(os.path.join(dest, name), "w") as f:
            f.write(content)


# ---------------------------------------------------------------------------------------------
# Step classes.

IDENTICAL, SAME_FAILURE, DIFFERENT, NOT_RUN = "IDENTICAL", "SAME-FAILURE", "DIFFERENT", "NOT-RUN"


def classify(a_text, b_text, a_failed, b_failed):
    """Class of one compared step. a_failed/b_failed: the side's step did not succeed (non-zero
    exit, signal, timeout, or a tool failure). Equal failures are SAME-FAILURE, never a match."""
    if a_text is None or b_text is None:
        return NOT_RUN
    if a_text != b_text:
        return DIFFERENT
    return SAME_FAILURE if (a_failed or b_failed) else IDENTICAL


if __name__ == "__main__":
    print(__doc__)
    print(Norm.RULES)
