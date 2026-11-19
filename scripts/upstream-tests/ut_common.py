"""Shared paths and helpers for scripts/upstream-tests (Bazel's own shell and Python tests).

Layout (all outside the repo):
  ~/.cache/blaze/work/upstream-src            writable copy of the pinned source, patched (see PATCH)
  ~/.cache/blaze/upstream-tests/outer-root     output_user_root of the outer (harness) Bazel
  ~/.cache/blaze/upstream-tests/repo-cache     repository cache of the outer Bazel (prepare fills it)
  ~/.cache/blaze/upstream-tests/test-repo-cache  the tests' REPOSITORY_CACHE: downloads hardlinked
                                               from repo-cache, contents/ emptied before each run
  ~/.cache/blaze/upstream-tests/install-bases  shared inner install bases, one per binary sha256
  ~/.cache/blaze/upstream-tests/manifest.json  the test targets (written by `prepare`)
  ~/.cache/blaze/upstream-tests/locks/        outer.lock (+ outer.holder.json), label-<label>.lock,
                                               clients/<pid> (lock-aware processes)
  ~/.cache/blaze/upstream-test-results/<label>/ results.json, logs/, bep.json, outer.log

Concurrency. Everything under ~/.cache/blaze/upstream-tests (the outer output root and its one
Bazel server, the tests' repository cache, the install bases, the staged binaries, the patched
source copy and the manifest) is shared by every run and is touched only while holding the
exclusive outer lock (`outer_lock`). A run holds it from before it stages the binary until it
has copied its logs out of bazel-testlogs, so runs execute one at a time. A label's results
directory is guarded by its own label lock (`label_lock`, taken before the outer lock).
Waiting for either lock prints who holds it and blocks; it never fails.
"""

import datetime
import fcntl
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time

HOME = os.path.expanduser("~")
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
PIN = json.load(open(os.path.join(REPO, "UPSTREAM.json")))
PIN_VERSION = PIN["current"]
PIN_INFO = PIN["pins"][PIN_VERSION]
PRISTINE_SRC = os.path.expanduser(PIN_INFO["source_checkout"])
ORACLE = os.path.expanduser(PIN_INFO["oracle_binary"])

STATE = os.path.join(HOME, ".cache/blaze/upstream-tests")
SRC = os.path.join(HOME, ".cache/blaze/work/upstream-src")
OUTER_ROOT = os.path.join(STATE, "outer-root")
REPO_CACHE = os.path.join(STATE, "repo-cache")
TEST_REPO_CACHE = os.path.join(STATE, "test-repo-cache")
INSTALL_BASES = os.path.join(STATE, "install-bases")
MANIFEST = os.path.join(STATE, "manifest.json")
RESULTS = os.path.join(HOME, ".cache/blaze/upstream-test-results")
LOCKS = os.path.join(STATE, "locks")
CLIENTS = os.path.join(LOCKS, "clients")

# Test packages in scope.
SCOPE = ["//src/test/shell/...", "//src/test/py/bazel/..."]

# Tests left out of the default selection, with the reason. They are recorded as "excluded".
EXCLUDED = {
    # They need Bazel binary variants (jdk_minimal, nojdk) built from source; the harness only
    # swaps //src:bazel, so they would test the from-source build, not the binary under test.
    "//src/test/shell/integration:minimal_jdk_test": "binary-variant",
    "//src/test/shell/integration:nojdk_startup_options_test": "binary-variant",
    # They test Bazel's source tree, not the binary under test: bootstrap from the distfile with
    # compile.sh, and jdeps over the from-source server jar.
    "//src/test/shell/bazel:bazel_bootstrap_distfile_test": "source-artifact",
    "//src/test/shell/bazel:bazel_bootstrap_distfile_tar_test": "source-artifact",
    "//src/test/shell/bazel:jdeps_test": "source-artifact",
    # Builds all of Bazel twice with the binary under test (timeout eternal, highcpu machine).
    # The capstone, not part of the local baseline; run it with --include-excluded.
    "//src/test/shell/bazel:bazel_determinism_test": "self-build",
}


def excluded_reason(test):
    """Why a manifest test is left out of the default selection, or None."""
    if test["label"] in EXCLUDED:
        return EXCLUDED[test["label"]]
    if "manual" in test["tags"]:
        return "manual"
    return None

# Resource limits for the outer Bazel and the whole job (other agents share the machine).
MEMORY_MAX = "14G"
DEFAULT_JOBS = 8
DEFAULT_TEST_JOBS = 6
OUTER_JVM = ["--host_jvm_args=-Xmx3g"]

PATCH_MARK = "# blaze-upstream-tests patch"

