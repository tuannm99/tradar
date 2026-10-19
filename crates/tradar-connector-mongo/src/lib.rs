//! MongoDB connector: a minimal shell-subset parser for the literal shape
//! `db.<collection>.<method>(<json-args>)` with optional chained cursor
//! calls (`.find(...).sort(...).limit(...).skip(...)`, `.count()`), not a
//! real JS engine. Anything outside that shape — `$where`, arbitrary
//! expressions, a chain method this doesn't recognize — is rejected with a
//! clear error rather than guessed at. Two mongosh shell helpers are
//! special-cased on top of that: `use <db>` switches which database
//! `db.<collection>...` targets, and `show dbs`/`show databases` lists
//! every database on the server.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::TryStreamExt;
use mongodb::bson::{Bson, Document};
use mongodb::options::Hint;

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo, Statement,
    qualify_colliding_names,
};
use tradar_query_workbench::query_engine::QueryEngine;

/// One `.method(args)` call in a chain -- `db.users.find({}).sort({name: 1})`
/// parses to two of these, `find` and `sort`.
#[derive(Debug)]
struct MethodCall {
    name: String,
    args: Vec<serde_json::Value>,
}

#[derive(Debug)]
struct ParsedQuery {
    collection: String,
    /// Always at least one call -- the first is the primary operation
    /// (`find`, `insertOne`, ...), any further ones are chained cursor
    /// calls. Which chain calls a given primary accepts (if any) is
    /// decided in `run_method`, not here -- this only knows shell syntax,
    /// not Mongo semantics.
    calls: Vec<MethodCall>,
}

impl ParsedQuery {
    fn primary(&self) -> &MethodCall {
        &self.calls[0]
    }

    fn chain(&self) -> &[MethodCall] {
        &self.calls[1..]
    }
}

