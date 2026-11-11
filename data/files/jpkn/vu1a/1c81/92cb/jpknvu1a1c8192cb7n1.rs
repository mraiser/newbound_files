const ALLOW: &str = "OPTIONS, PROPFIND, GET, HEAD, PUT, MKCOL, DELETE, COPY, MOVE";

fn resp(code: i64, reason: &str, body: &str, mime: &str) -> DataObject {
    let mut out = DataObject::new();
    out.put_boolean("nn_http", true);
    out.put_string("status", "ok");
    out.put_int("code", code);
    out.put_string("msg", reason);
    out.put_string("body", body);
    out.put_string("mimetype", mime);
    let mut headers = DataObject::new();
    headers.put_string("DAV", "1");
    out.put_object("headers", headers);
    out
}

fn fail(code: i64, message: &str) -> DataObject {
    let reason = match code {
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        412 => "Precondition Failed",
        415 => "Unsupported Media Type",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Internal Server Error",
    };
    resp(code, reason, message, "text/plain; charset=utf-8")
}

// Turn the far side's (or the transport's) error text into an HTTP status.
fn remote_err(message: &str) -> DataObject {
    for code in [400, 403, 404, 405, 415, 501] {
        if message.starts_with(&code.to_string()) { return fail(code, message); }
    }
    if message.contains("UNAUTHORIZED") { return fail(403, "The peer has not put you in its files group"); }
    if message.contains("No such peer") { return fail(404, "No such peer"); }
    if message.contains("No route") || message.contains("No live route") || message.contains("TIMEOUT") || message.contains("closed") {
        return fail(504, message);
    }
    fail(502, if message.is_empty() { "The peer did not answer the files request" } else { message })
}

fn decode_path(raw: &str) -> Result<String, String> {
    let input = raw.as_bytes();
    let mut bytes = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            if i + 2 >= input.len() { return Err("Incomplete percent escape in path".to_string()); }
            let hex = std::str::from_utf8(&input[i + 1..i + 3]).map_err(|_| "Invalid percent escape in path".to_string())?;
            bytes.push(u8::from_str_radix(hex, 16).map_err(|_| "Invalid percent escape in path".to_string())?);
            i += 3;
        }
        else { bytes.push(input[i]); i += 1; }
    }
    String::from_utf8(bytes).map_err(|_| "Path is not valid UTF-8".to_string())
}

// Text of the first <...name>value</...> element in a PROPFIND answer.
fn tag_value(xml: &str, name: &str) -> Option<String> {
    let key = format!("{}>", name);
    let start = xml.find(&key)? + key.len();
    let end = xml[start..].find('<')? + start;
    let v = xml[start..end].trim().to_string();
    if v.is_empty() { None } else { Some(v) }
}

fn peer_exec(uuid: &str, app: &str, cmd: &str, params: DataObject) -> DataObject {
    let (u, a, c) = (uuid.to_string(), app.to_string(), cmd.to_string());
    match catch_unwind(AssertUnwindSafe(|| crate::api::peer::service::exec(u, a, c, params))) {
        Ok(r) => r,
        Err(_) => {
            let mut o = DataObject::new();
            o.put_string("status", "err");
            o.put_string("msg", "Peer request failed");
            o
        }
    }
}

fn err_text(r: &DataObject) -> String {
    if r.has("msg") && r.get_property("msg").is_string() { r.get_string("msg") } else { String::new() }
}

let method = nn_method.to_uppercase();
let rest = match nn_path.strip_prefix("/files/peerdav/") {
    Some(r) => r,
    None => return fail(400, "Expected /files/peerdav/<peer uuid>/<path>"),
};
let (uuid, raw) = match rest.find('/') { Some(i) => (&rest[..i], &rest[i + 1..]), None => (rest, "") };
if uuid.is_empty() || !uuid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
    return fail(400, "Expected /files/peerdav/<peer uuid>/<path>");
}
let target = match decode_path(raw) {
    Ok(p) => p.trim_end_matches('/').to_string(),
    Err(message) => return fail(400, &message),
};
let prefix = format!("/files/peerdav/{}", uuid);

