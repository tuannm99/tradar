/// Kibana Dev Tools lets a request carry comments -- `# line` anywhere,
/// `// line` and `/* block */` inside the body -- and a commented-out clause
/// is the most ordinary thing to leave in a query buffer, so they are
/// dropped before the request is read (a body with one would otherwise fail
/// the local JSON check). `#` only counts at the start of a line (a JSON
/// string cannot span lines, so no body line legitimately starts with it);
/// `//` and `/* */` are only looked for in the body, string-aware, so a
/// path or a `"http://..."` value is never mistaken for one.
pub(crate) fn strip_comments(query: &str) -> String {
    let query: String = query
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let query = query.trim_start();
    let (request_line, body) = match query.split_once('\n') {
        Some((first, rest)) => (first, rest),
        None => return query.to_string(),
    };
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '"' => {
                out.push('"');
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        out.push(chars[i]);
                    } else if chars[i] == '"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                i = (i + 2).min(chars.len());
                out.push(' ');
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    format!("{request_line}\n{out}")
}

pub(crate) fn parse_query(query: &str) -> Option<(String, String, Option<String>)> {
    let query = strip_comments(query);
    let mut lines = query.lines();
    let first = lines.next()?.trim();
    let mut parts = first.splitn(2, char::is_whitespace);
    let method = parts.next()?.to_string();
    let path = parts.next()?.trim().to_string();
    if method.is_empty() || path.is_empty() {
        return None;
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    let body = body.trim();
    let body = if body.is_empty() {
        None
    } else {
        Some(body.to_string())
    };
    Some((method, path, body))
}

/// `Ok` when `body` is valid either as one JSON value, or as NDJSON -- one
/// JSON value per non-blank line, no separating commas -- the shape
/// `_bulk` and `_msearch` require instead of a single document. Checked as
/// a whole first (the common case, and what gives the clean single-value
/// parse error when it's actually meant to be one object); only re-checked
/// line by line if that fails, so an ordinary malformed single-object body
/// still reports the same error it always did rather than a confusing
/// per-line one.
pub(crate) fn validate_json_body(body: &str) -> Result<(), String> {
    if serde_json::from_str::<serde_json::Value>(body).is_ok() {
        return Ok(());
    }
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Err(err) = serde_json::from_str::<serde_json::Value>(line) {
            return Err(format!("invalid JSON body: {err}"));
        }
    }
    Ok(())
}

/// Escapes a string for safe interpolation inside a *single-quoted* shell
/// argument, using the standard close-quote / escaped-literal-quote /
/// reopen-quote technique: every `'` becomes `'\''`. Nothing else is special
/// inside single quotes, so this is sufficient on its own.
pub(crate) fn shell_escape_single_quoted(s: &str) -> String {
    s.replace('\'', r"'\''")
}

pub(crate) fn to_curl(base_url: &str, query: &str) -> Option<String> {
    let (method, path, body) = parse_query(query)?;
    let base_url = base_url.trim_end_matches('/');
    let url = format!("{base_url}/{}", path.trim_start_matches('/'));
    let url = shell_escape_single_quoted(&url);
    let method = method.to_uppercase();
    Some(match body {
        Some(body) => {
            let body = shell_escape_single_quoted(&body);
            format!("curl -X {method} '{url}' -H 'Content-Type: application/json' -d '{body}'")
        }
        None => format!("curl -X {method} '{url}'"),
    })
}

/// Whether a line opens a new request: an HTTP verb followed by a path,
/// which is what separates one console request from the next.
pub(crate) fn starts_request(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let Some(verb) = words.next() else {
        return false;
    };
    matches!(
        verb.to_ascii_uppercase().as_str(),
        "GET" | "POST" | "PUT" | "DELETE" | "HEAD" | "PATCH"
    ) && words.next().is_some()
}
