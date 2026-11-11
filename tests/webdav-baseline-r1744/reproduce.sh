#!/bin/sh
# Run with: nix-shell -p rclone --run 'sh reproduce.sh'
# Provide a current EXISTING admin session id through DAV_SESSION_ID.
# The cookie is passed only in the child environment. Do not enable shell -x.
set -eu
: "${DAV_SESSION_ID:?set DAV_SESSION_ID to a current existing-admin session id}"
export RCLONE_CONFIG_BASELINE_TYPE=webdav
export RCLONE_CONFIG_BASELINE_URL=http://127.0.0.1:8080/files/dav
export RCLONE_CONFIG_BASELINE_VENDOR=other
export RCLONE_CONFIG_BASELINE_HEADERS="Cookie,sessionid=<redacted>}"
unset DAV_SESSION_ID
W=$(mktemp -d /tmp/newbound-dav-baseline-reproduce.XXXXXX)
trap 'rm -rf "$W"' EXIT HUP INT TERM
python3 -c 'from pathlib import Path; import sys; Path(sys.argv[1]).write_bytes(bytes(range(256))*16)' "$W/payload.bin"
R='baseline:webdav-baseline-r1744-reproduce'
C='--config /dev/null --contimeout 5s --timeout 15s --retries 1 --low-level-retries 1 --stats 0'
rclone version
rclone lsd baseline: $C -vv --dump headers,bodies
rclone mkdir "$R" $C -vv --dump headers
# Expected pre-change result: PUT may be 201, but rclone exits 1 on file PROPFIND 501.
rclone copyto "$W/payload.bin" "$R/payload.bin" $C -vv --dump headers
rclone lsl "$R" $C -vv --dump headers,bodies
rclone copy "$R" "$W/downloaded" $C -vv --dump headers
cmp "$W/payload.bin" "$W/downloaded/payload.bin"
sha256sum "$W/payload.bin" "$W/downloaded/payload.bin"
# Actual file COPY/MOVE are observed with a collection-scoped filtered command.
rclone copy "$R" baseline:webdav-baseline-r1744-reproduce-copy --include payload.bin $C -vv --dump headers
rclone move "$R" baseline:webdav-baseline-r1744-reproduce-renamed --include payload.bin $C -vv --dump headers
rclone purge "$R" $C -vv --dump headers
# Expected pre-change: non-empty purge above returns 409; this two-step works.
rclone delete "$R" $C -vv --dump headers
rclone purge "$R" $C -vv --dump headers
# Cleanup any partial destination collections after the demonstrated 405s.
rclone delete baseline:webdav-baseline-r1744-reproduce-copy $C || true
rclone purge baseline:webdav-baseline-r1744-reproduce-copy $C || true
rclone delete baseline:webdav-baseline-r1744-reproduce-renamed $C || true
rclone purge baseline:webdav-baseline-r1744-reproduce-renamed $C || true
# No Cookie: each of these must be 403.
curl -sS -X PROPFIND -H 'Depth: 0' -o /dev/null -w 'anonymous PROPFIND=%{http_code}\n' "$RCLONE_CONFIG_BASELINE_URL/"
curl -sS -X GET -o /dev/null -w 'anonymous GET=%{http_code}\n' "$RCLONE_CONFIG_BASELINE_URL/"
curl -sS -X PUT --data-binary 'anonymous-must-not-write' -o /dev/null -w 'anonymous PUT=%{http_code}\n' "$RCLONE_CONFIG_BASELINE_URL/webdav-baseline-anonymous.bin"
