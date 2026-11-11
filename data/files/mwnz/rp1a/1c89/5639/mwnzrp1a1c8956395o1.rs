let resolved = resolve_path(path.clone());
if resolved.get_string("status") == "err" {
    panic!("403 Forbidden: {}", resolved.get_string("msg"));
}
if !resolved.get_boolean("exists") {
    panic!("404 Not Found: {}", path);
}
if !resolved.get_boolean("isdir") {
    panic!("405 Method Not Allowed: {} is not a folder", path);
}
let full = resolved.get_string("path");
let relative = resolved.get_string("relative");

// Archives are scratch files outside the share; anything older than an hour is swept here.
let zdir = Path::new("runtime/files/zips");
if let Err(e) = std::fs::create_dir_all(zdir) {
    panic!("500 Cannot create zip directory: {}", e);
}
let zdir = match std::fs::canonicalize(zdir) {
    Ok(p) => p,
    Err(e) => panic!("500 Cannot resolve zip directory: {}", e)
};
if let Ok(rd) = std::fs::read_dir(&zdir) {
    for entry in rd.flatten() {
        let old = entry.metadata().ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|age| age > Duration::from_secs(3600))
            .unwrap_or(false);
        if old { let _ = std::fs::remove_file(entry.path()); }
    }
}

let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
let dest = zdir.join(format!("{}-{}.zip", stamp, std::process::id()));

// A named folder is archived from its parent so the zip holds one top-level folder
// (and an empty folder still yields a valid archive); the share root is archived as its contents.
let full_path = Path::new(&full);
let (cwd, target) = if relative.is_empty() {
    (full_path.to_path_buf(), ".".to_string())
} else {
    let name = full_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    (full_path.parent().unwrap_or(full_path).to_path_buf(), format!("./{}", name))
};

// argv form, no shell. -y stores symlinks as links instead of following them out of the share.
let out = std::process::Command::new("zip")
    .arg("-qry").arg(&dest).arg(&target)
    .current_dir(&cwd)
    .output();
match out {
    Err(e) => panic!("500 Cannot run zip (is it installed?): {}", e),
    Ok(o) => {
        if !o.status.success() || !dest.is_file() {
            let _ = std::fs::remove_file(&dest);
            let why = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let why = if why.is_empty() { String::from_utf8_lossy(&o.stdout).trim().to_string() } else { why };
            panic!("500 zip failed: {}", if why.is_empty() { "nothing to archive".to_string() } else { why });
        }
    }
}
dest.to_string_lossy().to_string()