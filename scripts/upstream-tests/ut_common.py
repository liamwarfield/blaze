"""Shared paths and helpers for scripts/upstream-tests (Bazel's own shell and Python tests).

Layout (all outside the repo):
  ~/.cache/blaze/work/upstream-src            writable copy of the pinned source, patched (see PATCH)
  ~/.cache/blaze/upstream-tests/outer-root     output_user_root of the outer (harness) Bazel
  ~/.cache/blaze/upstream-tests/repo-cache     repository cache of the outer Bazel (prepare fills it)
  ~/.cache/blaze/upstream-tests/test-repo-cache  the tests' REPOSITORY_CACHE: downloads hardlinked
                                               from repo-cache, contents/ emptied before each run
  ~/.cache/blaze/upstream-tests/install-bases  shared inner install bases, one per binary sha256
  ~/.cache/blaze/upstream-tests/manifest.json  the test targets (written by `prepare`)
  ~/.cache/blaze/upstream-test-results/<label>/ results.json, logs/, bep.json, outer.log
"""

import hashlib
import json
import os
import shutil
import subprocess
import sys

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
    with open(bzl, "w") as f:
        f.write(UNDER_TEST_BZL)
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


def outer_job(args, log, stdout_file=None, memory_max=MEMORY_MAX, events_file=None):
    """Command line that runs the outer Bazel with `args` in a systemd unit capped at memory_max.

    Inside the unit: oom-guard protects the outer server, the outer Bazel runs with output to
    `log` (stdout to `stdout_file` if given), the server is shut down, and the unit's
    memory.events is copied to `events_file`."""
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
    return systemd_prefix(memory_max) + ["bash", "-c", script, "_"] + outer + list(args)


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
    cache (several GB per run) does not grow without bound."""
    contents = os.path.join(TEST_REPO_CACHE, "contents")
    if os.path.exists(contents):
        subprocess.run(["chmod", "-R", "u+w", contents], check=False)
        shutil.rmtree(contents)
    cas = os.path.join(TEST_REPO_CACHE, "content_addressable")
    os.makedirs(cas, exist_ok=True)
    src = os.path.join(REPO_CACHE, "content_addressable")
    if os.path.isdir(src):
        subprocess.run(["cp", "-a", "-l", "-n", src + "/.", cas + "/"], check=True)


def load_manifest():
    if not os.path.exists(MANIFEST):
        sys.exit("no manifest at %s; run scripts/upstream-tests/prepare first" % MANIFEST)
    return json.load(open(MANIFEST))


def ensure_dirs():
    for d in (STATE, OUTER_ROOT, REPO_CACHE, INSTALL_BASES, RESULTS):
        os.makedirs(d, exist_ok=True)


def copy_source():
    if os.path.exists(SRC):
        return False
    shutil.copytree(PRISTINE_SRC, SRC, symlinks=True)
    subprocess.run(["chmod", "-R", "u+w", SRC], check=True)
    return True
