#!/usr/bin/python3 -I
"""A persistent worker for the worker probe (scripts/workers/compare), standard library only.

Started by Bazel/Blaze as `worker.py [startup flags] --persistent_worker`, it reads WorkRequests
from stdin and writes WorkResponses to stdout, in the proto format (varint-delimited
blaze.worker.WorkRequest / WorkResponse) or, with `--protocol=json`, in the JSON format (one
object per line). With `--multiplex` it serves requests on threads and answers out of order.
Without `--persistent_worker` it is an ordinary tool: it expands its @flagfile arguments and does
one unit of work (the fallback when the strategy is not "worker").

A unit of work (the request's arguments, from the action's flag file):
  --in PATH --out PATH   copy PATH to OUT, upper-cased, prefixed by "worker-output:"
  --mode MODE            ok (default) | fail (exit code 1, an output message) | crash (the process
                         exits 3 without answering) | garbage (writes text to stdout instead of a
                         response, then exits) | slow (sleeps 30s first) | empty (exits 0 without
                         answering) | huge-stderr (writes 20000 bytes to the log, then crashes)
                         | cancelled (answers with was_cancelled set)
Paths are relative to the working directory, or to the request's sandbox_dir when set.
Everything this worker logs goes to stderr, which Bazel/Blaze redirect to the worker log file.
"""

import json
import os
import sys
import threading
import time


def log(msg):
    sys.stderr.write("probe-worker: %s\n" % msg)
    sys.stderr.flush()


# ---------------------------------------------------------------------------------------------
# Protocol buffers, by hand.

def read_varint(stream):
    shift, result = 0, 0
    while True:
        b = stream.read(1)
        if not b:
            if shift == 0:
                return None
            raise EOFError("truncated varint")
        result |= (b[0] & 0x7F) << shift
        if not b[0] & 0x80:
            return result
        shift += 7


def varint(n):
    if n < 0:
        n += 1 << 64
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def parse_fields(data):
    i, fields = 0, []
    while i < len(data):
        tag, n = 0, 0
        while True:
            b = data[i]
            i += 1
            tag |= (b & 0x7F) << n
            n += 7
            if not b & 0x80:
                break
        field, wire = tag >> 3, tag & 7
        if wire == 0:
            v, n = 0, 0
            while True:
                b = data[i]
                i += 1
                v |= (b & 0x7F) << n
                n += 7
                if not b & 0x80:
                    break
            fields.append((field, v))
        elif wire == 2:
            ln, n = 0, 0
            while True:
                b = data[i]
                i += 1
                ln |= (b & 0x7F) << n
                n += 7
                if not b & 0x80:
                    break
            fields.append((field, data[i:i + ln]))
            i += ln
        elif wire == 1:
            fields.append((field, data[i:i + 8]))
            i += 8
        elif wire == 5:
            fields.append((field, data[i:i + 4]))
            i += 4
        else:
            raise ValueError("wire type %d" % wire)
    return fields


def decode_request(data):
    req = {"arguments": [], "inputs": [], "requestId": 0, "cancel": False, "verbosity": 0,
           "sandboxDir": ""}
    for field, v in parse_fields(data):
        if field == 1:
            req["arguments"].append(v.decode("utf-8"))
        elif field == 2:
            inp = {}
            for f2, v2 in parse_fields(v):
                if f2 == 1:
                    inp["path"] = v2.decode("utf-8")
                elif f2 == 2:
                    inp["digest"] = v2.decode("utf-8")
            req["inputs"].append(inp)
        elif field == 3:
            req["requestId"] = v if v < (1 << 31) else v - (1 << 64)
        elif field == 4:
            req["cancel"] = bool(v)
        elif field == 5:
            req["verbosity"] = v
        elif field == 6:
            req["sandboxDir"] = v.decode("utf-8")
    return req


def encode_response(resp):
    out = bytearray()
    if resp.get("exitCode"):
        out += b"\x08" + varint(resp["exitCode"])
    if resp.get("output"):
        b = resp["output"].encode("utf-8")
        out += b"\x12" + varint(len(b)) + b
    if resp.get("requestId"):
        out += b"\x18" + varint(resp["requestId"])
    if resp.get("wasCancelled"):
        out += b"\x20\x01"
    return varint(len(out)) + bytes(out)


# ---------------------------------------------------------------------------------------------
# The work.

def expand_flagfiles(args):
    out = []
    for a in args:
        if a.startswith("@") and not a.startswith("@@"):
            with open(a[1:], encoding="utf-8") as f:
                out.extend(line for line in f.read().splitlines())
        elif a.startswith("--flagfile="):
            with open(a[len("--flagfile="):], encoding="utf-8") as f:
                out.extend(line for line in f.read().splitlines())
        else:
            out.append(a)
    return out


