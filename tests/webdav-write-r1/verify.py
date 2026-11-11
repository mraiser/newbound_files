#!/usr/bin/env python3
"""Real authenticated curl checks for contained recursive DELETE/COPY/MOVE.
Run from the instance root. DAV_COOKIE_HEADER names a chmod-600 file containing
'Cookie: sessionid=<temporary admin session>'. DAV_WORK names a private scratch
folder. The share test root is fixed; unrelated share content is never removed.
"""
from pathlib import Path
import hashlib
import os
import shutil
import subprocess
import urllib.parse

work = Path(os.environ.get("DAV_WORK", "/tmp/newbound-dav-write-r1"))
work.mkdir(mode=0o700, parents=True, exist_ok=True)
cookie_file = Path(os.environ.get("DAV_COOKIE_HEADER", str(work / "cookie_header")))
cookie = ["-H", "@" + str(cookie_file)]
base = "http://127.0.0.1:8080/files/dav"
root = "webdav-write-check-r1"
share = Path("runtime/files/shared")
root_path = share / root
outside = work / "outside"
checks = []
request_number = 0


def quote(path):
    return "/".join(urllib.parse.quote(part, safe="") for part in path.split("/"))


def request(method, path="", destination=None, overwrite=None, body=None,
            rawpath=None, auth=True, extra_headers=None):
    global request_number
    request_number += 1
    stem = work / f"{request_number:03d}-{method.lower()}"
    headers_file = stem.with_suffix(".headers")
    body_file = stem.with_suffix(".body")
    if rawpath is None:
        url = base + ("/" + quote(path) if path else "/")
    else:
        url = "http://127.0.0.1:8080" + rawpath
    cmd = ["curl", "-sS", "--http1.1", "--path-as-is", "--max-time", "20",
           "-X", method, "-D", str(headers_file), "-o", str(body_file)]
    if auth:
        cmd += cookie
    if destination is not None:
        cmd += ["-H", "Destination: " + destination]
    if overwrite is not None:
        cmd += ["-H", "Overwrite: " + overwrite]
    if extra_headers:
        for header in extra_headers:
            cmd += ["-H", header]
    if body is not None:
        cmd += ["--data-binary", "@-"]
    cmd += [url]
    process = subprocess.run(cmd, input=body, capture_output=True)
    assert process.returncode == 0, (method, path, process.stderr.decode())
    lines = headers_file.read_bytes().replace(b"\r\n", b"\n").split(b"\n")
    status = int(lines[0].split()[1])
    headers = {}
    for line in lines[1:]:
        if b":" in line:
            key, value = line.decode("latin1").split(":", 1)
            headers[key.lower()] = value.strip()
    return status, headers, body_file.read_bytes()


def check(label, condition, detail=""):
    assert condition, f"{label}: {detail}"
    checks.append(label + (" " + detail if detail else ""))
    print("PASS " + checks[-1], flush=True)


def status_is(label, actual, expected):
    check(label, actual == expected, f"status={actual} expected={expected}")


def local_bytes(relative):
    return (share / relative).read_bytes()


def local_hash(relative):
    return hashlib.sha256(local_bytes(relative)).hexdigest()


def assert_absent_outside():
    check("outside-unchanged", outside.read_bytes() == b"outside sentinel\x00\xff")


def safe_cleanup():
    # This script only ever removes its fixed top-level test collection.
    resolved_share = share.resolve()
    resolved_root = root_path.resolve()
    assert resolved_root.parent == resolved_share, "unsafe verification cleanup"
    if root_path.exists() and not root_path.is_symlink():
        shutil.rmtree(root_path)
    elif root_path.is_symlink():
        root_path.unlink()


