use crate::relax::*;

/// One `.method(args)` call in a chain -- `db.users.find({}).sort({name: 1})`
/// parses to two of these, `find` and `sort`.
#[derive(Debug)]
pub(crate) struct MethodCall {
    pub(crate) name: String,
    pub(crate) args: Vec<serde_json::Value>,
}

#[derive(Debug)]
pub(crate) struct ParsedQuery {
    pub(crate) collection: String,
    /// Always at least one call -- the first is the primary operation
    /// (`find`, `insertOne`, ...), any further ones are chained cursor
    /// calls. Which chain calls a given primary accepts (if any) is
    /// decided in `run_method`, not here -- this only knows shell syntax,
    /// not Mongo semantics.
    pub(crate) calls: Vec<MethodCall>,
}

impl ParsedQuery {
    pub(crate) fn primary(&self) -> &MethodCall {
        &self.calls[0]
    }

    pub(crate) fn chain(&self) -> &[MethodCall] {
        &self.calls[1..]
    }
}

/// Whether `line` begins a new shell statement -- a `db.<collection>...`
/// call or one of the two shell helpers `execute` special-cases (`use`,
/// `show dbs`/`show databases`) -- versus continuing the call above it
/// (a line that's part of a multi-line JSON argument). Mirrors
/// `tradar-connector-elasticsearch`'s `starts_request`; see
/// `split_statements`.
pub(crate) fn starts_mongo_statement(line: &str) -> bool {
    line == "db"
        || line.starts_with("db.")
        || line.starts_with("use ")
        || line == "show"
        || line.starts_with("show ")
}

/// `query` without its `// line` and `/* block */` comments -- mongosh
/// accepts them anywhere inside the call, and a commented-out filter line is
/// the most ordinary thing to leave in a query buffer. String contents
/// (`"http://x"`, `'it\'s'`) are copied as they are, and a comment becomes
/// nothing but whitespace so the line structure survives.
pub(crate) fn strip_js_comments(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            quote @ ('"' | '\'') => {
                out.push(quote);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        out.push(chars[i]);
                    } else if chars[i] == quote {
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
                    // Keep newlines so error positions still point at the right line.
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
    out
}

/// `text` as the shell would take it: comments gone, surrounding space gone,
/// and trailing `;` dropped (mongosh accepts `db.users.find();`).
pub(crate) fn clean_command(text: &str) -> String {
    strip_js_comments(text)
        .trim()
        .trim_end_matches(|c: char| c == ';' || c.is_whitespace())
        .to_string()
}

/// Everything one line of the Mongo console can be: the shell helpers
/// (`use`, `show ...`, bare `db`), a database-level call
/// (`db.getCollectionNames()`, `db.runCommand({...})`), or the usual
/// `db.<collection>.<method>(...)` query.
#[derive(Debug)]
pub(crate) enum ShellCommand {
    Use(String),
    ShowDbs,
    ShowCollections,
    ShowUsers,
    ShowRoles,
    ShowProfile,
    DbName,
    DbCall(MethodCall),
    Query(ParsedQuery),
}

/// The database-level methods supported -- anything else after `db.` and
/// before a `(` is a collection name.
pub(crate) const DB_METHODS: &[&str] = &[
    "getName",
    "getCollectionNames",
    "getCollectionInfos",
    "stats",
    "version",
    "serverStatus",
    "runCommand",
    "adminCommand",
];

/// `db.getSiblingDB("other").rest` -> (`Some("other")`, `db.rest`): the same
/// command aimed at another database for this one statement, without the
/// `use` that would switch the session. A plain command comes back as
/// `(None, text)`.
pub(crate) fn split_sibling_db(text: &str) -> anyhow::Result<(Option<String>, String)> {
    let cleaned = clean_command(text);
    let Some(after) = cleaned.strip_prefix("db.getSiblingDB(") else {
        return Ok((None, cleaned));
    };
    let close = find_matching_close_paren(after)
        .ok_or_else(|| anyhow::anyhow!("missing closing parenthesis"))?;
    let name = after[..close].trim().trim_matches(['"', '\'']).to_string();
    if name.is_empty() {
        anyhow::bail!("getSiblingDB needs a database name");
    }
    let tail = after[close + 1..].trim_start();
    if !tail.starts_with('.') {
        anyhow::bail!("expected a call after getSiblingDB(...), e.g. .getCollectionNames()");
    }
    Ok((Some(name), format!("db{tail}")))
}

pub(crate) fn parse_shell_command(text: &str) -> anyhow::Result<ShellCommand> {
    let text = clean_command(text);
    if let Some(rest) = text.strip_prefix("use ") {
        let name = rest.trim().trim_matches(['"', '\'']);
        if name.is_empty() {
            anyhow::bail!("use requires a database name");
        }
        return Ok(ShellCommand::Use(name.to_string()));
    }
    if let Some(what) = text.strip_prefix("show ") {
        return match what.trim() {
            "dbs" | "databases" => Ok(ShellCommand::ShowDbs),
            "collections" | "tables" => Ok(ShellCommand::ShowCollections),
            "users" => Ok(ShellCommand::ShowUsers),
            "roles" => Ok(ShellCommand::ShowRoles),
            "profile" => Ok(ShellCommand::ShowProfile),
            other => anyhow::bail!(
                "unsupported `show {other}` (supported: show dbs, show collections, show users, show roles, show profile)"
            ),
        };
    }
    if text == "show" {
        anyhow::bail!("`show` needs an argument: dbs, collections, users, roles or profile");
    }
    if text == "db" {
        return Ok(ShellCommand::DbName);
    }
    if let Some(rest) = text.strip_prefix("db.") {
        let head = rest.split('(').next().unwrap_or("").trim();
        if DB_METHODS.contains(&head) && rest.contains('(') {
            let open = rest.find('(').unwrap_or(0);
            let after_paren = &rest[open + 1..];
            let close = find_matching_close_paren(after_paren)
                .ok_or_else(|| anyhow::anyhow!("missing closing parenthesis"))?;
            if !after_paren[close + 1..].trim().is_empty() {
                anyhow::bail!("unexpected text after db.{head}(...)");
            }
            let args = split_top_level_args(&after_paren[..close])?
                .into_iter()
                .map(|arg| serde_json::from_str(&relax_json(arg.trim())))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow::anyhow!("invalid JSON argument: {e}"))?;
            return Ok(ShellCommand::DbCall(MethodCall {
                name: head.to_string(),
                args,
            }));
        }
    }
    Ok(ShellCommand::Query(parse_shell_query(&text)?))
}

