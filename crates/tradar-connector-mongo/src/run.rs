use futures_util::TryStreamExt;
use mongodb::bson::{Bson, Document};
use mongodb::options::Hint;

use tradar_query_workbench::query_driver::{self as query_driver, QueryResult};

use crate::convert::*;
use crate::shell::*;

pub(crate) async fn run_method(
    collection: &mongodb::Collection<Document>,
    primary: &MethodCall,
    chain: &[MethodCall],
) -> anyhow::Result<QueryResult> {
    run_method_in(None, collection, primary, chain).await
}

/// `run_method`, with the database `.explain()` needs to send its command
/// (`None` where only a collection is at hand).
pub(crate) async fn run_method_in(
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
