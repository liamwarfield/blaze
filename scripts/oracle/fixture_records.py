"""Test support: a scratch records root and loaders for the tools that have no .py suffix.

Used by test_compare.py and test_record.py. Nothing here touches ~/.cache/blaze.
"""

import contextlib
import importlib.machinery
import importlib.util
import io
import os
import shutil
import sys
import tempfile

import oracle_common as oc

HERE = os.path.dirname(os.path.abspath(__file__))


def load_tool(name, directory=HERE):
    """Import scripts/oracle/<name> (an executable without .py) as a module, without running it."""
    loader = importlib.machinery.SourceFileLoader("tool_" + name.replace("-", "_"), os.path.join(directory, name))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


def run_main(mod, argv):
    """Call mod.main() with sys.argv = [tool] + argv. Returns (exit status, stdout, stderr)."""
    out, err = io.StringIO(), io.StringIO()
    old = sys.argv
    sys.argv = [mod.__name__] + list(argv)
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                rc = mod.main()
            except SystemExit as e:
                rc = e.code
    finally:
        sys.argv = old
    return rc, out.getvalue(), err.getvalue()


class Records:
    """A temporary records root, summaries directory and scores directory.

    While it exists, oracle_common.RECORDS, CORPUS_SUMMARIES and CORPUS_SCORES point into it.
    """

    def __init__(self, testcase):
        self.root = tempfile.mkdtemp(prefix="oracle-test-")
        saved = (oc.RECORDS, oc.CORPUS_SUMMARIES, oc.CORPUS_SCORES)
        oc.RECORDS = os.path.join(self.root, "records")
        oc.CORPUS_SUMMARIES = os.path.join(self.root, "corpus", "oracle")
        oc.CORPUS_SCORES = os.path.join(self.root, "corpus", "scores")
        os.makedirs(oc.RECORDS)

        def restore():
            oc.RECORDS, oc.CORPUS_SUMMARIES, oc.CORPUS_SCORES = saved
            shutil.rmtree(self.root, ignore_errors=True)

        testcase.addCleanup(restore)

    def add(self, label, project, surfaces, manifest=None, summary=True):
        """Write a recording. `surfaces` maps a surface to its items; a surface that is not named
        is SKIPPED; a value ("FAILED", "why") is a failed surface. Unless the caller gives
        exit_codes, every command the recorded surfaces need exited 0.
        """
        import surfaces as surf

        surfaces = dict(surfaces)
        if "exit_codes" not in surfaces:
            codes = {"info": 0}
            for s in surfaces:
                for c in surf.SURFACE_COMMANDS.get(s, []):
                    codes[c] = 0
            surfaces["exit_codes"] = codes
        rd = oc.record_dir(label, project)
        os.makedirs(os.path.join(rd, "items"))
        files, counts = {}, {}
        for s in oc.SURFACES:
            v = surfaces.get(s)
            if v is None:
                res = {"surface": s, "status": "SKIPPED", "items": {}}
            elif isinstance(v, tuple):
                res = {"surface": s, "status": v[0], "items": {}, "error": v[1]}
            else:
                res = {"surface": s, "status": "OK", "items": v}
            path = os.path.join(rd, "items", s + ".json")
            oc.write_json(path, res, indent=None)
            files["items/%s.json" % s] = oc.sha256_file(path)
            counts[s] = {"status": res["status"], "items": len(res["items"])}
        m = {"label": label, "project": project, "created": "2026-01-01T00:00:00+00:00",
             "binary": "/bin/" + label, "binary_sha256": oc.sha256_bytes(label.encode()),
             "surfaces": counts, "files": files, "extractor_sha256": oc.extractor_sha256(),
             "commands": [{"name": k, "exit_code": v} for k, v in surfaces["exit_codes"].items()],
             "project_command": ["build", "//..."]}
        m.update(manifest or {})
        oc.write_json(os.path.join(rd, "manifest.json"), m)
        if summary and label == oc.REFERENCE_LABEL:
            self.write_summary(project)
        return rd

    def write_summary(self, project):
        """The committed summary of the reference recording, as `record` writes it."""
        m = oc.load_manifest(oc.REFERENCE_LABEL, project)
        os.makedirs(oc.CORPUS_SUMMARIES, exist_ok=True)
        oc.write_json(os.path.join(oc.CORPUS_SUMMARIES, project + ".json"), {"files_sha256": m["files"]})
