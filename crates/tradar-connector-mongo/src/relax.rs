/// The index of the `)` matching the `(` that ended right before `text`
/// starts, tracking `(`/`{`/`[` nesting and skipping over JSON string
/// contents (so a `)`/`}` inside a quoted string can't be mistaken for the
/// real close). `None` if the parens never balance. Needed once chained
/// calls exist: the old code just took everything up to the last `)` in
/// the buffer, which only worked because there was never more than one
/// call to find the end of.
pub(crate) fn find_matching_close_paren(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '{' | '[' => depth += 1,
            ')' if depth == 0 => return Some(i),
            ')' | '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Turns the mongosh-style object literal people actually type -- unquoted
/// keys, single-quoted strings, a trailing comma -- into strict JSON, which
/// is all `serde_json` accepts: `{name: 'ann', tags: ['a', 'b',],}` becomes
/// `{"name": "ann", "tags": ["a", "b"]}`. Strict JSON passes through
/// unchanged, so this is safe to apply to every argument.
///
/// Also understands the handful of shell constructors people paste from
/// `mongosh` output -- `ObjectId("..")`, `ISODate("..")`, `NumberLong(..)`,
/// `NumberInt(..)`, `NumberDecimal("..")` -- as their Extended JSON form
/// (`{"$oid": ".."}`, ...), which is what the BSON conversion downstream
/// reads. This is also what makes the driver's own row edit work: the
/// results grid shows an id as `ObjectId("..")` and `edit_sql` writes that
/// same text back into the `updateOne`/`deleteOne` filter.
///
/// Deliberately small: no `//` comments, no dotted unquoted keys (`a.b: 1`
/// is not valid JavaScript either -- quote it).
pub(crate) fn relax_json(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                // A double-quoted string is already JSON: copy it as it is.
                out.push(c);
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
            '\'' => {
                out.push('"');
                i += 1;
                while i < chars.len() {
                    match chars[i] {
                        '\\' if i + 1 < chars.len() => {
                            if chars[i + 1] == '\'' {
                                out.push('\'');
                            } else {
                                out.push('\\');
                                out.push(chars[i + 1]);
                            }
                            i += 2;
                        }
                        '\'' => {
                            i += 1;
                            break;
                        }
                        '"' => {
                            out.push_str("\\\"");
                            i += 1;
                        }
                        other => {
                            out.push(other);
                            i += 1;
                        }
                    }
                }
                out.push('"');
            }
            c if c.is_alphabetic() || c == '_' || c == '$' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
                {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                if let Some((json, end)) = constructor_call(&word, &chars, i) {
                    out.push_str(&json);
                    i = end;
                    continue;
                }
                // A key is a bare word right after `{` or `,` and right
                // before `:` -- `true`/`null`/`$gt`-as-a-value stay as written.
                let prev = out.chars().rev().find(|c| !c.is_whitespace());
                let next = chars[i..].iter().find(|c| !c.is_whitespace());
                if matches!(prev, Some('{') | Some(',')) && next == Some(&':') {
                    out.push('"');
                    out.push_str(&word);
                    out.push('"');
                } else {
                    out.push_str(&word);
                }
            }
            ',' => {
                // A trailing comma before `}`/`]` is not JSON.
                let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
                if !matches!(next, Some('}') | Some(']')) {
                    out.push(',');
                }
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    out
}

/// `ObjectId("..")`-style shell constructors as Extended JSON. `at` is the
/// index just past the constructor's name; returns the replacement and the
/// index just past the closing paren, or `None` when `word` is not one of the
/// known constructors or its call is not a single plain literal.
pub(crate) fn constructor_call(word: &str, chars: &[char], at: usize) -> Option<(String, usize)> {
    let key = match word {
        "ObjectId" => Some("$oid"),
        "ISODate" => Some("$date"),
        "NumberLong" => Some("$numberLong"),
        "NumberDecimal" => Some("$numberDecimal"),
        "NumberInt" => None,
        _ => return None,
    };
    let mut i = at;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if chars.get(i) != Some(&'(') {
        return None;
    }
    let close = chars[i + 1..].iter().position(|&c| c == ')')? + i + 1;
    let inner: String = chars[i + 1..close].iter().collect();
    let inner = inner.trim();
    let literal = match (inner.chars().next(), inner.chars().last()) {
        (Some(q @ ('"' | '\'')), Some(l)) if l == q && inner.len() >= 2 => {
            inner[1..inner.len() - 1].to_string()
        }
        _ if !inner.is_empty()
            && inner
                .chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == '.') =>
        {
            inner.to_string()
        }
        _ => return None,
    };
    let json = match key {
        // NumberInt(5) is just the number 5.
        None => literal,
        Some(k) => format!("{{\"{k}\": {}}}", serde_json::Value::String(literal)),
    };
    Some((json, close + 1))
}

/// Splits `text` (the args of one call, comma-separated JSON values) on
/// top-level commas -- tracking `{}`/`[]` depth and skipping over JSON
/// string contents, the same way `find_matching_close_paren` does, and for
/// the same reason: a `}`/`,` inside a quoted string value (e.g.
/// `{"note": "a, b"}`) isn't a real delimiter, and counting it as one
/// either miscounts the nesting depth or splits one argument into two.
pub(crate) fn split_top_level_args(text: &str) -> anyhow::Result<Vec<&str>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            ',' if depth == 0 => {
                args.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        anyhow::bail!("unbalanced braces in arguments");
    }
    args.push(&text[start..]);
    Ok(args)
}
