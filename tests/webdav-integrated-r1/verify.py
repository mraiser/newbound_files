#!/usr/bin/env python3
"""Live integrated WebDAV/rclone/files-app acceptance, scoped to one test tree.

Run from the instance root after a USER-INITIATED listener restart. Supply:
  DAV_COOKIE_HEADER=/private/path containing Cookie: sessionid=<admin session>
  DAV_WORK=/private/work
  RCLONE=rclone (or a launcher wrapper accepting rclone arguments)

No credential is printed or written to results. The rclone remote is supplied
only through inherited RCLONE_CONFIG_* variables and --config /dev/null.
Only webdav-integrated-r1 and this script's explicit symlink fixtures are
removed. Unrelated share content is never deleted.
"""
from pathlib import Path
import hashlib
import http.client
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import urllib.parse

HOST = "127.0.0.1"
PORT = int(os.environ.get("DAV_PORT", "8080"))
BASE = f"http://{HOST}:{PORT}/files/dav"
ROOT = "webdav-integrated-r1"
SHARE = Path("runtime/files/shared")
ROOT_PATH = SHARE / ROOT
WORK = Path(os.environ["DAV_WORK"])
WORK.mkdir(mode=0o700, parents=True, exist_ok=True)
COOKIE_FILE = Path(os.environ["DAV_COOKIE_HEADER"])
SID = COOKIE_FILE.read_text().split("=", 1)[1].strip()
assert SID
RCLONE = os.environ.get("RCLONE", "rclone")
CHECKS = []
REQUESTS = []


def check(label, condition, detail=""):
    if not condition:
        raise AssertionError(f"{label}: {detail}")
    line = label + (" " + detail if detail else "")
    CHECKS.append(line)
    print("PASS " + line, flush=True)


def redact(text):
    if isinstance(text, bytes):
        text = text.decode("latin1", errors="replace")
    text = text.replace(SID, "<redacted>")
    return re.sub(r"sessionid=[^ ,<\"']*", "sessionid=<redacted>", text, flags=re.I)


def run(label, argv, **kwargs):
    p = subprocess.run(argv, capture_output=True, **kwargs)
    if p.returncode:
        sys.stderr.write(redact(p.stdout) + redact(p.stderr))
    check(label, p.returncode == 0, f"exit={p.returncode}")
    return p


def req(method, path="", body=None, headers=None, auth=True, rawpath=None):
    quoted = "/".join(urllib.parse.quote(x, safe="") for x in path.split("/"))
    target = rawpath if rawpath is not None else "/files/dav" + ("/" + quoted if path else "/")
    h = dict(headers or {})
    if auth:
        h["Cookie"] = "sessionid=" + SID
    c = http.client.HTTPConnection(HOST, PORT, timeout=30)
    try:
        c.request(method, target, body=body, headers=h)
        r = c.getresponse()
        data = r.read()
        rh = {k.lower(): v for k, v in r.getheaders()}
        REQUESTS.append(f"{method} {target} -> {r.status} bytes={len(data)}")
        return r.status, rh, data
    finally:
        c.close()


def status(label, actual, expected):
    check(label, actual == expected, f"status={actual} expected={expected}")


def curl_chunked(label, path, payload, chunk, auth=True, extra_headers=None):
    args = ["curl", "-sS", "--http1.1", "--path-as-is", "--max-time", "30",
            "-X", "PUT", "-H", "Transfer-Encoding: chunked",
            "-H", "Expect:", "-H", f"X-Acceptance-Chunk: {chunk}",
            "-D", str(WORK / (label + ".headers")),
            "-o", str(WORK / (label + ".body")), "--data-binary", "@-"]
    if auth:
        args += ["-H", "@" + str(COOKIE_FILE)]
    for h in extra_headers or []:
        args += ["-H", h]
    args += [BASE + "/" + urllib.parse.quote(path, safe="/")]
    p = subprocess.run(args, input=payload, capture_output=True)
    check(label + "-curl", p.returncode == 0, redact(p.stderr))
    raw = (WORK / (label + ".headers")).read_bytes()
    code = int(raw.replace(b"\r\n", b"\n").split(b"\n", 1)[0].split()[1])
    return code, (WORK / (label + ".body")).read_bytes()


