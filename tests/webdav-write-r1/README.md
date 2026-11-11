# Contained recursive DELETE and overwrite-aware COPY/MOVE

## Implementation

Changed `files.files.dav`, `files.files.resolve_path`, and added the admin-only
internal helper `files.files.dav_tree`. The existing anonymous
`delete_path` command was not broadened. The HTTP `dav` endpoint remains
admin-only; no authentication records or permission groups were changed.

- DELETE recursively removes regular files and collections below the share,
  but never the DAV root. It validates the entire tree with `symlink_metadata`
  before mutation and refuses links rather than following or unlinking them.
- COPY supports files and trees. MOVE renames files and trees on the share's
  filesystem. Source equals destination, ancestor/descendant replacement,
  copying or moving a collection into its own subtree, and root sources or
  destinations are forbidden.
- Destination accepts origin-form paths and same-authority absolute http/https
  URIs. Foreign authorities, other DAV prefixes, query/fragment forms,
  relative destinations, and malformed URI schemes are rejected. Percent
  decoding occurs exactly once, then both source and destination use
  `resolve_path`.
- `resolve_path` rejects absolute/traversal paths, repeated separators,
  leading-dot components, control characters, and symlinks in every existing
  ancestry component. `dav_tree` additionally rejects links anywhere in a
  source tree or an overwrite destination tree.
- Overwrite defaults to T and accepts exactly uppercase T or F. F returns 412
  without changing source or destination. T replaces (does not merge) an old
  tree and returns 204; creation returns 201. Missing destination parents
  return 409; missing source resources return 404; containment violations and
  protected root/self operations return 403.
- COPY builds a complete staging tree before touching an old destination.
  MOVE stages via same-filesystem rename. A replaced destination is held as a
  backup until installation succeeds, with rollback attempted on rename
  failure. No mutation happens on path/overwrite/tree validation failure.
- OPTIONS and 405 Allow advertise exactly OPTIONS, PROPFIND, GET, HEAD, PUT,
  MKCOL, DELETE, COPY, MOVE. LOCK and UNLOCK remain unimplemented and return
  405. There is no token-only lock stub and no Depth:infinity COPY mode.
- Concurrent writers are not serialized; path validation is not a lock against
  another process changing a tree between validation and mutation. Runtime
  filesystem/copy failures may return 500; rollback is best-effort for I/O
  errors, not a general transaction manager.

The command changes hot-load in the `files` FFI crate. The listener was not
changed and no instance restart was performed or required for these command
changes. The separate listener-body work's restart status is unchanged and is
outside this subtask.

## Verification

Client: curl 8.21.0 on Linux aarch64, HTTP/1.1 to `127.0.0.1:8080`. Test
payloads were binary and included NUL, CRLF, 0xff, and multi-kilobyte trees;
byte equality was checked directly and a source SHA-256 was recorded by the
script. A temporary admin session cookie was supplied in a chmod-600 file and
was never printed or archived.

From the instance root, supply a current temporary admin session in the file
named by `DAV_COOKIE_HEADER`, containing one line
`Cookie: sessionid=<temporary admin session>`:

```sh
umask 077
W=$(mktemp -d /tmp/newbound-dav-write-r1.XXXXXX)
# Populate "$W/cookie_header" with the temporary admin Cookie header.
DAV_COOKIE_HEADER="$W/cookie_header" DAV_WORK="$W" \
  python3 tests/webdav-write-r1/verify.py
```

The script only deletes its own fixed `webdav-write-check-r1` share tree and
its own temporary outside sentinels. It does not read credentials from this
asset or change users/groups.

Final real run: exit 0, **114 assertions passed**. It verified:

- exact OPTIONS/Allow and LOCK/UNLOCK 405;
- nested MKCOL/PUT/GET setup and byte parity;
- file COPY and tree COPY through origin-form and absolute-form Destination;
- nested file and tree MOVE, source removal, and byte equality;
- Overwrite F -> 412 preserving both sides; T -> 204 replacing file bytes and
  replacing rather than merging an old collection;
- missing destination parents -> 409 with no partial output, missing sources
  and missing DELETE targets -> 404;
- same resource, ancestor/descendant transfers, root deletion and root
  transfers -> 403 with retained source bytes;
- encoded traversal/absolute/backslash paths, repeated decoded separators,
  foreign prefixes/authorities, and out-of-root destinations -> 403;
- relative Destination and lowercase overwrite -> 400;
- direct, nested, destination and ancestry symlinks -> 403, with external
  sentinels unchanged and no partial destination;
- anonymous COPY/MOVE/DELETE -> 403;
- recursive DELETE of a nested moved tree and of the cleaned test root -> 204;
- rejected symlink fixtures remained symlinks, then only those exact fixture
  links were locally unlinked so the final normal tree could be deleted by DAV.

A final isolated probe cleanup also returned 204; anonymous OPTIONS after the
run returned 403. The share contained none of the `webdav-write-*` or
`.newbound-dav-*` test/staging artifacts. Temporary cookie/work files and the
admin session were removed after verification.

This is scoped protocol verification, not a claim that Finder, Windows
Explorer, rclone, or the integrated pre-existing read/upload/UI acceptance has
been rerun. Those belong to the parent's integrated acceptance subtask.
