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

/// Splits the `insecure` option off an HTTP connection's `target`:
/// `https://host/base?insecure=1` -> (`https://host/base`, `true`). The value
/// `1`, `true`, `yes` or nothing at all turns it on, `0`/`false`/`no` off. Any
/// other query parameter stays in the base URL untouched.
pub fn split_insecure(target: &str) -> (String, bool) {
    let Some((base, query)) = target.split_once('?') else {
        return (target.to_string(), false);
    };
    let mut insecure = false;
    let mut rest = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        match pair.split_once('=').unwrap_or((pair, "")) {
            ("insecure", value) => {
                insecure = !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "no");
            }
            _ => rest.push(pair),
        }
    }
    if rest.is_empty() {
        (base.to_string(), insecure)
    } else {
        (format!("{base}?{}", rest.join("&")), insecure)
    }
}

#[cfg(test)]
mod insecure_tests {
    use super::*;

    #[test]
    fn insecure_is_taken_off_the_target() {
        assert_eq!(
            split_insecure("https://h.io/api?insecure=1"),
            ("https://h.io/api".to_string(), true)
        );
        assert_eq!(
            split_insecure("https://h.io?insecure"),
            ("https://h.io".to_string(), true)
        );
        assert_eq!(
            split_insecure("https://h.io?a=1&insecure=true&b=2"),
            ("https://h.io?a=1&b=2".to_string(), true)
        );
        assert_eq!(
            split_insecure("https://h.io?insecure=0"),
            ("https://h.io".to_string(), false)
        );
        assert_eq!(
            split_insecure("https://h.io/api"),
            ("https://h.io/api".to_string(), false)
        );
    }
}
