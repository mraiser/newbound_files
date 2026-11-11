let resolved = resolve_path(path.clone());
if resolved.get_string("status") == "err" { return resolved; }
if resolved.get_string("relative").is_empty() {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", "put_file: path is empty");
    return e;
}
let target = resolved.get_string("path");
let p = std::path::Path::new(&target);
if p.is_dir() {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", &format!("put_file: {} is a directory", path));
    return e;
}
if let Some(parent) = p.parent() {
    if let Err(err) = std::fs::create_dir_all(parent) {
        let mut e = DataObject::new();
        e.put_string("status", "err");
        e.put_string("msg", &format!("put_file: cannot create parent directory: {}", err));
        return e;
    }
}
let bytes = if content.is_bytes() {
    content.bytes().get_data()
} else if content.is_string() {
    content.string().into_bytes()
} else {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", &format!("put_file: content must be bytes (got {})", content.type_name_owned()));
    return e;
};
if let Err(err) = std::fs::write(p, &bytes) {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", &format!("put_file: cannot write {}: {}", path, err));
    return e;
}
let mut out = DataObject::new();
out.put_string("path", &resolved.get_string("relative"));
out.put_int("size", bytes.len() as i64);
out