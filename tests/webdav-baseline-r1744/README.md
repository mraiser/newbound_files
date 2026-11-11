# Pre-change /files/dav WebDAV baseline (rclone 1.74.4)

Captured 2026-11-06 against the unchanged live listener on
`http://127.0.0.1:8080/files/dav` (port read from `config.properties`). This is
characterization only: no protocol source was changed, no instance restart was
performed, and no authentication/grant setting was changed.

## Host, client, and authentication

- Host: Linux 6.18.34 aarch64, NixOS 26.05 (64 bit).
- Reproducer: `nix-shell -p rclone --run 'rclone version'`.
- Observed: `rclone v1.74.4`, Go 1.26.5, `os/arch: arm64`, build tag `cmount`.
- Full version output is `logs/version.log`.
- `nix-shell (Nix) 2.34.8` was the installed launcher.
- The rclone remote was supplied by inherited process environment, with
  `--config /dev/null`; no rclone config file was created.
- Authentication was an existing, live admin session chosen for its shorter
  expiry (about 15 minutes at selection), passed only as
  `Cookie: sessionid=<redacted> The session id was never printed or saved.
  No anonymous grant or new user/session was created.
- Raw `Cookie` and every `Set-Cookie` response are redacted in this archive.
  Search checks found no live `sessionid=<redacted> in the sanitized files.

## Reproduction

Run from the instance root. Supply a CURRENT existing admin session id in the
shell; do not commit or echo it. `reproduce.sh` refuses to run without it.

```sh
nix-shell -p rclone --run 'rclone version'
export DAV_SESSION_ID='<current-existing-admin-session-id>'
nix-shell -p rclone --run 'sh /path/to/reproduce.sh'
```

The environment entries used for every command are:

```sh
RCLONE_CONFIG_BASELINE_TYPE=webdav
RCLONE_CONFIG_BASELINE_URL=http://127.0.0.1:8080/files/dav
RCLONE_CONFIG_BASELINE_VENDOR=other
RCLONE_CONFIG_BASELINE_HEADERS="Cookie,sessionid=<redacted>}"
```

The common flags were `--config /dev/null --contimeout 5s --timeout 15s
--retries 1 --low-level-retries 1 --stats 0`. Add `-vv --dump headers` for the
wire trace, or `-vv --dump headers,bodies` to include the XML PROPFIND bodies.
The first list/mkdir/upload sequence also used `--no-traverse`; this does not
suppress the observed destination-file PROPFIND.

## Results: attempted versus unattempted

