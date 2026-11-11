fn failure(code: i64, message: impl Into<String>) -> DataObject {
    let mut out = DataObject::new();
    out.put_string("status", "err");
    out.put_int("code", code);
    out.put_string("msg", &message.into());
    out
}

fn io_error(context: &str, error: std::io::Error) -> (i64, String) {
    (500, format!("{}: {}", context, error))
}

// Validate the whole tree before any mutation. In particular, do not silently
// unlink a symlink encountered during DELETE or an overwrite: reject it.
fn validate_tree(path: &Path) -> Result<(), (i64, String)> {
    let metadata = fs::symlink_metadata(path).map_err(|e| io_error("Cannot inspect DAV resource", e))?;
    let kind = metadata.file_type();
    if kind.is_symlink() {
        return Err((403, format!("Symbolic links are forbidden: {}", path.display())));
    }
    if !kind.is_file() && !kind.is_dir() {
        return Err((403, format!("Only regular files and collections are supported: {}", path.display())));
    }
    if kind.is_dir() {
        let entries = fs::read_dir(path).map_err(|e| io_error("Cannot read DAV collection", e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_error("Cannot read DAV collection entry", e))?;
            validate_tree(&entry.path())?;
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|e| format!("Cannot inspect copy source: {}", e))?;
    let kind = metadata.file_type();
    if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) {
        return Err(format!("Copy source changed or is not a regular file/collection: {}", source.display()));
    }
    if kind.is_file() {
        fs::copy(source, destination).map_err(|e| format!("Cannot copy DAV file: {}", e))?;
        return Ok(());
    }
    fs::create_dir(destination).map_err(|e| format!("Cannot create staged DAV collection: {}", e))?;
    let entries = fs::read_dir(source).map_err(|e| format!("Cannot read copy source: {}", e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Cannot read copy source entry: {}", e))?;
        copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn remove_tree(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("Cannot inspect delete target: {}", e))?;
    let kind = metadata.file_type();
    if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) {
        return Err(format!("Delete target changed or is not a regular file/collection: {}", path.display()));
    }
    if kind.is_dir() {
        let entries = fs::read_dir(path).map_err(|e| format!("Cannot read delete target: {}", e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Cannot read delete target entry: {}", e))?;
            remove_tree(&entry.path())?;
        }
        fs::remove_dir(path).map_err(|e| format!("Cannot remove DAV collection: {}", e))?;
    }
    else {
        fs::remove_file(path).map_err(|e| format!("Cannot remove DAV file: {}", e))?;
    }
    Ok(())
}

fn temporary_sibling(parent: &Path, label: &str) -> PathBuf {
    // unique_session_id is random enough for these single-use, non-public names;
    // the nonexistence check also prevents replacing a pre-existing user path.
    loop {
        let candidate = parent.join(format!(".newbound-dav-{}-{}", label, unique_session_id()));
        if fs::symlink_metadata(&candidate).is_err() { return candidate; }
    }
}

fn rollback_staging(staging: &Path, source: &Path, moving: bool) {
    if moving {
        let _ = fs::rename(staging, source);
    }
    else {
        let _ = remove_tree(staging);
    }
}

let operation = operation.to_uppercase();
if operation != "DELETE" && operation != "COPY" && operation != "MOVE" {
    return failure(400, "DAV tree operation must be DELETE, COPY, or MOVE");
}
if operation != "DELETE" && overwrite != "T" && overwrite != "F" {
    return failure(400, "DAV Overwrite must be T or F");
}

let source_resolved = resolve_path(source.clone());
if source_resolved.get_string("status") == "err" {
    return failure(403, source_resolved.get_string("msg"));
}
if source_resolved.get_string("relative").is_empty() {
    return failure(403, "The DAV root cannot be deleted, moved, or copied into itself");
}
let source_path = PathBuf::from(source_resolved.get_string("path"));
let source_metadata = match fs::symlink_metadata(&source_path) {
    Ok(metadata) => metadata,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return failure(404, "DAV source does not exist"),
    Err(e) => return failure(500, format!("Cannot inspect DAV source: {}", e)),
};
if source_metadata.file_type().is_symlink() {
    return failure(403, "Symbolic-link DAV sources are forbidden");
}
if let Err((code, message)) = validate_tree(&source_path) { return failure(code, message); }

if operation == "DELETE" {
    if let Err(message) = remove_tree(&source_path) { return failure(500, message); }
    let mut out = DataObject::new();
    out.put_string("path", &source_resolved.get_string("relative"));
    out.put_boolean("isdir", source_metadata.is_dir());
    return out;
}

// Destination has exactly the same common containment checks as source.
let destination_resolved = resolve_path(destination.clone());
if destination_resolved.get_string("status") == "err" {
    return failure(403, destination_resolved.get_string("msg"));
}
if destination_resolved.get_string("relative").is_empty() {
    return failure(403, "The DAV root cannot be overwritten or used as a transfer destination");
}
let destination_path = PathBuf::from(destination_resolved.get_string("path"));
if source_path == destination_path {
    return failure(403, "DAV source and destination are the same resource");
}
let source_is_collection = source_metadata.is_dir();
if source_is_collection && destination_path.starts_with(&source_path) {
    return failure(403, "A collection cannot be copied or moved into its own subtree");
}
// Replacing an ancestor would consume a COPY source (and makes MOVE rollback
// ambiguous). Reject before touching either path rather than risking loss.
if destination_path.starts_with(&source_path) || source_path.starts_with(&destination_path) {
    return failure(403, "DAV source and destination cannot contain one another");
}
let destination_parent = match destination_path.parent() {
    Some(parent) => parent,
    None => return failure(409, "DAV destination parent collection does not exist"),
};
match fs::symlink_metadata(destination_parent) {
    Ok(metadata) if metadata.file_type().is_symlink() => return failure(403, "Symbolic-link destination ancestry is forbidden"),
    Ok(metadata) if !metadata.is_dir() => return failure(409, "DAV destination parent is not a collection"),
    Ok(_) => {},
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return failure(409, "DAV destination parent collection does not exist"),
    Err(e) => return failure(500, format!("Cannot inspect DAV destination parent: {}", e)),
}
let destination_exists = match fs::symlink_metadata(&destination_path) {
    Ok(metadata) => {
        if metadata.file_type().is_symlink() { return failure(403, "Symbolic-link DAV destinations are forbidden"); }
        true
    },
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
    Err(e) => return failure(500, format!("Cannot inspect DAV destination: {}", e)),
};
if destination_exists && overwrite == "F" {
    return failure(412, "DAV destination exists and Overwrite is F");
}
if destination_exists {
    if let Err((code, message)) = validate_tree(&destination_path) { return failure(code, message); }
}

// All validation above is complete before the first mutation. COPY builds a
// complete staging tree before touching the old destination. MOVE stages by
// rename on the same share filesystem. A pre-existing destination is retained
// as a backup until the staged resource has been installed successfully.
let staging = temporary_sibling(destination_parent, "staging");
let moving = operation == "MOVE";
if moving {
    if let Err(e) = fs::rename(&source_path, &staging) {
        return failure(500, format!("Cannot stage DAV move: {}", e));
    }
}
else if let Err(message) = copy_tree(&source_path, &staging) {
    let _ = remove_tree(&staging);
    return failure(500, message);
}

let backup = if destination_exists {
    let path = temporary_sibling(destination_parent, "backup");
    if let Err(e) = fs::rename(&destination_path, &path) {
        rollback_staging(&staging, &source_path, moving);
        return failure(500, format!("Cannot preserve overwritten DAV destination: {}", e));
    }
    Some(path)
}
else { None };

if let Err(e) = fs::rename(&staging, &destination_path) {
    if let Some(path) = &backup { let _ = fs::rename(path, &destination_path); }
    rollback_staging(&staging, &source_path, moving);
    return failure(500, format!("Cannot install DAV destination: {}", e));
}
if let Some(path) = &backup {
    if let Err(message) = remove_tree(path) {
        return failure(500, format!("DAV transfer committed but overwrite backup cleanup failed: {}", message));
    }
}
let mut out = DataObject::new();
out.put_string("source", &source_resolved.get_string("relative"));
out.put_string("destination", &destination_resolved.get_string("relative"));
out.put_boolean("overwritten", destination_exists);
out.put_boolean("isdir", source_is_collection);
out