#!/usr/bin/env python3
"""Authenticated curl verification for scoped files.dav HEAD/PROPFIND.
Run from the instance root. Set DAV_COOKIE_HEADER to a chmod-600 file whose
single line is 'Cookie: sessionid=<temporary admin session>'. The credential is
never printed or included in the report. The script deletes only its own tree.
"""
from pathlib import Path
import os
import shutil
import socket
import subprocess
import urllib.parse
import xml.etree.ElementTree as ET

work = Path(os.environ.get("DAV_WORK", "/tmp/newbound-dav-read-r1"))
work.mkdir(mode=0o700, parents=True, exist_ok=True)
cookie_file = Path(os.environ["DAV_COOKIE_HEADER"])
cookie = ["-H", "@" + str(cookie_file)]
base = "http://127.0.0.1:8080/files/dav"
D = "{DAV:}"
checks = []
root = "webdav-read-check-r1"
folder = root + "/space & < café 集合"
name = "résumé & < data.txt"
file = folder + "/" + name
payload = ("Newbound WebDAV UTF-8: café 集合 & <tag>\n" * 20).encode()
share = Path("runtime/files/shared")
root_path = share / root
outside = work / "outside.txt"


def quote(path):
    return "/".join(urllib.parse.quote(part, safe="") for part in path.split("/"))


def request(method, path="", body=None, depth=None, rawpath=None, auth=True):
    number = f"{len(checks):02d}-{method.lower()}"
    header_file = work / (number + ".headers")
    body_file = work / (number + ".body")
    url = base + (("/" + quote(path)) if path else "/") if rawpath is None else "http://127.0.0.1:8080" + rawpath
    cmd = ["curl", "-sS", "--path-as-is", "--max-time", "10"]
    if auth:
        cmd += cookie
    if method == "HEAD":
        # -I is HEAD semantics; -X HEAD makes curl expect Content-Length bytes.
        cmd += ["-I", "-o", str(body_file)]
    else:
        cmd += ["-X", method, "-D", str(header_file), "-o", str(body_file)]
    if depth is not None:
        cmd += ["-H", "Depth: " + depth]
    if body is not None:
        cmd += ["--data-binary", "@-"]
    cmd += [url]
    process = subprocess.run(cmd, input=body, capture_output=True)
    assert process.returncode == 0, (method, path, process.stderr.decode())
    if method == "HEAD":
        raw = body_file.read_bytes()
        header, separator, tail = raw.partition(b"\r\n\r\n")
        assert separator and not tail, ("curl HEAD framing", len(raw), len(tail))
        header_file.write_bytes(header + b"\r\n\r\n")
        body_bytes = tail
    else:
        body_bytes = body_file.read_bytes()
    lines = header_file.read_bytes().replace(b"\r\n", b"\n").split(b"\n")
    status = int(lines[0].split()[1])
    headers = {}
    for line in lines[1:]:
        if b":" in line:
            key, value = line.decode("latin1").split(":", 1)
            headers[key.lower()] = value.strip()
    return status, headers, body_bytes


def check(label, condition, detail):
    assert condition, label + ": " + detail
    checks.append(label + " " + detail)
    print("PASS " + checks[-1])


