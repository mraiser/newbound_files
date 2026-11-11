let resolved = resolve_path(path.clone());
if resolved.get_string("status") == "err" {
    panic!("403 Forbidden: {}", resolved.get_string("msg"));
}
if !resolved.get_boolean("exists") {
    panic!("404 Not Found: {}", path);
}
if resolved.get_boolean("isdir") {
    panic!("405 Method Not Allowed: {} is a collection", path);
}
let full = resolved.get_string("path");
let mime = mime_type(full.clone());
if mime.is_empty() {
    panic!("415 Unsupported Media Type: {}", path);
}
full