// PROPFIND on the peer's own files.propfind, with hrefs under THIS mount point.
let pf = |path: &str, depth: &str, xml: String| -> Result<String, DataObject> {
    let mut p = DataObject::new();
    p.put_string("path", path);
    p.put_string("href_prefix", &prefix);
    p.put_string("depth", depth);
    p.put_string("request_xml", &xml);
    let mut r = peer_exec(uuid, "files", "propfind", p);
    // A panic inside the far command can come back wrapped as data:{status:err,msg}.
    if r.has("data") && r.get_property("data").is_object() {
        let d = r.get_object("data");
        if d.has("status") && d.get_property("status").is_string() && d.get_string("status") == "err" { r = d; }
    }
    let ok = r.has("status") && r.get_property("status").is_string() && r.get_string("status") == "ok";
    if ok && r.has("msg") && r.get_property("msg").is_string() { Ok(r.get_string("msg")) }
    else { Err(remote_err(&err_text(&r))) }
};
let propfind = |depth: &str, xml: String| pf(&target, depth, xml);

// A write on the peer's own files command; the peer gates it with its files-write group.
let write = |cmd: &str, p: DataObject| -> Result<DataObject, DataObject> {
    let r = peer_exec(uuid, "files", cmd, p);
    let is_err = |o: &DataObject| o.has("status") && o.get_property("status").is_string() && o.get_string("status") == "err";
    if is_err(&r) || !r.has("status") {
        let text = err_text(&r);
        if text.contains("UNAUTHORIZED") { return Err(fail(403, "The peer has not put you in its files-write group")); }
        return Err(remote_err(&text));
    }
    let d = if r.has("data") && r.get_property("data").is_object() { r.get_object("data") } else { DataObject::new() };
    if is_err(&d) {
        let code = if d.has("code") && d.get_property("code").is_int() { d.get_int("code") } else { 500 };
        return Err(fail(code, &err_text(&d)));
    }
    Ok(d)
};