UNDER_TEST_BZL = '''\
{mark}
# Provides the Bazel-compatible binary under test as @blaze_under_test//:bazel.
# Set it with --repo_env=BLAZE_UNDER_TEST=<absolute path>.

def _impl(rctx):
    path = rctx.getenv("BLAZE_UNDER_TEST")
    if not path:
        fail("set --repo_env=BLAZE_UNDER_TEST=<absolute path of the binary under test>")
    p = rctx.path(path)
    if not p.exists:
        fail("BLAZE_UNDER_TEST does not exist: " + path)
    rctx.watch(p)
    rctx.symlink(p, "bazel")
    rctx.file("BUILD.bazel", 'exports_files(["bazel"])\\n')

blaze_under_test = repository_rule(implementation = _impl, local = True)
'''.format(mark=PATCH_MARK)

MODULE_APPEND = '''
{mark}
blaze_under_test = use_repo_rule("//:blaze_under_test.bzl", "blaze_under_test")

blaze_under_test(name = "blaze_under_test")
'''.format(mark=PATCH_MARK)

# //src:bazel (the output of :bazel-bin) becomes a copy of the binary under test.
SRC_BUILD_OLD = '''    ("-dev", "_jdk_allmodules"),
    ("", "_jdk_minimal"),
    ("_jdk_allmodules", "_jdk_allmodules"),'''
SRC_BUILD_NEW = '''    ("-dev", "_jdk_allmodules"),
    ("_jdk_allmodules", "_jdk_allmodules"),'''
SRC_BUILD_APPEND = '''
{mark}
# //src:bazel is the binary under test, not a from-source build.
genrule(
    name = "bazel-bin",
    srcs = ["@blaze_under_test//:bazel"],
    outs = ["bazel"],
    cmd = "cp -L $< $@ && chmod 0755 $@",
    executable = 1,
    output_to_bindir = 1,
    tags = ["no-remote", "no-cache"],
    visibility = [
        "//:__pkg__",
        "//scripts:__pkg__",
        "//scripts/packages:__pkg__",
        "//src/java:__subpackages__",
        "//src/test:__subpackages__",
        "//tools/compliance:__pkg__",
    ],
)
'''.format(mark=PATCH_MARK)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def apply_patch(src=SRC):
    """Idempotently patches the writable source copy so //src:bazel is the binary under test."""
    bzl = os.path.join(src, "blaze_under_test.bzl")
    if not os.path.exists(bzl) or open(bzl).read() != UNDER_TEST_BZL:
        with open(bzl + ".tmp", "w") as f:
            f.write(UNDER_TEST_BZL)
        os.rename(bzl + ".tmp", bzl)
    mod = os.path.join(src, "MODULE.bazel")
    text = open(mod).read()
    if PATCH_MARK not in text:
        with open(mod, "a") as f:
            f.write(MODULE_APPEND)
    build = os.path.join(src, "src/BUILD")
    text = open(build).read()
    if PATCH_MARK not in text:
        if SRC_BUILD_OLD not in text:
            sys.exit("src/BUILD does not have the expected bazel_binary list; update ut_common.py")
        text = text.replace(SRC_BUILD_OLD, SRC_BUILD_NEW) + SRC_BUILD_APPEND
        with open(build, "w") as f:
            f.write(text)


def systemd_prefix(memory_max=MEMORY_MAX, name=None):
    cmd = [
        "systemd-run", "--user", "--collect", "--wait", "--pipe", "--quiet",
        "-p", "MemoryMax=" + memory_max, "-p", "MemorySwapMax=0", "-p", "OOMPolicy=continue",
        "--working-directory=" + SRC,
    ]
    if name:
        cmd += ["--unit=" + name]
    for var in ("PATH", "HOME", "USER", "LANG", "LC_ALL", "TERM"):
        if os.environ.get(var):
            cmd += ["--setenv=%s=%s" % (var, os.environ[var])]
    return cmd + ["--"]


def outer_job(args, log, stdout_file=None, memory_max=MEMORY_MAX, events_file=None, command=None):
    """Command line that runs the outer Bazel with `args` in a systemd unit capped at memory_max.

    Inside the unit: oom-guard protects the outer server, the outer Bazel runs with output to
    `log` (stdout to `stdout_file` if given), the server is shut down, and the unit's
    memory.events is copied to `events_file`. With `command`, that command runs in place of the
    outer Bazel (it runs the outer Bazel itself; `args` is ignored). The caller holds
    outer_lock."""
    here = os.path.dirname(os.path.abspath(__file__))
    outer = outer_bazel()
    redirect = ("> %s 2> %s" % (stdout_file, log)) if stdout_file else ("> %s 2>&1" % log)
    script = (
        '%s %s & guard=$!; '
        '"$@" %s; rc=$?; '
        '%s shutdown >/dev/null 2>&1; kill $guard 2>/dev/null; '
        'cg=/sys/fs/cgroup$(sed -n "s/^0:://p" /proc/self/cgroup); '
        '%s'
        'exit $rc'
    ) % (os.path.join(here, "oom-guard"), OUTER_ROOT, redirect, " ".join(outer),
         ('cp "$cg/memory.events" %s 2>/dev/null; ' % events_file) if events_file else "")
    main = list(command) if command else outer + list(args)
    return systemd_prefix(memory_max) + ["bash", "-c", script, "_"] + main


