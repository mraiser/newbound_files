fn err(message: String) -> DataObject {
    let mut out = DataObject::new();
    out.put_string("status", "err");
    out.put_string("msg", &message);
    out
}

// The files app and its DAV adapter share one deliberately scoped root.
// Create only this application-owned runtime directory, never a caller path.
let root = Path::new("runtime/files/shared");
if let Err(e) = std::fs::create_dir_all(root) {
    return err(format!("Cannot create files share root: {}", e));
}
let canonical_root = match std::fs::canonicalize(root) {
    Ok(p) => p,
    Err(e) => return err(format!("Cannot resolve files share root: {}", e))
};

let decoded = path.replace('\\', "/");
let mut rel = PathBuf::new();
if decoded.trim().is_empty() || decoded.trim() == "." {
    // Empty means the shared root itself.
}
else {
    if decoded.chars().any(|c| c == '\0' || c.is_control()) {
        return err("Path contains a forbidden character".to_string());
    }
    // Do not let PathBuf normalization turn an ambiguous URL into another
    // resource: repeated separators and leading dot components are rejected
    // explicitly, consistently for sources and parsed Destination paths.
    if decoded.starts_with('/') || decoded.starts_with("./") || decoded.contains("//") {
        return err("Absolute, empty, and leading-dot path components are forbidden".to_string());
    }
    for component in Path::new(&decoded).components() {
        match component {
            Component::Normal(s) => rel.push(s),
            _ => return err("Absolute paths, '.', '..' and platform prefixes are forbidden".to_string())
        }
    }
    if rel.as_os_str().is_empty() {
        return err("Path does not name a resource below the share root".to_string());
    }
}

// Inspect every existing component without following symlinks. This applies to
// reads as well as writes, and catches links in intermediate parent folders.
let mut probe = canonical_root.clone();
for component in rel.components() {
    probe.push(component.as_os_str());
    if let Ok(md) = std::fs::symlink_metadata(&probe) {
        if md.file_type().is_symlink() {
            return err("Symbolic links are not served by the files share".to_string());
        }
    }
}

let full = canonical_root.join(&rel);
let mut out = DataObject::new();
out.put_string("status", "ok");
out.put_string("path", &full.to_string_lossy());
let relative = if rel.as_os_str().is_empty() { String::new() } else { rel.to_string_lossy().to_string() };
out.put_string("relative", &relative);
out.put_string("root", &canonical_root.to_string_lossy());
out.put_boolean("exists", full.exists());
out.put_boolean("isdir", full.is_dir());
out