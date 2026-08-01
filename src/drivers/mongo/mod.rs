//! MongoDB driver: a minimal shell-subset parser for the literal shape
//! `db.<collection>.<method>(<json-args>)`, not a real JS engine. Anything
//! outside that shape — chained methods, `$where`, arbitrary expressions —
//! is rejected with a clear error rather than guessed at.

pub struct ParsedQuery {
    pub collection: String,
    pub method: String,
    pub args: Vec<serde_json::Value>,
}

pub fn parse_shell_query(query: &str) -> anyhow::Result<ParsedQuery> {
    let query = query.trim();
    let rest = query
        .strip_prefix("db.")
        .ok_or_else(|| anyhow::anyhow!("expected a query starting with \"db.<collection>.<method>(...)\""))?;

    let dot = rest
        .find('.')
        .ok_or_else(|| anyhow::anyhow!("missing collection name"))?;
    let collection = rest[..dot].to_string();
    let rest = &rest[dot + 1..];

    let paren = rest.find('(').ok_or_else(|| anyhow::anyhow!("missing method call"))?;
    let method = rest[..paren].to_string();
    let rest = rest[paren + 1..].trim_end();
    let args_text = rest
        .strip_suffix(')')
        .ok_or_else(|| anyhow::anyhow!("missing closing parenthesis"))?;

    let args = split_top_level_args(args_text)?
        .into_iter()
        .map(|arg| serde_json::from_str(arg.trim()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("invalid JSON argument: {e}"))?;

    Ok(ParsedQuery { collection, method, args })
}

fn split_top_level_args(text: &str) -> anyhow::Result<Vec<&str>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        match c {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_find_with_a_filter() {
        let parsed = parse_shell_query(r#"db.users.find({"active": true})"#).unwrap();

        assert_eq!(parsed.collection, "users");
        assert_eq!(parsed.method, "find");
        assert_eq!(parsed.args, vec![serde_json::json!({"active": true})]);
    }

    #[test]
    fn parses_multiple_top_level_arguments() {
        let parsed =
            parse_shell_query(r#"db.users.updateOne({"_id": 1}, {"$set": {"name": "Ada"}})"#)
                .unwrap();

        assert_eq!(parsed.collection, "users");
        assert_eq!(parsed.method, "updateOne");
        assert_eq!(
            parsed.args,
            vec![
                serde_json::json!({"_id": 1}),
                serde_json::json!({"$set": {"name": "Ada"}})
            ]
        );
    }

    #[test]
    fn parses_a_method_call_with_no_arguments() {
        let parsed = parse_shell_query("db.users.find()").unwrap();

        assert_eq!(parsed.args, Vec::<serde_json::Value>::new());
    }

    #[test]
    fn rejects_input_that_does_not_start_with_db() {
        assert!(parse_shell_query("users.find({})").is_err());
    }

    #[test]
    fn rejects_malformed_json_arguments() {
        assert!(parse_shell_query("db.users.find({not json})").is_err());
    }
}