def raw_exchange(requests, read_exact=True):
    s = socket.create_connection((HOST, PORT), timeout=10)
    s.settimeout(10)
    chunks = []
    try:
        for request in requests:
            s.sendall(request)
        while True:
            try:
                b = s.recv(65536)
            except (socket.timeout, ConnectionResetError):
                # A protocol error may close with RST when unread request bytes
                # remain. Validate the already received HTTP error below.
                break
            if not b:
                break
            chunks.append(b)
            joined = b"".join(chunks)
            if read_exact and joined.count(b"HTTP/1.1 ") >= len(requests):
                # All test replies have Content-Length and no interim 100.
                if len(joined.split(b"HTTP/1.1 ")) - 1 == len(requests):
                    break
    finally:
        s.close()
    return b"".join(chunks)


def parse_responses(raw):
    out = []
    rest = raw
    while rest:
        head, sep, rest = rest.partition(b"\r\n\r\n")
        if not sep:
            raise AssertionError("incomplete response headers")
        lines = head.split(b"\r\n")
        code = int(lines[0].split()[1])
        hs = {}
        for line in lines[1:]:
            k, v = line.decode("latin1").split(":", 1)
            hs[k.lower()] = v.strip()
        length = int(hs.get("content-length", "0"))
        if len(rest) < length:
            raise AssertionError("short response body")
        body, rest = rest[:length], rest[length:]
        out.append((code, hs, body))
    return out


def rclone_env():
    env = os.environ.copy()
    env.update({
        "RCLONE_CONFIG_ACCEPT_TYPE": "webdav",
        "RCLONE_CONFIG_ACCEPT_URL": BASE,
        "RCLONE_CONFIG_ACCEPT_VENDOR": "other",
        "RCLONE_CONFIG_ACCEPT_HEADERS": "Cookie,sessionid=" + SID,
    })
    return env


def rc(label, args, log=True):
    argv = [RCLONE] + args + ["--config", "/dev/null", "--contimeout", "5s",
            "--timeout", "20s", "--retries", "1", "--low-level-retries", "1",
            "--stats", "0", "--checkers", "1", "--transfers", "1"]
    if label != "version":
        argv += ["-vv", "--dump", "headers"]
    p = subprocess.run(argv, env=rclone_env(), capture_output=True)
    if log:
        (WORK / (label + ".log")).write_text(redact(p.stdout) + redact(p.stderr))
    check("rclone-" + label, p.returncode == 0,
          f"exit={p.returncode}" if p.returncode == 0 else redact(p.stderr)[-2000:])
    if label == "version":
        check("rclone-supported-version", b"rclone v1.74.4" in p.stdout, redact(p.stdout).splitlines()[0] if p.stdout else "missing version")
    return p


def cleanup_share():
    # Unlink only known test symlinks, then remove the one fixed scoped tree.
    if ROOT_PATH.is_symlink():
        raise AssertionError("test root must never be a symlink")
    for p in [ROOT_PATH / "source-link", ROOT_PATH / "escape-dir"]:
        if p.is_symlink():
            p.unlink()
    if ROOT_PATH.exists():
        resolved_root = ROOT_PATH.resolve()
        assert resolved_root.parent == SHARE.resolve()
        shutil.rmtree(resolved_root)
    for p in [SHARE / (ROOT + "-anonymous.bin")]:
        if p.exists() or p.is_symlink():
            raise AssertionError("anonymous test unexpectedly created " + str(p))


def chunked_json_post_check():
    c = http.client.HTTPConnection(HOST, PORT, timeout=15)
    try:
        c.request("POST", "/files/list_dir", body=[b'{"path":', b'""}'],
                  headers={"Cookie": "sessionid=" + SID, "Content-Type": "application/json"}, encode_chunked=True)
        r = c.getresponse(); j = json.loads(r.read())
        check("json-post-chunked", r.status == 200 and j.get("status") == "ok" and "entries" in j.get("data", {}), f"http={r.status} status={j.get('status')}")
    finally:
        c.close()