/// Whether `line` begins a new shell statement -- a `db.<collection>...`
/// call or one of the two shell helpers `execute` special-cases (`use`,
/// `show dbs`/`show databases`) -- versus continuing the call above it
/// (a line that's part of a multi-line JSON argument). Mirrors
/// `tradar-connector-elasticsearch`'s `starts_request`; see
/// `split_statements`.
fn starts_mongo_statement(line: &str) -> bool {
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
fn strip_js_comments(input: &str) -> String {
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
fn clean_command(text: &str) -> String {
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
enum ShellCommand {
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
const DB_METHODS: &[&str] = &[
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
fn split_sibling_db(text: &str) -> anyhow::Result<(Option<String>, String)> {
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

fn parse_shell_command(text: &str) -> anyhow::Result<ShellCommand> {
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

fn parse_shell_query(query: &str) -> anyhow::Result<ParsedQuery> {
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

/// The index of the `)` matching the `(` that ended right before `text`
/// starts, tracking `(`/`{`/`[` nesting and skipping over JSON string
/// contents (so a `)`/`}` inside a quoted string can't be mistaken for the
/// real close). `None` if the parens never balance. Needed once chained
/// calls exist: the old code just took everything up to the last `)` in
/// the buffer, which only worked because there was never more than one
/// call to find the end of.
fn find_matching_close_paren(text: &str) -> Option<usize> {
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
fn relax_json(input: &str) -> String {
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
fn constructor_call(word: &str, chars: &[char], at: usize) -> Option<(String, usize)> {
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
fn split_top_level_args(text: &str) -> anyhow::Result<Vec<&str>> {
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

struct MongoDriver {
    uri: String,
    client: Option<mongodb::Client>,
    /// The database `db.<collection>...` currently targets. `None` until
    /// either the connection URI names a default database (picked up in
    /// `connect()`) or a `use <db>` statement runs; `current_db_name()`
    /// falls back to `"test"` in that case, the same default `mongosh`
    /// itself uses when no database was ever selected. A `Mutex` rather
    /// than `&mut self` because `QueryDriver::execute` takes `&self` --
    /// this is the one piece of session state a shell interpreter needs
    /// that a stateless query driver otherwise wouldn't.
    current_db: Mutex<Option<String>>,
}

impl MongoDriver {
    fn new(uri: &str) -> Self {
        Self {
            uri: uri.to_string(),
            client: None,
            current_db: Mutex::new(None),
        }
    }

    fn current_db_name(&self) -> String {
        self.current_db
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "test".to_string())
    }

    /// The console helpers that are not `db.<collection>.<method>()`:
    /// `show collections|users|roles`, bare `db`, and the database-level
    /// calls (`db.getCollectionNames()`, `db.stats()`, `db.runCommand({..})`
    /// ...). Replies are shaped like `mongosh` prints them -- a list is one
    /// array value, a command's reply one document.
    async fn run_db_helper(
        &self,
        command: ShellCommand,
        db: mongodb::Database,
    ) -> anyhow::Result<QueryResult> {
        let one = |value: serde_json::Value| QueryResult::Documents {
            items: vec![value],
            truncated: false,
        };
        let extjson = |doc: Document| {
            let mut value = Bson::Document(doc).into_relaxed_extjson();
            simplify_extjson(&mut value);
            value
        };
        let run = |doc: Document| {
            let db = db.clone();
            async move { db.run_command(doc).await }
        };
        Ok(match command {
            ShellCommand::DbName => one(serde_json::json!(db.name())),
            ShellCommand::ShowCollections => {
                let mut names = db.list_collection_names().await?;
                names.sort_unstable();
                QueryResult::Documents {
                    items: names
                        .into_iter()
                        .map(|name| serde_json::json!({ "name": name }))
                        .collect(),
                    truncated: false,
                }
            }
            ShellCommand::ShowUsers => {
                let reply = run(mongodb::bson::doc! { "usersInfo": 1 }).await?;
                one(extjson(reply))
            }
            ShellCommand::ShowRoles => {
                let reply = run(mongodb::bson::doc! { "rolesInfo": 1 }).await?;
                one(extjson(reply))
            }
            // mongosh lists the latest few slow operations; the profiler
            // writes them to `system.profile` (empty until profiling is on).
            ShellCommand::ShowProfile => {
                let collection = db.collection::<Document>("system.profile");
                let find = MethodCall {
                    name: "find".into(),
                    args: vec![serde_json::json!({})],
                };
                let chain = [
                    MethodCall {
                        name: "sort".into(),
                        args: vec![serde_json::json!({ "ts": -1 })],
                    },
                    MethodCall {
                        name: "limit".into(),
                        args: vec![serde_json::json!(20)],
                    },
                ];
                let mut result = run_method(&collection, &find, &chain).await?;
                if let QueryResult::Documents { items, .. } = &mut result {
                    for doc in items {
                        simplify_extjson(doc);
                    }
                }
                result
            }
            ShellCommand::DbCall(call) => {
                let arg = |i: usize| -> anyhow::Result<Document> {
                    let value = call.args.get(i).ok_or_else(|| {
                        anyhow::anyhow!("db.{}() requires a command document", call.name)
                    })?;
                    json_to_document(value.clone())
                };
                match call.name.as_str() {
                    "getName" => one(serde_json::json!(db.name())),
                    "getCollectionNames" => {
                        let mut names = db.list_collection_names().await?;
                        names.sort_unstable();
                        one(serde_json::json!(names))
                    }
                    "getCollectionInfos" => {
                        let specs: Vec<_> = db.list_collections().await?.try_collect().await?;
                        QueryResult::Documents {
                            items: specs
                                .iter()
                                .map(|spec| serde_json::to_value(spec).unwrap_or_default())
                                .collect(),
                            truncated: false,
                        }
                    }
                    "stats" => one(extjson(run(mongodb::bson::doc! { "dbStats": 1 }).await?)),
                    "serverStatus" => one(extjson(
                        run(mongodb::bson::doc! { "serverStatus": 1 }).await?,
                    )),
                    "version" => {
                        let info = run(mongodb::bson::doc! { "buildInfo": 1 }).await?;
                        one(serde_json::json!(
                            info.get_str("version").unwrap_or("unknown")
                        ))
                    }
                    "runCommand" => one(extjson(run(arg(0)?).await?)),
                    "adminCommand" => {
                        let client = self
                            .client
                            .as_ref()
                            .expect("connect() must be called first");
                        one(extjson(
                            client.database("admin").run_command(arg(0)?).await?,
                        ))
                    }
                    other => anyhow::bail!("unsupported db.{other}()"),
                }
            }
            ShellCommand::Use(_) | ShellCommand::ShowDbs | ShellCommand::Query(_) => {
                unreachable!("handled by `execute`")
            }
        })
    }

    fn database(&self) -> mongodb::Database {
        let client = self
            .client
            .as_ref()
            .expect("connect() must be called first");
        client.database(&self.current_db_name())
    }
}

#[async_trait]
impl QueryDriver for MongoDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = mongodb::Client::with_uri_str(&self.uri).await?;
        // `ping` is a no-data admin command that answers regardless of
        // which database it's run against (or whether that database has
        // ever been written to), so connectivity can be verified even when
        // the URI names no database at all -- unlike the old code, which
        // required a default database just to have somewhere to point this
        // check at.
        client
            .database("admin")
            .run_command(mongodb::bson::doc! { "ping": 1 })
            .await?;
        // A URI that does name one (`mongodb://host/mydb`) still auto-selects
        // it, matching what `mongosh` does when given the same URI.
        if let Some(db) = client.default_database() {
            *self.current_db.lock().unwrap() = Some(db.name().to_string());
        }
        self.client = Some(client);
        Ok(())
    }

    /// The shell shapes this driver actually parses (the primary methods,
    /// the chained cursor calls `find(...)` alone accepts, plus `use`/`show
    /// dbs`) are a closed set -- completing anything else there would
    /// mislead. The `$`-operators are different: they're free-form JSON
    /// keys *inside* a method's filter/update/pipeline argument, so every
    /// one MongoDB itself recognizes is fair to suggest regardless of which
    /// method it ends up nested in.
    fn keywords(&self) -> &'static [&'static str] {
        &[
            "db",
            "use",
            "show",
            "profile",
            "getSiblingDB",
            "dbs",
            "databases",
            "find",
            "findOne",
            "countDocuments",
            "estimatedDocumentCount",
            "distinct",
            "getIndexes",
            "aggregate",
            "insertOne",
            "insertMany",
            "updateOne",
            "updateMany",
            "deleteOne",
            "deleteMany",
            // Chained cursor calls, `find(...)` only.
            "sort",
            "limit",
            "skip",
            "hint",
            "explain",
            "count",
            // Query/filter operators.
            "$eq",
            "$gt",
            "$gte",
            "$lt",
            "$lte",
            "$ne",
            "$in",
            "$nin",
            "$and",
            "$or",
            "$not",
            "$nor",
            "$exists",
            "$regex",
            "$type",
            "$mod",
            "$all",
            "$elemMatch",
            "$size",
            "$text",
            "$search",
            // Update operators.
            "$set",
            "$unset",
            "$inc",
            "$mul",
            "$rename",
            "$min",
            "$max",
            "$currentDate",
            "$setOnInsert",
            "$push",
            "$pull",
            "$pullAll",
            "$pop",
            "$addToSet",
            "$each",
            // Aggregation pipeline stages.
            "$match",
            "$group",
            "$sort",
            "$limit",
            "$skip",
            "$project",
            "$addFields",
            "$lookup",
            "$unwind",
            "$count",
            "$facet",
            "$bucket",
            "$sample",
            "$replaceRoot",
            "$out",
            "$merge",
            "$graphLookup",
            // Aggregation expressions/accumulators.
            "$sum",
            "$avg",
            "$first",
            "$last",
            "$cond",
            "$ifNull",
            "$concat",
            "$toLower",
            "$toUpper",
        ]
    }

    /// A `db.<collection>.<method>(...)` call, or a `use`/`show dbs`
    /// shell helper -- same idiom as `tradar-connector-elasticsearch`'s
    /// `starts_request`, which this mirrors so a call written across
    /// several lines for readability (the normal mongosh/Compass style,
    /// e.g. `db.orders.find({\n  status: "open"\n})`) merges into one
    /// statement instead of each of its lines becoming its own (each
    /// independently invalid, since none of them alone is a complete
    /// call) the way a flat one-statement-per-line split used to treat
    /// it.
    fn split_statements(&self, text: &str) -> Vec<Statement> {
        let mut statements: Vec<Statement> = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim();
            let line_start = offset + (line.len() - line.trim_start().len());
            offset += line.len();
            if trimmed.is_empty() {
                continue;
            }
            match (starts_mongo_statement(trimmed), statements.last_mut()) {
                (false, Some(current)) => {
                    // Continuation of the call above: extend it to here.
                    current.end = line_start + trimmed.len();
                    current.text = text[current.start..current.end].trim_end().to_string();
                }
                _ => statements.push(Statement {
                    text: trimmed.to_string(),
                    start: line_start,
                    end: line_start + trimmed.len(),
                }),
            }
        }
        statements
    }

    async fn ping(&self) -> anyhow::Result<()> {
        self.database()
            .run_command(mongodb::bson::doc! { "ping": 1 })
            .await?;
        Ok(())
    }

    /// A collection has no fixed schema, so this reads one document per
    /// collection and reports the fields it happened to have -- a guess,
    /// not a guarantee, but still a real improvement over an empty list for
    /// autocomplete and for browsing the navigator. One round trip per
    /// collection is the cost of that guess; acceptable against a normal
    /// instance's collection count, same trade-off SQLite already makes
    /// with its own per-table `PRAGMA` round trip -- but unlike SQLite's
    /// (a local file, no network latency to pay N times over), these
    /// round trips run **concurrently** via `join_all` rather than one
    /// after another, so connect time is bounded by the slowest single
    /// sample plus the driver's own connection-pool contention, not by
    /// N times a network round trip. A collection that fails to sample
    /// (empty, or a transient error) just reports no columns rather than
    /// failing schema browsing for every other collection -- same idea one
    /// level up for a database that fails to list its own collections: it
    /// just contributes none, rather than failing every other database's
    /// worth of navigator entries.
    ///
    /// Covers **every** database on the server (`admin`/`local`/`config`
    /// excluded, same reasoning as Postgres skipping `pg_catalog` or
    /// Cassandra skipping its `system*` keyspaces), each grouped under its
    /// own `SchemaInfo::schema` for the navigator's database level --
    /// listing databases and, for each, its collections are both
    /// concurrent round trips (`join_all`, same idiom as the per-collection
    /// sampling below), not the connect-time default database alone the
    /// way this used to work. Still a one-time snapshot fetched at connect
    /// (see `MongoConnector::connect`) with no live-refresh hook, so a
    /// database created afterward doesn't appear until reconnecting -- same
    /// limitation as always, now covering every database instead of one. A
    /// `use <db>` from the editor still only changes what `db.<collection>`
    /// queries target, independent of what the navigator already shows.
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let client = self
            .client
            .as_ref()
            .expect("connect() must be called first");
        let db_names: Vec<String> = client
            .list_databases()
            .await?
            .into_iter()
            .map(|spec| spec.name)
            .filter(|name| !matches!(name.as_str(), "admin" | "local" | "config"))
            .collect();

        let collection_lists = futures_util::future::join_all(db_names.iter().map(|db_name| {
            let db = client.database(db_name);
            async move { db.list_collection_names().await }
        }))
        .await;
        let targets: Vec<(String, String)> = db_names
            .into_iter()
            .zip(collection_lists)
            .flat_map(|(db_name, names)| {
                names
                    .unwrap_or_default()
                    .into_iter()
                    .map(move |name| (db_name.clone(), name))
            })
            .collect();

        let samples = futures_util::future::join_all(targets.iter().map(|(db_name, name)| {
            let collection = client.database(db_name).collection::<Document>(name);
            async move { collection.find_one(Document::new()).await }
        }))
        .await;
        let indexed_fields =
            futures_util::future::join_all(targets.iter().map(|(db_name, name)| {
                let collection = client.database(db_name).collection::<Document>(name);
                async move { indexed_field_names(&collection).await }
            }))
            .await;
        let mut schema: Vec<SchemaInfo> = targets
            .into_iter()
            .zip(samples)
            .zip(indexed_fields)
            .map(|(((db_name, name), sample), indexed_fields)| {
                let mut columns = match sample {
                    Ok(Some(doc)) => {
                        let mut columns = Vec::new();
                        flatten_document("", &doc, &mut columns);
                        columns
                    }
                    Ok(None) | Err(_) => Vec::new(),
                };
                for column in &mut columns {
                    if indexed_fields.contains(&column.name) {
                        column.indexed = true;
                    }
                }
                SchemaInfo {
                    name,
                    columns,
                    kind: None,
                    ttl: None,
                    schema: Some(db_name),
                    // Every Mongo collection is the same kind -- no
                    // views/functions the way Postgres has, so no extra
                    // folder layer under the database (see the roadmap's
                    // note on this connector for the same premise).
                    object_kind: None,
                }
            })
            .collect();

        // Only the rare same-name-in-two-databases collection gets
        // qualified -- see the function's own doc comment.
        qualify_colliding_names(&mut schema);
        Ok(schema)
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let (sibling, command_text) = split_sibling_db(query)?;
        let db = match &sibling {
            Some(name) => self
                .client
                .as_ref()
                .expect("connect() must be called first")
                .database(name),
            None => self.database(),
        };
        let parsed = match parse_shell_command(&command_text)? {
            ShellCommand::Use(name) => {
                *self.current_db.lock().unwrap() = Some(name.clone());
                return Ok(QueryResult::Documents {
                    items: vec![serde_json::json!({ "switched to db": name })],
                    truncated: false,
                });
            }
            ShellCommand::ShowDbs => {
                let client = self
                    .client
                    .as_ref()
                    .expect("connect() must be called first");
                let docs = client
                    .list_databases()
                    .await?
                    .into_iter()
                    .map(|spec| {
                        serde_json::json!({
                            "name": spec.name,
                            "sizeOnDisk": spec.size_on_disk,
                            "empty": spec.empty,
                        })
                    })
                    .collect();
                return Ok(QueryResult::Documents {
                    items: docs,
                    truncated: false,
                });
            }
            ShellCommand::Query(parsed) => parsed,
            helper => return self.run_db_helper(helper, db).await,
        };
        let collection = db.collection::<Document>(&parsed.collection);
        let mut result =
            run_method_in(Some(&db), &collection, parsed.primary(), parsed.chain()).await?;
        // Every arm of `run_method` produces `Documents` via either
        // `into_relaxed_extjson()` or serializing a driver result struct
        // that itself contains `Bson` values (`inserted_id`, ...) -- both
        // paths wrap an ObjectId/DateTime the same way, so simplifying
        // once here covers every method uniformly instead of repeating
        // the call at each of the eight call sites in `run_method`.
        if let QueryResult::Documents { items, .. } = &mut result {
            for doc in items {
                simplify_extjson(doc);
            }
        }
        Ok(result)
    }

    /// The collection a `find` reads from -- `None` for `aggregate`,
    /// `findOne`/`countDocuments`, and every write method, same
    /// conservative "when in doubt, refuse" principle `single_table_source`
    /// uses for SQL: `aggregate`'s pipeline can reshape/join/group
    /// documents well past "one row = one document in this collection", so
    /// there's no source a generated `updateOne`/`deleteOne` could safely
    /// aim at. Chained `.sort()`/`.limit()`/`.skip()` don't change that --
    /// they narrow/reorder which documents `find` returns, never reshape
    /// one -- so those stay editable; a chained `.count()` doesn't return
    /// documents at all, so that one isn't. A plain `find`'s own optional
    /// 2nd (projection) argument can still hide the key/edited field from
    /// the result, but that's caught generically downstream (`editable_row`
    /// errors "the key column '_id' isn't in this result"), not here.
    fn edit_source(&self, query: &str) -> Option<String> {
        // A `getSiblingDB` query reads another database than the session's,
        // and a generated `updateOne` would run against the wrong one.
        if clean_command(query).starts_with("db.getSiblingDB(") {
            return None;
        }
        let parsed = parse_shell_query(query.trim()).ok()?;
        if parsed.primary().name != "find" {
            return None;
        }
        let chain_is_safe = parsed
            .chain()
            .iter()
            .all(|call| matches!(call.name.as_str(), "sort" | "limit" | "skip" | "hint"));
        chain_is_safe.then(|| parsed.collection.clone())
    }

    /// Always `_id` -- said outright so the plugin does not have to run
    /// `list_schema` (which samples a document from *every* collection) after
    /// each result just to learn it.
    fn edit_key_columns(&self, _source: &str) -> Option<Vec<String>> {
        Some(vec!["_id".to_string()])
    }

    /// Builds the `updateOne`/`deleteOne` this driver would run for `edit`,
    /// filtered by `edit.key` (always just `_id`, the one field
    /// `list_schema` marks `primary_key: true` -- see that field's own
    /// comment). Both the filter's key value and a `SetValue`'s new value
    /// go through `mongo_value_literal` -- see its own doc comment for why
    /// a value can't just be quoted as a plain string the way SQL's
    /// `build_sql_edit` does.
    fn edit_sql(&self, edit: &query_driver::RowEdit) -> Option<String> {
        let collection = &edit.table;
        let filter = mongo_filter_literal(&edit.key);
        Some(match &edit.change {
            query_driver::RowChange::SetValue { column, value } => format!(
                "db.{collection}.updateOne({filter}, {{$set: {{{}: {}}}}})",
                mongo_field_key(column),
                mongo_value_literal(value)
            ),
            query_driver::RowChange::SetValues(pairs) => format!(
                "db.{collection}.updateOne({filter}, {{$set: {{{}}}}})",
                pairs
                    .iter()
                    .map(|(column, value)| format!(
                        "{}: {}",
                        mongo_field_key(column),
                        mongo_value_literal(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            query_driver::RowChange::DeleteRow => {
                format!("db.{collection}.deleteOne({filter})")
            }
        })
    }

    fn crud_snippet(
        &self,
        entry: &SchemaInfo,
        op: tradar_core::action::CrudOp,
        columns: &[String],
    ) -> Option<String> {
        let collection = &entry.name;
        let all_fields: Vec<&str> = entry.columns.iter().map(|c| c.name.as_str()).collect();

        // `_id` is the one field whose real value looks nothing like a
        // generic placeholder (`ObjectId("...")`, not a bare literal) --
        // see `flatten_document`'s own note that `_id` is the closest
        // thing a document has to a primary key.
        let value_for = |field: &str| -> String {
            if field == "_id" {
                "ObjectId(\"<id>\")".to_string()
            } else {
                "<value>".to_string()
            }
        };

        // One field per line once there's more than one -- same reasoning
        // as the SQL builder's `bulleted`. A field name that isn't a bare
        // JS identifier (a dotted path like `address.city`, from a nested
        // document -- see `flatten_document`) is quoted, since `db.x.find`
        // is read by a real JS-ish parser (`mongosh`), not just displayed.
        let object_literal = |fields: &[&str]| -> String {
            match fields {
                [] => "{}".to_string(),
                [field] => format!("{{{}: {}}}", mongo_field_key(field), value_for(field)),
                fields => {
                    let body = fields
                        .iter()
                        .map(|f| format!("  {}: {}", mongo_field_key(f), value_for(f)))
                        .collect::<Vec<_>>()
                        .join(",\n");
                    format!("{{\n{body}\n}}")
                }
            }
        };

        Some(match op {
            tradar_core::action::CrudOp::Read => {
                let picked: Vec<&str> = all_fields
                    .iter()
                    .copied()
                    .filter(|c| columns.iter().any(|s| s.as_str() == *c))
                    .collect();
                if picked.is_empty() {
                    format!("db.{collection}.find({{}})")
                } else {
                    let projection = picked
                        .iter()
                        .map(|c| format!("{}: 1", mongo_field_key(c)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("db.{collection}.find({{}}, {{{projection}}})")
                }
            }
            tradar_core::action::CrudOp::Create => {
                let insert_fields = query_driver::pick_columns(columns, &all_fields, &all_fields);
                format!(
                    "db.{collection}.insertOne({})",
                    nested_insert_literal(&insert_fields, &value_for)
                )
            }
            tradar_core::action::CrudOp::Update => {
                let non_id: Vec<&str> =
                    all_fields.iter().copied().filter(|c| *c != "_id").collect();
                let set_fields = query_driver::pick_columns(columns, &all_fields, &non_id);
                format!(
                    "db.{collection}.updateOne({{_id: ObjectId(\"<id>\")}}, {{$set: {}}})",
                    object_literal(&set_fields)
                )
            }
            tradar_core::action::CrudOp::Delete => {
                let filter_fields = query_driver::pick_columns(columns, &all_fields, &["_id"]);
                format!(
                    "db.{collection}.deleteOne({})",
                    object_literal(&filter_fields)
                )
            }
        })
    }
}

/// A JS object key for `field` -- bare when it's a valid identifier, quoted
/// otherwise (a dotted path like `address.city` from a nested document, see
/// `flatten_document`). `db.x.find({...})` snippets are read by a real
/// JS-ish parser (`mongosh`), not just displayed, so an unquoted dot would
/// produce a syntax error rather than a working query with a wrong field.
fn mongo_field_key(field: &str) -> String {
    let is_bare = field
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && field
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if is_bare {
        field.to_string()
    } else {
        format!("\"{}\"", field.replace('"', "\\\""))
    }
}

/// One level of the tree `nested_insert_literal` groups dotted field paths
/// into -- a leaf holds the *full* original field path (what `value_for`
/// needs to pick the right placeholder), a group holds its children in
/// first-seen order.
enum InsertField {
    Leaf(String),
    Group(Vec<(String, InsertField)>),
}

/// Inserts `field` (one of `nested_insert_literal`'s fields, e.g.
/// `"address.city"`) into the tree being built, splitting on `.` one
/// level at a time and descending into (or creating) the group for each
/// segment but the last. `remaining` shrinks on each recursive call;
/// `original` stays the full path, for the leaf to carry at the bottom.
fn insert_field_path(roots: &mut Vec<(String, InsertField)>, remaining: &str, original: &str) {
    match remaining.split_once('.') {
        None => roots.push((
            remaining.to_string(),
            InsertField::Leaf(original.to_string()),
        )),
        Some((head, rest)) => {
            if let Some((_, InsertField::Group(children))) =
                roots.iter_mut().find(|(key, _)| key == head)
            {
                insert_field_path(children, rest, original);
                return;
            }
            let mut children = Vec::new();
            insert_field_path(&mut children, rest, original);
            roots.push((head.to_string(), InsertField::Group(children)));
        }
    }
}

/// A nested JS object literal for `insertOne`'s document, merging dotted
/// field paths that share a prefix (`address.city`, `address.zip`) into
/// one real nested object -- `{address: {city: <value>, zip: <value>}}`,
/// not a single flat field literally named `"address.city"`. Unlike
/// `object_literal` (used by Update/Delete, where Mongo itself reads a
/// dotted key in `$set`/a filter as a path expression, so quoting it flat
/// there is already correct), `insertOne` has no such convention: a dotted
/// key in the document it inserts is just a field name with a literal dot
/// in it, not a path -- exactly the mismatch the navigator's own dotted
/// names (from `flatten_document` sampling a real nested document) would
/// otherwise reproduce as a wrong Create snippet.
fn nested_insert_literal(fields: &[&str], value_for: &dyn Fn(&str) -> String) -> String {
    let mut roots: Vec<(String, InsertField)> = Vec::new();
    for &field in fields {
        insert_field_path(&mut roots, field, field);
    }
    render_insert_group(&roots, value_for, 1)
}

fn render_insert_node(
    node: &InsertField,
    value_for: &dyn Fn(&str) -> String,
    indent: usize,
) -> String {
    match node {
        InsertField::Leaf(field) => value_for(field),
        InsertField::Group(children) => render_insert_group(children, value_for, indent),
    }
}

/// Same one-per-line-once-there's-more-than-one shape `object_literal`
/// uses, except a multi-field group's closing `}` lands one level
/// shallower than its own body, and a nested group recurses with one more
/// level of indent -- `indent` counts 2-space units from the outermost
/// `{`.
fn render_insert_group(
    children: &[(String, InsertField)],
    value_for: &dyn Fn(&str) -> String,
    indent: usize,
) -> String {
    match children {
        [] => "{}".to_string(),
        [(key, node)] => format!(
            "{{{}: {}}}",
            mongo_field_key(key),
            render_insert_node(node, value_for, indent)
        ),
        children => {
            let pad = "  ".repeat(indent);
            let closing_pad = "  ".repeat(indent - 1);
            let body = children
                .iter()
                .map(|(key, node)| {
                    format!(
                        "{pad}{}: {}",
                        mongo_field_key(key),
                        render_insert_node(node, value_for, indent + 1)
                    )
                })
                .collect::<Vec<_>>()
                .join(",\n");
            format!("{{\n{body}\n{closing_pad}}}")
        }
    }
}

/// `{_id: ObjectId("..."), ...}` -- every `(column, displayed value)` pair
/// in a `RowEdit::key` as one mongosh filter object, values run through
/// `mongo_value_literal`. In practice this is always just `_id` (the one
/// column `list_schema` marks as a key), but built generically the same
/// shape `build_sql_edit`'s `WHERE` clause is.
fn mongo_filter_literal(key: &[(String, String)]) -> String {
    let body = key
        .iter()
        .map(|(column, value)| {
            format!(
                "{}: {}",
                mongo_field_key(column),
                mongo_value_literal(value)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{body}}}")
}

/// Infers a mongosh literal from a value as it's displayed in the grid or
/// typed into the row-edit prompt. Unlike SQL (`build_sql_edit`'s own doc
/// comment: Postgres/SQLite coerce a quoted `'1'` to a number where a
/// number is wanted), a quoted JS string is never silently reinterpreted as
/// another type -- `{_id: "ObjectId(...)"}"` would filter for that literal
/// *string*, not call the constructor, so quoting everything as SQL does
/// would break every structured field.
///
/// `ObjectId("...")`/`ISODate("...")` (exactly the shape `simplify_extjson`
/// formats a document's own such fields as, so a key's displayed value
/// round-trips straight back into a working filter) pass through as-is;
/// anything else that parses as JSON -- a number, `true`/`false`, `null`,
/// or a typed-out array/object literal -- also passes through as-is; every
/// other text (the common case, an ordinary string field) is JSON-quoted.
/// A known, deliberate limitation of inferring type from typed text rather
/// than asking the user to pick one: a `Decimal128`/`Binary`/other BSON
/// type with no bare-text mongosh literal of its own has no way to be
/// entered here and is sent as a plain string instead.
fn mongo_value_literal(value: &str) -> String {
    let trimmed = value.trim();
    let is_constructor_call = |prefix: &str| {
        trimmed.starts_with(prefix) && trimmed.ends_with("\")") && trimmed.contains('"')
    };
    if is_constructor_call("ObjectId(\"") || is_constructor_call("ISODate(\"") {
        return trimmed.to_string();
    }
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return trimmed.to_string();
    }
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

async fn run_method(
    collection: &mongodb::Collection<Document>,
    primary: &MethodCall,
    chain: &[MethodCall],
) -> anyhow::Result<QueryResult> {
    run_method_in(None, collection, primary, chain).await
}

/// `run_method`, with the database `.explain()` needs to send its command
/// (`None` where only a collection is at hand).
async fn run_method_in(
    db: Option<&mongodb::Database>,
    collection: &mongodb::Collection<Document>,
    primary: &MethodCall,
    chain: &[MethodCall],
) -> anyhow::Result<QueryResult> {
    let method = primary.name.as_str();
    let args = &primary.args;

    let doc_arg = |i: usize| -> anyhow::Result<Document> {
        let value = args
            .get(i)
            .ok_or_else(|| anyhow::anyhow!("{method} requires at least {} argument(s)", i + 1))?;
        json_to_document(value.clone())
    };

    // Every arm below reads a fixed number of leading arguments and ignores
    // the rest; `max_args` rejects anything beyond what that arm actually
    // consumes, per the module's promise to reject unsupported shapes with a
    // clear error rather than silently guessing.
    let max_args = |max: usize| -> anyhow::Result<()> {
        if args.len() > max {
            anyhow::bail!("{method} does not support more than {max} argument(s)");
        }
        Ok(())
    };

    // Only `find` accepts a chain (`.sort()`/`.limit()`/`.skip()`/
    // `.count()`, applied below) -- every other method rejects one outright
    // rather than silently ignoring it.
    let reject_chain = || -> anyhow::Result<()> {
        if let Some(call) = chain.first() {
            anyhow::bail!("{method}(...) does not support chained .{}()", call.name);
        }
        Ok(())
    };

    match method {
        "find" => {
            // Filter, and now an optional projection as the 2nd argument.
            max_args(2)?;
            let filter = if args.is_empty() {
                Document::new()
            } else {
                doc_arg(0)?
            };
            let projection = args
                .get(1)
                .map(|p| json_to_document(p.clone()))
                .transpose()?;

            // `.count()` and `.explain()` have to be the chain's last call
            // (nothing after them means anything); everything before
            // configures the cursor.
            let (mut sort, mut limit, mut skip, mut hint) = (None, None, None, None);
            let (mut wants_count, mut wants_explain) = (false, false);
            for (i, call) in chain.iter().enumerate() {
                let last = i == chain.len() - 1;
                match call.name.as_str() {
                    "sort" => {
                        if call.args.len() != 1 {
                            anyhow::bail!("sort(...) requires exactly 1 argument");
                        }
                        sort = Some(json_to_document(call.args[0].clone())?);
                    }
                    "limit" => {
                        limit =
                            Some(call.args.first().and_then(|v| v.as_i64()).ok_or_else(|| {
                                anyhow::anyhow!("limit(...) requires a numeric argument")
                            })?);
                    }
                    "skip" => {
                        skip =
                            Some(call.args.first().and_then(|v| v.as_u64()).ok_or_else(|| {
                                anyhow::anyhow!("skip(...) requires a numeric argument")
                            })?);
                    }
                    // `hint({field: 1})` or `hint("index_name")`.
                    "hint" => {
                        hint = Some(match call.args.as_slice() {
                            [serde_json::Value::String(name)] => Hint::Name(name.clone()),
                            [doc @ serde_json::Value::Object(_)] => {
                                Hint::Keys(json_to_document(doc.clone())?)
                            }
                            _ => anyhow::bail!("hint(...) takes an index name or a key document"),
                        });
                    }
                    "count" if last => {
                        if !call.args.is_empty() {
                            anyhow::bail!("count() does not take arguments");
                        }
                        wants_count = true;
                    }
                    "count" => anyhow::bail!("count() must be the last call in the chain"),
                    "explain" if last => wants_explain = true,
                    "explain" => anyhow::bail!("explain() must be the last call in the chain"),
                    other => anyhow::bail!(
                        "find(...) does not support chained .{other}() -- only \
                         .sort()/.limit()/.skip()/.hint()/.count()/.explain() are implemented"
                    ),
                }
            }

            if wants_explain {
                let db = db.ok_or_else(|| anyhow::anyhow!("explain() needs a database"))?;
                let mut find = mongodb::bson::doc! { "find": collection.name(), "filter": filter };
                if let Some(sort) = sort {
                    find.insert("sort", sort);
                }
                if let Some(projection) = projection {
                    find.insert("projection", projection);
                }
                if let Some(limit) = limit {
                    find.insert("limit", limit);
                }
                if let Some(skip) = skip {
                    find.insert("skip", skip as i64);
                }
                match hint {
                    Some(Hint::Name(name)) => find.insert("hint", name),
                    Some(Hint::Keys(keys)) => find.insert("hint", keys),
                    _ => None,
                };
                let reply = db
                    .run_command(
                        mongodb::bson::doc! { "explain": find, "verbosity": "executionStats" },
                    )
                    .await?;
                return Ok(QueryResult::Documents {
                    items: vec![Bson::Document(reply).into_relaxed_extjson()],
                    truncated: false,
                });
            }

            // The server counts (an index answers it without reading a
            // single document) -- not a client-side walk of the cursor.
            if wants_count {
                let mut count = collection.count_documents(filter);
                // `limit(0)` means "no limit" in the shell; a `$limit: 0` stage
                // would be an error.
                if let Some(limit) = limit.filter(|l| *l != 0) {
                    count = count.limit(limit.unsigned_abs());
                }
                if let Some(skip) = skip {
                    count = count.skip(skip);
                }
                if let Some(hint) = hint {
                    count = count.hint(hint);
                }
                return Ok(QueryResult::Documents {
                    items: vec![serde_json::json!({ "count": count.await? })],
                    truncated: false,
                });
            }

            let mut builder = collection.find(filter);
            if let Some(projection) = projection {
                builder = builder.projection(projection);
            }
            if let Some(sort) = sort {
                builder = builder.sort(sort);
            }
            if let Some(limit) = limit {
                builder = builder.limit(limit);
            }
            if let Some(skip) = skip {
                builder = builder.skip(skip);
            }
            if let Some(hint) = hint {
                builder = builder.hint(hint);
            }

            let mut cursor = builder.await?;
            let mut docs = Vec::new();
            let mut truncated = false;
            while let Some(doc) = cursor.try_next().await? {
                if docs.len() >= query_driver::max_rows() {
                    truncated = true;
                    break;
                }
                docs.push(Bson::Document(doc).into_relaxed_extjson());
            }
            Ok(QueryResult::Documents {
                items: docs,
                truncated,
            })
        }
        "findOne" => {
            reject_chain()?;
            max_args(2)?;
            let filter = if args.is_empty() {
                Document::new()
            } else {
                doc_arg(0)?
            };
            let mut builder = collection.find_one(filter);
            if let Some(projection) = args.get(1) {
                builder = builder.projection(json_to_document(projection.clone())?);
            }
            let docs = match builder.await? {
                Some(doc) => vec![Bson::Document(doc).into_relaxed_extjson()],
                None => Vec::new(),
            };
            Ok(QueryResult::Documents {
                items: docs,
                truncated: false,
            })
        }
        "countDocuments" => {
            reject_chain()?;
            max_args(1)?;
            let filter = if args.is_empty() {
                Document::new()
            } else {
                doc_arg(0)?
            };
            let count = collection.count_documents(filter).await?;
            Ok(QueryResult::Documents {
                items: vec![serde_json::json!({ "count": count })],
                truncated: false,
            })
        }
        "estimatedDocumentCount" => {
            reject_chain()?;
            max_args(0)?;
            let count = collection.estimated_document_count().await?;
            Ok(QueryResult::Documents {
                items: vec![serde_json::json!({ "count": count })],
                truncated: false,
            })
        }
        "distinct" => {
            reject_chain()?;
            max_args(2)?;
            let field = args
                .first()
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("distinct requires a field name string"))?
                .to_string();
            let filter = if args.len() > 1 {
                doc_arg(1)?
            } else {
                Document::new()
            };
            let values = collection.distinct(field, filter).await?;
            let cap = query_driver::max_rows();
            let truncated = values.len() > cap;
            let items = values
                .into_iter()
                .take(cap)
                .map(|v| serde_json::json!({ "value": v.into_relaxed_extjson() }))
                .collect();
            Ok(QueryResult::Documents { items, truncated })
        }
        "getIndexes" => {
            reject_chain()?;
            max_args(0)?;
            let mut cursor = collection.list_indexes().await?;
            let mut items = Vec::new();
            while let Some(index) = cursor.try_next().await? {
                let options = index.options.as_ref();
                items.push(serde_json::json!({
                    "name": options.and_then(|o| o.name.clone()),
                    "key": Bson::Document(index.keys.clone()).into_relaxed_extjson(),
                    "unique": options.and_then(|o| o.unique).unwrap_or(false),
                }));
            }
            Ok(QueryResult::Documents {
                items,
                truncated: false,
            })
        }
        "aggregate" => {
            reject_chain()?;
            // Real Mongo shell syntax passes a single pipeline array:
            // db.<coll>.aggregate([{"$match": ...}, {"$group": ...}]). Unwrap
            // that into individual stage documents. Fall back to treating
            // each top-level argument as its own stage document, for the
            // non-shell varargs form: aggregate({stage}, {stage}).
            let stage_values: Vec<serde_json::Value> = match args.as_slice() {
                [serde_json::Value::Array(stages)] => stages.clone(),
                other => other.to_vec(),
            };
            let pipeline = stage_values
                .into_iter()
                .map(json_to_document)
                .collect::<anyhow::Result<Vec<_>>>()?;
            let mut cursor = collection.aggregate(pipeline).await?;
            let mut docs = Vec::new();
            let mut truncated = false;
            while let Some(doc) = cursor.try_next().await? {
                if docs.len() >= query_driver::max_rows() {
                    truncated = true;
                    break;
                }
                docs.push(Bson::Document(doc).into_relaxed_extjson());
            }
            Ok(QueryResult::Documents {
                items: docs,
                truncated,
            })
        }
        "insertOne" => {
            reject_chain()?;
            max_args(1)?;
            let result = collection.insert_one(doc_arg(0)?).await?;
            Ok(QueryResult::Documents {
                items: vec![serde_json::to_value(result)?],
                truncated: false,
            })
        }
        "insertMany" => {
            reject_chain()?;
            max_args(1)?;
            let docs_arg = args
                .first()
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("insertMany requires an array argument"))?;
            let docs = docs_arg
                .iter()
                .cloned()
                .map(json_to_document)
                .collect::<anyhow::Result<Vec<_>>>()?;
            let result = collection.insert_many(docs).await?;
            Ok(QueryResult::Documents {
                items: vec![serde_json::to_value(result)?],
                truncated: false,
            })
        }
        "updateOne" => {
            reject_chain()?;
            // filter, update; a 3rd "options" argument is not supported.
            max_args(2)?;
            let result = collection.update_one(doc_arg(0)?, doc_arg(1)?).await?;
            Ok(QueryResult::Documents {
                items: vec![serde_json::to_value(result)?],
                truncated: false,
            })
        }
        "updateMany" => {
            reject_chain()?;
            max_args(2)?;
            let result = collection.update_many(doc_arg(0)?, doc_arg(1)?).await?;
            Ok(QueryResult::Documents {
                items: vec![serde_json::to_value(result)?],
                truncated: false,
            })
        }
        "deleteOne" => {
            reject_chain()?;
            max_args(1)?;
            let result = collection.delete_one(doc_arg(0)?).await?;
            Ok(QueryResult::Documents {
                items: vec![serde_json::to_value(result)?],
                truncated: false,
            })
        }
        "deleteMany" => {
            reject_chain()?;
            max_args(1)?;
            let result = collection.delete_many(doc_arg(0)?).await?;
            Ok(QueryResult::Documents {
                items: vec![serde_json::to_value(result)?],
                truncated: false,
            })
        }
        other => {
            anyhow::bail!("unsupported query: db.<collection>.{other}(...) is not implemented")
        }
    }
}

/// Reformats MongoDB Extended JSON's `{"$oid": "<hex>"}` (an `ObjectId`)
/// and `{"$date": "<iso>"}` (a `DateTime`) into the same shell-literal
/// text `mongosh` itself prints them as -- `ObjectId("<hex>")` /
/// `ISODate("<iso>")` -- recursively through nested documents/arrays.
/// Deliberately *not* a bare string: an `ObjectId` and a plain string
/// field that happens to contain the same-looking hex text must stay
/// visually distinguishable (`ObjectId("507f...")` vs `"507f..."`), which
/// a bare-string unwrap -- tried first, then reverted after user feedback
/// -- would erase entirely. `into_relaxed_extjson()` -- and, identically,
/// `serde_json`'s own `Serialize` impl for `Bson` when a driver result
/// struct (`inserted_id`, ...) gets serialized -- still wraps these two
/// even in "relaxed" mode, since JSON has no native representation for
/// either. Safe to reformat here: this driver's results are read-only
/// display, never re-encoded back into BSON, and the type itself is also
/// visible separately via the navigator's schema (`Bson::element_type()`
/// in `flatten_document`). Any other extJSON wrapper (`$numberLong`,
/// `$numberDecimal`, `$binary`, ...) is left alone -- reformatting those
/// would lose a distinction (e.g. an int that overflowed `i32`) that
/// isn't purely cosmetic the way an id or a timestamp is.
fn simplify_extjson(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let reformatted = if map.len() == 1 {
                map.get("$oid")
                    .and_then(|v| v.as_str())
                    .map(|hex| format!("ObjectId(\"{hex}\")"))
                    .or_else(|| {
                        map.get("$date")
                            .and_then(|v| v.as_str())
                            .map(|iso| format!("ISODate(\"{iso}\")"))
                    })
            } else {
                None
            };
            match reformatted {
                Some(s) => *value = serde_json::Value::String(s),
                None => {
                    for v in map.values_mut() {
                        simplify_extjson(v);
                    }
                }
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                simplify_extjson(v);
            }
        }
        _ => {}
    }
}

/// Every field named in any of this collection's indexes, `_id` excluded
/// (every collection has the implicit `_id_` index, which `primary_key`
/// already surfaces -- listing it again here would just be noise). Field
/// names come straight from `IndexModel::keys`, the same dotted-path form
/// `flatten_document` already produces for a nested field's own key
/// (Mongo's own index syntax, e.g. `{"address.city": 1}`), so a match
/// against `ColumnInfo::name` needs no extra parsing. Errors (no
/// permission to list indexes, a dropped collection) come back as "no
/// indexed fields known" rather than failing schema browsing over it --
/// same fallback `list_schema` already uses for a sample that fails to
/// load.
async fn indexed_field_names(
    collection: &mongodb::Collection<Document>,
) -> std::collections::HashSet<String> {
    let Ok(mut cursor) = collection.list_indexes().await else {
        return std::collections::HashSet::new();
    };
    let mut fields = std::collections::HashSet::new();
    while let Ok(Some(index)) = cursor.try_next().await {
        for key in index.keys.keys() {
            if key != "_id" {
                fields.insert(key.clone());
            }
        }
    }
    fields
}

fn json_to_document(value: serde_json::Value) -> anyhow::Result<Document> {
    let bson = Bson::try_from(value)?;
    bson.as_document()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("expected a JSON object"))
}

/// Flattens a sampled document into `parent.child` paths, the same way
/// `tradar-connector-elasticsearch` flattens a mapping's `properties` -- a nested
/// field is addressed that way in a query anyway. Arrays are reported as a
/// single `array` field rather than expanded per element: a doc's array
/// entries aren't guaranteed to share a shape, so there's no one type to
/// report for "the third element of every document's tags array".
fn flatten_document(prefix: &str, doc: &Document, out: &mut Vec<ColumnInfo>) {
    for (name, value) in doc {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        match value {
            Bson::Document(nested) => flatten_document(&path, nested, out),
            other => {
                let mut column = ColumnInfo::new(path, bson_type_name(other));
                // `_id` is the closest thing a MongoDB document has to a
                // primary key -- every collection has exactly one, and it's
                // what a row edit in the results grid would key on if that
                // were ever built for Mongo.
                column.primary_key = name == "_id";
                out.push(column);
            }
        }
    }
}

/// The BSON element type's own name (`Bson`'s `Debug` derive already spells
/// it exactly this way: `Int32`, `ObjectId`, `String`, ...), passed through
/// rather than normalized -- same philosophy as `ColumnInfo::type_name`
/// elsewhere: show what the database actually says.
fn bson_type_name(value: &Bson) -> String {
    format!("{:?}", value.element_type())
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "mongo",
    display_name: "MongoDB",
    icon: "🍃",
    capabilities: &[Capability::Query, Capability::Schema],
};

struct MongoConnector;

#[async_trait]
impl Connector for MongoConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let mut driver = MongoDriver::new(&connection.target);
        tradar_connector_spi::with_connect_timeout(&connection.target, driver.connect()).await?;
        let driver: Arc<dyn QueryDriver> = Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(MongoConnector)
}

#[cfg(test)]
mod tests;