def parse_work(args):
    work = {"in": None, "out": None, "mode": "ok"}
    i = 0
    while i < len(args):
        a = args[i]
        if a in ("--in", "--out", "--mode") and i + 1 < len(args):
            work[a[2:]] = args[i + 1]
            i += 2
            continue
        i += 1
    return work


def do_work(work, base):
    """Returns (exit code, output message)."""
    mode = work["mode"]
    if mode == "slow":
        time.sleep(30)
    if mode == "fail":
        return 1, "probe worker: failure requested for %s\n" % work["out"]
    inp = os.path.join(base, work["in"]) if base else work["in"]
    out = os.path.join(base, work["out"]) if base else work["out"]
    with open(inp, encoding="utf-8") as f:
        text = f.read()
    with open(out, "w", encoding="utf-8") as f:
        f.write("worker-output:\n" + text.upper())
    return 0, "probe worker: wrote %s\n" % work["out"]


# ---------------------------------------------------------------------------------------------
# The loop.

class Out:
    def __init__(self, json_protocol):
        self.json = json_protocol
        self.lock = threading.Lock()

    def send(self, resp):
        with self.lock:
            if self.json:
                sys.stdout.write(json.dumps(resp, separators=(",", ":")) + "\n")
                sys.stdout.flush()
            else:
                sys.stdout.buffer.write(encode_response(resp))
                sys.stdout.buffer.flush()


def misbehave_before_answer(work):
    """The modes that never answer. Returns only for answering modes."""
    mode = work["mode"]
    if mode == "crash":
        log("crash requested; exiting with 3")
        os._exit(3)
    if mode == "huge-stderr":
        sys.stderr.write("x" * 20000 + "\n")
        sys.stderr.flush()
        os._exit(4)
    if mode == "garbage":
        log("garbage requested")
        sys.stdout.write("this is not a WorkResponse\n")
        sys.stdout.flush()
        os._exit(0)
    if mode == "empty":
        log("empty requested")
        os._exit(0)


def handle(req, out, verbose_seen):
    if req["cancel"]:
        log("cancel request %d" % req["requestId"])
        out.send({"requestId": req["requestId"], "wasCancelled": True})
        return
    if req["verbosity"] and not verbose_seen[0]:
        verbose_seen[0] = True
        log("verbosity %d" % req["verbosity"])
    work = parse_work(req["arguments"])
    misbehave_before_answer(work)
    if work["mode"] == "cancelled":
        out.send({"requestId": req["requestId"], "wasCancelled": True})
        return
    try:
        code, msg = do_work(work, req["sandboxDir"])
    except Exception as e:  # noqa: BLE001
        code, msg = 2, "probe worker: %s: %s\n" % (type(e).__name__, e)
    resp = {"exitCode": code, "output": msg}
    if req["requestId"]:
        resp["requestId"] = req["requestId"]
    out.send(resp)


def read_request(json_protocol):
    if json_protocol:
        line = sys.stdin.readline()
        if not line:
            return None
        if not line.strip():
            return read_request(json_protocol)
        d = json.loads(line)
        return {"arguments": d.get("arguments", []), "inputs": d.get("inputs", []),
                "requestId": d.get("requestId", 0), "cancel": d.get("cancel", False),
                "verbosity": d.get("verbosity", 0), "sandboxDir": d.get("sandboxDir", "")}
    size = read_varint(sys.stdin.buffer)
    if size is None:
        return None
    data = sys.stdin.buffer.read(size)
    return decode_request(data)


def main(argv):
    if "--persistent_worker" not in argv:
        work = parse_work(expand_flagfiles(argv))
        if work["mode"] in ("crash", "garbage", "empty", "huge-stderr", "slow", "cancelled"):
            work["mode"] = "ok"
        code, msg = do_work(work, "")
        sys.stderr.write(msg)
        return code
    json_protocol = "--protocol=json" in argv
    multiplex = "--multiplex" in argv
    log("started (%s, %s)" % ("json" if json_protocol else "proto",
                              "multiplex" if multiplex else "singleplex"))
    out = Out(json_protocol)
    verbose_seen = [False]
    threads = []
    while True:
        req = read_request(json_protocol)
        if req is None:
            log("stdin closed; exiting")
            break
        if multiplex:
            t = threading.Thread(target=handle, args=(req, out, verbose_seen))
            t.start()
            threads.append(t)
        else:
            handle(req, out, verbose_seen)
    for t in threads:
        t.join()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
