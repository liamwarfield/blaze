"""HTTP cache stand-in for scripts/remote-http/early-answer-probe (after the round-8 review probe).

usage: early-answer-server.py PORT MODE LOG
       early-answer-server.py help

MODE store: a plain store (PUT 200, GET 200/404).
MODE early: a PUT to /cas/ with Content-Length > 1 MiB is answered 403 at once, the body is not
            read and the connection is held open for 30 s; everything else as store.
MODE auth:  every request without "Authorization: Bearer good" is answered 401 with
            `error="invalid_token"`; for a PUT with Content-Length > 1 MiB before its body is
            read (the connection then held open for 30 s); everything else as store.
Every request is logged to LOG (method, path, length, authorization)."""
import socket, sys, threading, time
if len(sys.argv) < 2 or sys.argv[1] in ("help", "-h", "--help"):
    print(__doc__)
    sys.exit(0)
port, mode, log = int(sys.argv[1]), sys.argv[2], sys.argv[3]
store = {}
lock = threading.Lock()
UNAUTHORIZED = (b"HTTP/1.1 401 Unauthorized\r\n"
                b"WWW-Authenticate: Bearer realm=\"x\",error=\"invalid_token\"\r\n"
                b"Content-Length: 0\r\n\r\n")
def note(s):
    with lock:
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
                store[path] = f.read(n)
                c.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            else:
                b = store.get(path)
                if b is None:
                    c.sendall(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                else:
                    c.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n" % len(b) + b)
    except Exception as e:
        note(f"  error {e!r}")
    finally:
        c.close()
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", port)); s.listen(64)
while True:
    c, _ = s.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
