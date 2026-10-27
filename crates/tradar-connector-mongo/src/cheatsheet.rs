//! The MongoDB cheat sheet (mongosh syntax, as this connector reads it) -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ──
    CheatEntry::new("Explore", "Databases", r##"show dbs"##),
    CheatEntry::new(
        "Explore",
        "Collections of this database",
        r##"show collections"##,
    ),
    CheatEntry::new(
        "Explore",
        "Collection names as a list",
        r##"db.getCollectionNames()"##,
    ),
    CheatEntry::new(
        "Explore",
        "Collections with their options",
        r##"db.getCollectionInfos()"##,
    ),
    CheatEntry::new("Explore", "Database statistics", r##"db.stats()"##),
    CheatEntry::new(
        "Explore",
        "Database statistics in megabytes",
        r##"db.runCommand({dbStats: 1, scale: 1048576})"##,
    ),
    CheatEntry::new(
        "Explore",
        "Collection statistics",
        r##"db.runCommand({collStats: "${collection}"})"##,
    ),
    CheatEntry::new(
        "Explore",
        "Indexes of a collection",
        r##"db.${collection}.getIndexes()"##,
    ),
    CheatEntry::new(
        "Explore",
        "Index usage of a collection",
        r##"db.${collection}.aggregate([{$indexStats: {}}])"##,
    ),
    CheatEntry::new(
        "Explore",
        "Estimated document count (fast)",
        r##"db.${collection}.estimatedDocumentCount()"##,
    ),
    CheatEntry::new("Explore", "Server version", r##"db.version()"##),
    CheatEntry::new(
        "Explore",
        "Who am I connected as",
        r##"db.runCommand({connectionStatus: 1})"##,
    ),
    CheatEntry::new("Explore", "Users of this database", r##"show users"##),
    CheatEntry::new("Explore", "Roles of this database", r##"show roles"##),
    // ── Server ──
    CheatEntry::new("Server", "Server status", r##"db.serverStatus()"##),
    CheatEntry::new(
        "Server",
        "Operations running now",
        r##"db.adminCommand({currentOp: 1, active: true})"##,
    ),
    CheatEntry::new(
        "Server",
        "Operations running longer than 5 seconds",
        r##"db.adminCommand({currentOp: 1, active: true, secs_running: {$gt: 5}})"##,
    ),
    CheatEntry::new(
        "Server",
        "Replica set status",
        r##"db.adminCommand({replSetGetStatus: 1})"##,
    ),
    CheatEntry::new(
        "Server",
        "Time spent per collection (top)",
        r##"db.adminCommand({top: 1})"##,
    ),
    CheatEntry::new(
        "Server",
        "All databases with their size",
        r##"db.adminCommand({listDatabases: 1})"##,
    ),
    CheatEntry::new(
        "Server",
        "Slowest operations (profiler)",
        r##"show profile"##,
    ),
    CheatEntry::new("Server", "Ping", r##"db.adminCommand({ping: 1})"##),
    // ── Find ──
    CheatEntry::new(
        "Find",
        "All documents of a collection",
        r##"db.${collection}.find({})"##,
    ),
    CheatEntry::new(
        "Find",
        "Latest 20 (newest _id first)",
        r##"db.${collection}.find({}).sort({_id: -1}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "By _id",
        r##"db.${collection}.find({_id: ObjectId("${id}")})"##,
    ),
    CheatEntry::new("Find", "One document", r##"db.${collection}.findOne({})"##),
    CheatEntry::new(
        "Find",
        "A field equal to a value",
        r##"db.${collection}.find({${field}: "${value}"}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "A field not equal to a value",
        r##"db.${collection}.find({${field}: {$ne: "${value}"}}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "A field matching a pattern (case-insensitive)",
        r##"db.${collection}.find({${field}: {$regex: "${pattern}", $options: "i"}}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "A field in a list of values",
        r##"db.${collection}.find({${field}: {$in: ["${a}", "${b}"]}}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "A number in a range",
        r##"db.${collection}.find({${field}: {$gte: 10, $lt: 100}}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "Either of two conditions ($or)",
        r##"db.${collection}.find({$or: [{${field}: "${a}"}, {${other_field}: "${b}"}]}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "Both conditions ($and)",
        r##"db.${collection}.find({$and: [{${field}: "${a}"}, {${other_field}: "${b}"}]}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "A field of an embedded document",
        r##"db.${collection}.find({"${field}.${sub_field}": "${value}"}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "Documents missing a field",
        r##"db.${collection}.find({${field}: {$exists: false}}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "Documents where a field is null",
        r##"db.${collection}.find({${field}: null}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "Documents where a field has a type",
        r##"db.${collection}.find({${field}: {$type: "string"}}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "An array containing a value",
        r##"db.${collection}.find({${field}: "${value}"}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "An array with an element that matches",
        r##"db.${collection}.find({${field}: {$elemMatch: {${sub_field}: "${value}"}}}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "An array of a given size",
        r##"db.${collection}.find({${field}: {$size: 3}}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "Created after a date",
        r##"db.${collection}.find({createdAt: {$gte: ISODate("${date}T00:00:00Z")}}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "Only some fields",
        r##"db.${collection}.find({}, {${field}: 1}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "Sort and page (skip / limit)",
        r##"db.${collection}.find({}).sort({${field}: -1}).skip(20).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "Full-text search (needs a text index)",
        r##"db.${collection}.find({$text: {$search: "${value}"}}).limit(20)"##,
    ),
    CheatEntry::new(
        "Find",
        "Count the documents that match",
        r##"db.${collection}.countDocuments({${field}: "${value}"})"##,
    ),
    // ── Aggregate ──
    CheatEntry::new(
        "Aggregate",
        "Count documents",
        r##"db.${collection}.countDocuments({})"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "Distinct values of a field",
        r##"db.${collection}.distinct("${field}")"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "Top values of a field with their count",
        r##"db.${collection}.aggregate([{$group: {_id: "$${field}", n: {$sum: 1}}}, {$sort: {n: -1}}, {$limit: 20}])"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "Match, then group",
        r##"db.${collection}.aggregate([
  {$match: {${field}: "${value}"}},
  {$group: {_id: "$${group}", n: {$sum: 1}}},
  {$sort: {n: -1}}
])"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "Sum and average of a number per group",
        r##"db.${collection}.aggregate([{$group: {_id: "$${group}", total: {$sum: "$${field}"}, average: {$avg: "$${field}"}}}])"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "Documents per day",
        r##"db.${collection}.aggregate([{$group: {_id: {$dateToString: {format: "%Y-%m-%d", date: "$createdAt"}}, n: {$sum: 1}}}, {$sort: {_id: -1}}, {$limit: 30}])"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "Join another collection ($lookup)",
        r##"db.${collection}.aggregate([
  {$lookup: {from: "${other_collection}", localField: "${field}", foreignField: "${other_field}", as: "joined"}},
  {$limit: 20}
])"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "One document per array element ($unwind)",
        r##"db.${collection}.aggregate([{$unwind: "$${field}"}, {$limit: 20}])"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "Choose and rename fields ($project)",
        r##"db.${collection}.aggregate([{$project: {_id: 0, ${field}: 1, renamed: "$${other_field}"}}, {$limit: 20}])"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "Add a computed field ($addFields)",
        r##"db.${collection}.aggregate([{$addFields: {size: {$size: "$${array_field}"}}}, {$limit: 20}])"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "Documents grouped into ranges ($bucket)",
        r##"db.${collection}.aggregate([{$bucket: {groupBy: "$${number_field}", boundaries: [0, 10, 100, 1000], default: "other"}}])"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "A random sample of documents",
        r##"db.${collection}.aggregate([{$sample: {size: 10}}])"##,
    ),
    CheatEntry::new(
        "Aggregate",
        "Count the documents that reach a stage ($count)",
        r##"db.${collection}.aggregate([{$match: {${field}: "${value}"}}, {$count: "total"}])"##,
    ),
    // ── Write ──
    CheatEntry::new(
        "Write",
        "Insert one document",
        r##"db.${collection}.insertOne({${field}: "${value}"})"##,
    ),
    CheatEntry::new(
        "Write",
        "Insert several documents",
        r##"db.${collection}.insertMany([{${field}: "${a}"}, {${field}: "${b}"}])"##,
    ),
    CheatEntry::new(
        "Write",
        "Update one document ($set)",
        r##"db.${collection}.updateOne({_id: ObjectId("${id}")}, {$set: {${field}: "${value}"}})"##,
    ),
    CheatEntry::new(
        "Write",
        "Update every document that matches",
        r##"db.${collection}.updateMany({${field}: "${value}"}, {$set: {${other_field}: "${other_value}"}})"##,
    ),
    CheatEntry::new(
        "Write",
        "Add to a number ($inc)",
        r##"db.${collection}.updateOne({_id: ObjectId("${id}")}, {$inc: {${field}: 1}})"##,
    ),
    CheatEntry::new(
        "Write",
        "Add to an array ($push)",
        r##"db.${collection}.updateOne({_id: ObjectId("${id}")}, {$push: {${field}: "${value}"}})"##,
    ),
    CheatEntry::new(
        "Write",
        "Add to an array only if missing ($addToSet)",
        r##"db.${collection}.updateOne({_id: ObjectId("${id}")}, {$addToSet: {${field}: "${value}"}})"##,
    ),
    CheatEntry::new(
        "Write",
        "Remove from an array ($pull)",
        r##"db.${collection}.updateOne({_id: ObjectId("${id}")}, {$pull: {${field}: "${value}"}})"##,
    ),
    CheatEntry::new(
        "Write",
        "Remove a field ($unset)",
        r##"db.${collection}.updateMany({}, {$unset: {${field}: ""}})"##,
    ),
    CheatEntry::new(
        "Write",
        "Rename a field ($rename)",
        r##"db.${collection}.updateMany({}, {$rename: {${field}: "${new_field}"}})"##,
    ),
    CheatEntry::new(
        "Write",
        "Update, or insert if there is none (upsert)",
        r##"db.runCommand({update: "${collection}", updates: [{q: {${field}: "${value}"}, u: {$set: {${other_field}: "${other_value}"}}, upsert: true}]})"##,
    ),
    CheatEntry::new(
        "Write",
        "Update and return the document (findAndModify)",
        r##"db.runCommand({findAndModify: "${collection}", query: {_id: ObjectId("${id}")}, update: {$set: {${field}: "${value}"}}, new: true})"##,
    ),
    CheatEntry::new(
        "Write",
        "Delete one document",
        r##"db.${collection}.deleteOne({_id: ObjectId("${id}")})"##,
    ),
    CheatEntry::new(
        "Write",
        "Delete every document that matches",
        r##"db.${collection}.deleteMany({${field}: "${value}"})"##,
    ),
    // ── Schema ──
    CheatEntry::new(
        "Schema",
        "Create a collection",
        r##"db.runCommand({create: "${new_collection}"})"##,
    ),
    CheatEntry::new(
        "Schema",
        "Create an index",
        r##"db.runCommand({createIndexes: "${collection}", indexes: [{key: {${field}: 1}, name: "${index_name}"}]})"##,
    ),
    CheatEntry::new(
        "Schema",
        "Create a unique index",
        r##"db.runCommand({createIndexes: "${collection}", indexes: [{key: {${field}: 1}, name: "${index_name}", unique: true}]})"##,
    ),
    CheatEntry::new(
        "Schema",
        "Create an index that deletes old documents (TTL)",
        r##"db.runCommand({createIndexes: "${collection}", indexes: [{key: {createdAt: 1}, name: "createdAt_ttl", expireAfterSeconds: 86400}]})"##,
    ),
    CheatEntry::new(
        "Schema",
        "Create a text index",
        r##"db.runCommand({createIndexes: "${collection}", indexes: [{key: {${field}: "text"}, name: "${index_name}"}]})"##,
    ),
    CheatEntry::new(
        "Schema",
        "Drop an index",
        r##"db.runCommand({dropIndexes: "${collection}", index: "${index_name}"})"##,
    ),
    CheatEntry::new(
        "Schema",
        "Rename a collection",
        r##"db.adminCommand({renameCollection: "${database}.${collection}", to: "${database}.${new_collection}"})"##,
    ),
    CheatEntry::new(
        "Schema",
        "Drop a collection",
        r##"db.runCommand({drop: "${collection}"})"##,
    ),
    // ── Performance ──
    CheatEntry::new(
        "Performance",
        "Explain a find (execution statistics)",
        r##"db.${collection}.find({${field}: "${value}"}).explain("executionStats")"##,
    ),
    CheatEntry::new(
        "Performance",
        "Force an index for a find",
        r##"db.${collection}.find({${field}: "${value}"}).hint({${field}: 1})"##,
    ),
];
