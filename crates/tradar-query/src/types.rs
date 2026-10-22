#[derive(Debug, Clone, PartialEq)]
pub struct SchemaInfo {
    pub name: String,
    /// The table's columns, for backends where that's a meaningful concept.
    /// Empty for the document/key-value drivers (MongoDB collections have
    /// no fixed schema, Redis keys have no columns at all), and for any
    /// driver that hasn't been taught to report them yet -- the sidebar
    /// treats an empty list as "nothing to expand", not as an error.
    pub columns: Vec<ColumnInfo>,
    /// The backend's own name for what kind of entry this is (Redis:
    /// `"string"`/`"hash"`/`"list"`/`"set"`/`"zset"`/... from `TYPE`).
    /// `None` for backends where every entry is the same kind (a SQL table,
    /// a Mongo collection) and there's nothing to distinguish -- the browse
    /// sidebar treats `None` as "not applicable".
    pub kind: Option<String>,
    /// Seconds until this entry expires (Redis: `TTL`). `None` for backends
    /// with no expiry concept, and also for a Redis key with no expiry set
    /// (`TTL` returns `-1`) -- the browse sidebar treats both the same way,
    /// as nothing to show.
    pub ttl: Option<i64>,
    /// The schema/database/keyspace this entry belongs to, for a backend
    /// that groups its tables that way (Postgres schema, Cassandra
    /// keyspace, MongoDB database) -- `None` for one that doesn't
    /// (SQLite, Elasticsearch, Redis). `flatten_outline` in
    /// `tradar-query-workbench`'s `query_screen.rs` groups entries by this
    /// and inserts a level in the navigator only when it's `Some`
    /// somewhere in the list, so a driver that never sets it keeps the
    /// flat connection → table → column tree unchanged.
    pub schema: Option<String>,
    /// This entry's kind, for a backend that has more than one kind of
    /// object to browse (Postgres: `"table"`/`"view"`/`"function"`/
    /// `"procedure"`) -- `None` when every entry from this backend is the
    /// same kind, which is every backend but Postgres today. Lowercase,
    /// singular: `flatten_outline` owns turning it into a display label
    /// ("Tables", "Views", ...), so a driver never has to think about
    /// navigator presentation. Same grouping idea as `schema`, one level
    /// deeper -- a driver that never sets it gets no extra folder either.
    pub object_kind: Option<String>,
}

impl SchemaInfo {
    /// A schema entry with no column detail -- what the non-columnar
    /// drivers return.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            columns: Vec::new(),
            kind: None,
            ttl: None,
            schema: None,
            object_kind: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    /// The backend's own type name (`INTEGER`, `character varying`, ...),
    /// passed through rather than normalized: the point is to show what the
    /// database actually says.
    pub type_name: String,
    /// Part of this table's primary key. What makes an edit in the results
    /// grid expressible as SQL: without a key there is no `WHERE` that
    /// names exactly the row the user is pointing at, and a statement that
    /// might hit several rows is not one to run on their behalf.
    pub primary_key: bool,
    /// The table/column this column references via a foreign key, when the
    /// backend has that concept and reports it (Postgres, SQLite today).
    /// `None` for a column with no FK, and always `None` for a backend
    /// without referential integrity (Cassandra, and the document/key-value
    /// drivers) -- completion and any future ERD treat `None` as "nothing
    /// to relate", not as "not yet looked up".
    pub foreign_key: Option<ForeignKeyRef>,
    /// Covered by an index other than the primary key, for a backend that
    /// reports its indexes and doesn't already expose them some other way
    /// (MongoDB today -- Postgres/SQLite/Cassandra's own index concept
    /// isn't surfaced here at all yet, only Mongo's, since `primary_key`
    /// already covers a SQL table's own most useful case). `false` by
    /// default for every driver that hasn't been taught to report it.
    pub indexed: bool,
}

impl ColumnInfo {
    /// A column with no key information -- what the drivers that don't
    /// report one return.
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
            primary_key: false,
            foreign_key: None,
            indexed: false,
        }
    }
}

/// The table/column a `ColumnInfo` points to via a foreign key.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignKeyRef {
    /// The referenced table's name, in the same form `SchemaInfo::name`
    /// uses for that backend (so it can be matched straight against other
    /// `SchemaInfo` entries without reparsing).
    pub table: String,
    pub column: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryResult {
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        /// The backend had more rows than `MAX_ROWS`; what's here is a
        /// prefix. Surfaced in the UI so a truncated result can't be
        /// mistaken for the whole answer.
        truncated: bool,
    },
    Documents {
        items: Vec<serde_json::Value>,
        /// Same meaning as `Table::truncated` -- the backend (Mongo's
        /// `find`/`aggregate` cursor) had more documents than `MAX_ROWS`;
        /// `items` is a prefix. Every other driver that returns `Documents`
        /// (Elasticsearch, Redis, RabbitMQ) already caps itself upstream --
        /// a single `_search`/`SCAN`/queue-peek call, bounded by its own
        /// request size -- so they always pass `false`.
        truncated: bool,
    },
    /// A statement that changed data instead of returning it -- `INSERT`,
    /// `UPDATE`, `DELETE`, DDL. Without this, those came back as an empty
    /// `Table` and rendered as "0 rows", indistinguishable from a `SELECT`
    /// that matched nothing: no way to tell whether the statement ran.
    Affected { rows: u64 },
}

/// One runnable statement inside a buffer that may hold several, plus
/// where it sits -- the offsets are what let "run the statement under the
/// cursor" find the right one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub text: String,
    pub start: usize,
    pub end: usize,
}
