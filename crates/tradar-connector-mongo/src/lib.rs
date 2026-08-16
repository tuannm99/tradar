//! MongoDB connector: a minimal shell-subset parser for the literal shape
//! `db.<collection>.<method>(<json-args>)`, not a real JS engine. Anything
//! outside that shape — chained methods, `$where`, arbitrary expressions —
//! is rejected with a clear error rather than guessed at.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::TryStreamExt;
use mongodb::bson::{Bson, Document};

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    ColumnInfo, QueryDriver, QueryResult, SchemaInfo, Statement,
};
use tradar_query_workbench::query_engine::QueryEngine;

struct ParsedQuery {
    collection: String,
    method: String,
    args: Vec<serde_json::Value>,
}

fn parse_shell_query(query: &str) -> anyhow::Result<ParsedQuery> {
    let query = query.trim();
    let rest = query.strip_prefix("db.").ok_or_else(|| {
        anyhow::anyhow!("expected a query starting with \"db.<collection>.<method>(...)\"")
    })?;

    let dot = rest
        .find('.')
        .ok_or_else(|| anyhow::anyhow!("missing collection name"))?;
    let collection = rest[..dot].to_string();
    let rest = &rest[dot + 1..];

    let paren = rest
        .find('(')
        .ok_or_else(|| anyhow::anyhow!("missing method call"))?;
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

    Ok(ParsedQuery {
        collection,
        method,
        args,
    })
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

struct MongoDriver {
    uri: String,
    client: Option<mongodb::Client>,
}

impl MongoDriver {
    fn new(uri: &str) -> Self {
        Self {
            uri: uri.to_string(),
            client: None,
        }
    }

    fn database(&self) -> anyhow::Result<mongodb::Database> {
        let client = self
            .client
            .as_ref()
            .expect("connect() must be called first");
        client
            .default_database()
            .ok_or_else(|| anyhow::anyhow!("connection string must include a default database"))
    }
}

#[async_trait]
impl QueryDriver for MongoDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = mongodb::Client::with_uri_str(&self.uri).await?;
        let db = client
            .default_database()
            .ok_or_else(|| anyhow::anyhow!("connection string must include a default database"))?;
        db.run_command(mongodb::bson::doc! { "ping": 1 }).await?;
        self.client = Some(client);
        Ok(())
    }

    /// The shell shapes this driver actually parses -- see the module
    /// docs; anything else is rejected, so completing it would mislead.
    fn keywords(&self) -> &'static [&'static str] {
        &[
            "db",
            "find",
            "aggregate",
            "insertOne",
            "insertMany",
            "updateOne",
            "updateMany",
            "deleteOne",
            "deleteMany",
            "$match",
            "$group",
            "$sort",
            "$limit",
            "$project",
            "$lookup",
            "$unwind",
            "$set",
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
            "$exists",
            "$regex",
        ]
    }

    /// One `db.<collection>.<method>(...)` per line: the parser accepts a
    /// single call, so a line is exactly a statement.
    fn split_statements(&self, text: &str) -> Vec<Statement> {
        let mut statements = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                let start = offset + (line.len() - line.trim_start().len());
                statements.push(Statement {
                    text: trimmed.to_string(),
                    start,
                    end: start + trimmed.len(),
                });
            }
            offset += line.len();
        }
        statements
    }

    async fn ping(&self) -> anyhow::Result<()> {
        self.database()?
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
    /// with its own per-table `PRAGMA` round trip. A collection that fails
    /// to sample (empty, or a transient error) just reports no columns
    /// rather than failing schema browsing for every other collection.
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let db = self.database()?;
        let names = db.list_collection_names().await?;
        let mut schema = Vec::with_capacity(names.len());
        for name in names {
            let sample = db
                .collection::<Document>(&name)
                .find_one(Document::new())
                .await;
            let columns = match sample {
                Ok(Some(doc)) => {
                    let mut columns = Vec::new();
                    flatten_document("", &doc, &mut columns);
                    columns
                }
                Ok(None) | Err(_) => Vec::new(),
            };
            schema.push(SchemaInfo {
                name,
                columns,
                kind: None,
                ttl: None,
            });
        }
        Ok(schema)
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let parsed = parse_shell_query(query)?;
        let db = self.database()?;
        let collection = db.collection::<Document>(&parsed.collection);
        let mut result = run_method(&collection, &parsed.method, &parsed.args).await?;
        // Every arm of `run_method` produces `Documents` via either
        // `into_relaxed_extjson()` or serializing a driver result struct
        // that itself contains `Bson` values (`inserted_id`, ...) -- both
        // paths wrap an ObjectId/DateTime the same way, so simplifying
        // once here covers every method uniformly instead of repeating
        // the call at each of the eight call sites in `run_method`.
        if let QueryResult::Documents(docs) = &mut result {
            for doc in docs {
                simplify_extjson(doc);
            }
        }
        Ok(result)
    }

    fn crud_snippet(&self, entry: &SchemaInfo, op: tradar_core::action::CrudOp) -> Option<String> {
        let collection = &entry.name;
        Some(match op {
            tradar_core::action::CrudOp::Read => format!("db.{collection}.find({{}})"),
            tradar_core::action::CrudOp::Create => format!("db.{collection}.insertOne({{}})"),
            tradar_core::action::CrudOp::Update => {
                format!("db.{collection}.updateOne({{_id: ObjectId(\"<id>\")}}, {{$set: {{}}}})")
            }
            tradar_core::action::CrudOp::Delete => {
                format!("db.{collection}.deleteOne({{_id: ObjectId(\"<id>\")}})")
            }
        })
    }
}

