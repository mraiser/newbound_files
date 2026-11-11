# HTTP/1.1 chunked input and HEAD listener verification

Verified 2026-11-06 without restarting the live instance.

## Production changes

`app.service.init` now decodes one HTTP/1.1 chunked request body before method-specific handling. It accepts hexadecimal sizes and RFC 9112 chunk extensions, requires CRLF framing, validates trailers, and caps aggregate decoded bytes at 64 MiB (not just each chunk). The decoded bytes and length are supplied to DAV PUT and other non-POST commands. Chunked POST is delivered once through the raw body instead of being read again by the legacy POST parser. Invalid framing receives 400 and `Connection: close`; aggregate overflow receives 413 and closes, so unread invalid/oversized payload is never interpreted as a subsequent request. Content-Length POST bodies have the same cap and validate their length.

The listener has one response-body branch for all representations. HEAD emits the status and headers, including a supplied or computed Content-Length, but does not emit file, byte-stream, or string bodies. GET file byte parity and length are retained. Authentication dispatch is unchanged.

## Exact checks

- `dev.dev.compile {lib:app, ctl:service, cmd:init}` returned `{status:ok,msg:OK}` after the final changes.
- `cargo build --manifest-path webdav_listener_test/Cargo.toml` succeeded in a separate process and crate.
- `python3 webdav_listener_test/test.py` passed 37 checks and exited 0. Its Rust listener/decoder extracts were byte-compared to the generated production `app.service.init`; only documented test dispatch, event hooks, and cookie-session setup differed.
- Multi-chunk binary upload (131079 bytes including NUL/CRLF/0xff) returned the exact decoded payload; SHA-256 `ac640a530e8999db36f7ac4672f7b15d4948f62246a5d67829c1605beb2ea3e9`.
- Passed zero chunk, one-byte-split size header, quoted extension with an embedded semicolon, multiple extensions, trailers, exact 64 MiB accepted, aggregate 64 MiB + 1 rejected, single oversized size rejected with 413, malformed sizes/extensions/CRLF/trailers rejected with 400 and a closed connection (no second response).
- Passed consecutive Content-Length -> chunked and chunked -> Content-Length requests on one keep-alive connection.
- Passed HEAD for file, bytes, JSON, generic missing-file 404, and a DAV-shaped 404 with preserved error length; all emitted no body. GET file matched all 131079 bytes and retained Content-Length. A subsequent request after HEAD was correctly framed.
- Passed ordinary Content-Length JSON POST parameter parsing and chunked JSON raw-body delivery without a second socket read; oversized and invalid Content-Length POST returned 413 and 400 respectively.

The archive `webdav-listener-harness.tar.gz` contains the isolated harness, raw-socket test driver, dependency lockfile, and passing results log. It uses test handlers, not live user credentials or a live listener. Temporary large payload and target files were removed after archiving.

## Live-instance boundary

A USER-INITIATED INSTANCE RESTART IS REQUIRED before these listener changes become live. The agent did not restart the instance. Live chunked uploads and new HEAD behavior must be accepted after that restart; the tests above verify compiled production parsing/response logic in an isolated raw-socket process, not activation in the currently running old listener.
