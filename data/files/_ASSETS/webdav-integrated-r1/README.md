# Integrated WebDAV/files acceptance r1 — VERIFIED after human restart

## Final result and restart boundary

Final live acceptance ran on 2026-11-06 against `target/release/newbound`
PID 222345 (started 09:17:12 local time), after the owner's SECOND reported
USER-INITIATED restart. Tests connected to `127.0.0.1:8080`;
`config.properties` says `http_port=8080`, and the owner reported the bind
address restored to `0.0.0.0`. The agent did not restart or stop the instance,
change its listener configuration, or alter users, groups, or permission
policy.

The earlier r1 run against PID 213437 was partial: malformed chunk framing and
chunked JSON POST exposed two old-listener defects. Those compiled fixes are
active after the second human restart. The final strict run below did NOT set
`DAV_SKIP_KNOWN_LISTENER_ERROR`: both checks passed, with zero skips.

Final checks actually run, all from the instance root:

```sh
env -u DAV_SKIP_KNOWN_LISTENER_ERROR \
  DAV_COOKIE_HEADER=/tmp/newbound-dav-integrated-r1/cookie_header \
  DAV_WORK=/tmp/newbound-dav-integrated-r1/strict RCLONE=rclone \
  nix-shell -p rclone --run \
  'python3 data/files/_ASSETS/webdav-integrated-r1/verify.py'
```

**Result: exit 0, 114 assertions, zero skips.** Its final line was:

```text
VERIFIED: 114 live integrated assertions passed; rclone byte parity, chunked/framing, security, JSON/UI and cleanup
```

The two previously failing checks are explicit in `strict/results.txt`:
`invalid-chunk-framing-400-close responses=1` and
`json-post-chunked http=200 status=ok`.

The component suites were then rerun against the same listener:

```sh
env DAV_COOKIE_HEADER=/tmp/newbound-dav-integrated-r1/cookie_header \
  DAV_WORK=/tmp/newbound-dav-integrated-r1/read-final \
  python3 data/files/_ASSETS/webdav-read-r1/verify.py
```

**Result: exit 0, 41 assertions**, XML parsed and scoped cleanup completed.

```sh
env DAV_COOKIE_HEADER=/tmp/newbound-dav-integrated-r1/cookie_header \
  DAV_WORK=/tmp/newbound-dav-integrated-r1/write-final \
  python3 data/files/_ASSETS/webdav-write-r1/verify.py
```

**Result: exit 0, 114 assertions**, byte comparisons and scoped cleanup
completed.

A separate curl-only shell acceptance exercised the original simple DELETE
contract. **Result: exit 0, 9 checks**: MKCOL root/empty 201; binary
Content-Length PUT 201; GET 200 and `cmp` exit 0 for 19 bytes; DELETE file 204
then GET 404; DELETE empty collection 204; DELETE scoped empty root 204 then
Depth:0 PROPFIND 404; scoped artifacts absent. Exact operations/results are in
`original-delete/results.txt` inside the evidence archive.

## Reproducible client versions and settings

- Host: NixOS 26.05 (Yarara), Linux 6.18.34 aarch64.
- rclone **v1.74.4**, Go 1.26.5, os/arch arm64, launched by
  `nix-shell (Nix) 2.34.8`.
- curl **8.21.0**, HTTP/1.1; Python **3.12.13**.
- DAV URL: `http://127.0.0.1:8080/files/dav`; rclone vendor `other`.
- Remote name `accept`; configuration was supplied only through inherited
  `RCLONE_CONFIG_ACCEPT_TYPE=webdav`,
  `RCLONE_CONFIG_ACCEPT_URL=http://127.0.0.1:8080/files/dav`,
  `RCLONE_CONFIG_ACCEPT_VENDOR=other`, and
  `RCLONE_CONFIG_ACCEPT_HEADERS=Cookie,sessionid=<temporary admin session>`.
  Every invocation used `--config /dev/null`; no rclone config file was
  created.
- The in-memory temporary admin session was supplied in a chmod-600 Cookie
  header file. It contains the platform's normal `count` field: the first
  strict attempt omitted it and failed immediately with listener 500 (`count`
  missing), before any DAV operation. Adding that field to the temporary
  session fixed the test fixture; no production code/auth policy changed. The
  complete strict harness then passed.
- Common rclone flags: `--contimeout 5s --timeout 20s --retries 1
  --low-level-retries 1 --stats 0 --checkers 1 --transfers 1 -vv
  --dump headers`.

## rclone 1.74.4 end-to-end workflow

Under only `runtime/files/shared/webdav-integrated-r1/rclone`, the harness ran
`rclone version`, `lsd accept:`, `mkdir`, `lsd` of the test parent, `copyto`
upload, `lsl`, directory-scoped `copy` download, `moveto` file rename,
`copyto` server-side copy, and `purge`. Every rclone process exited 0.

- List/mkdir passed; post-mkdir listing passed.
- Upload used **Content-Length**, not chunked encoding: file PROPFIND 404,
  redundant parent MKCOL 405 (tolerated by this workflow), PUT 201, then file
  PROPFIND 207.
- Download used collection PROPFIND 207 and GET 200; exact **131087-byte**
  binary equality passed with SHA-256
  `344e093b232520f6e03bbe8d08ee0fc595f1506188cb9e1ed07f2c6f2ee56c7d`.