async fn run_method(
    collection: &mongodb::Collection<Document>,
    method: &str,
    args: &[serde_json::Value],
) -> anyhow::Result<QueryResult> {
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

    match method {
        "find" => {
            // Only a filter is supported; projection (a 2nd argument) is not
            // implemented yet.
            max_args(1)?;
            let filter = if args.is_empty() {
                Document::new()
            } else {
                doc_arg(0)?
            };
            let mut cursor = collection.find(filter).await?;
            let mut docs = Vec::new();
            while let Some(doc) = cursor.try_next().await? {
                docs.push(Bson::Document(doc).into_relaxed_extjson());
            }
            Ok(QueryResult::Documents(docs))
        }
        "aggregate" => {
            // Real Mongo shell syntax passes a single pipeline array:
            // db.<coll>.aggregate([{"$match": ...}, {"$group": ...}]). Unwrap
            // that into individual stage documents. Fall back to treating
            // each top-level argument as its own stage document, for the
            // non-shell varargs form: aggregate({stage}, {stage}).
            let stage_values: Vec<serde_json::Value> = match args {
                [serde_json::Value::Array(stages)] => stages.clone(),
                other => other.to_vec(),
            };
            let pipeline = stage_values
                .into_iter()
                .map(json_to_document)
                .collect::<anyhow::Result<Vec<_>>>()?;
            let mut cursor = collection.aggregate(pipeline).await?;
            let mut docs = Vec::new();
            while let Some(doc) = cursor.try_next().await? {
                docs.push(Bson::Document(doc).into_relaxed_extjson());
            }
            Ok(QueryResult::Documents(docs))
        }
        "insertOne" => {
            max_args(1)?;
            let result = collection.insert_one(doc_arg(0)?).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        "insertMany" => {
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
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        "updateOne" => {
            // filter, update; a 3rd "options" argument is not supported.
            max_args(2)?;
            let result = collection.update_one(doc_arg(0)?, doc_arg(1)?).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        "updateMany" => {
            max_args(2)?;
            let result = collection.update_many(doc_arg(0)?, doc_arg(1)?).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        "deleteOne" => {
            max_args(1)?;
            let result = collection.delete_one(doc_arg(0)?).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        "deleteMany" => {
            max_args(1)?;
            let result = collection.delete_many(doc_arg(0)?).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
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
mod tests {
    use super::*;

    #[test]
    fn crud_snippet_covers_all_four_ops() {
        let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");
        let entry = SchemaInfo::new("users");

        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read),
            Some("db.users.find({})".to_string())
        );
        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Create),
            Some("db.users.insertOne({})".to_string())
        );
        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Update),
            Some("db.users.updateOne({_id: ObjectId(\"<id>\")}, {$set: {}})".to_string())
        );
        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Delete),
            Some("db.users.deleteOne({_id: ObjectId(\"<id>\")})".to_string())
        );
    }

    #[test]
    fn simplify_extjson_formats_an_oid_as_a_mongosh_style_literal() {
        let mut value = serde_json::json!({"$oid": "507f1f77bcf86cd799439011"});

        simplify_extjson(&mut value);

        assert_eq!(
            value,
            serde_json::json!("ObjectId(\"507f1f77bcf86cd799439011\")")
        );
    }

    #[test]
    fn simplify_extjson_formats_a_date_as_a_mongosh_style_literal() {
        let mut value = serde_json::json!({"$date": "2026-08-16T00:00:00Z"});

        simplify_extjson(&mut value);

        assert_eq!(
            value,
            serde_json::json!("ISODate(\"2026-08-16T00:00:00Z\")")
        );
    }

    #[test]
    fn simplify_extjson_keeps_an_objectid_distinguishable_from_a_plain_string_field() {
        // The whole point: a real string field that happens to contain
        // the exact same hex text as an id must not render identically
        // to the id itself.
        let mut value = serde_json::json!({
            "_id": {"$oid": "507f1f77bcf86cd799439011"},
            "note": "507f1f77bcf86cd799439011",
        });

        simplify_extjson(&mut value);

        assert_eq!(
            value["_id"],
            serde_json::json!("ObjectId(\"507f1f77bcf86cd799439011\")")
        );
        assert_eq!(value["note"], serde_json::json!("507f1f77bcf86cd799439011"));
        assert_ne!(value["_id"], value["note"]);
    }

    #[test]
    fn simplify_extjson_formats_nested_and_array_occurrences() {
        let mut value = serde_json::json!({
            "_id": {"$oid": "507f1f77bcf86cd799439011"},
            "name": "Ada",
            "createdBy": {"id": {"$oid": "507f191e810c19729de860ea"}},
            "history": [{"$date": "2026-08-01T00:00:00Z"}, {"$date": "2026-08-02T00:00:00Z"}],
        });

        simplify_extjson(&mut value);

        assert_eq!(
            value,
            serde_json::json!({
                "_id": "ObjectId(\"507f1f77bcf86cd799439011\")",
                "name": "Ada",
                "createdBy": {"id": "ObjectId(\"507f191e810c19729de860ea\")"},
                "history": ["ISODate(\"2026-08-01T00:00:00Z\")", "ISODate(\"2026-08-02T00:00:00Z\")"],
            })
        );
    }

    #[test]
    fn simplify_extjson_leaves_other_wrapper_shapes_alone() {
        // `$numberLong`/`$numberDecimal`/`$binary`/... are not touched --
        // see the function's own doc comment for why only `$oid`/`$date`
        // are safe to treat as purely cosmetic.
        let mut value = serde_json::json!({"$numberLong": "9223372036854775807"});

        simplify_extjson(&mut value);

        assert_eq!(
            value,
            serde_json::json!({"$numberLong": "9223372036854775807"})
        );
    }

    #[test]
    fn simplify_extjson_leaves_a_plain_object_with_a_dollar_key_and_other_keys_alone() {
        // A one-key check, not "has a $oid/$date key anywhere" -- a real
        // document field could coincidentally be named that.
        let mut value = serde_json::json!({"$oid": "not-actually-an-id", "other": 1});

        simplify_extjson(&mut value);

        assert_eq!(
            value,
            serde_json::json!({"$oid": "not-actually-an-id", "other": 1})
        );
    }

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

    fn doc_from_json(json: serde_json::Value) -> Document {
        json_to_document(json).unwrap()
    }

    #[test]
    fn flatten_document_reports_a_column_per_top_level_field() {
        let doc = doc_from_json(serde_json::json!({
            "_id": 1,
            "name": "Ada",
            "active": true,
        }));
        let mut columns = Vec::new();

        flatten_document("", &doc, &mut columns);

        let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["_id", "name", "active"]);
    }

    #[test]
    fn flatten_document_marks_only_id_as_the_primary_key() {
        let doc = doc_from_json(serde_json::json!({"_id": 1, "name": "Ada"}));
        let mut columns = Vec::new();

        flatten_document("", &doc, &mut columns);

        let key: Vec<&str> = columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(key, vec!["_id"]);
    }

    #[test]
    fn flatten_document_turns_a_nested_object_into_dotted_paths() {
        let doc = doc_from_json(serde_json::json!({
            "address": {"city": "Hanoi", "zip": "100000"},
        }));
        let mut columns = Vec::new();

        flatten_document("", &doc, &mut columns);

        let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["address.city", "address.zip"]);
    }

    #[test]
    fn flatten_document_does_not_expand_array_elements() {
        let doc = doc_from_json(serde_json::json!({"tags": ["a", "b", "c"]}));
        let mut columns = Vec::new();

        flatten_document("", &doc, &mut columns);

        assert_eq!(
            columns.len(),
            1,
            "one field for the array, not one per element"
        );
        assert_eq!(columns[0].name, "tags");
        assert_eq!(columns[0].type_name, "Array");
    }

    use testcontainers_modules::mongo::Mongo;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    #[tokio::test]
    async fn connect_succeeds_for_a_running_mongo() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));

        let result = driver.connect().await;

        assert!(result.is_ok(), "connect failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn execute_insert_one_then_find_round_trips_a_document() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();

        driver
            .execute(r#"db.users.insertOne({"name": "Ada"})"#)
            .await
            .unwrap();
        let result = driver
            .execute(r#"db.users.find({"name": "Ada"})"#)
            .await
            .unwrap();

        match result {
            QueryResult::Documents(docs) => {
                assert_eq!(docs.len(), 1);
                assert_eq!(docs[0]["name"], "Ada");
            }
            other => panic!("expected Documents, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn find_reports_the_id_as_an_objectid_literal_not_a_wrapped_oid() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();
        driver
            .execute(r#"db.users.insertOne({"name": "Ada"})"#)
            .await
            .unwrap();

        let result = driver
            .execute(r#"db.users.find({"name": "Ada"})"#)
            .await
            .unwrap();

        let QueryResult::Documents(docs) = result else {
            panic!("expected Documents");
        };
        let id = docs[0]["_id"].as_str().expect("_id must be a string");
        assert!(
            id.starts_with("ObjectId(\"") && id.ends_with("\")"),
            "was: {id:?}"
        );
    }

    #[tokio::test]
    async fn insert_one_s_own_inserted_id_is_also_an_objectid_literal() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();

        let result = driver
            .execute(r#"db.users.insertOne({"name": "Ada"})"#)
            .await
            .unwrap();

        let QueryResult::Documents(docs) = result else {
            panic!("expected Documents");
        };
        let id = docs[0]["insertedId"]
            .as_str()
            .expect("insertedId must be a string");
        assert!(
            id.starts_with("ObjectId(\"") && id.ends_with("\")"),
            "was: {id:?}"
        );
    }

    #[tokio::test]
    async fn execute_aggregate_accepts_the_real_shell_pipeline_array_syntax() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();
        driver
            .execute(r#"db.orders.insertMany([{"item": "pen", "qty": 2}, {"item": "pencil", "qty": 5}])"#)
            .await
            .unwrap();

        let result = driver
            .execute(r#"db.orders.aggregate([{"$match": {"item": "pen"}}])"#)
            .await
            .unwrap();

        match result {
            QueryResult::Documents(docs) => {
                assert_eq!(docs.len(), 1);
                assert_eq!(docs[0]["item"], "pen");
                assert_eq!(docs[0]["qty"], 2);
            }
            other => panic!("expected Documents, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_find_rejects_a_projection_argument_it_does_not_support() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();
        driver
            .execute(r#"db.users.insertOne({"name": "Ada"})"#)
            .await
            .unwrap();

        let result = driver.execute(r#"db.users.find({}, {"name": 1})"#).await;

        assert!(
            result.is_err(),
            "expected find with a projection argument to be rejected, got {result:?}"
        );
    }

    #[tokio::test]
    async fn list_schema_returns_created_collections() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();
        driver
            .execute(r#"db.users.insertOne({"name": "Ada"})"#)
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        assert!(
            schema.iter().any(|entry| entry.name == "users"),
            "schema was: {:?}",
            schema.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn list_schema_reports_the_fields_of_a_sampled_document() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();
        driver
            .execute(r#"db.users.insertOne({"name": "Ada", "age": 36})"#)
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        let users = schema.iter().find(|s| s.name == "users").unwrap();
        let names: Vec<&str> = users.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"_id"), "columns were: {names:?}");
        assert!(names.contains(&"name"), "columns were: {names:?}");
        assert!(names.contains(&"age"), "columns were: {names:?}");
    }

    #[tokio::test]
    async fn list_schema_marks_underscore_id_as_the_primary_key() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();
        driver
            .execute(r#"db.users.insertOne({"name": "Ada"})"#)
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        let users = schema.iter().find(|s| s.name == "users").unwrap();
        let key: Vec<&str> = users
            .columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(key, vec!["_id"]);
    }

    #[tokio::test]
    async fn an_empty_collection_reports_no_columns_rather_than_failing() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();
        // Creates the collection without inserting a document to sample.
        driver
            .database()
            .unwrap()
            .create_collection("empty")
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        let empty = schema.iter().find(|s| s.name == "empty").unwrap();
        assert!(empty.columns.is_empty());
    }

    #[tokio::test]
    async fn ping_succeeds_against_a_reachable_mongo() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();

        assert!(driver.ping().await.is_ok());
    }

    #[tokio::test]
    async fn execute_rejects_an_unsupported_method() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();

        let result = driver.execute("db.users.drop()").await;

        assert!(result.is_err());
    }
}