match method.as_str() {
    "OPTIONS" => {
        let mut out = resp(200, "OK", "", "text/plain; charset=utf-8");
        let mut headers = out.get_object("headers");
        headers.put_string("Allow", ALLOW);
        out.put_object("headers", headers);
        out
    },
    "PROPFIND" => {
        let depth = if nn_headers.has("DEPTH") { nn_headers.get_string("DEPTH") } else { "infinity".to_string() };
        if depth != "0" && depth != "1" { return fail(501, "Only Depth: 0 and Depth: 1 are supported"); }
        let xml = if nn_body.is_string() { nn_body.string() }
            else if nn_body.is_bytes() {
                match String::from_utf8(nn_body.bytes().get_data()) {
                    Ok(s) => s,
                    Err(_) => return fail(400, "PROPFIND body is not valid UTF-8"),
                }
            }
            else { String::new() };
        match propfind(&depth, xml) {
            Ok(xml) => resp(207, "Multi-Status", &xml, "application/xml; charset=utf-8"),
            Err(e) => e,
        }
    },
    "GET" | "HEAD" => {
        let head = method == "HEAD";
        // One Depth 0 PROPFIND first: it answers 404 / collection and gives the length.
        let stat = match propfind("0", String::new()) {
            Ok(xml) => xml,
            Err(mut e) => {
                if head { let n = e.get_string("body").len() as i64; e.put_int("len", n); e.put_string("body", ""); }
                return e;
            }
        };
        if target.is_empty() || stat.contains("collection/>") {
            let mut e = fail(405, "GET and HEAD require a file resource");
            if head { let n = e.get_string("body").len() as i64; e.put_int("len", n); e.put_string("body", ""); }
            return e;
        }
        let len = tag_value(&stat, "getcontentlength").and_then(|v| v.parse::<i64>().ok());
        let mime = tag_value(&stat, "getcontenttype").unwrap_or_else(|| "application/octet-stream".to_string());
        let mut out = resp(200, "OK", "", &mime);
        if head {
            out.put_int("len", len.unwrap_or(0));
            return out;
        }
        if uuid == "local" {
            // Loopback for the test harness: this instance's own share.
            return match catch_unwind(|| get_file(target.clone())) {
                Ok(file) => { out.put_string("file", &file); out },
                Err(_) => fail(404, "File request failed"),
            };
        }
        // The peer streams the file back through its HTTP-over-peer door (peer.peer.local).
        let mut params = DataObject::new();
        params.put_string("path", &target);
        let mut req = DataObject::new();
        req.put_string("path", "/files/get_file");
        req.put_object("params", params);
        req.put_object("headers", DataObject::new());
        let system = DataStore::globals().get_object("system");
        if system.has("users") {
            let users = system.get_object("users");
            if users.has(uuid) {
                let user = users.get_object(uuid);
                if user.has("session_id") && user.get_property("session_id").is_string() {
                    req.put_string("sessionid", &user.get_string("session_id"));
                }
            }
        }
        let mut o = DataObject::new();
        o.put_object("request", req);
        let r = peer_exec(uuid, "peer", "local", o);
        if r.has("data") && r.get_property("data").is_object() {
            let d = r.get_object("data");
            if d.has("stream_id") && d.get_property("stream_id").is_int() {
                let id = d.get_int("stream_id");
                let u = uuid.to_string();
                return match catch_unwind(AssertUnwindSafe(|| crate::api::peer::service::get_stream(u, id))) {
                    Ok(bytes) => {
                        out.remove_property("body");
                        out.put_bytes("body", bytes);
                        if let Some(n) = len { out.put_int("len", n); }
                        out
                    },
                    Err(_) => fail(502, "Could not join the peer's file stream"),
                };
            }
        }
        remote_err(&err_text(&r))
    },
    "MKCOL" => {
        if target.is_empty() { return fail(405, "MKCOL target already exists"); }
        match propfind("0", String::new()) {
            Ok(_) => return fail(405, "MKCOL target already exists"),
            Err(e) => if e.get_int("code") != 404 { return e; },
        }
        if let Some(i) = target.rfind('/') {
            match pf(&target[..i], "0", String::new()) {
                Ok(xml) => if !xml.contains("collection/>") { return fail(409, "MKCOL parent is not a collection"); },
                Err(e) => return if e.get_int("code") == 404 { fail(409, "MKCOL parent collection does not exist") } else { e },
            }
        }
        let mut p = DataObject::new();
        p.put_string("path", &target);
        match write("make_collection", p) {
            Ok(_) => resp(201, "Created", "", "text/plain"),
            Err(e) => e,
        }
    },
    "DELETE" | "COPY" | "MOVE" => {
        if target.is_empty() { return fail(403, "The share root cannot be deleted, copied or moved"); }
        let mut p = DataObject::new();
        p.put_string("operation", &method);
        p.put_string("source", &target);
        if method == "DELETE" {
            p.put_string("destination", "");
            p.put_string("overwrite", "T");
            return match write("dav_tree", p) {
                Ok(_) => resp(204, "No Content", "", "text/plain"),
                Err(e) => e,
            };
        }
        let dest = match nn_headers.try_get_string("DESTINATION") {
            Ok(v) => v.trim().to_string(),
            Err(_) => return fail(400, "COPY and MOVE require a Destination header"),
        };
        if dest.contains('?') || dest.contains('#') { return fail(400, "Destination query strings and fragments are not supported"); }
        let dpath = match dest.split_once("://") {
            Some((_, rem)) => {
                let (authority, path) = match rem.find('/') { Some(i) => (&rem[..i], &rem[i..]), None => (rem, "/") };
                let host = nn_headers.try_get_string("HOST").unwrap_or_default();
                if !host.is_empty() && !authority.eq_ignore_ascii_case(host.trim()) {
                    return fail(403, "Destination URI authority does not match this DAV server");
                }
                path.to_string()
            },
            None => dest.clone(),
        };
        let drel = match dpath.strip_prefix(&prefix) {
            Some(s) if s.starts_with('/') => match decode_path(&s[1..]) {
                Ok(d) => d.trim_end_matches('/').to_string(),
                Err(message) => return fail(400, &message),
            },
            _ => return fail(403, "Destination is outside this peer's mount"),
        };
        let overwrite = nn_headers.try_get_string("OVERWRITE").unwrap_or("T".to_string());
        if overwrite != "T" && overwrite != "F" { return fail(400, "Overwrite must be exactly T or F"); }
        p.put_string("destination", &drel);
        p.put_string("overwrite", &overwrite);
        match write("dav_tree", p) {
            Ok(d) => if d.has("overwritten") && d.get_property("overwritten").is_boolean() && d.get_boolean("overwritten") {
                resp(204, "No Content", "", "text/plain")
            } else { resp(201, "Created", "", "text/plain") },
            Err(e) => e,
        }
    },
    "PUT" => {
        if target.is_empty() { return fail(405, "PUT requires a file path"); }
        let existed = match propfind("0", String::new()) {
            Ok(xml) => { if xml.contains("collection/>") { return fail(405, "PUT target is a collection"); } true },
            Err(e) => { if e.get_int("code") != 404 { return e; } false },
        };
        if let Some(i) = target.rfind('/') {
            match pf(&target[..i], "0", String::new()) {
                Ok(xml) => if !xml.contains("collection/>") { return fail(409, "PUT parent is not a collection"); },
                Err(e) => return if e.get_int("code") == 404 { fail(409, "PUT parent collection does not exist") } else { e },
            }
        }
        let done = |r: Result<DataObject, DataObject>| match r {
            Ok(_) => if existed { resp(204, "No Content", "", "text/plain") } else { resp(201, "Created", "", "text/plain") },
            Err(e) => e,
        };
        if uuid == "local" {
            // Loopback for the test harness: this instance's own share.
            let d = put_file(target.clone(), nn_body.clone(), DataObject::new());
            let bad = d.has("status") && d.get_property("status").is_string() && d.get_string("status") == "err";
            return done(if bad {
                let code = if d.has("code") && d.get_property("code").is_int() { d.get_int("code") } else { 500 };
                Err(fail(code, &err_text(&d)))
            } else { Ok(d) });
        }
        let data: Vec<u8> = if nn_body.is_bytes() { nn_body.bytes().get_data() }
            else if nn_body.is_string() { nn_body.string().into_bytes() }
            else { Vec::new() };
        let mut p = DataObject::new();
        p.put_string("path", &target);
        if data.is_empty() {
            p.put_string("content", "");
            return done(write("put_file", p));
        }
        // Open a stream to the peer, pump the body into it, and have the peer's put_file join it.
        let u = uuid.to_string();
        let id = catch_unwind(AssertUnwindSafe(|| crate::api::peer::service::new_stream(u))).unwrap_or(-1);
        if id == -1 { return fail(504, "No live route to the peer"); }
        let mut content = DataObject::new();
        content.put_int("stream", id);
        content.put_int("size", data.len() as i64);
        p.put_object("content", content);
        let close = |u: String| { let _ = catch_unwind(AssertUnwindSafe(|| crate::api::peer::service::close_stream(u, id, true))); };
        let u = uuid.to_string();
        let pump = std::thread::spawn(move || {
            for chunk in data.chunks(16000) {
                let (u2, b) = (u.clone(), DataBytes::from_bytes(&chunk.to_vec()));
                let sent = catch_unwind(AssertUnwindSafe(|| crate::api::peer::service::stream_write(u2, id, b))).unwrap_or(false);
                if !sent { break; }
            }
            let _ = catch_unwind(AssertUnwindSafe(|| crate::api::peer::service::close_stream(u, id, true)));
        });
        let r = write("put_file", p);
        // A refused or failed put never joined the stream: retire it so the pump stops.
        if r.is_err() { close(uuid.to_string()); }
        let _ = pump.join();
        done(r)
    },
    _ => {
        let mut out = fail(405, &format!("{} is not supported by this DAV endpoint", method));
        let mut headers = out.get_object("headers");
        headers.put_string("Allow", ALLOW);
        out.put_object("headers", headers);
        out
    }
}