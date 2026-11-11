fn http_response(code: i64, reason: &str, body: String, mimetype: &str) -> DataObject {
    let mut out = DataObject::new();
    out.put_boolean("nn_http", true);
    out.put_string("status", "ok");
    out.put_int("code", code);
    out.put_string("msg", reason);
    out.put_string("body", &body);
    out.put_string("mimetype", mimetype);
    let mut headers = DataObject::new();
    headers.put_string("DAV", "1");
    out.put_object("headers", headers);
    out
}

fn http_error(code: i64, reason: &str, message: &str) -> DataObject {
    http_response(code, reason, message.to_string(), "text/plain; charset=utf-8")
}

fn request_body(body: &ndata::data::Data) -> Result<String, String> {
    if body.is_string() { return Ok(body.string()); }
    if body.is_bytes() {
        let bytes = body.bytes();
        return String::from_utf8(bytes.get_data()).map_err(|_| "PROPFIND body is not valid UTF-8".to_string());
    }
    if body.is_null() { return Ok(String::new()); }
    Err("PROPFIND body must be UTF-8 XML".to_string())
}

fn panic_message(e: Box<dyn std::any::Any + Send>, fallback: &str) -> String {
    if let Some(s) = e.downcast_ref::<String>() { s.clone() }
    else if let Some(s) = e.downcast_ref::<&str>() { s.to_string() }
    else { fallback.to_string() }
}

fn head_error(method: &str, mut out: DataObject) -> DataObject {
    if method == "HEAD" {
        // A successful HEAD already carries the underlying file length. For a
        // HEAD error, advertise the length the equivalent GET error would use.
        if !out.has("len") {
            let len = if out.has("body") { out.get_string("body").len() as i64 } else { 0 };
            out.put_int("len", len);
        }
        out.put_string("body", "");
    }
    out
}

fn status_error(message: String, fallback: &str) -> DataObject {
    let code = if message.starts_with("400") { 400 }
        else if message.starts_with("403") { 403 }
        else if message.starts_with("404") { 404 }
        else if message.starts_with("405") { 405 }
        else if message.starts_with("415") { 415 }
        else if message.starts_with("501") { 501 }
        else { 500 };
    let reason = match code {
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        415 => "Unsupported Media Type",
        501 => "Not Implemented",
        _ => "Internal Server Error",
    };
    http_error(code, reason, if message.is_empty() { fallback } else { &message })
}

fn decode_path(raw: &str) -> Result<String, String> {
    let input = raw.as_bytes();
    let mut bytes = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            if i + 2 >= input.len() { return Err("Incomplete percent escape in path".to_string()); }
            let hex = std::str::from_utf8(&input[i + 1..i + 3]).map_err(|_| "Invalid percent escape in path".to_string())?;
            let byte = u8::from_str_radix(hex, 16).map_err(|_| "Invalid percent escape in path".to_string())?;
            bytes.push(byte);
            i += 3;
        }
        else {
            bytes.push(input[i]);
            i += 1;
        }
    }
    String::from_utf8(bytes).map_err(|_| "Path is not valid UTF-8".to_string())
}

fn clean_relative(raw_path: &str) -> Result<String, String> {
    let suffix = raw_path.strip_prefix("/files/dav").ok_or_else(|| "Expected a /files/dav path".to_string())?;
    if !suffix.is_empty() && !suffix.starts_with('/') { return Err("Invalid DAV path boundary".to_string()); }
    // Remove only the one separator belonging to the DAV prefix. A second
    // slash remains leading and is rejected by the shared path resolver,
    // rather than being silently normalized into an in-share destination.
    let relative = suffix.strip_prefix('/').unwrap_or(suffix);
    decode_path(relative)
}

