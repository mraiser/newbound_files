fn err(code: i64, msg: &str) -> DataObject {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_int("code", code);
    e.put_string("msg", &format!("put_file: {}", msg));
    e
}

let resolved = resolve_path(path.clone());
if resolved.get_string("status") == "err" { return resolved; }
if resolved.get_string("relative").is_empty() { return err(400, "path is empty"); }
let target = resolved.get_string("path");
let p = std::path::Path::new(&target);
if p.is_dir() { return err(405, &format!("{} is a directory", path)); }

// The source: bytes in hand, or a peer stream the caller opened for us to join.
let mut expect: i64 = -1;
let source: DataBytes = if content.is_bytes() || content.is_string() {
    let data = if content.is_bytes() { content.bytes().get_data() } else { content.string().into_bytes() };
    let b = DataBytes::from_bytes(&data);
    b.close_write();
    b
}
else if content.is_object() {
    let c = content.object();
    let ok = c.has("stream") && c.get_property("stream").is_int() && c.has("size") && c.get_property("size").is_int();
    if !ok { return err(400, "stream content must be {stream, size}"); }
    expect = c.get_int("size");
    if expect < 0 { return err(400, "size must not be negative"); }
    let peer = nn_session.try_get_string("username").unwrap_or_default();
    if peer.is_empty() { return err(400, "stream content needs a peer session"); }
    let id = c.get_int("stream");
    match catch_unwind(AssertUnwindSafe(|| crate::api::peer::service::get_stream(peer, id))) {
        Ok(b) => b,
        Err(_) => return err(502, "could not join the caller's stream"),
    }
}
else { return err(400, &format!("content must be bytes, a string or a stream (got {})", content.type_name_owned())); };

let parent = match p.parent() { Some(d) => d.to_path_buf(), None => return err(400, "path has no parent") };
if let Err(e) = std::fs::create_dir_all(&parent) { return err(500, &format!("cannot create parent directory: {}", e)); }
let existed = p.exists();

// Write beside the target, then rename: a broken upload never replaces a good file.
let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
let tmp = parent.join(format!(".nbupload-{}-{}", std::process::id(), nanos));
let mut file = match std::fs::File::create(&tmp) {
    Ok(f) => f,
    Err(e) => return err(500, &format!("cannot write {}: {}", path, e)),
};
let mut total: i64 = 0;
let mut starved = 0;
let mut failed = String::new();
loop {
    if expect >= 0 && total >= expect { break; }
    // A drained stream whose writer has closed is closed for reading; reading it again panics.
    if !source.is_read_open() { break; }
    let chunk = source.read(0x10000);
    if chunk.is_empty() {
        if !source.is_write_open() && source.current_len() == 0 { break; }
        starved += 1;
        if starved > 3000 { failed = "the stream stalled".to_string(); break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
        continue;
    }
    starved = 0;
    total += chunk.len() as i64;
    if let Err(e) = file.write_all(&chunk) { failed = format!("cannot write {}: {}", path, e); break; }
}
if content.is_object() && source.is_read_open() { source.close_read(); }
drop(file);
if failed.is_empty() && expect >= 0 && total != expect {
    failed = format!("received {} of {} bytes", total, expect);
}
if !failed.is_empty() {
    let _ = std::fs::remove_file(&tmp);
    return err(if failed.starts_with("cannot") { 500 } else { 400 }, &failed);
}
if let Err(e) = std::fs::rename(&tmp, p) {
    let _ = std::fs::remove_file(&tmp);
    return err(500, &format!("cannot write {}: {}", path, e));
}
let mut out = DataObject::new();
out.put_string("path", &resolved.get_string("relative"));
out.put_int("size", total);
out.put_boolean("created", !existed);
out