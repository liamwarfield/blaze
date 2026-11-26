"""Normaliser: removes run-specific values from recorded Bazel outputs.

A Context holds the absolute paths and names of one recording (workspace copy, output user root,
output base, execroot, install base, $HOME, user, host, record directory, binary). Everything that
names them is replaced by a placeholder, so two recordings made in different directories, by
different binaries, on different days compare equal when Bazel's behaviour is equal.

Generic rules (applied to every string, see Context.text):
  * absolute paths -> <EXECROOT>, <OUTPUT_BASE>, <INSTALL_BASE>, <OUTPUT_USER_ROOT>, <WORKSPACE>,
    <RECORD_DIR>, <BINARY>, <BINARY_DIR>, <HOME> (longest first, so a path is replaced by its most specific
    placeholder);
  * sandbox directory numbers (<OUTPUT_BASE>/sandbox/<strategy>/<N>/) -> <N>;
  * UUIDs -> <UUID>; ISO-8601 timestamps -> <TIMESTAMP>;
  * the user name and host name as whole words -> <USER>, <HOST>.
Format-specific rules (timing fields, durations, PIDs, ordering) live in surfaces.py and in the
stderr helper below.
"""

import re

UUID_RE = re.compile(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
TIMESTAMP_RE = re.compile(
    r"\b\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?"
)
SANDBOX_RE = re.compile(r"(<OUTPUT_BASE>/sandbox/[A-Za-z0-9_-]+/)\d+(?=/|$|\b)")
SANDBOX_RE_B = re.compile(rb"(<OUTPUT_BASE>/sandbox/[A-Za-z0-9_-]+/)\d+(?=/|$|\b)")

# Placeholder names in priority order (used only for documentation and tests).
PLACEHOLDERS = [
    "execroot",
    "output_base",
    "install_base",
    "output_user_root",
    "workspace",
    "record_dir",
    "binary",
    "binary_dir",
    "home",
]


class Context:
    """Run-specific values of one recording and the functions that erase them."""

    def __init__(self, paths, user=None, host=None):
        # paths: dict name -> absolute path (names from PLACEHOLDERS).
        self.paths = {k: v for k, v in paths.items() if v}
        self.user = user
        self.host = host
        pairs = []
        for name, value in self.paths.items():
            value = value.rstrip("/")
            if len(value) < 2:
                continue
            pairs.append((value, "<" + name.upper() + ">"))
        # Longest first: a path is replaced by its most specific placeholder.
        pairs.sort(key=lambda p: (-len(p[0]), p[1]))
        self._pairs = pairs
        self._bpairs = [(a.encode(), b.encode()) for a, b in pairs]
        self._word_res = []
        for value, ph in ((user, "<USER>"), (host, "<HOST>")):
            if value and len(value) >= 3:
                self._word_res.append(
                    (re.compile(r"(?<![A-Za-z0-9_.-])" + re.escape(value) + r"(?![A-Za-z0-9_])"), ph)
                )

    @classmethod
    def from_dict(cls, d):
        return cls(d.get("paths", {}), d.get("user"), d.get("host"))

    def to_dict(self):
        return {"paths": dict(self.paths), "user": self.user, "host": self.host}

    def paths_only(self, s):
        for a, b in self._pairs:
            if a in s:
                s = s.replace(a, b)
        return s

    def text(self, s):
        if not isinstance(s, str) or not s:
            return s
        s = self.paths_only(s)
        if "<OUTPUT_BASE>/sandbox/" in s:
            s = SANDBOX_RE.sub(r"\1<N>", s)
        if "-" in s:
            s = UUID_RE.sub("<UUID>", s)
            s = TIMESTAMP_RE.sub("<TIMESTAMP>", s)
        for rx, ph in self._word_res:
            s = rx.sub(ph, s)
        return s

    def bytes(self, data):
        """Path placeholders for file contents (no word or timestamp rules on binary data)."""
        changed = False
        for a, b in self._bpairs:
            if a in data:
                data = data.replace(a, b)
                changed = True
        if changed and b"<OUTPUT_BASE>/sandbox/" in data:
            data = SANDBOX_RE_B.sub(rb"\1<N>", data)
        return data, changed

    def deep(self, obj):
        """Apply text() to every string (keys included) of a JSON value."""
        if isinstance(obj, str):
            return self.text(obj)
        if isinstance(obj, list):
            return [self.deep(x) for x in obj]
        if isinstance(obj, dict):
            return {self.text(k): self.deep(v) for k, v in obj.items()}
        return obj


# ---------------------------------------------------------------------------------------------
# stderr

# Lines of the build/test command's stderr that carry information worth comparing.
_STDERR_KEEP = re.compile(r"^(INFO|WARNING|ERROR|DEBUG|FAIL|FAILED):|^//|^@|^Executed |^\s+//|^There were tests")
_STDERR_DROP = [
    re.compile(r"^INFO: Invocation ID: "),
    re.compile(r"^INFO: Elapsed time: "),
    re.compile(r"^INFO: Streaming build results to: "),
    re.compile(r"^INFO: Reading rc options for "),
    re.compile(r"^INFO: Options provided by the client:"),
    re.compile(r"^INFO: Current date is "),
    re.compile(r"^INFO: Build Event Protocol files produced successfully"),
    re.compile(r"^INFO: From "),  # action output banners; order is scheduling-dependent
]
# The client's line when it starts a server ("Starting local Bazel server (9.3.2) and connecting
# to it..."). It has no prefix, so _STDERR_KEEP drops it. A recording made in daemon mode
# (`record --blaze-server daemon`) keeps it: there a server start in the middle of the command
# sequence is a difference to the oracle (audit 3, B1). One-process recordings print it on every
# command by construction, so it stays dropped for them. oracle_common.server_states reads it too.
SERVER_START_RE = re.compile(r"^Starting local \S+ server\b", re.M)
_DURATION_RE = re.compile(r"\b\d+(?:\.\d+)?\s?(?:ms|s|min|h)\b")
_PID_RE = re.compile(r"\b(pid[ =:]?)\d+", re.IGNORECASE)
_ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


def stderr_summary(ctx, text, keep_server_start=False):
    """Normalised, de-duplicated summary lines of a Bazel command's stderr, sorted.

    Keeps INFO/WARNING/ERROR/DEBUG lines and the test summary; drops progress, banners and lines
    that only carry timing or identity. Durations and PIDs become placeholders. Returns a dict
    line -> count (the order of these lines depends on scheduling, so it is not compared).
    keep_server_start: also keep the client's server-start line (SERVER_START_RE).
    """
    out = {}
    for raw in text.splitlines():
        line = _ANSI_RE.sub("", raw).rstrip()
        if "\r" in line:
            line = line.split("\r")[-1]
        if not (_STDERR_KEEP.search(line) or (keep_server_start and SERVER_START_RE.search(line))):
            continue
        if any(rx.search(line) for rx in _STDERR_DROP):
            continue
        line = ctx.text(line)
        line = _DURATION_RE.sub("<DUR>", line)
        line = _PID_RE.sub(r"\1<PID>", line)
        line = re.sub(r"\s+", " ", line).strip()
        out[line] = out.get(line, 0) + 1
    return out


def stderr_tail(text, n=30):
    lines = [_ANSI_RE.sub("", l) for l in text.splitlines()]
    return lines[-n:]