fn operation_error(result: DataObject, default_code: i64) -> DataObject {
    let message = if result.has("msg") { result.get_string("msg") } else { "DAV operation failed".to_string() };
    let code = if message.contains("does not exist") { 404 }
        else if message.contains("non-empty") { 409 }
        else if message.contains("is a directory") { 405 }
        else { default_code };
    let reason = match code { 404 => "Not Found", 405 => "Method Not Allowed", 409 => "Conflict", _ => "Internal Server Error" };
    http_error(code, reason, &message)
}

fn destination_relative(raw: &str, request_host: &str) -> Result<String, (i64, String)> {
    let value = raw.trim();
    if value.is_empty() { return Err((400, "Destination header is empty".to_string())); }
    if value.contains('?') || value.contains('#') {
        return Err((400, "Destination query strings and fragments are not supported".to_string()));
    }
    let path = if value.starts_with('/') {
        value.to_string()
    }
    else {
        let (scheme, remainder) = value.split_once("://").ok_or_else(|| {
            (400, "Destination must be an absolute URI or an origin-form absolute path".to_string())
        })?;
        if scheme != "http" && scheme != "https" {
            return Err((400, "Only http and https Destination URIs are supported".to_string()));
        }
        let split = remainder.find('/');
        let authority = if let Some(index) = split { &remainder[..index] } else { remainder };
        if authority.is_empty() || authority.contains('@') || authority.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err((400, "Destination URI has an invalid authority".to_string()));
        }
        if request_host.is_empty() {
            return Err((400, "An absolute Destination URI requires the request Host header".to_string()));
        }
        if !authority.eq_ignore_ascii_case(request_host.trim()) {
            return Err((403, "Destination URI authority does not match this DAV server".to_string()));
        }
        if let Some(index) = split { remainder[index..].to_string() } else { "/".to_string() }
    };
    clean_relative(&path).map_err(|message| (403, format!("Destination is outside this DAV share: {}", message)))
}

fn transfer_error(result: DataObject) -> DataObject {
    let message = if result.has("msg") { result.get_string("msg") } else { "DAV transfer failed".to_string() };
    let code = if result.has("code") { result.get_int("code") } else { 500 };
    let reason = match code {
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        412 => "Precondition Failed",
        _ => "Internal Server Error",
    };
    http_error(code, reason, &message)
}

let method = nn_method.to_uppercase();
let target = match clean_relative(&nn_path) {
    Ok(p) => p,
    Err(message) => return head_error(&method, http_error(403, "Forbidden", &message))
};
let resolved = resolve_path(target.clone());
if resolved.get_string("status") == "err" {
    return head_error(&method, http_error(403, "Forbidden", &resolved.get_string("msg")));
}
let is_root = resolved.get_string("relative").is_empty();
let absolute_target = resolved.get_string("path");
let path = Path::new(&absolute_target);