def outer_bazel(startup_extra=()):
    """The outer (harness) Bazel: always the pinned oracle, own output root."""
    return [ORACLE, "--output_user_root=" + OUTER_ROOT] + OUTER_JVM + list(startup_extra)


def common_flags(jobs=DEFAULT_JOBS):
    return [
        "--repository_cache=" + REPO_CACHE,
        "--jobs=%d" % jobs,
        "--local_resources=cpu=%d" % jobs,
        "--worker_max_instances=%d" % max(1, jobs // 2),
        "--color=no",
        "--curses=no",
        "--show_progress_rate_limit=30",
    ]


def reset_test_repo_cache():
    """The tests' repository cache: every download the outer cache has (hardlinked, as upstream
    CI's prepared cache), and an empty repo contents cache, so runs start alike and the contents
    cache (several GB per run) does not grow without bound. Call it holding outer_lock: the
    running tests of another run write into this cache."""
    contents = os.path.join(TEST_REPO_CACHE, "contents")
    if os.path.exists(contents):
        # Move it aside first, so a failure part-way never leaves a half-emptied cache in place.
        trash = contents + ".old-%d" % os.getpid()
        os.rename(contents, trash)
        subprocess.run(["chmod", "-R", "u+w", trash], check=False)
        subprocess.run(["rm", "-rf", trash], check=True)
    for old in os.listdir(TEST_REPO_CACHE) if os.path.isdir(TEST_REPO_CACHE) else []:
        if old.startswith("contents.old-"):  # left by a reset that failed
            subprocess.run(["chmod", "-R", "u+w", os.path.join(TEST_REPO_CACHE, old)], check=False)
            subprocess.run(["rm", "-rf", os.path.join(TEST_REPO_CACHE, old)], check=False)
    cas = os.path.join(TEST_REPO_CACHE, "content_addressable")
    os.makedirs(cas, exist_ok=True)
    src = os.path.join(REPO_CACHE, "content_addressable")
    if os.path.isdir(src):
        subprocess.run(["cp", "-a", "-l", "-n", src + "/.", cas + "/"], check=True)


def _holder(path):
    try:
        return json.load(open(path))
    except (OSError, ValueError):
        return None


def _describe(h):
    if not h:
        return "an unknown process (no holder record)"
    return "%s (pid %s, %s, since %s)" % (h.get("label") or h.get("what"), h.get("pid"),
                                         "alive" if _alive(h.get("pid")) else "exited",
                                         h.get("start"))


def _alive(pid):
    try:
        os.kill(int(pid), 0)
        return True
    except (OSError, TypeError, ValueError):
        return False


class _Lock:
    """Exclusive flock on `path`; the holder's description is kept in `path`.holder.json.

    flock is released by the kernel when the process exits, so a crashed holder never blocks
    later runs; its stale holder record is only informational."""

    def __init__(self, path, what, label=None, extra=None):
        self.path, self.holder_path = path, path[:-len(".lock")] + ".holder.json"
        self.info = {"what": what, "label": label, "pid": os.getpid(),
                     "start": datetime.datetime.now().isoformat(timespec="seconds"),
                     "argv": sys.argv}
        self.info.update(extra or {})
        self.fd = None

    def acquire(self, poll=5):
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        fd = os.open(self.path, os.O_RDWR | os.O_CREAT, 0o644)
        shown = None
        waited = time.time()
        while True:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                h = _holder(self.holder_path)
                key = json.dumps(h, sort_keys=True)
                if key != shown:
                    print("waiting for %s, held by %s" % (os.path.basename(self.path),
                                                          _describe(h)), flush=True)
                    shown = key
                time.sleep(poll)
        if shown is not None:
            print("got %s after %.0fs" % (os.path.basename(self.path), time.time() - waited),
                  flush=True)
        self.fd = fd
        tmp = self.holder_path + ".%d.tmp" % os.getpid()
        json.dump(self.info, open(tmp, "w"), indent=1)
        os.rename(tmp, self.holder_path)
        return self

    def release(self):
        if self.fd is None:
            return
        try:
            if (_holder(self.holder_path) or {}).get("pid") == os.getpid():
                os.unlink(self.holder_path)
        except OSError:
            pass
        fcntl.flock(self.fd, fcntl.LOCK_UN)
        os.close(self.fd)
        self.fd = None

    def __enter__(self):
        return self.acquire()

    def __exit__(self, *exc):
        self.release()


def outer_lock(what, label=None, extra=None):
    """The lock over all shared state in STATE (see the module doc). Use as a context manager."""
    return _Lock(os.path.join(LOCKS, "outer.lock"), what, label, extra)


def label_lock(label, what):
    """The lock over one label's results directory (does not create the directory)."""
    return _Lock(os.path.join(LOCKS, "label-%s.lock" % label), what, label)


def register_client():
    """Marks this process as lock-aware (see wait_for_unlocked_runs)."""
    os.makedirs(CLIENTS, exist_ok=True)
    for pid in os.listdir(CLIENTS):  # left by processes that were killed
        if not _alive(pid):
            try:
                os.unlink(os.path.join(CLIENTS, pid))
            except OSError:
                pass
    open(os.path.join(CLIENTS, str(os.getpid())), "w").write(" ".join(sys.argv) + "\n")


def unregister_client():
    try:
        os.unlink(os.path.join(CLIENTS, str(os.getpid())))
    except OSError:
        pass


def _proc_args(pid):
    try:
        return open("/proc/%s/cmdline" % pid, "rb").read().split(b"\0")
    except OSError:
        return []


def _age_s(pid):
    try:
        start = int(open("/proc/%s/stat" % pid).read().rsplit(")", 1)[1].split()[19])
        return float(open("/proc/uptime").read().split()[0]) - start / os.sysconf("SC_CLK_TCK")
    except (OSError, ValueError, IndexError):
        return 0.0


def unlocked_runs():
    """Live processes that use the shared state without holding the outer lock:
      - harness scripts started before the lock existed: not registered in CLIENTS and older
        than a minute (a lock-aware script registers itself as its first action);
      - outer Bazel clients (not servers) on OUTER_ROOT, and per-function drivers (`run
        _inner`): called while holding the outer lock, so no lock-aware run is in its outer
        phase, and any found belong to an older script or to a job whose `run` was killed
        while its systemd unit kept going."""
    me = os.getpid()
    registered = set(os.listdir(CLIENTS)) if os.path.isdir(CLIENTS) else set()
    ancestors = set()
    p = me
    while p and p > 1:
        ancestors.add(p)
        try:
            p = int(open("/proc/%d/stat" % p).read().rsplit(")", 1)[1].split()[1])
        except (OSError, ValueError, IndexError):
            break
    found = []
    root_arg = ("--output_user_root=" + OUTER_ROOT).encode()
    for pid in os.listdir("/proc"):
        if not pid.isdigit() or int(pid) in ancestors:
            continue
        args = [a for a in _proc_args(pid) if a]
        if not args:
            continue
        text = [a.decode(errors="replace") for a in args]
        script = next((a for a in text[:3] if a.endswith(("upstream-tests/run",
                                                          "upstream-tests/prepare"))), None)
        if script:
            rest = text[text.index(script) + 1:]
            if rest and rest[0] == "_inner":
                found.append((pid, " ".join(text)[:200]))
                continue
            if pid in registered or _age_s(pid) < 60 or (
                    rest and rest[0] in ("help", "--help", "-h", "list", "reparse")):
                continue
            found.append((pid, " ".join(text)[:200]))
        elif root_arg in args and os.path.basename(text[0]) != "java" and \
                not any(a.startswith("--server_jvm_out") or a == "--server" for a in text) and \
                os.path.realpath(text[0]) == os.path.realpath(ORACLE):
            found.append((pid, " ".join(text)[:200]))
    return found


def wait_for_unlocked_runs(poll=30):
    """Blocks while unlocked_runs() finds anything (call it holding the outer lock)."""
    shown = None
    while True:
        found = unlocked_runs()
        if not found:
            return
        key = sorted(p for p, _ in found)
        if key != shown:
            print("waiting for runs that do not take the lock (started before it existed):",
                  flush=True)
            for pid, cmd in found:
                print("  pid %s: %s" % (pid, cmd), flush=True)
            shown = key
        time.sleep(poll)


def load_manifest():
    if not os.path.exists(MANIFEST):
        sys.exit("no manifest at %s; run scripts/upstream-tests/prepare first" % MANIFEST)
    return json.load(open(MANIFEST))


def ensure_dirs():
    for d in (STATE, OUTER_ROOT, REPO_CACHE, INSTALL_BASES, RESULTS, LOCKS):
        os.makedirs(d, exist_ok=True)


def copy_source():
    if os.path.exists(SRC):
        return False
    shutil.copytree(PRISTINE_SRC, SRC, symlinks=True)
    subprocess.run(["chmod", "-R", "u+w", SRC], check=True)
    return True
