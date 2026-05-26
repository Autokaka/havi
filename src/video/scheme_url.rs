use std::collections::HashMap;

pub struct Query {
    pub path: String,
    pub params: HashMap<String, String>,
}

pub fn parse_url(url: &str, scheme: &str) -> Option<Query> {
    let prefix = format!("{scheme}://");
    let rest = url.strip_prefix(&prefix)?;
    let (path, qs) = rest.split_once('?').unwrap_or((rest, ""));
    let mut params = HashMap::new();
    for kv in qs.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            params.insert(k.to_string(), percent_decode(v));
        }
    }
    Some(Query { path: path.trim_end_matches('/').to_string(), params })
}

pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') && !s.contains('+') { return s.to_string(); }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push(u8::try_from(hi * 16 + lo).unwrap_or(0));
                i += 3;
                continue;
            }
        }
        out.push(if b == b'+' { b' ' } else { b });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
