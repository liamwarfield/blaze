"""Shared by the run wrappers of scripts/remote/: run a command in a `systemd-run --wait` unit and
record its real status from inside the unit (see `unit-supervisor help`).

`systemd-run --wait` returns 0 when the unit's main process died by SIGPIPE, SIGHUP, SIGINT or
SIGTERM; its own status is used only when the supervisor recorded nothing (the unit was killed
before the command ended, for example by the memory limit), and a warning says so.
"""

import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SUPERVISOR = os.path.join(HERE, "unit-supervisor")


def unit_argv(status_path, cmd, unit_name=None, memory_max="12G"):
    """The argv that runs `env -i HOME=... PATH=/usr/bin:/bin <cmd>` under the supervisor in a
    transient unit; the supervisor writes the status to `status_path`."""
    if os.path.lexists(status_path):
        os.remove(status_path)
    unit = ["systemd-run", "--user", "--collect", "--wait", "--pipe", "--quiet", "--same-dir"]
    if unit_name:
        unit.append("--unit=" + unit_name)
    unit += ["-p", "MemoryMax=" + memory_max, "-p", "MemorySwapMax=0",
             "/usr/bin/python3", "-I", SUPERVISOR, status_path,
             "env", "-i", "HOME=" + os.environ["HOME"], "PATH=/usr/bin:/bin"]
    return unit + list(cmd)


def collect(status_path, systemd_code, exit_dir=None):
    """Reads the status the supervisor recorded. Returns (code, signal name or None); a signal
    death gives 128+N. With `exit_dir`, writes `<exit_dir>/exit` (the number) and, after a
    signal, `<exit_dir>/signal` (its name, e.g. SIGPIPE)."""
    try:
        words = open(status_path).read().split()
    except FileNotFoundError:
        words = []
    sig = None
    if words[:1] == ["exit"]:
        code = int(words[1])
    elif words[:1] == ["signal"]:
        code = 128 + int(words[1])
        sig = words[2]
    else:
        print("warning: no status recorded inside the unit; systemd-run returned %d (0 here does "
              "not exclude a death by signal)" % systemd_code, file=sys.stderr)
        code = systemd_code
    if exit_dir is not None:
        with open(os.path.join(exit_dir, "exit"), "w") as f:
            f.write("%d\n" % code)
        sig_path = os.path.join(exit_dir, "signal")
        if sig:
            with open(sig_path, "w") as f:
                f.write(sig + "\n")
        elif os.path.lexists(sig_path):
            os.remove(sig_path)
    return code, sig


def describe(code, sig):
    """`141 (killed by SIGPIPE)` after a signal, else the plain number; for `exit code:` lines."""
    return "%d (killed by %s)" % (code, sig) if sig else "%d" % code


def run(status_path, cmd, cwd, stdout_path, stderr_path, exit_dir=None, memory_max="12G"):
    """Runs cmd in a unit with stdout/stderr to the given files; returns (code, signal)."""
    with open(stdout_path, "wb") as so, open(stderr_path, "wb") as se:
        rc = subprocess.run(unit_argv(status_path, cmd, memory_max=memory_max), cwd=cwd,
                            stdout=so, stderr=se).returncode
    return collect(status_path, rc, exit_dir)
