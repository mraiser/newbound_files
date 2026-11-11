# newbound_files

The **files** app for [Newbound](https://github.com/mraiser/newbound): share a folder with your peers, browse theirs, and mount a peer's share on your filesystem over WebDAV. Everything that crosses between machines travels over the encrypted Newbound peer channel.

## What it does

- **A shared folder.** Each instance shares `runtime/files/shared`. Nothing outside that folder is reachable: every path goes through one resolver that rejects traversal (`.` and `..`), absolute paths, control characters and symlinks.
- **A browser UI** at `/files/index.html`: your own share plus every known peer in a sidebar, with breadcrumbs, a listing, downloads, and upload to your own share.
- **WebDAV for your own share** at `/files/dav/`.
- **WebDAV for a peer's share** at `/files/peerdav/<peer uuid>/`. Your mount client talks to your own instance, which translates each request into calls to the peer.

## Install

In a running Newbound instance, import this repo with the Development app's GitHub import (`dev.github.import`) using the repo URL. That clones it into `repositories/newbound_files` and links `data/files`. The library compiles to its own crate, so the instance needs one restart afterwards.

Mounting needs a Newbound core with opt-in HTTP Basic auth in the listener (`basicauth=true` in `config.properties`).

## Sharing with a peer

Access is by security group on the **sharing** machine. In the security app, add the peer's user to:

| Group | Allows |
|---|---|
| `files` | List and download |
| `files-write` | Upload, create folders, delete, copy, move |

The groups are independent: `files-write` does not imply `files`. A peer in neither group sees nothing. Both machines need this library installed.

## Mounting a peer's share

1. Add `basicauth=true` to `config.properties` on your own machine and restart.
2. Find the peer's uuid in the peer app.
3. Mount with an admin user of your own instance (`<port>` is `http_port` in `config.properties`):

```
rclone mount :webdav: /mnt/peer \
  --webdav-url http://127.0.0.1:<port>/files/peerdav/<peer uuid>/ \
  --webdav-user <admin> --webdav-pass $(rclone obscure <password>)
```

Your own share mounts the same way from `/files/dav/`. `/files/peerdav/local/` is a loopback onto your own share through the peer adapter, for testing.

Basic auth sends the password unencrypted on every request. Mount against `127.0.0.1` only, unless something in front of the listener adds TLS.

## Limits

- **Uploads are capped at 64 MiB per file.** The listener buffers the whole request body in memory.
- **No Range requests.** Seeking inside a large file downloads it again from the start.
- **No LOCK/UNLOCK.** rclone and davfs2 work without locking; macOS Finder mounts such a share read-only.
- **The mounting user must be an admin** on the mounting machine. `dav` and `peerdav` are admin-only.
- **The UI cannot upload to a peer.** Use a mount for that.

## Commands

All in the `files` control of the `files` library.

| Command | Access | Purpose |
|---|---|---|
| `list_dir` | `files` | Directory listing as JSON, for the UI |
| `get_file` | `files` | One file, streamed |
| `propfind` | `files` | WebDAV PROPFIND XML, Depth 0/1 |
| `put_file` | `files-write` | Write a file from bytes, a string, or a peer stream; written to a temporary file and renamed into place |
| `make_collection` | `files-write` | Create a folder |
| `delete_path` | `files-write` | Delete a file or an empty folder |
| `dav_tree` | `files-write` | Recursive DELETE, and COPY/MOVE with Overwrite handling |
| `dav` | admin | WebDAV adapter for the local share |
| `peerdav` | admin | WebDAV adapter for a peer's share |
| `resolve_path` | internal | The path containment check the others share |

## Repo layout

- `data/files/` is the library: a Newbound store, the source of truth. Do not edit it by hand; change it through the `dev.code` commands in a running instance.
- `tests/` holds WebDAV test harnesses (curl and rclone). They are kept out of the library so they are not served or shipped to peers. They currently hardcode `http://127.0.0.1:8080/files/dav`. See `tests/README.md`.

Not in the repo: `runtime/files` (the shared folder itself) and the generated `files/` crate.
