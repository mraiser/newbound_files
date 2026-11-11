const MAX_PROPFIND_REQUEST: usize = 64 * 1024;

fn xml_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

// href escaping is per segment: slashes stay separators, while URI/XML
// delimiters and every non-ASCII UTF-8 byte are percent-encoded.
fn href_segment(value: &str) -> String {
    let mut out = String::new();
    for b in value.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(*b, b'-' | b'.' | b'_' | b'~') {
            out.push(*b as char);
        }
        else { out.push_str(&format!("%{:02X}", b)); }
    }
    out
}

fn resource_href(prefix: &str, relative: &str, isdir: bool) -> String {
    let mut out = prefix.trim_end_matches('/').to_string();
    for segment in relative.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        out.push_str(&href_segment(segment));
    }
    if out.is_empty() { out.push('/'); }
    if isdir && !out.ends_with('/') { out.push('/'); }
    out
}

fn rfc1123(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let wd = (days + 4).rem_euclid(7);
    let names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        names[wd as usize], d, months[(m - 1) as usize], y, h, mi, s)
}

fn local_name(name: &str) -> String {
    let name = name.trim();
    name.rsplit(|c| c == ':' || c == '}').next().unwrap_or(name).trim().to_string()
}

fn parse_tag(tag: &str) -> Result<(String, String, bool, bool), String> {
    let mut t = tag.trim();
    let closing = t.starts_with('/');
    if closing { t = t[1..].trim_start(); }
    let self_closing = t.ends_with('/');
    if self_closing { t = t[..t.len() - 1].trim_end(); }
    if t.is_empty() || closing && self_closing { return Err("Malformed XML tag".to_string()); }
    let mut name_end = 0;
    for (i, c) in t.char_indices() {
        if c.is_whitespace() { break; }
        if !c.is_alphanumeric() && !matches!(c, ':' | '_' | '-' | '.' | '}') {
            return Err("Invalid XML element name".to_string());
        }
        name_end = i + c.len_utf8();
    }
    if name_end == 0 { return Err("Missing XML element name".to_string()); }
    let name = t[..name_end].to_string();
    let attrs = t[name_end..].trim();
    if attrs.contains('<') || attrs.contains('>') { return Err("Malformed XML attributes".to_string()); }
    Ok((name, attrs.to_string(), closing, self_closing))
}

fn namespace(attrs: &str, prefix: &str) -> Option<String> {
    let needle = if prefix.is_empty() { "xmlns".to_string() } else { format!("xmlns:{}", prefix) };
    for part in attrs.split_whitespace() {
        if let Some(value) = part.strip_prefix(&(needle.clone() + "=")) {
            if value.len() >= 2 {
                let quote = value.as_bytes()[0] as char;
                if (quote == '"' || quote == '\'') && value.ends_with(quote) {
                    return Some(value[1..value.len()-1].to_string());
                }
            }
        }
    }
    None
}

#[derive(Clone)]
struct Scope {
    default: Option<String>,
    prefixes: Vec<(String, String)>,
}

fn child_scope(parent: &Scope, attrs: &str) -> Scope {
    let mut child = parent.clone();
    if let Some(ns) = namespace(attrs, "") { child.default = Some(ns); }
    for part in attrs.split_whitespace() {
        if let Some(rest) = part.strip_prefix("xmlns:") {
            if let Some((prefix, value)) = rest.split_once('=') {
                if value.len() >= 2 && (value.starts_with('"') || value.starts_with('\'')) {
                    let ns = value[1..value.len()-1].to_string();
                    child.prefixes.retain(|(p, _)| p != prefix);
                    child.prefixes.push((prefix.to_string(), ns));
                }
            }
        }
    }
    child
}

fn element_namespace(name: &str, scope: &Scope) -> Option<String> {
    if let Some((prefix, _)) = name.split_once(':') {
        scope.prefixes.iter().find(|(p, _)| p == prefix).map(|(_, ns)| ns.clone())
    }
    else { scope.default.clone() }
}

#[derive(Clone)]
struct PropRequest {
    namespace: Option<String>,
    name: String,
    qname: String,
}

#[derive(Clone, Copy, PartialEq)]
enum PropMode { All, Names, Selected }

fn decode_entities(text: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').ok_or_else(|| "Unterminated XML entity".to_string())?;
        let entity = &rest[1..end];
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if entity.starts_with("#x") => {
                let n = u32::from_str_radix(&entity[2..], 16).map_err(|_| "Invalid XML character reference".to_string())?;
                out.push(char::from_u32(n).ok_or_else(|| "Invalid XML character reference".to_string())?);
            },
            _ if entity.starts_with('#') => {
                let n = entity[1..].parse::<u32>().map_err(|_| "Invalid XML character reference".to_string())?;
                out.push(char::from_u32(n).ok_or_else(|| "Invalid XML character reference".to_string())?);
            },
            _ => return Err("Only predefined XML entities are supported".to_string()),
        }
        rest = &rest[end+1..];
    }
    out.push_str(rest);
    Ok(out)
}