- Rename: MOVE 201, old source 404, renamed bytes equal.
- Copy: COPY 201, source retained, destination bytes equal.
- Recursive purge: DELETE 204, collection PROPFIND 404.
- Final recursive DELETE of the scoped integrated root returned 204.

Sanitized per-operation logs (`strict/*.log`), 82 summarized HTTP requests,
and all 114 assertions are archived. Logs contain non-fatal rclone debug
warnings that it cannot parse the listener's RFC 1123 `Date` header using its
layout; no operation failed and this is not claimed as complete HTTP-date
compatibility.

## Curl/raw-socket and security coverage

Across the strict 114, read 41, write 114, and simple-DELETE 9 checks:

- OPTIONS 200 with exact Allow `OPTIONS, PROPFIND, GET, HEAD, PUT, MKCOL,
  DELETE, COPY, MOVE` and `DAV: 1`; unsupported methods/LOCK/UNLOCK 405.
- Original collection PROPFIND/GET/PUT/MKCOL/DELETE, file PROPFIND, HEAD with
  GET-equivalent length/type and no body, Depth 0/1 XML, property selection,
  and XML/URL escaping passed.
- File/tree COPY/MOVE passed, including absolute same-authority Destination,
  Overwrite F 412 with preservation, T 204 replacement, missing source 404,
  missing Destination 400, missing parent 409, and recursive DELETE 204.
- Content-Length binary uploads (19, 65536, 66133, and 131087 bytes across
  runs) and chunked uploads of 1, 131079, and zero bytes passed exact download
  byte comparison. Raw keep-alive chunked→Content-Length and
  Content-Length→chunked sequences each returned [201,201], all payloads
  equal.
- Invalid chunk size returned exactly one framed HTTP/1.1 400 with
  `Connection: close`, no second response, and no file artifact.
- Missing resources 404, existing MKCOL 405, malformed PROPFIND XML 400,
  unsupported Depth 501, and expected COPY/MOVE 400/403/404/409/412 statuses
  passed.
- Encoded traversal, encoded absolute/backslash/repeated-separator paths,
  source/destination/nested/ancestry symlink escapes, and foreign-authority,
  wrong-prefix, and out-of-root COPY/MOVE destinations returned 403 where
  specified. External sentinel bytes stayed unchanged; no partial destination
  was created.
- Anonymous DAV OPTIONS/PROPFIND/GET/HEAD/PUT/MKCOL/DELETE/COPY/MOVE/POST all
  returned **403**, and anonymous PUT created no artifact. The pre-existing
  separate `/files/list_dir` JSON GET remains anonymously available and
  returned 200; this was not broadened or confused with the admin-only DAV
  boundary.
- Authenticated normal JSON GET, Content-Length POST, and chunked POST to
  `/files/list_dir` each returned HTTP 200 with `status=ok` and
  `data.entries`. `/files/` returned 200 with boot HTML (462 bytes);
  `index.css` (22 bytes) and `index.js` (77 bytes) returned 200 with their
  actual boot content. This is an HTTP/UI smoke test, not a native-browser
  click-through.

All harnesses perform scoped cleanup in `finally` or their own cleanup path.
Final checks found no `webdav-integrated-r1`, component, baseline,
anonymous-write, or `.newbound-dav-*` artifacts in the share; its inventory
was empty. Temporary private work/cookie files are removed after archiving;
the temporary session is revoked and its old Cookie is checked to return DAV
403.

## Native desktop clients — independently reported

- **Finder: NOT TESTED because unavailable.** This is Linux, not macOS; no
  Finder or `osascript` command was available. No Finder request was made;
  no Finder protocol success or failure is inferred.
- **Windows Explorer: NOT TESTED because unavailable.** No Windows host or
  `explorer.exe`, `powershell.exe`, or `cmd.exe` interop was available. No
  Explorer request was made; no Explorer protocol success or failure is
  inferred.

## Honest protocol limits

- LOCK/UNLOCK are **not implemented** and return 405. OPTIONS advertises DAV
  class 1 and the nine supported methods only; there is no lock-token stub.
  rclone 1.74.4 passed without locking. No Finder/Explorer lock requirement
  can be inferred.
- PROPFIND supports Depth 0/1, not infinity. COPY recursively stages a tree;
  there is no `Depth: infinity` COPY mode. Concurrent writers are not
  serialized; path validation is not an inter-process lock, and rollback on
  I/O failure is best effort.
- PUT creates missing parents. MOVE requires same-filesystem rename. Request
  bodies are capped at 64 MiB. No claim of complete RFC WebDAV/desktop-client
  compatibility is made.
- The observed rclone upload used Content-Length; chunked upload was
  independently exercised by curl/raw sockets.

## Evidence and rerunning

`results-final.tar.gz` contains 219 sanitized files under
`webdav-integrated-r1-evidence/`: `SUMMARY.txt`, strict/read/write/original-
DELETE results, response headers/bodies, and rclone request traces. Archive
scan found **zero session-ID occurrences and zero unredacted session Cookie
headers**. Its SHA-256 is recorded in the final acceptance result. The
previous `results.tar.gz` is superseded by `results-final.tar.gz`.

To rerun, create a normal temporary ADMIN session through platform session
facilities (including the normal `count` field) and store exactly one line
`Cookie: sessionid=<temporary admin session>` in a chmod-600 file. Use a new
private work directory, the three commands above, and no skip variable. The
scripts only delete their fixed scoped test trees and explicit symlink
fixtures. Revoke the session and remove private cookie/work files afterward;
do not put credentials in an asset or shell history.