pub(crate) fn parse_shell_query(query: &str) -> anyhow::Result<ParsedQuery> {
    let query = clean_command(query);
    let query = query.as_str();
    let rest = query.strip_prefix("db.").ok_or_else(|| {
        anyhow::anyhow!("expected a query starting with \"db.<collection>.<method>(...)\"")
    })?;

    // `db.getCollection("name").find()` -- how mongosh addresses a collection
    // whose name is not a plain identifier.
    let (collection, mut rest) = if let Some(after) = rest.strip_prefix("getCollection(") {
        let close = find_matching_close_paren(after)
            .ok_or_else(|| anyhow::anyhow!("missing closing parenthesis"))?;
        let name = after[..close].trim().trim_matches(['"', '\'']).to_string();
        if name.is_empty() {
            anyhow::bail!("getCollection needs a collection name");
        }
        let tail = after[close + 1..].trim_start();
        let tail = tail
            .strip_prefix('.')
            .ok_or_else(|| anyhow::anyhow!("expected a method after getCollection(...)"))?;
        (name, tail)
    } else {
        let dot = rest
            .find('.')
            .ok_or_else(|| anyhow::anyhow!("missing collection name"))?;
        (rest[..dot].to_string(), &rest[dot + 1..])
    };

    let mut calls = Vec::new();
    loop {
        let paren = rest
            .find('(')
            .ok_or_else(|| anyhow::anyhow!("missing method call"))?;
        let name = rest[..paren].trim().to_string();
        let after_paren = &rest[paren + 1..];
        let close = find_matching_close_paren(after_paren)
            .ok_or_else(|| anyhow::anyhow!("missing closing parenthesis"))?;
        let args_text = &after_paren[..close];
        let args = split_top_level_args(args_text)?
            .into_iter()
            .map(|arg| serde_json::from_str(&relax_json(arg.trim())))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("invalid JSON argument: {e}"))?;
        calls.push(MethodCall { name, args });

        let after_call = after_paren[close + 1..].trim_start();
        if let Some(next) = after_call.strip_prefix('.') {
            rest = next;
        } else if after_call.is_empty() {
            break;
        } else {
            anyhow::bail!("unexpected text after a method call: {after_call:?}");
        }
    }

    Ok(ParsedQuery { collection, calls })
}