// Parse the bounded request grammar without DTD/entity expansion. A property's
// prefix matters only through its actual namespace binding, not its spelling.
fn parse_request(input: &str) -> Result<(PropMode, Vec<PropRequest>), String> {
    if input.trim().is_empty() { return Ok((PropMode::All, Vec::new())); }
    if input.len() > MAX_PROPFIND_REQUEST { return Err("PROPFIND request exceeds 64 KiB".to_string()); }
    let mut pos = 0usize;
    let mut stack: Vec<(String, Scope)> = Vec::new();
    let mut root_seen = false;
    let mut root_closed = false;
    let mut mode: Option<PropMode> = None;
    let mut requests: Vec<PropRequest> = Vec::new();
    while pos < input.len() {
        if input[pos..].starts_with("<?") {
            if root_seen { return Err("Misplaced XML processing instruction".to_string()); }
            let end = input[pos+2..].find("?>").ok_or_else(|| "Malformed XML declaration".to_string())? + pos + 2;
            pos = end + 2;
            continue;
        }
        if input[pos..].starts_with("<!--") {
            let end = input[pos+4..].find("-->").ok_or_else(|| "Malformed XML comment".to_string())? + pos + 4;
            pos = end + 3;
            continue;
        }
        if input[pos..].starts_with('<') {
            let end = input[pos+1..].find('>').ok_or_else(|| "Unterminated XML tag".to_string())? + pos + 1;
            let (name, attrs, closing, self_closing) = parse_tag(&input[pos+1..end])?;
            pos = end + 1;
            if closing {
                let (open, _) = stack.pop().ok_or_else(|| "Unexpected XML closing tag".to_string())?;
                if open != name { return Err("Mismatched XML closing tag".to_string()); }
                if stack.is_empty() { root_closed = true; }
                continue;
            }
            if root_closed { return Err("More than one XML root element".to_string()); }
            let parent_scope = stack.last().map(|(_, s)| s.clone()).unwrap_or(Scope { default: None, prefixes: Vec::new() });
            let scope = child_scope(&parent_scope, &attrs);
            let depth = stack.len();
            let local = local_name(&name);
            if depth == 0 {
                if root_seen || local != "propfind" || element_namespace(&name, &scope).as_deref() != Some("DAV:") {
                    return Err("Expected a DAV:propfind document".to_string());
                }
                root_seen = true;
            }
            else if depth == 1 {
                let selected = match local.as_str() {
                    "allprop" => Some(PropMode::All),
                    "propname" => Some(PropMode::Names),
                    "prop" => Some(PropMode::Selected),
                    "include" => None,
                    _ => return Err("Unsupported PROPFIND request element".to_string()),
                };
                if let Some(selected) = selected {
                    if mode.is_some() { return Err("Conflicting or repeated PROPFIND request modes".to_string()); }
                    mode = Some(selected);
                }
                else if mode != Some(PropMode::All) { return Err("DAV:include requires an allprop request".to_string()); }
            }
            else if depth == 2 && mode == Some(PropMode::Selected) && local_name(&stack[1].0) == "prop" {
                if self_closing {
                    requests.push(PropRequest { namespace: element_namespace(&name, &scope), name: local.clone(), qname: name.clone() });
                }
                else { return Err("Requested property elements must be empty".to_string()); }
            }
            else if stack.iter().any(|(n, _)| local_name(n) == "include") {
                // allprop/include extension names are ignored by this store.
            }
            else { return Err("Nested elements are not valid in this PROPFIND request".to_string()); }
            if !self_closing { stack.push((name, scope)); }
            else if depth == 0 { root_closed = true; }
            continue;
        }
        let end = input[pos..].find('<').map(|n| pos+n).unwrap_or(input.len());
        let text = decode_entities(&input[pos..end])?;
        if !text.trim().is_empty() && !stack.iter().any(|(n, _)| local_name(n) == "include") {
            return Err("Unexpected text in PROPFIND request".to_string());
        }
        pos = end;
    }
    if !stack.is_empty() || !root_seen || !root_closed { return Err("Incomplete PROPFIND XML document".to_string()); }
    let mode = mode.unwrap_or(PropMode::All);
    if mode == PropMode::Selected && requests.is_empty() { return Err("DAV:prop must request at least one property".to_string()); }
    Ok((mode, requests))
}

#[derive(Clone)]
struct Resource {
    relative: String,
    name: String,
    isdir: bool,
    size: i64,
    mtime: i64,
    full: String,
}

