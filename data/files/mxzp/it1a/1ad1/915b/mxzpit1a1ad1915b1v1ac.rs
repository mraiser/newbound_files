let resolved = resolve_path(path.clone());
if resolved.get_string("status") == "err" { return resolved; }
if resolved.get_string("relative").is_empty() {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", "make_collection: path is empty");
    return e;
}
let target = resolved.get_string("path");
let p = std::path::Path::new(&target);
let existed = p.is_dir();
if p.exists() && !existed {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", &format!("make_collection: {} exists and is not a directory", path));
    return e;
}
if let Err(err) = std::fs::create_dir_all(p) {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", &format!("make_collection: cannot create {}: {}", path, err));
    return e;
}
let mut out = DataObject::new();
out.put_string("path", &resolved.get_string("relative"));
out.put_boolean("created", !existed);
out