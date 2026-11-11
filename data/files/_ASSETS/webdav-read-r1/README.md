# Scoped WebDAV HEAD / file-aware PROPFIND verification

## Scope and implementation

Changed only `files.files.dav` and `files.files.propfind`; the common
`files.files.resolve_path` containment implementation and admin-only `dav`
routing were preserved. No listener source, authentication policy, COPY/MOVE,
recursive DELETE, or chunked-upload code was changed. No instance restart was
performed or required: the commands compiled and hot-swapped.

- HEAD returns the same status, Content-Type and Content-Length as GET for the
  same file, but no response body. Missing-resource and containment-error HEAD
  responses also have GET-equivalent metadata and zero body bytes.
- PROPFIND supports files and collections at Depth 0/1 (Depth 1 on a file acts
  as Depth 0). Missing Depth is rejected; only 0 and 1 are accepted.
- Request bodies support rclone's requested-property form, allprop, propname,
  and empty-body compatibility. Supported DAV properties are displayname,
  resourcetype, getcontentlength, getlastmodified, and getcontenttype. Unknown
  properties get a separate 404 propstat. Malformed XML returns 400.
- Href path segments are UTF-8 percent-encoded; slashes stay separators and
  collections have trailing slashes. Display names are separately XML escaped.
- Single-pass path decoding and common resolve_path checks reject encoded
  traversal, absolute/backslash paths, file symlinks, and intermediate symlinks.
- This is intentionally not a general WebDAV property store or lock
  implementation. It supports the demonstrated read-side client properties.

## Exact verification command

Host client: curl 8.21.0 on Linux aarch64; XML parsed with Python 3.12
ElementTree. From the instance root, using a temporary admin session cookie
stored only in a chmod-600 file (the ID was never printed or put in evidence):

```sh
DAV_COOKIE_HEADER="$W/cookie_header" DAV_WORK="$W" \
  python3 data/files/_ASSETS/webdav-read-r1/verify.py
```

The cookie header is `Cookie: sessionid=<temporary admin session>`. The
reproducer does not create credentials or weaken anonymous permissions; the
caller supplies the temporary authenticated session. It runs real HTTP curl
requests to 127.0.0.1:8080 and deletes only its own `webdav-read-check-r1` tree.
It uses `curl -I`, not `curl -X HEAD`: generic `-X HEAD` makes curl expect the
advertised GET bytes and produces a false curl error 18 for a correct HEAD.
Additional raw socket requests verify success and containment-error HEAD
responses have zero bytes after their header blocks.

Final result: exit 0, **41 assertions passed**. `results.tar.gz` contains the
complete cookie-free assertion report (`results.txt`) and a concise
`HEAD-GET-comparison.txt`. Temporary admin sessions were invalidated,
cookie/work files removed, and the share test tree and symlinks checked absent
after the run. A final unauthenticated PROPFIND was 403; the Newbound process
retained its original start time (no listener restart).

## Reproducer input

`verify.py` is the executable reproducer. It uploads a known 880-byte UTF-8
file under a directory and filename containing spaces, ampersands, `<`, and
non-ASCII text, and checks:

- MKCOL/PUT setup and byte-for-byte GET response;
- HEAD/GET status, MIME and Content-Length parity plus zero curl/raw body;
- HEAD 404 and encoded traversal/absolute 403 metadata with empty raw bodies;
- file Depth 0, collection Depth 0/1, requested props, allprop, propname,
  unknown-property 404 propstat, and XML parseability;
- UTF-8 hrefs and XML text round-tripping, including literal `%2e` names;
- Depth 2/infinity/-1 -> 501, encoded traversal/absolute/backslash -> 403;
- direct and intermediate symlink GET/HEAD/PROPFIND -> 403;
- malformed XML -> 400 and anonymous GET/HEAD/PROPFIND -> 403.

This is scoped protocol verification, not a claim of full desktop mounting,
Finder/Windows support, COPY/MOVE support, recursive deletion, locking, or
chunked uploads. Those remain separately scoped parent/sibling work.