try:
    safe_cleanup()
    outside.mkdir(mode=0o700, exist_ok=True)
    (outside / "outside.txt").write_bytes(b"outside sentinel\x00\xff")
    outside = outside / "outside.txt"

    status, headers, _ = request("OPTIONS")
    allowed = {item.strip() for item in headers.get("allow", "").split(",")}
    expected = {"OPTIONS", "PROPFIND", "GET", "HEAD", "PUT", "MKCOL", "DELETE", "COPY", "MOVE"}
    check("options-exact", allowed == expected and headers.get("dav") == "1",
          f"Allow={headers.get('allow')} DAV={headers.get('dav')}")
    status, headers, _ = request("BREW", root)
    allowed = {item.strip() for item in headers.get("allow", "").split(",")}
    check("allow-exact-on-405", status == 405 and allowed == expected,
          f"status={status} Allow={headers.get('allow')}")
    for method in ("LOCK", "UNLOCK"):
        status, _, _ = request(method, root)
        status_is(method.lower() + "-not-advertised-or-stubbed", status, 405)

    for folder in (root, root + "/alpha", root + "/alpha/one", root + "/alpha/one/deep",
                   root + "/dest", root + "/preserve"):
        status, _, _ = request("MKCOL", folder)
        status_is("mkcol-" + folder.replace("/", "-"), status, 201)
    alpha = (bytes(range(256)) * 8) + b"\x00alpha-final\r\n\xff"
    beta = hashlib.sha256(b"beta").digest() * 129 + b"beta-tail"
    original = b"original destination must survive Overwrite F\x00\xff"
    moved = hashlib.sha256(b"renamed-tree").digest() * 97 + b"renamed-tail"
    for name, payload in (("alpha/one/deep/payload.bin", alpha),
                          ("alpha/one/deep/beta.bin", beta),
                          ("dest/original.bin", original),
                          ("preserve/original.bin", original)):
        status, _, _ = request("PUT", root + "/" + name, body=payload)
        status_is("put-" + name.replace("/", "-"), status, 201)
    status, _, body = request("GET", root + "/alpha/one/deep/payload.bin")
    check("source-byte-parity", status == 200 and body == alpha,
          f"status={status} sha256={hashlib.sha256(body).hexdigest()}")

    # File COPY: origin-form Destination and missing Destination handling.
    destination = base + "/" + quote(root + "/copied-file.bin")
    status, _, _ = request("COPY", root + "/alpha/one/deep/payload.bin", destination=destination)
    status_is("copy-file-origin", status, 201)
    check("copy-file-bytes", local_bytes(root + "/copied-file.bin") == alpha)
    check("copy-file-source-retained", local_bytes(root + "/alpha/one/deep/payload.bin") == alpha)
    status, _, _ = request("COPY", root + "/copied-file.bin")
    status_is("copy-missing-destination", status, 400)

    # Collection COPY using an absolute URI with UTF-8/escaped destination.
    copied_tree = root + "/copied tree 集合"
    destination = base + "/" + quote(copied_tree)
    status, _, _ = request("COPY", root + "/alpha", destination=destination)
    status_is("copy-tree-absolute", status, 201)
    check("copy-tree-payload-bytes", local_bytes(copied_tree + "/one/deep/payload.bin") == alpha)
    check("copy-tree-beta-bytes", local_bytes(copied_tree + "/one/deep/beta.bin") == beta)

    # Overwrite F preserves destination and does not disturb source.
    status, _, _ = request("COPY", root + "/alpha/one/deep/payload.bin",
                           destination="/files/dav/" + quote(root + "/dest/original.bin"), overwrite="F")
    status_is("copy-overwrite-f", status, 412)
    check("copy-f-preserved", local_bytes(root + "/dest/original.bin") == original)
    check("copy-f-source-preserved", local_bytes(root + "/alpha/one/deep/payload.bin") == alpha)

    # Overwrite T replaces file bytes and returns 204.
    status, _, _ = request("COPY", root + "/alpha/one/deep/payload.bin",
                           destination="/files/dav/" + quote(root + "/dest/original.bin"), overwrite="T")
    status_is("copy-overwrite-t", status, 204)
    check("copy-t-replaced", local_bytes(root + "/dest/original.bin") == alpha)
    check("copy-t-source-preserved", local_bytes(root + "/alpha/one/deep/payload.bin") == alpha)

    # Collection overwrite T replaces, rather than merges, the old tree.
    status, _, _ = request("PUT", root + "/dest/stale.bin", body=b"stale")
    status_is("put-stale-destination", status, 201)
    status, _, _ = request("COPY", root + "/alpha/one",
                           destination="/files/dav/" + quote(root + "/dest"), overwrite="T")
    status_is("copy-tree-overwrite-t", status, 204)
    check("tree-overwrite-replaced-not-merged", not (root_path / "dest/stale.bin").exists()
          and local_bytes(root + "/dest/deep/payload.bin") == alpha)

    # Missing parents conflict and leave no partial destination.
    for method in ("COPY", "MOVE"):
        status, _, _ = request(method, root + "/alpha/one/deep/payload.bin",
                               destination="/files/dav/" + quote(root + "/missing/parent/out.bin"))
        status_is(method.lower() + "-missing-parent", status, 409)
    check("missing-parent-no-artifact", not (root_path / "missing").exists())
    status, _, _ = request("COPY", root + "/does-not-exist",
                           destination="/files/dav/" + quote(root + "/unused"))
    status_is("copy-missing-source", status, 404)
    status, _, _ = request("DELETE", root + "/does-not-exist")
    status_is("delete-missing", status, 404)

    # Equality, self-subtree, ancestor replacement, and root operations are forbidden.
    status, _, _ = request("COPY", root + "/alpha",
                           destination="/files/dav/" + quote(root + "/alpha"))
    status_is("copy-same", status, 403)
    for method in ("COPY", "MOVE"):
        status, _, _ = request(method, root + "/alpha",
                               destination="/files/dav/" + quote(root + "/alpha/one/nested"))
        status_is(method.lower() + "-own-subtree", status, 403)
    status, _, _ = request("MOVE", root + "/alpha/one",
                           destination="/files/dav/" + quote(root + "/alpha"), overwrite="T")
    status_is("move-onto-ancestor", status, 403)
    check("ancestor-validation-preserved", local_bytes(root + "/alpha/one/deep/payload.bin") == alpha)
    for method, path, destination in (("DELETE", "", None),
                                      ("COPY", "", "/files/dav/" + quote(root + "/root-copy")),
                                      ("MOVE", root + "/alpha", "/files/dav/")):
        status, _, _ = request(method, path, destination=destination)
        status_is(method.lower() + "-root-rejected", status, 403)
    check("root-and-source-survive", root_path.is_dir() and local_bytes(root + "/alpha/one/deep/payload.bin") == alpha)

    # Encoded traversal, encoded absolute paths, foreign/out-of-root destinations.
    attack_destinations = [
        "/files/dav/%2e%2e/dav-write-escape",
        "/files/dav/" + root + "/%2E%2E/dav-write-escape",
        "/files/dav/%2Ftmp%2Fdav-write-escape",
        "/files/dav/%5C..%5Cdav-write-escape",
        "/files/dav//etc/passwd",
        "/other/outside",
        "http://127.0.0.1:8080/other/outside",
        "http://example.invalid:8080/files/dav/" + root + "/foreign",
        "https://example.invalid/files/dav/" + root + "/foreign",
    ]
    for index, destination in enumerate(attack_destinations):
        status, _, _ = request("COPY", root + "/alpha/one/deep/payload.bin", destination=destination)
        status_is(f"destination-rejected-{index}", status, 403)
        check(f"destination-source-preserved-{index}", local_bytes(root + "/alpha/one/deep/payload.bin") == alpha)
    status, _, _ = request("COPY", "/files/dav/%2e%2e/outside",
                           destination="/files/dav/" + quote(root + "/escape"), rawpath="/files/dav/%2e%2e/outside")
    status_is("source-encoded-traversal", status, 403)
    status, _, _ = request("COPY", root + "/alpha", destination="relative/path")
    status_is("relative-destination", status, 400)
    status, _, _ = request("COPY", root + "/alpha",
                           destination="/files/dav/" + quote(root + "/lower-overwrite"), overwrite="t")
    status_is("lowercase-overwrite", status, 400)

    # Symlink source, destination, nested source, nested destination, and ancestry.
    link_root = root_path / "links"
    link_root.mkdir()
    link_dir = root_path / "linkdir"
    link_dir.mkdir()
    external_dir = work / "external-dir"
    external_dir.mkdir(exist_ok=True)
    (external_dir / "kept.bin").write_bytes(b"outside sentinel\x00\xff")
    source_link = link_root / "source-link"
    source_link.symlink_to(outside, target_is_directory=False)
    nested_source = root_path / "nested-source"
    nested_source.mkdir()
    (nested_source / "safe.bin").write_bytes(b"safe")
    (nested_source / "escape").symlink_to(outside, target_is_directory=False)
    destination_link = link_root / "destination-link"
    destination_link.symlink_to(outside, target_is_directory=False)
    ancestry_link = link_root / "ancestry-link"
    ancestry_link.symlink_to(external_dir, target_is_directory=True)
    nested_destination = root_path / "nested-destination"
    nested_destination.mkdir()
    (nested_destination / "safe.bin").write_bytes(b"old")
    (nested_destination / "escape").symlink_to(outside, target_is_directory=False)
    cases = [
        ("DELETE", str(Path(root) / "links/source-link"), None, "T"),
        ("COPY", str(Path(root) / "links/source-link"), "/files/dav/" + root + "/link-copy", "T"),
        ("DELETE", str(Path(root) / "nested-source"), None, "T"),
        ("COPY", str(Path(root) / "nested-source"), "/files/dav/" + root + "/nested-copy", "T"),
        ("COPY", str(Path(root) / "alpha"), "/files/dav/" + quote(root + "/links/destination-link"), "T"),
        ("COPY", str(Path(root) / "alpha"), "/files/dav/" + quote(root + "/links/ancestry-link/escape"), "T"),
        ("COPY", str(Path(root) / "alpha"), "/files/dav/" + quote(root + "/nested-destination"), "T"),
    ]
    for index, (method, source, destination, overwrite) in enumerate(cases):
        status, _, _ = request(method, source, destination=destination, overwrite=overwrite)
        status_is(f"symlink-rejected-{index}", status, 403)
        assert_absent_outside()
    check("nested-source-symlink-preserved", (nested_source / "escape").is_symlink())
    check("nested-destination-symlink-preserved", (nested_destination / "escape").is_symlink())
    check("nested-destination-old-preserved", (nested_destination / "safe.bin").read_bytes() == b"old")
    check("destination-ancestry-no-artifact", not (external_dir / "escape").exists())
    check("no-partial-nested-copy", not (root_path / "nested-copy").exists())

    # A complete nested MOVE, with source gone and bytes retained at destination.
    status, _, _ = request("PUT", copied_tree + "/one/deep/rename.bin", body=moved)
    status_is("put-rename-payload", status, 201)
    renamed_tree = root + "/renamed tree"
    status, _, _ = request("MOVE", copied_tree,
                           destination="/files/dav/" + quote(renamed_tree))
    status_is("move-tree", status, 201)
    check("move-source-gone", not (share / copied_tree).exists())
    check("move-payload-bytes", local_bytes(renamed_tree + "/one/deep/payload.bin") == alpha)
    check("move-rename-bytes", local_bytes(renamed_tree + "/one/deep/rename.bin") == moved)
    status, _, _ = request("MOVE", root + "/copied-file.bin",
                           destination="/files/dav/" + quote(root + "/renamed-file.bin"))
    status_is("move-file", status, 201)
    check("move-file-source-gone", not (root_path / "copied-file.bin").exists())
    check("move-file-bytes", local_bytes(root + "/renamed-file.bin") == alpha)
    status, _, _ = request("MOVE", root + "/renamed-file.bin",
                           destination="/files/dav/" + quote(root + "/preserve/original.bin"), overwrite="F")
    status_is("move-overwrite-f", status, 412)
    check("move-f-destination-preserved", local_bytes(root + "/preserve/original.bin") == original)
    check("move-f-source-preserved", local_bytes(root + "/renamed-file.bin") == alpha)
    status, _, _ = request("MOVE", root + "/renamed-file.bin",
                           destination="/files/dav/" + quote(root + "/preserve/original.bin"), overwrite="T")
    status_is("move-overwrite-t", status, 204)
    check("move-t-replaced", local_bytes(root + "/preserve/original.bin") == alpha)
    check("move-t-source-gone", not (root_path / "renamed-file.bin").exists())

    for method in ("COPY", "MOVE", "DELETE"):
        status, _, _ = request(method, root + "/alpha",
                               destination="/files/dav/" + quote(root + "/anonymous") if method != "DELETE" else None,
                               auth=False)
        status_is("anonymous-" + method.lower(), status, 403)
    check("anonymous-write-did-not-run", local_bytes(root + "/alpha/one/deep/payload.bin") == alpha)

    # The rejected symlink fixtures must remain forbidden to DAV. Unlink only
    # those exact known test links locally, leaving a normal nested tree for
    # the final recursive DELETE. Never traverse any of these link targets.
    fixture_links = [source_link, destination_link, ancestry_link,
                     nested_source / "escape", nested_destination / "escape"]
    for link in fixture_links:
        check("fixture-link-preserved-" + str(link.relative_to(root_path)), link.is_symlink())
        link.unlink()
    assert_absent_outside()

    # Demonstrate recursive DELETE through HTTP, including a freshly made tree.
    status, _, _ = request("DELETE", renamed_tree)
    status_is("recursive-delete-renamed-tree", status, 204)
    check("recursive-delete-renamed-gone", not (share / renamed_tree).exists())
    status, _, _ = request("DELETE", root)
    status_is("recursive-delete-test-root", status, 204)
    check("recursive-delete-test-root-gone", not root_path.exists())
    assert_absent_outside()
    print(f"VERIFIED: {len(checks)} authenticated real-curl assertions passed; nested trees byte-compared and removed", flush=True)
    (work / "results.txt").write_text("\n".join(checks) + "\n")
finally:
    safe_cleanup()
    print("CLEANUP: removed only the scoped DAV share test tree", flush=True)
