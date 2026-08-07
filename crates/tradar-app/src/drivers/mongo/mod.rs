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

use async_trait::async_trait;
use futures_util::TryStreamExt;
use mongodb::bson::{Bson, Document};

use crate::drivers::{Driver, QueryResult, SchemaInfo};

pub struct MongoDriver {
    uri: String,
    client: Option<mongodb::Client>,
}

impl MongoDriver {
    pub fn new(uri: &str) -> Self {
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
impl Driver for MongoDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = mongodb::Client::with_uri_str(&self.uri).await?;
        let db = client
            .default_database()
            .ok_or_else(|| anyhow::anyhow!("connection string must include a default database"))?;
        db.run_command(mongodb::bson::doc! { "ping": 1 }).await?;
        self.client = Some(client);
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let names = self.database()?.list_collection_names().await?;
        Ok(names.into_iter().map(|name| SchemaInfo { name }).collect())
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let parsed = parse_shell_query(query)?;
        let db = self.database()?;
        let collection = db.collection::<Document>(&parsed.collection);
        run_method(&collection, &parsed.method, &parsed.args).await
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

fn json_to_document(value: serde_json::Value) -> anyhow::Result<Document> {
    let bson = Bson::try_from(value)?;
    bson.as_document()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("expected a JSON object"))
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
            QueryResult::Table { .. } => panic!("expected Documents"),
        }
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
            QueryResult::Table { .. } => panic!("expected Documents"),
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
    async fn execute_rejects_an_unsupported_method() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();

        let result = driver.execute("db.users.drop()").await;

        assert!(result.is_err());
    }
}
