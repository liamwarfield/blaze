"""HTTP cache stand-in for scripts/remote-http/early-answer-probe (after the round-8 review probe).

usage: early-answer-server.py PORT|unix:PATH MODE LOG [CERT_PEM KEY_PEM]
       early-answer-server.py help

MODE store: a plain store (PUT 200, GET 200/404).
MODE early: a PUT to /cas/ with Content-Length > 1 MiB is answered 403 at once, the body is not
            read and the connection is held open for 30 s; everything else as store.
MODE auth:  every request without "Authorization: Bearer good" is answered 401 with
            `error="invalid_token"`; for a PUT with Content-Length > 1 MiB before its body is
            read (the connection then held open for 30 s); everything else as store.
MODE snappy: a store whose GET answers carry `Content-Encoding: snappy` (the framing format:
            the stream identifier, then uncompressed-data chunks of at most 65536 bytes with a
            zero checksum, then one compressed chunk of a literal and a copy for the last bytes).
MODE snappy-bad: snappy, with a reserved unskippable chunk (type 0x02) after the identifier.
MODE slow: a store that reads the body of a PUT at 64 KiB per second (logs when the client
            closes the connection, or when the body is complete, in seconds after the head).
With CERT_PEM and KEY_PEM the server speaks TLS. Every request is logged to LOG (method, path, length, authorization),
each line with a suffix ` @<ms since the first connection> c<connection number>`; each accepted connection is
logged as `  accept`."""
import os, socket, ssl, sys, threading, time, struct
if len(sys.argv) < 2 or sys.argv[1] in ("help", "-h", "--help"):
    print(__doc__)
    sys.exit(0)
port, mode, log = sys.argv[1], sys.argv[2], sys.argv[3]
tls = None
if len(sys.argv) > 5:
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(sys.argv[4], sys.argv[5])
store = {}
lock = threading.Lock()
UNAUTHORIZED = (b"HTTP/1.1 401 Unauthorized\r\n"
                b"WWW-Authenticate: Bearer realm=\"x\",error=\"invalid_token\"\r\n"
                b"Content-Length: 0\r\n\r\n")
def chunk(kind, body):
    return bytes([kind]) + struct.pack("<I", len(body))[:3] + body
def snappy_frame(data, bad):
    out = chunk(0xff, b"sNaPpY")
    if bad:
        out += chunk(0x02, b"")
    tail = data[-6:] if len(data) >= 6 else b""
    head = data[: len(data) - len(tail)]
    for i in range(0, len(head), 65536):
        out += chunk(0x01, b"\0\0\0\0" + head[i:i + 65536])
    if tail:
        # preamble 6, literal of 3 bytes, then a copy of 3 bytes at offset 3 when the tail repeats,
        # else a literal of the other 3.
        if tail[:3] == tail[3:]:
            body = bytes([6, 2 << 2]) + tail[:3] + bytes([0b10 | (2 << 2), 3, 0])
        else:
            body = bytes([6, 5 << 2]) + tail
        out += chunk(0x00, b"\0\0\0\0" + body)
    return out
T0 = []
local = threading.local()
def note(s):
    now = time.monotonic()
    with lock:
        if not T0:
            T0.append(now)
        s = "%s @%.1fms c%s" % (s, (now - T0[0]) * 1000, getattr(local, "conn", "?"))
        open(log, "a").write(s + "\n")
def serve(c):
    f = c.makefile("rb")
    try:
        while True:
            line = f.readline()
            if not line:
                return
            method, path, _ = line.decode().split(" ", 2)
            hdr = {}
            while True:
                h = f.readline().decode().strip()
                if not h:
                    break
                k, v = h.split(":", 1)
                hdr[k.strip().lower()] = v.strip()
            n = int(hdr.get("content-length", "0"))
            auth = hdr.get("authorization", "")
            note(f"{method} {path} len={n} auth={auth!r}")
            big = method == "PUT" and n > (1 << 20)
            if mode == "early" and big and path.startswith("/cas/"):
                c.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 6\r\n\r\nnope!\n")
                note("  answered 403 before the body")
                time.sleep(30)
                return
            if mode == "auth" and auth != "Bearer good":
                if big:
                    c.sendall(UNAUTHORIZED)
                    note("  answered 401 before the body")
                    time.sleep(30)
                    return
                if method == "PUT":
                    f.read(n)
                c.sendall(UNAUTHORIZED)
                note("  answered 401")
                continue
            if method == "PUT":
                if mode == "slow":
                    body = b""
                    t0 = time.monotonic()
                    while len(body) < n:
                        piece = f.read(min(65536, n - len(body)))
                        if not piece:
                            note("  client closed after %d bytes at %.1f s" % (len(body), time.monotonic() - t0))
                            return
                        body += piece
                        time.sleep(1)
                    note("  body complete at %.1f s" % (time.monotonic() - t0))
                    store[path] = body
                else:
                    store[path] = f.read(n)
                c.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            else:
                b = store.get(path)
                if b is None:
                    c.sendall(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                elif mode.startswith("snappy"):
                    framed = snappy_frame(b, mode == "snappy-bad")
                    c.sendall(b"HTTP/1.1 200 OK\r\nContent-Encoding: snappy\r\n"
                              b"Content-Length: %d\r\n\r\n" % len(framed) + framed)
                else:
                    c.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n" % len(b) + b)
    except Exception as e:
        note(f"  error {e!r}")
    finally:
        c.close()
if port.startswith("unix:"):
    path = port[len("unix:"):]
    if os.path.exists(path):
        os.remove(path)
    s = socket.socket(socket.AF_UNIX)
    s.bind(path)
else:
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", int(port)))
s.listen(64)
def handshake_and_serve(c, n):
    local.conn = n
    note("  accept")
    if tls is not None:
        try:
            c = tls.wrap_socket(c, server_side=True)
        except Exception as e:
            note(f"  tls error {e!r}")
            c.close()
            return
    serve(c)
accepted = 0
while True:
    c, _ = s.accept()
    accepted += 1
    threading.Thread(target=handshake_and_serve, args=(c, accepted), daemon=True).start()
