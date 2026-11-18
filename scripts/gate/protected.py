"""Protected paths: which staged changes need an approval, and whether one was given.

Used by scripts/git-hooks/pre-commit and commit-msg. The list is scripts/gate/protected-paths.txt
(its header explains the format and the rule). Standard library only.
"""

import os
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
LIST = os.path.join(HERE, "protected-paths.txt")
ENV = "BLAZE_PROTECTED_OK"
TRAILER = "Protected-Paths-OK"
# An approval names a batch and says why; a bare "1" or "yes" is not one.
MIN_REASON = 12


def parse_rules(text, name=LIST):
    """[(prefix, mode)] with mode "protected" or "append-only"."""
    rules = []
    for n, line in enumerate(text.splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        mode = "protected"
        if line.startswith("append-only "):
            mode, line = "append-only", line[len("append-only "):].strip()
            if not line.endswith("/"):
                raise ValueError("%s:%d: append-only needs a directory (ending in /)" % (name, n))
        if line.startswith("/") or ".." in line.split("/") or " " in line:
            raise ValueError("%s:%d: not a repository-relative path: %r" % (name, n, line))
        rules.append((line, mode))
    return rules


def load_rules(path=LIST):
    with open(path) as f:
        return parse_rules(f.read(), path)


def committed_rules(repo=None):
    """The rules a commit is judged by: the list as committed in HEAD, so that a commit cannot
    shorten the list it is checked against. Before the list's first commit (or outside a
    repository) the file in the working tree is used.
    """
    rel = os.path.relpath(LIST, os.path.dirname(os.path.dirname(HERE)))
    cmd = ["git"] + (["-C", repo] if repo else [])
    p = subprocess.run(cmd + ["show", "HEAD:" + rel], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    if p.returncode != 0:
        return load_rules()
    return parse_rules(p.stdout.decode(), "HEAD:" + rel)


def rule_for(path, rules):
    """The longest rule that covers `path`, or None."""
    best = None
    for prefix, mode in rules:
        covers = path.startswith(prefix) if prefix.endswith("/") else path == prefix
        if covers and (best is None or len(prefix) > len(best[0])):
            best = (prefix, mode)
    return best


def violations(changes, rules):
    """[(status, path, rule prefix)] for the staged changes that need an approval.

    `changes` is [(status, path)] with git's one-letter status (A added, M modified, D deleted,
    T type change; renames arrive as D + A).
    """
    out = []
    for status, path in changes:
        rule = rule_for(path, rules)
        if rule is None:
            continue
        prefix, mode = rule
        if mode == "append-only" and status == "A":
            continue
        out.append((status, path, prefix))
    return out


def staged_changes(repo=None):
    """[(status, path)] of the index against HEAD (against the empty tree for a first commit)."""
    cmd = ["git"] + (["-C", repo] if repo else [])
    head = subprocess.run(cmd + ["rev-parse", "--verify", "-q", "HEAD"], stdout=subprocess.DEVNULL).returncode == 0
    base = "HEAD" if head else subprocess.run(
        cmd + ["hash-object", "-t", "tree", "/dev/null"], stdout=subprocess.PIPE, check=True).stdout.decode().strip()
    raw = subprocess.run(cmd + ["diff", "--cached", "--no-renames", "--name-status", "-z", base],
                         stdout=subprocess.PIPE, check=True).stdout.decode("utf-8", "surrogateescape")
    parts = raw.split("\0")
    return [(parts[i][:1], parts[i + 1]) for i in range(0, len(parts) - 1, 2)]


def approval(environ=None):
    """The approval text from $BLAZE_PROTECTED_OK, or None when it is missing or too short to
    name a batch and a reason."""
    text = " ".join(((environ if environ is not None else os.environ).get(ENV) or "").split())
    return text if len(text) >= MIN_REASON else None


def rule_text():
    return (
        "Protected paths (scripts/gate/protected-paths.txt): the referee, the gate, the baselines,\n"
        "the pin and the contract change only in a batch that names the path and that root approved.\n"
        "To commit such a batch, name it:\n\n"
        "    %s=\"<batch>: <why>\" git commit ...\n\n"
        "(at least %d characters; the text is added to the commit message as `%s:`).\n"
        "If your task did not name these paths, unstage them: `git restore --staged <path>`."
        % (ENV, MIN_REASON, TRAILER))


def check(changes, rules, environ=None):
    """(exit status, lines to print) for a commit with these staged changes."""
    bad = violations(changes, rules)
    if not bad:
        return 0, []
    lines = ["%d staged change%s to protected paths:" % (len(bad), "" if len(bad) == 1 else "s")]
    lines += ["  %s %s   (rule: %s)" % v for v in bad[:40]]
    if len(bad) > 40:
        lines.append("  ... %d more" % (len(bad) - 40))
    ok = approval(environ)
    if ok is None:
        given = (environ if environ is not None else os.environ).get(ENV)
        if given:
            lines.append("%s is set but too short to name a batch and a reason." % ENV)
        return 1, lines + ["", "COMMIT REJECTED.", rule_text()]
    return 0, lines + ["approved by %s: %s" % (ENV, ok)]


def add_trailer(message, text):
    """`message` with a `Protected-Paths-OK: text` trailer, unless it already has that one."""
    line = "%s: %s" % (TRAILER, text)
    body = message.rstrip("\n")
    if line in body.split("\n"):
        return message
    last = body.rsplit("\n\n", 1)[-1] if "\n\n" in body else ""
    in_trailers = bool(last) and all(
        l.split(":", 1)[0].replace("-", "").isalnum() and ": " in l for l in last.split("\n"))
    return body + ("\n" if in_trailers else "\n\n") + line + "\n"
