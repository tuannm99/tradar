/// A command line as `redis-cli` reads it: arguments split on whitespace,
/// except that a token *starting* with `"` or `'` runs to the matching quote
/// and may contain spaces -- `SET greeting "hello world"`. Inside `"..."` the
/// escapes `\n \r \t \b \a \\ \" \xHH` are understood (anything else after a
/// backslash is that character); inside `'...'` only `\'` is. A quote in the
/// middle of an unquoted token is just a character, so `SET k it's` keeps
/// working, and an unterminated quote is an error rather than a silent
/// mis-split.
pub(crate) fn split_command(text: &str) -> anyhow::Result<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let mut token = String::new();
        match chars[i] {
            quote @ ('"' | '\'') => {
                i += 1;
                let mut closed = false;
                while i < chars.len() {
                    let c = chars[i];
                    if c == quote {
                        closed = true;
                        i += 1;
                        break;
                    }
                    if c == '\\' && i + 1 < chars.len() {
                        i += 1;
                        let e = chars[i];
                        if quote == '\'' {
                            if e == '\'' {
                                token.push('\'');
                            } else {
                                token.push('\\');
                                token.push(e);
                            }
                        } else {
                            match e {
                                'n' => token.push('\n'),
                                'r' => token.push('\r'),
                                't' => token.push('\t'),
                                'b' => token.push('\u{8}'),
                                'a' => token.push('\u{7}'),
                                'x' if i + 2 < chars.len()
                                    && chars[i + 1].is_ascii_hexdigit()
                                    && chars[i + 2].is_ascii_hexdigit() =>
                                {
                                    let hex: String = chars[i + 1..=i + 2].iter().collect();
                                    token.push(u8::from_str_radix(&hex, 16)? as char);
                                    i += 2;
                                }
                                other => token.push(other),
                            }
                        }
                        i += 1;
                        continue;
                    }
                    token.push(c);
                    i += 1;
                }
                if !closed {
                    anyhow::bail!("unterminated {quote} quote in the command");
                }
                if i < chars.len() && !chars[i].is_whitespace() {
                    anyhow::bail!(
                        "a closing quote must be followed by a space or the end of the command"
                    );
                }
            }
            _ => {
                while i < chars.len() && !chars[i].is_whitespace() {
                    token.push(chars[i]);
                    i += 1;
                }
            }
        }
        out.push(token);
    }
    Ok(out)
}

/// `arg` as one token of a command line: bare when it is plain, otherwise
/// double-quoted with the escapes `split_command` reads back -- a key with
/// a space in it must survive being written into a snippet.
pub(crate) fn quote_arg(arg: &str) -> String {
    if !arg.is_empty()
        && !arg
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
    {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