fn metadata_ms(md: &std::fs::Metadata) -> i64 {
    md.modified().ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn render_property(resource: &Resource, request: &PropRequest, value_only: bool) -> (bool, String) {
    let name = request.name.as_str();
    let dav = request.namespace.as_deref().unwrap_or("DAV:") == "DAV:";
    let tag = if dav {
        format!("D:{}", name)
    }
    else if let Some(ns) = &request.namespace {
        // A request-scoped prefix is not valid in the independent response.
        // Use the local name with its expanded namespace on this element.
        format!("{} xmlns=\"{}\"", local_name(&request.qname), xml_text(ns))
    }
    else { local_name(&request.qname) };
    if value_only && dav && matches!(name, "displayname" | "resourcetype" | "getcontentlength" | "getlastmodified" | "getcontenttype") {
        return (true, format!("<{}/>", tag));
    }
    let rendered = if dav {
        match name {
            "displayname" => Some(format!("<{}>{}</{}>", tag, xml_text(if resource.isdir && resource.relative.is_empty() { "/" } else { &resource.name }), tag)),
            "resourcetype" => Some(if resource.isdir { format!("<{}><D:collection/></{}>", tag, tag) } else { format!("<{}/>", tag) }),
            "getcontentlength" => Some(format!("<{}>{}</{}>", tag, if resource.isdir { 0 } else { resource.size }, tag)),
            "getlastmodified" => Some(format!("<{}>{}</{}>", tag, rfc1123(resource.mtime), tag)),
            "getcontenttype" => {
                let content_type = if resource.isdir { "httpd/unix-directory".to_string() } else { mime_type(resource.full.clone()) };
                Some(format!("<{}>{}</{}>", tag, xml_text(&content_type), tag))
            },
            _ => None,
        }
    }
    else { None };
    if let Some(rendered) = rendered { (true, rendered) }
    else { (false, tag) }
}

fn dav_property(name: &str) -> PropRequest {
    PropRequest { namespace: Some("DAV:".to_string()), name: name.to_string(), qname: format!("D:{}", name) }
}

fn append_response(xml: &mut String, prefix: &str, resource: &Resource, mode: PropMode, requested: &[PropRequest]) {
    xml.push_str("  <D:response>\n");
    xml.push_str(&format!("    <D:href>{}</D:href>\n", xml_text(&resource_href(prefix, &resource.relative, resource.isdir))));
    let props: Vec<PropRequest> = if mode == PropMode::Selected {
        requested.to_vec()
    }
    else {
        vec![dav_property("displayname"), dav_property("resourcetype"), dav_property("getcontentlength"), dav_property("getlastmodified"), dav_property("getcontenttype")]
    };
    let mut found = String::new();
    let mut missing = String::new();
    for prop in props {
        let (ok, rendered) = render_property(resource, &prop, mode == PropMode::Names);
        if ok {
            found.push_str("        ");
            found.push_str(&rendered);
            found.push('\n');
        }
        else { missing.push_str(&format!("        <{} />\n", rendered)); }
    }
    if !found.is_empty() {
        xml.push_str("    <D:propstat>\n      <D:prop>\n");
        xml.push_str(&found);
        xml.push_str("      </D:prop>\n      <D:status>HTTP/1.1 200 OK</D:status>\n    </D:propstat>\n");
    }
    if !missing.is_empty() {
        xml.push_str("    <D:propstat>\n      <D:prop>\n");
        xml.push_str(&missing);
        xml.push_str("      </D:prop>\n      <D:status>HTTP/1.1 404 Not Found</D:status>\n    </D:propstat>\n");
    }
    xml.push_str("  </D:response>\n");
}

let resolved = resolve_path(path.clone());
if resolved.get_string("status") == "err" { panic!("403 Forbidden: {}", resolved.get_string("msg")); }
if !resolved.get_boolean("exists") { panic!("404 Not Found: DAV resource does not exist"); }
if depth != "0" && depth != "1" { panic!("501 Not Implemented: Only Depth: 0 and Depth: 1 are supported"); }
let (mode, requested) = match parse_request(&request_xml) {
    Ok(v) => v,
    Err(e) => panic!("400 Bad Request: {}", e),
};
let relative = resolved.get_string("relative");
let full = resolved.get_string("path");
let md = std::fs::metadata(&full).unwrap();
let isdir = md.is_dir();
let own = Resource {
    name: if relative.is_empty() { "/".to_string() } else { relative.rsplit('/').next().unwrap_or(&relative).to_string() },
    relative: relative.clone(),
    isdir,
    size: md.len() as i64,
    mtime: metadata_ms(&md),
    full: full.clone(),
};
let mut resources = vec![own];
if isdir && depth == "1" {
    let listing = list_dir(relative.clone());
    if listing.has("status") && listing.get_string("status") == "err" { panic!("500 Internal Server Error: {}", listing.get_string("msg")); }
    let entries = listing.get_array("entries");
    for i in 0..entries.len() {
        let e = entries.get_object(i);
        resources.push(Resource {
            relative: e.get_string("path"),
            name: e.get_string("name"),
            isdir: e.get_boolean("isdir"),
            size: e.get_int("size"),
            mtime: e.get_int("mtime"),
            full: Path::new(&full).join(e.get_string("name")).to_string_lossy().to_string(),
        });
    }
}
let mut prefix = href_prefix.trim().to_string();
if prefix.is_empty() { prefix = "/files/dav".to_string(); }
if !prefix.starts_with('/') { prefix = format!("/{}", prefix); }
let prefix = prefix.trim_end_matches('/').to_string();
let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">\n");
for resource in resources { append_response(&mut xml, &prefix, &resource, mode, &requested); }
xml.push_str("</D:multistatus>\n");
xml