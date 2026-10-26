//! The Redis cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Server ──────────────────────────────────────────────────────────
    CheatEntry::new("Server", "Ping", "PING"),
    CheatEntry::new("Server", "Server info", "INFO server"),
    CheatEntry::new("Server", "Memory usage", "INFO memory"),
    CheatEntry::new("Server", "Keys per database", "INFO keyspace"),
    CheatEntry::new("Server", "Number of keys in this database", "DBSIZE"),
    CheatEntry::new("Server", "Clients connected", "CLIENT LIST"),
    CheatEntry::new("Server", "Slowest commands (slow log)", "SLOWLOG GET 10"),
    CheatEntry::new(
        "Server",
        "Config values matching a pattern",
        "CONFIG GET ${pattern}",
    ),
    // ── Keys ────────────────────────────────────────────────────────────
    CheatEntry::new(
        "Keys",
        "Find keys by pattern (SCAN, not KEYS)",
        "SCAN 0 MATCH ${pattern} COUNT 100",
    ),
    CheatEntry::new("Keys", "Type of a key", "TYPE ${key}"),
    CheatEntry::new("Keys", "Time to live of a key", "TTL ${key}"),
    CheatEntry::new("Keys", "Memory used by a key", "MEMORY USAGE ${key}"),
    CheatEntry::new(
        "Keys",
        "Internal encoding of a key",
        "OBJECT ENCODING ${key}",
    ),
    // ── Read a value ────────────────────────────────────────────────────
    CheatEntry::new("Read", "String", "GET ${key}"),
    CheatEntry::new("Read", "Hash (all fields)", "HGETALL ${key}"),
    CheatEntry::new("Read", "List (all items)", "LRANGE ${key} 0 -1"),
    CheatEntry::new("Read", "Set (all members)", "SMEMBERS ${key}"),
    CheatEntry::new(
        "Read",
        "Sorted set with scores",
        "ZRANGE ${key} 0 -1 WITHSCORES",
    ),
    CheatEntry::new(
        "Read",
        "Stream (first 10 entries)",
        "XRANGE ${key} - + COUNT 10",
    ),
];
