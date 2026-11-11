let resolved = resolve_path(path.clone());
if resolved.get_string("status") == "err" { return resolved; }
if resolved.get_string("relative").is_empty() {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", "delete_path: the shared root cannot be deleted");
    return e;
}
let target = resolved.get_string("path");
let p = std::path::Path::new(&target);
if !p.exists() {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", &format!("delete_path: {} does not exist", path));
    return e;
}
let isdir = p.is_dir();
if isdir {
    match std::fs::read_dir(p) {
        Ok(mut it) => {
            if it.next().is_some() {
                let mut e = DataObject::new();
                e.put_string("status", "err");
                e.put_string("msg", &format!("delete_path: {} is a non-empty directory; refusing to recurse - delete its contents first", path));
                return e;
            }
        },
        Err(err) => {
            let mut e = DataObject::new();
            e.put_string("status", "err");
            e.put_string("msg", &format!("delete_path: cannot read {}: {}", path, err));
            return e;
        }
    }
    if let Err(err) = std::fs::remove_dir(p) {
        let mut e = DataObject::new();
        e.put_string("status", "err");
        e.put_string("msg", &format!("delete_path: cannot remove directory {}: {}", path, err));
        return e;
    }
} else if let Err(err) = std::fs::remove_file(p) {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", &format!("delete_path: cannot remove file {}: {}", path, err));
    return e;
}
let mut out = DataObject::new();
out.put_string("path", &resolved.get_string("relative"));
out.put_boolean("isdir", isdir);
out