try:
    status, _, _ = request("MKCOL", root)
    check("mkcol-root", status == 201, str(status))
    status, _, _ = request("MKCOL", folder)
    check("mkcol-special", status == 201, str(status))
    status, _, _ = request("PUT", file, payload)
    check("put-file", status == 201, str(status))
    get_status, get_headers, get_body = request("GET", file)
    check("get-file", get_status == 200 and get_body == payload, f"status={get_status} bytes={len(get_body)}")
    status, headers, body = request("HEAD", file)
    check("head-get", status == get_status and headers.get("content-type") == get_headers.get("content-type") and headers.get("content-length") == get_headers.get("content-length") == str(len(payload)), f"status={status} type={headers.get('content-type')} length={headers.get('content-length')}")
    check("head-empty-curl", not body, f"body_bytes={len(body)}")

    cookie_value = cookie_file.read_text().strip()
    sock = socket.create_connection(("127.0.0.1", 8080), 5)
    sock.settimeout(2)
    request_text = f"HEAD /files/dav/{quote(file)} HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nConnection: close\r\n{cookie_value}\r\n\r\n"
    sock.sendall(request_text.encode())
    raw = b""
    try:
        while True:
            chunk = sock.recv(65536)
            if not chunk:
                break
            raw += chunk
    except socket.timeout:
        pass
    finally:
        sock.close()
    _, separator, tail = raw.partition(b"\r\n\r\n")
    check("head-empty-raw", separator == b"\r\n\r\n" and not tail, f"response_body_bytes={len(tail)}")

    # Containment errors must not expose their GET error body on HEAD either.
    for rawpath in ["/files/dav/%2e%2e/outside.txt", "/files/dav/%2Fetc%2Fpasswd"]:
        sock = socket.create_connection(("127.0.0.1", 8080), 5)
        sock.settimeout(2)
        sock.sendall((f"HEAD {rawpath} HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nConnection: close\r\n{cookie_value}\r\n\r\n").encode())
        raw = b""
        try:
            while True:
                chunk = sock.recv(65536)
                if not chunk:
                    break
                raw += chunk
        except socket.timeout:
            pass
        finally:
            sock.close()
        header, separator, tail = raw.partition(b"\r\n\r\n")
        check("head-forbidden-empty-" + ("traversal" if "2e" in rawpath else "absolute"), header.startswith(b"HTTP/1.1 403") and separator and not tail, "403 with zero body bytes")

    rclone_xml = b'<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:displayname/><d:getlastmodified/><d:getcontentlength/><d:resourcetype/></d:prop></d:propfind>'
    status, _, body = request("PROPFIND", file, rclone_xml, "0")
    xml = ET.fromstring(body)
    responses = xml.findall(D + "response")
    check("file-propfind", status == 207 and len(responses) == 1, f"status={status} responses={len(responses)}")
    response = responses[0]
    href = response.findtext(D + "href")
    properties = response.find(D + "propstat/" + D + "prop")
    expected = "/files/dav/" + quote(file)
    check("file-href", href == expected, href or "missing")
    check("file-props", properties.findtext(D + "displayname") == name and properties.findtext(D + "getcontentlength") == str(len(payload)) and properties.find(D + "resourcetype/" + D + "collection") is None, f"displayname={properties.findtext(D + 'displayname')} length={properties.findtext(D + 'getcontentlength')}")

    status, _, body = request("PROPFIND", folder, None, "0")
    xml = ET.fromstring(body)
    check("collection-depth0", status == 207 and len(xml.findall(D + "response")) == 1, f"responses={len(xml.findall(D + 'response'))}")
    status, _, body = request("PROPFIND", folder, None, "1")
    xml = ET.fromstring(body)
    responses = xml.findall(D + "response")
    check("collection-depth1", status == 207 and len(responses) == 2, f"responses={len(responses)}")
    by_href = {response.findtext(D + "href"): response for response in responses}
    self_href = "/files/dav/" + quote(folder) + "/"
    check("collection-hrefs", self_href in by_href and expected in by_href, "; ".join(by_href))
    child = by_href[expected]
    check("special-xml-text", child.findtext(D + "propstat/" + D + "prop/" + D + "displayname") == name, "unicode, space, ampersand and < round-trip")
    check("collection-type", by_href[self_href].find(D + "propstat/" + D + "prop/" + D + "resourcetype/" + D + "collection") is not None, "D:collection present")

    for label, request_xml in [("allprop", '<D:propfind xmlns:D="DAV:"><D:allprop/></D:propfind>'), ("propname", '<D:propfind xmlns:D="DAV:"><D:propname/></D:propfind>')]:
        status, _, body = request("PROPFIND", file, request_xml.encode(), "0")
        xml = ET.fromstring(body)
        prop = xml.find(D + "response/" + D + "propstat/" + D + "prop")
        names = [element.tag for element in list(prop)]
        check(label, status == 207 and D + "displayname" in names and D + "resourcetype" in names, f"properties={len(names)}")
        if label == "propname":
            check("propname-empty", all(len(element) == 0 and not (element.text or "").strip() for element in list(prop)), "empty property elements")

    selected = '<D:propfind xmlns:D="DAV:" xmlns:Z="urn:test"><D:prop><D:getcontentlength/><Z:unknown/></D:prop></D:propfind>'
    status, _, body = request("PROPFIND", file, selected.encode(), "0")
    xml = ET.fromstring(body)
    statuses = {propstat.findtext(D + "status"): propstat for propstat in xml.findall(D + "response/" + D + "propstat")}
    check("selected-props", status == 207 and "HTTP/1.1 200 OK" in statuses and "HTTP/1.1 404 Not Found" in statuses, "200 found and 404 unknown propstat")
    check("selected-content", statuses["HTTP/1.1 200 OK"].findtext(D + "prop/" + D + "getcontentlength") == str(len(payload)), "requested length returned")

    for label, depth in [("depth-infinity", "infinity"), ("depth-two", "2"), ("depth-negative", "-1")]:
        status, _, _ = request("PROPFIND", root, None, depth)
        check(label, status == 501, f"status={status}")
    for label, rawpath in [("encoded-traversal", "/files/dav/%2e%2e/outside.txt"), ("encoded-absolute", "/files/dav/%2Fetc%2Fpasswd"), ("encoded-backslash", "/files/dav/%5Cetc%5Cpasswd")]:
        status, _, _ = request("PROPFIND", rawpath=rawpath, depth="0")
        check(label, status == 403, f"status={status}")
    # Percent decoding is deliberately single-pass: this is an ordinary name,
    # not a second chance to turn percent escapes into path separators.
    status, _, body = request("PROPFIND", root + "/literal %2e%2e.txt", b"literal percent name", "0")
    assert status == 404
    status, _, _ = request("PUT", root + "/literal %2e%2e.txt", b"literal percent name")
    check("literal-percent-name", status == 201, "double encoding is not traversal")
    status, _, body = request("PROPFIND", root + "/literal %2e%2e.txt", None, "0")
    xml = ET.fromstring(body)
    check("literal-percent-href", status == 207 and xml.findtext(D + "response/" + D + "href").endswith("/literal%20%252e%252e.txt"), "single-pass percent handling")

    outside.write_text("must not be served")
    os.symlink(str(outside), root_path / "escape-link")
    os.symlink(str(work), root_path / "escape-dir")
    for label, path in [("symlink-file", root + "/escape-link"), ("symlink-intermediate", root + "/escape-dir/outside.txt")]:
        for method in ["GET", "PROPFIND", "HEAD"]:
            status, _, _ = request(method, path, depth="0" if method == "PROPFIND" else None)
            check(label + "-" + method.lower(), status == 403, f"status={status}")

    status, _, _ = request("PROPFIND", file, b'<D:propfind xmlns:D="DAV:"><D:prop></D:propfind>', "0")
    check("malformed-xml", status == 400, f"status={status}")
    for method in ["GET", "HEAD", "PROPFIND"]:
        status, _, _ = request(method, file, depth="0" if method == "PROPFIND" else None, auth=False)
        check("anonymous-" + method.lower(), status == 403, f"status={status}")
    head_status, head_headers, head_body = request("HEAD", root + "/not-found.txt")
    get_status, get_headers, _ = request("GET", root + "/not-found.txt")
    check("head-get-error", head_status == get_status == 404 and head_headers.get("content-length") == get_headers.get("content-length") and head_headers.get("content-type") == get_headers.get("content-type") and not head_body, f"status={head_status} length={head_headers.get('content-length')} empty=true")
    print(f"VERIFIED: {len(checks)} authenticated curl/raw-socket assertions passed; XML parsed with ElementTree")
    report = work / "results.txt"
    report.write_text(chr(10).join(checks) + chr(10))
finally:
    resolved_root = share.resolve()
    resolved_test = root_path.resolve()
    assert resolved_test.parent == resolved_root, "refusing unsafe test cleanup"
    for link in [root_path / "escape-link", root_path / "escape-dir"]:
        if link.is_symlink():
            link.unlink()
    if root_path.exists():
        shutil.rmtree(root_path)
    assert not root_path.exists() and not outside.exists() or outside.parent == work
    # Do not leave the temporary admin cookie in the verification work tree.
    print("CLEANUP: removed scoped share test tree")
