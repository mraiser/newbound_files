let resolved = resolve_path(path.clone());
if resolved.get_string("status") == "err" { return resolved; }
let target = resolved.get_string("path");
if !resolved.get_boolean("exists") {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", &format!("list_dir: {} does not exist", path));
    return e;
}
if !resolved.get_boolean("isdir") {
    let mut e = DataObject::new();
    e.put_string("status", "err");
    e.put_string("msg", &format!("list_dir: {} is not a directory", path));
    return e;
}
let names = list(target.clone());
let mut dirs: Vec<String> = Vec::new();
let mut files: Vec<String> = Vec::new();
for i in 0..names.len() {
    let n = names.get_string(i);
    if n == "." || n == ".." { continue; }
    if std::path::Path::new(&target).join(&n).is_dir() { dirs.push(n); }
    else { files.push(n); }
}
dirs.sort();
files.sort();
let relative = resolved.get_string("relative");
let mut entries = DataArray::new();
for n in dirs.iter().chain(files.iter()) {
    let full = std::path::Path::new(&target).join(n);
    let mut e = DataObject::new();
    e.put_string("name", n);
    let entry_path = if relative.is_empty() { n.clone() } else { format!("{}/{}", relative, n) };
    e.put_string("path", &entry_path);
    let mut isdir = false;
    let mut size = 0i64;
    let mut mtime = 0i64;
    if let Ok(md) = std::fs::metadata(&full) {
        isdir = md.is_dir();
        size = md.len() as i64;
        if let Ok(t) = md.modified() {
            if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                mtime = d.as_millis() as i64;
            }
        }
    }
    e.put_boolean("isdir", isdir);
    e.put_int("size", size as i64);
    e.put_int("mtime", mtime);
    entries.push_object(e);
}
let mut out = DataObject::new();
out.put_string("path", &relative);
out.put_array("entries", entries);
out.put_int("count", (dirs.len() + files.len()) as i64);
out