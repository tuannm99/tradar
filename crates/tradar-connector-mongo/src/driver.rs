use mongodb::bson::Document;
use std::sync::Mutex;

use async_trait::async_trait;

use tradar_query::{
    self as query_driver, QueryDriver, QueryResult, SchemaInfo, Statement, qualify_colliding_names,
};

use crate::convert::*;
use crate::keywords::KEYWORDS;
use crate::run::*;
use crate::shell::*;
use crate::snippets::*;

pub(crate) struct MongoDriver {
    pub(crate) uri: String,
    pub(crate) client: Option<mongodb::Client>,
    /// The database `db.<collection>...` currently targets. `None` until
    /// either the connection URI names a default database (picked up in
    /// `connect()`) or a `use <db>` statement runs; `current_db_name()`
    /// falls back to `"test"` in that case, the same default `mongosh`
    /// itself uses when no database was ever selected. A `Mutex` rather
    /// than `&mut self` because `QueryDriver::execute` takes `&self` --
    /// this is the one piece of session state a shell interpreter needs
    /// that a stateless query driver otherwise wouldn't.
    pub(crate) current_db: Mutex<Option<String>>,
}

impl MongoDriver {
    pub(crate) fn new(uri: &str) -> Self {
        Self {
            uri: uri.to_string(),
            client: None,
            current_db: Mutex::new(None),
        }
    }

    pub(crate) fn current_db_name(&self) -> String {
        self.current_db
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "test".to_string())
    }

    pub(crate) fn database(&self) -> mongodb::Database {
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
        KEYWORDS
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

    fn snippet_options(&self) -> Vec<tradar_query::SnippetOption> {
        mongo_snippet_options()
    }

    fn option_snippet(&self, entry: &SchemaInfo, id: &str, columns: &[String]) -> Option<String> {
        mongo_extra_snippet(entry, id)
            .or_else(|| self.crud_snippet(entry, tradar_query::CrudOp::from_id(id)?, columns))
    }

    fn crud_snippet(
        &self,
        entry: &SchemaInfo,
        op: tradar_query::CrudOp,
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
            tradar_query::CrudOp::Read => {
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
            tradar_query::CrudOp::Create => {
                let insert_fields = query_driver::pick_columns(columns, &all_fields, &all_fields);
                format!(
                    "db.{collection}.insertOne({})",
                    nested_insert_literal(&insert_fields, &value_for)
                )
            }
            tradar_query::CrudOp::Update => {
                let non_id: Vec<&str> =
                    all_fields.iter().copied().filter(|c| *c != "_id").collect();
                let set_fields = query_driver::pick_columns(columns, &all_fields, &non_id);
                format!(
                    "db.{collection}.updateOne({{_id: ObjectId(\"<id>\")}}, {{$set: {}}})",
                    object_literal(&set_fields)
                )
            }
            tradar_query::CrudOp::Delete => {
                let filter_fields = query_driver::pick_columns(columns, &all_fields, &["_id"]);
                format!(
                    "db.{collection}.deleteOne({})",
                    object_literal(&filter_fields)
                )
            }
        })
    }
}