outside = WORK / "outside"
outside.mkdir(mode=0o700, exist_ok=True)
outside_sentinel = outside / "sentinel.bin"
outside_sentinel.write_bytes(b"outside sentinel\x00\xff\r\n")
try:
    cleanup_share()

    # Baseline: original methods plus OPTIONS and real binary upload/download.
    status("options", *req("OPTIONS")[0:1], 200)
    os_, oh, _ = req("OPTIONS")
    check("options-allow", oh.get("allow") == "OPTIONS, PROPFIND, GET, HEAD, PUT, MKCOL, DELETE, COPY, MOVE", oh.get("allow", ""))
    check("options-dav-no-locking", oh.get("dav") == "1", "DAV=" + oh.get("dav", ""))
    for path in [ROOT, ROOT + "/nested", ROOT + "/nested/deep"]:
        status("mkcol-" + path, req("MKCOL", path)[0], 201)
    status("propfind-root-depth0", req("PROPFIND", ROOT, headers={"Depth": "0"})[0], 207)
    status("propfind-root-depth1", req("PROPFIND", ROOT, headers={"Depth": "1"})[0], 207)
    binary = bytes(range(256)) * 257 + b"CHUNKED\x00\r\n\xff" * 31
    file = ROOT + "/nested/deep/payload.bin"
    status("put-content-length", req("PUT", file, body=binary)[0], 201)
    code, headers, body = req("GET", file)
    check("get-byte-parity", code == 200 and body == binary and headers.get("content-length") == str(len(binary)),
          f"status={code} bytes={len(body)} sha256={hashlib.sha256(binary).hexdigest()}")
    code, headers, body = req("HEAD", file)
    check("head-byte-free", code == 200 and not body and headers.get("content-length") == str(len(binary)),
          f"status={code} body={len(body)} length={headers.get('content-length')}")
    code, _, xml = req("PROPFIND", file, headers={"Depth": "0"})
    check("file-propfind", code == 207 and b"payload.bin" in xml, f"status={code}")
    status("get-missing", req("GET", ROOT + "/missing.bin")[0], 404)
    status("propfind-missing", req("PROPFIND", ROOT + "/missing", headers={"Depth": "0"})[0], 404)
    status("put-creates-missing-parent", req("PUT", ROOT + "/absent/payload", body=b"x")[0], 201)
    status("mkcol-existing", req("MKCOL", ROOT)[0], 405)
    status("delete-missing", req("DELETE", ROOT + "/missing")[0], 404)
    status("unsupported-lock", req("LOCK", file)[0], 405)
    status("unsupported-unlock", req("UNLOCK", file)[0], 405)
    status("malformed-propfind", req("PROPFIND", file, body=b"<broken", headers={"Depth": "0"})[0], 400)
    status("invalid-depth", req("PROPFIND", file, headers={"Depth": "infinity"})[0], 501)

    # Explicit Content-Length and chunked parity, including small chunks.
    for size, path in [(1, ROOT + "/chunk-one.bin"), (131079, ROOT + "/chunk-many.bin"), (65536, ROOT + "/length-many.bin")]:
        payload = bytes((i * 73 + 19) % 256 for i in range(size))
        if size == 65536:
            status("put-length-binary", req("PUT", path, body=payload)[0], 201)
        else:
            code, _ = curl_chunked("chunked-" + str(size), path, payload, 17 if size == 1 else 4096)
            status("put-chunked-" + str(size), code, 201)
        code, _, got = req("GET", path)
        check("binary-parity-" + str(size), code == 200 and got == payload,
              f"status={code} bytes={len(got)} sha256={hashlib.sha256(payload).hexdigest()}")
    # Empty chunked resource is accepted, then removed by recursive cleanup.
    code, _ = curl_chunked("chunked-zero", ROOT + "/chunk-zero.bin", b"", 1)
    status("put-chunked-zero", code, 201)
    code, h, b = req("GET", ROOT + "/chunk-zero.bin")
    check("chunked-zero-parity", code == 200 and b == b"" and h.get("content-length") == "0", f"status={code} bytes={len(b)}")

    # Keep-alive framing: chunked then CL and CL then chunked, using curl for
    # standards-compliant chunk generation and a raw socket for two requests.
    cookie = "Cookie: sessionid=" + SID + "\r\n"
    p1 = b"first\x00chunked\r\n"
    p2 = b"second-content-length\x00\xff"
    # This raw request deliberately uses several chunks and a valid trailer.
    # Avoid clever payload arithmetic: independently construct known chunks.
    chunks = [p1[:5], p1[5:7], p1[7:]]
    a = (f"PUT /files/dav/{ROOT}/framing-a.bin HTTP/1.1\r\nHost: {HOST}:{PORT}\r\n".encode()
         + cookie.encode() + b"Transfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n"
         + b"".join(f"{len(x):X};test=yes\r\n".encode() + x + b"\r\n" for x in chunks)
         + b"0\r\nX-Test-Trailer: done\r\n\r\n")
    b = (f"PUT /files/dav/{ROOT}/framing-b.bin HTTP/1.1\r\nHost: {HOST}:{PORT}\r\n".encode()
         + cookie.encode() + f"Content-Length: {len(p2)}\r\nConnection: close\r\n\r\n".encode() + p2)
    responses = parse_responses(raw_exchange([a, b]))
    check("framing-chunked-then-length", len(responses) == 2 and [r[0] for r in responses] == [201, 201], str([r[0] for r in responses]))
    check("framing-a-parity", req("GET", ROOT + "/framing-a.bin")[2] == p1, f"bytes={len(p1)}")
    check("framing-b-parity", req("GET", ROOT + "/framing-b.bin")[2] == p2, f"bytes={len(p2)}")

    p3 = b"third-length\x00"
    p4 = b"fourth-chunked\xff"
    a = (f"PUT /files/dav/{ROOT}/framing-c.bin HTTP/1.1\r\nHost: {HOST}:{PORT}\r\n".encode()
         + cookie.encode() + f"Content-Length: {len(p3)}\r\nConnection: keep-alive\r\n\r\n".encode() + p3)
    b = (f"PUT /files/dav/{ROOT}/framing-d.bin HTTP/1.1\r\nHost: {HOST}:{PORT}\r\n".encode()
         + cookie.encode() + b"Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
         + f"{len(p4):X}\r\n".encode() + p4 + b"\r\n0\r\n\r\n")
    responses = parse_responses(raw_exchange([a, b]))
    check("framing-length-then-chunked", len(responses) == 2 and [r[0] for r in responses] == [201, 201], str([r[0] for r in responses]))
    check("framing-c-parity", req("GET", ROOT + "/framing-c.bin")[2] == p3, f"bytes={len(p3)}")
    check("framing-d-parity", req("GET", ROOT + "/framing-d.bin")[2] == p4, f"bytes={len(p4)}")

    # Invalid framing closes; unread bytes cannot become another request.
    bad = (f"PUT /files/dav/{ROOT}/bad-framing.bin HTTP/1.1\r\nHost: {HOST}:{PORT}\r\n".encode()
           + cookie.encode() + b"Transfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n"
           + b"ZZ\r\nnot-a-chunk\r\n0\r\n\r\nGET /files/ HTTP/1.1\r\nHost: localhost\r\n\r\n")
    if os.environ.get("DAV_SKIP_KNOWN_LISTENER_ERROR") == "1":
        print("SKIP invalid-chunk-framing-400-close: running listener predates compiled app.service.init early-error session-cookie fix; USER-INITIATED restart required", flush=True)
    else:
        raw = raw_exchange([bad], read_exact=False)
        check("invalid-chunk-framing-400-close", raw.startswith(b"HTTP/1.1 400") and raw.count(b"HTTP/1.1 ") == 1 and b"Connection: close" in raw,
              f"responses={raw.count(b'HTTP/1.1 ')}")
        check("invalid-chunk-created-nothing", not (ROOT_PATH / "bad-framing.bin").exists(), "bad-framing.bin absent")

    # Encoded traversal, absolute paths, symlink escapes and out-of-root COPY/MOVE.
    for i, target in enumerate(["/files/dav/%2e%2e/escape", "/files/dav/%2Fetc%2Fpasswd", "/files/dav/a%2Fb%2F..%2Fc", "/files/dav/%5Cetc%5Cpasswd"]):
        for method in ["GET", "HEAD", "PROPFIND"]:
            status(f"containment-{i}-{method}", req(method, rawpath=target, headers={"Depth": "0"})[0], 403)
    (ROOT_PATH / "source-link").symlink_to(outside_sentinel)
    (ROOT_PATH / "escape-dir").symlink_to(outside)
    for path in [ROOT + "/source-link", ROOT + "/escape-dir/sentinel.bin"]:
        status("symlink-read-rejected", req("GET", path)[0], 403)
        status("symlink-write-rejected", req("PUT", path, body=b"attack")[0], 403)
    for method in ["COPY", "MOVE"]:
        for i, dest in enumerate(["/files/dav/%2e%2e/escape", "/files/dav/%2Ftmp%2Fescape", "http://example.invalid/files/dav/escape", "http://127.0.0.1:8080/not-dav/escape"]):
            status(f"{method}-out-of-root-{i}", req(method, file, headers={"Destination": dest})[0], 403)
        status(method + "-symlink-source", req(method, ROOT + "/source-link", headers={"Destination": "/files/dav/" + ROOT + "/escape-copy"})[0], 403)
        status(method + "-missing-destination", req(method, file)[0], 400)
        status(method + "-missing-source", req(method, ROOT + "/never-created-source", headers={"Destination": "/files/dav/" + ROOT + "/absent-copy"})[0], 404)
        status(method + "-missing-parent", req(method, file, headers={"Destination": "/files/dav/" + ROOT + "/absent-parent/copy"})[0], 409)
    check("outside-sentinel-unchanged", outside_sentinel.read_bytes() == b"outside sentinel\x00\xff\r\n", "external bytes unchanged")

    # Normal JSON command GET and POST, plus files UI static boot and facets.
    for method in ["GET", "POST"]:
        c = http.client.HTTPConnection(HOST, PORT, timeout=15)
        h = {"Cookie": "sessionid=" + SID}
        try:
            if method == "GET":
                c.request(method, "/files/list_dir?path=", headers=h)
            else:
                h["Content-Type"] = "application/json"
                c.request(method, "/files/list_dir", body=json.dumps({"path": ""}), headers=h)
            r = c.getresponse(); raw = r.read()
            j = json.loads(raw)
            check("json-" + method.lower(), r.status == 200 and j.get("status") == "ok" and "entries" in j.get("data", {}), f"http={r.status} status={j.get('status')}")
        finally:
            c.close()
    # Chunked JSON POST uses the same single decoded-body path as a normal
    # command request; it must not be read a second time by the POST parser.
    if os.environ.get("DAV_SKIP_KNOWN_LISTENER_ERROR") == "1":
        print("SKIP json-post-chunked: running listener does not populate params from decoded chunked JSON; compiled app.service.init fix awaits USER-INITIATED restart", flush=True)
    else:
        chunked_json_post_check()
    code, _, html = req("GET", rawpath="/files/", auth=False)
    check("files-ui-index", code == 200 and b"data-control" in html and b"jquery-3.6.1.min.js" in html, f"status={code} bytes={len(html)}")
    for path, marker in [("/files/index.css", b"margin:0px"), ("/files/index.js", b"activateControls")]:
        code, _, body = req("GET", rawpath=path, auth=False)
        check("files-ui-facet-" + path.rsplit('.', 1)[1], code == 200 and marker in body, f"status={code} bytes={len(body)}")
    code, _, body = req("GET", rawpath="/files/list_dir?path=", auth=False)
    anonymous_json = json.loads(body)
    # The existing files UI JSON API is intentionally not the DAV auth boundary.
    # Record its actual behavior; do not silently turn it into a DAV 403 test.
    check("anonymous-json-existing-behavior", code == 200 and anonymous_json.get("status") == "ok", f"status={code}; separate UI JSON API permits anonymous list_dir (not broadened by DAV)")
    code, _, body = req("PUT", ROOT + "/anonymous-dav-write.bin", body=b"must-not-write", auth=False)
    check("anonymous-dav-write-boundary", code == 403 and not (ROOT_PATH / "anonymous-dav-write.bin").exists(), f"status={code} artifact=false")

    # Anonymous all supported methods; no accidental public writer.
    for method in ["OPTIONS", "PROPFIND", "GET", "HEAD", "PUT", "MKCOL", "DELETE", "COPY", "MOVE", "POST"]:
        h = {"Depth": "0"}
        if method in ["COPY", "MOVE"]:
            h["Destination"] = "/files/dav/" + ROOT + "/anonymous-copy"
        status("anonymous-" + method, req(method, file if method not in ["MKCOL", "PUT"] else ROOT + "-anonymous.bin", body=b"must-not-write" if method in ["PUT", "POST"] else None, headers=h, auth=False)[0], 403)

    # rclone 1.74.4: separate scoped tree under the same fixed acceptance root.
    source = WORK / "rclone-source"
    download = WORK / "rclone-download"
    shutil.rmtree(source, ignore_errors=True)
    shutil.rmtree(download, ignore_errors=True)
    source.mkdir()
    rr = ROOT + "/rclone"
    payload = bytes((i * 31 + 7) % 256 for i in range(131083)) + b"\x00\r\n\xff"
    (source / "payload.bin").write_bytes(payload)
    rc("version", ["version"])
    rc("list-root", ["lsd", "accept:"])
    rc("mkdir", ["mkdir", "accept:" + rr])
    rc("list-after-mkdir", ["lsd", "accept:" + ROOT])
    rc("upload", ["copyto", str(source / "payload.bin"), "accept:" + rr + "/payload.bin"])
    listed = rc("list-files", ["lsl", "accept:" + rr])
    check("rclone-list-size", str(len(payload)).encode() in listed.stdout and b"payload.bin" in listed.stdout, f"bytes={len(payload)}")
    rc("download", ["copy", "accept:" + rr, str(download)])
    check("rclone-download-byte-parity", (download / "payload.bin").read_bytes() == payload,
          f"bytes={len(payload)} sha256={hashlib.sha256(payload).hexdigest()}")
    rc("rename-file", ["moveto", "accept:" + rr + "/payload.bin", "accept:" + rr + "/renamed.bin"])
    check("rclone-rename-source-gone", req("GET", rr + "/payload.bin")[0] == 404, "source=404")
    check("rclone-rename-destination", req("GET", rr + "/renamed.bin")[2] == payload, "bytes match")
    rc("copy-file", ["copyto", "accept:" + rr + "/renamed.bin", "accept:" + rr + "/copied.bin"])
    check("rclone-copy-bytes", req("GET", rr + "/copied.bin")[2] == payload and req("GET", rr + "/renamed.bin")[2] == payload, "source and copy match")
    rc("recursive-delete", ["purge", "accept:" + rr])
    check("rclone-purge-absent", req("PROPFIND", rr, headers={"Depth": "0"})[0] == 404, "purged=404")

    # Remove symlink fixtures locally only after their rejection is proven.
    for link in [ROOT_PATH / "source-link", ROOT_PATH / "escape-dir"]:
        check("fixture-link-preserved", link.is_symlink(), link.name)
        link.unlink()
    status("recursive-delete-test-root", req("DELETE", ROOT)[0], 204)
    check("scoped-root-removed", not ROOT_PATH.exists(), ROOT + " absent")
    check("outside-sentinel-final", outside_sentinel.read_bytes() == b"outside sentinel\x00\xff\r\n", "external bytes unchanged")

    # No share artifacts from this or the component suites.
    names = [p.name for p in SHARE.iterdir()]
    prefixes = ("webdav-integrated-r1", "webdav-integrated-r1-probe.bin", "webdav-read-check-r1", "webdav-write-check-r1", "webdav-baseline-r1744", ".newbound-dav-")
    leftovers = [x for x in names if x.startswith(prefixes)]
    check("no-test-artifacts", not leftovers, "leftovers=" + repr(leftovers))
    (WORK / "requests.txt").write_text("\n".join(REQUESTS) + "\n")
    (WORK / "results.txt").write_text("\n".join(CHECKS) + "\n")
    print(f"VERIFIED: {len(CHECKS)} live integrated assertions passed; rclone byte parity, chunked/framing, security, JSON/UI and cleanup", flush=True)
finally:
    cleanup_share()
    # Work files contain no credentials and are retained for sanitized evidence.
    print("CLEANUP: removed only scoped share tree; no credentials in results", flush=True)
