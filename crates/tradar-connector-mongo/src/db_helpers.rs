use futures_util::TryStreamExt;
use mongodb::bson::{Bson, Document};

use tradar_query::QueryResult;

use crate::convert::*;
use crate::driver::*;
use crate::run::*;
use crate::shell::*;

impl MongoDriver {
    /// The console helpers that are not `db.<collection>.<method>()`:
    /// `show collections|users|roles`, bare `db`, and the database-level
    /// calls (`db.getCollectionNames()`, `db.stats()`, `db.runCommand({..})`
    /// ...). Replies are shaped like `mongosh` prints them -- a list is one
    /// array value, a command's reply one document.
    pub(crate) async fn run_db_helper(
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
}
