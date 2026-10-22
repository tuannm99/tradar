//! Turning what the user typed into a request: headers and the URL relative
//! to the connection's base. Shared by the TUI session and `tradar-server`.

/// Parses `raw` (one `Key: Value` header per line, blank lines ignored)
/// into pairs -- kept as a free function so it's unit-testable without a
/// running client.
pub fn parse_headers(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (key, value) = line.split_once(':')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Resolves what the user typed in the URL field against the connection's
/// base URL. `path` is used as-is when it already names a scheme, or when
/// there's no base to combine it with -- letting a request target something
/// entirely unrelated to the connection's base, same as a real Postman
/// request can.
pub fn resolve_url(base: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") || base.is_empty() {
        return path.to_string();
    }
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}
