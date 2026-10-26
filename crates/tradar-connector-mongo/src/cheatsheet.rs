//! The MongoDB cheat sheet -- see `QueryDriver::cheatsheet`. mongosh syntax,
//! as this connector reads it.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ─────────────────────────────────────────────────────────
    CheatEntry::new("Explore", "Databases", "show dbs"),
    CheatEntry::new(
        "Explore",
        "Collections of this database",
        "show collections",
    ),
    CheatEntry::new("Explore", "Database statistics", "db.stats()"),
    CheatEntry::new(
        "Explore",
        "Collection statistics",
        "db.runCommand({collStats: \"${collection}\"})",
    ),
    CheatEntry::new(
        "Explore",
        "Index usage of a collection",
        "db.${collection}.aggregate([{$indexStats: {}}])",
    ),
    CheatEntry::new("Explore", "Server version", "db.version()"),
    // ── Server ──────────────────────────────────────────────────────────
    CheatEntry::new("Server", "Server status", "db.serverStatus()"),
    CheatEntry::new(
        "Server",
        "Operations running now",
        "db.adminCommand({currentOp: 1, active: true})",
    ),
    // ── Find ────────────────────────────────────────────────────────────
    CheatEntry::new(
        "Find",
        "All documents of a collection",
        "db.${collection}.find({})",
    ),
    CheatEntry::new(
        "Find",
        "Latest 20 (newest _id first)",
        "db.${collection}.find({}).sort({_id: -1}).limit(20)",
    ),
    CheatEntry::new(
        "Find",
        "By _id",
        "db.${collection}.find({_id: ObjectId(\"${id}\")})",
    ),
    CheatEntry::new(
        "Find",
        "A field equal to a value",
        "db.${collection}.find({${field}: \"${value}\"}).limit(20)",
    ),
    CheatEntry::new(
        "Find",
        "A field matching a pattern (case-insensitive)",
        "db.${collection}.find({${field}: {$regex: \"${pattern}\", $options: \"i\"}}).limit(20)",
    ),
    CheatEntry::new(
        "Find",
        "Documents missing a field",
        "db.${collection}.find({${field}: {$exists: false}}).limit(20)",
    ),
    CheatEntry::new(
        "Find",
        "Created after a date",
        "db.${collection}.find({createdAt: {$gte: ISODate(\"${date}T00:00:00Z\")}}).limit(20)",
    ),
    CheatEntry::new(
        "Find",
        "Only some fields",
        "db.${collection}.find({}, {${field}: 1}).limit(20)",
    ),
    // ── Aggregate ───────────────────────────────────────────────────────
    CheatEntry::new(
        "Aggregate",
        "Count documents",
        "db.${collection}.countDocuments({})",
    ),
    CheatEntry::new(
        "Aggregate",
        "Distinct values of a field",
        "db.${collection}.distinct(\"${field}\")",
    ),
    CheatEntry::new(
        "Aggregate",
        "Top values of a field with their count",
        "db.${collection}.aggregate([{$group: {_id: \"$${field}\", n: {$sum: 1}}}, {$sort: {n: -1}}, {$limit: 20}])",
    ),
    CheatEntry::new(
        "Aggregate",
        "Match, then group",
        "db.${collection}.aggregate([\n  {$match: {${field}: \"${value}\"}},\n  {$group: {_id: \"$${group}\", n: {$sum: 1}}},\n  {$sort: {n: -1}}\n])",
    ),
    // ── Performance ─────────────────────────────────────────────────────
    CheatEntry::new(
        "Performance",
        "Explain a find",
        "db.${collection}.find({${field}: \"${value}\"}).explain(\"executionStats\")",
    ),
];