match method.as_str() {
    "OPTIONS" => {
        let mut out = http_response(200, "OK", String::new(), "text/plain; charset=utf-8");
        let mut headers = out.get_object("headers");
        headers.put_string("Allow", "OPTIONS, PROPFIND, GET, HEAD, PUT, MKCOL, DELETE, COPY, MOVE");
        out.put_object("headers", headers);
        out
    },
    "PROPFIND" => {
        if !path.exists() { return http_error(404, "Not Found", "DAV resource does not exist"); }
        let depth = if nn_headers.has("DEPTH") { nn_headers.get_string("DEPTH") } else { "infinity".to_string() };
        if depth != "0" && depth != "1" {
            return http_error(501, "Not Implemented", "Only Depth: 0 and Depth: 1 are supported");
        }
        let request_xml = match request_body(&nn_body) {
            Ok(body) => body,
            Err(message) => return http_error(400, "Bad Request", &message),
        };
        match std::panic::catch_unwind(|| propfind(target.clone(), "/files/dav".to_string(), depth, request_xml)) {
            Ok(xml) => http_response(207, "Multi-Status", xml, "application/xml; charset=utf-8"),
            Err(e) => status_error(panic_message(e, "PROPFIND failed"), "PROPFIND failed"),
        }
    },
    "GET" | "HEAD" => {
        if is_root { return head_error(&method, http_error(405, "Method Not Allowed", "GET and HEAD require a file resource")); }
        let out = match std::panic::catch_unwind(|| get_file(target.clone())) {
            Ok(file) => {
                let mut out = http_response(200, "OK", String::new(), &mime_type(file.clone()));
                if method == "HEAD" {
                    // The current listener does not suppress a file stream for
                    // HEAD. Keep a zero-length string body, but advertise the
                    // exact metadata GET would use; the listener framing fix
                    // owns actual HEAD suppression, outside this command item.
                    let len = std::fs::metadata(&file).map(|m| m.len() as i64).unwrap_or(0);
                    out.put_int("len", len);
                }
                else { out.put_string("file", &file); }
                out
            },
            Err(e) => status_error(panic_message(e, "File request failed"), "File request failed"),
        };
        head_error(&method, out)
    },
    "PUT" => {
        if is_root { return http_error(409, "Conflict", "PUT requires a file path below the DAV root"); }
        if !nn_body.is_bytes() && !nn_body.is_string() {
            return http_error(400, "Bad Request", "PUT body must be raw bytes");
        }
        let existed = path.is_file();
        if path.is_dir() { return http_error(405, "Method Not Allowed", "PUT target is a collection"); }
        let result = put_file(target.clone(), nn_body.clone());
        if result.has("status") && result.get_string("status") == "err" { return operation_error(result, 500); }
        if existed { http_response(204, "No Content", String::new(), "text/plain") }
        else { http_response(201, "Created", String::new(), "text/plain") }
    },
    "MKCOL" => {
        if is_root || path.exists() { return http_error(405, "Method Not Allowed", "MKCOL target already exists"); }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.is_dir() {
                return http_error(409, "Conflict", "MKCOL parent collection does not exist");
            }
        }
        let result = make_collection(target.clone());
        if result.has("status") && result.get_string("status") == "err" { return operation_error(result, 500); }
        http_response(201, "Created", String::new(), "text/plain")
    },
    "DELETE" => {
        if is_root { return http_error(403, "Forbidden", "The DAV root cannot be deleted"); }
        let result = dav_tree("DELETE".to_string(), target.clone(), String::new(), "T".to_string());
        if result.has("status") && result.get_string("status") == "err" { return transfer_error(result); }
        http_response(204, "No Content", String::new(), "text/plain")
    },
    "COPY" | "MOVE" => {
        let destination_header = match nn_headers.try_get_string("DESTINATION") {
            Ok(value) => value,
            Err(_) => return http_error(400, "Bad Request", "COPY and MOVE require a Destination header"),
        };
        let host = nn_headers.try_get_string("HOST").unwrap_or_else(|_| String::new());
        let destination = match destination_relative(&destination_header, &host) {
            Ok(value) => value,
            Err((code, message)) => {
                let reason = if code == 400 { "Bad Request" } else { "Forbidden" };
                return http_error(code, reason, &message);
            }
        };
        let overwrite = nn_headers.try_get_string("OVERWRITE").unwrap_or("T".to_string());
        if overwrite != "T" && overwrite != "F" {
            return http_error(400, "Bad Request", "Overwrite must be exactly T or F");
        }
        let result = dav_tree(method.clone(), target.clone(), destination, overwrite);
        if result.has("status") && result.get_string("status") == "err" { return transfer_error(result); }
        if result.get_boolean("overwritten") { http_response(204, "No Content", String::new(), "text/plain") }
        else { http_response(201, "Created", String::new(), "text/plain") }
    },
    _ => {
        let mut out = http_error(405, "Method Not Allowed", &format!("{} is not supported by this DAV endpoint", method));
        let mut headers = out.get_object("headers");
        headers.put_string("Allow", "OPTIONS, PROPFIND, GET, HEAD, PUT, MKCOL, DELETE, COPY, MOVE");
        out.put_object("headers", headers);
        out
    }
}