| Operation | Actual rclone command/sequence | Wire result | Client result |
|---|---|---|---|
| List root | `rclone lsd baseline:` | PROPFIND `/files/dav/`, Depth 1 -> 207; XML root response 200 | exit 0 |
| Make collection | `rclone mkdir baseline:webdav-baseline-r1744-ops` | PROPFIND absent -> 404; MKCOL -> 201 | exit 0. Repeating `mkdir` on an existing collection: PROPFIND -> 207, MKCOL -> 405, rclone exit 0 |
| Upload | `rclone copyto payload.bin baseline:webdav-baseline-r1744/payload2.bin` | file PROPFIND -> 404; redundant parent MKCOL -> 405; PUT Content-Length 4096 -> 201; follow-up file PROPFIND Depth 0 -> 501 | **exit 1**, despite server storing all 4096 bytes. First concrete upload incompatibility: `PROPFIND currently supports collections only` |
| List collection | `rclone lsl baseline:webdav-baseline-r1744` | collection PROPFIND Depth 0 -> 207, then Depth 1 -> 207; child entry says 4096 | exit 0, lists both `payload.bin` and later `payload2.bin` |
| Direct-file download | `rclone copyto baseline:webdav-baseline-r1744/payload.bin ...` and `rclone cat baseline:.../payload.bin` | file PROPFIND -> 501; **GET unattempted in these direct-file chains** | exit 1 |
| Directory-scoped download | `rclone copy baseline:webdav-baseline-r1744 ./downloaded` | Depth 0/1 PROPFIND -> 207; GET each file -> 200, Content-Length 4096 | exit 0; both files copied |
| Download parity | `cmp source downloaded/payload.bin`; `cmp source downloaded/payload2.bin`; `sha256sum ...` | both `cmp` exit 0; identical hash `c8f5d0341d54d951a71b136e6e2afcb14d11ed8489a7ae126a8fee0df6ecf193` | byte-for-byte pass for both 4096-byte binary files |
| Direct-file rename/copy/delete | `rclone moveto`, `copyto`, `deletefile` addressed directly at the file | each stopped at file PROPFIND -> 501; **MOVE/COPY/DELETE unattempted in these direct-file chains** | exit 1 |
| File copy | collection-scoped `rclone copy baseline:webdav-baseline-r1744-ops baseline:webdav-baseline-r1744-copy --include seed.bin` | destination MKCOL -> 201; actual COPY file request with absolute HTTP Destination -> 405; Allow lacks COPY | exit 1: `COPY is not supported by this DAV endpoint` |
| File rename/move | collection-scoped `rclone move baseline:webdav-baseline-r1744-ops baseline:webdav-baseline-r1744-renamed --include seed.bin` | destination MKCOL -> 201; actual MOVE file request with Destination -> 405; Allow lacks MOVE | exit 1: `MOVE is not supported by this DAV endpoint` |
| Directory rename/move | `rclone move baseline:webdav-baseline-r1744-move-probe baseline:webdav-baseline-r1744-moved-probe` | source PROPFIND -> 207; destination PROPFIND -> 404; actual MOVE directory -> 405 | exit 1: `DirMove MOVE call failed` |
| Non-empty recursive delete | `rclone purge baseline:webdav-baseline-r1744` | collection PROPFIND -> 207; DELETE `/.../webdav-baseline-r1744/` -> 409 | exit 1: `non-empty directory; refusing to recurse` |
| Files then empty collection delete | `rclone delete baseline:webdav-baseline-r1744`; then `rclone purge ...` | DELETE each file -> 204; after empty, DELETE collection -> 204 | both exit 0 |
| Anonymous access | curl with no Cookie: OPTIONS, PROPFIND, GET, PUT | every request -> **HTTP 403 FORBIDDEN**; anonymous PUT file absent afterward | admin-only boundary preserved |

All commands and results are in `commands.txt` and `results.txt`. Important
logs are in `logs/`. The `--dump` traces contain the full request/response
headers (and PROPFIND XML where `bodies` was used), not inferred summaries.

## Additional observed client incompatibility

rclone repeatedly logs `Couldn't parse Date:` for listener responses such as
`Date: Fri, 06 Nov 2026 10:22:52 +0000`. This is a real observed warning from
rclone 1.74.4. It did not prevent the 207 listings or GET transfers above.
No claim is made that correcting file PROPFIND alone resolves it.

## Native desktop client availability (separate from rclone)

- **Finder: unavailable / NOT TESTED.** Finder requires macOS; this host is
  Linux (kernel 6.18.34 aarch64), not macOS. No Finder HTTP request was made,
  so no Finder-specific protocol failure or pass is claimed.
- **Windows Explorer: unavailable / NOT TESTED.** This is a Linux host with no
  Windows Explorer and no `explorer.exe`, `powershell.exe`, or `cmd.exe`
  interop commands. No Explorer HTTP request was made, so no Explorer-specific
  protocol failure or pass is claimed.

These are availability results, not protocol diagnoses. A DISPLAY variable was
present, but that does not make either native client available.

## Scope boundaries

HEAD, chunked upload, locking, XML escaping edge cases, symlink/traversal
regressions, desktop mounting, and a single all-operations rclone acceptance
run were not tested by this characterization item. In the observed upload the
request used Content-Length, not chunked transfer encoding. Unsupported-method
Allow was observed on COPY/MOVE responses. No conclusion is drawn about
whether Finder or Explorer requires locking.

Several early shell-wrapper probes used an invalid literal curl Cookie and got
403; those are excluded from this archive because they say nothing about the
endpoint with valid authentication. The valid rclone sequence itself supplied
the GET, COPY, MOVE, and DELETE evidence above. An early first-pass upload
sometimes skipped its PUT after destination-file PROPFIND returned 501; the
later fresh-file upload definitely issued PUT -> 201 and then failed its
post-upload PROPFIND -> 501. Both behaviors are why direct-file commands must
not be described as if their target verb necessarily reached the wire.

All created share test trees (`webdav-baseline-r1744`, `-ops`, `-copy`,
`-renamed`, `-move-probe`, `-moved-probe`) and the anonymous PUT target were
checked absent at the end. No protocol or security code was modified and no
instance restart was performed.
