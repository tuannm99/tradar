//! The Redis cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Server ──
    CheatEntry::new("Server", "Ping", r##"PING"##),
    CheatEntry::new("Server", "Server info", r##"INFO server"##),
    CheatEntry::new("Server", "Memory usage", r##"INFO memory"##),
    CheatEntry::new("Server", "Clients connected (summary)", r##"INFO clients"##),
    CheatEntry::new(
        "Server",
        "Statistics (hits, misses, evictions)",
        r##"INFO stats"##,
    ),
    CheatEntry::new("Server", "Persistence (RDB / AOF)", r##"INFO persistence"##),
    CheatEntry::new("Server", "Replication", r##"INFO replication"##),
    CheatEntry::new("Server", "Keys per database", r##"INFO keyspace"##),
    CheatEntry::new(
        "Server",
        "Calls and time per command",
        r##"INFO commandstats"##,
    ),
    CheatEntry::new("Server", "Number of keys in this database", r##"DBSIZE"##),
    CheatEntry::new(
        "Server",
        "Role of this server (master or replica)",
        r##"ROLE"##,
    ),
    CheatEntry::new("Server", "Server time", r##"TIME"##),
    CheatEntry::new("Server", "Last successful save", r##"LASTSAVE"##),
    CheatEntry::new("Server", "Latest latency spikes", r##"LATENCY LATEST"##),
    CheatEntry::new("Server", "Memory report", r##"MEMORY STATS"##),
    CheatEntry::new("Server", "Memory advice", r##"MEMORY DOCTOR"##),
    // ── Clients ──
    CheatEntry::new("Clients", "Client list", r##"CLIENT LIST"##),
    CheatEntry::new("Clients", "Who am I connected as", r##"ACL WHOAMI"##),
    CheatEntry::new(
        "Clients",
        "Close a client by id",
        r##"CLIENT KILL ID ${id}"##,
    ),
    // ── Slow log & config ──
    CheatEntry::new(
        "Slow log & config",
        "Slowest commands (slow log)",
        r##"SLOWLOG GET 10"##,
    ),
    CheatEntry::new(
        "Slow log & config",
        "Number of slow log entries",
        r##"SLOWLOG LEN"##,
    ),
    CheatEntry::new(
        "Slow log & config",
        "Clear the slow log",
        r##"SLOWLOG RESET"##,
    ),
    CheatEntry::new(
        "Slow log & config",
        "Config values matching a pattern",
        r##"CONFIG GET ${pattern}"##,
    ),
    CheatEntry::new(
        "Slow log & config",
        "Change a config value",
        r##"CONFIG SET ${parameter} ${value}"##,
    ),
    CheatEntry::new(
        "Slow log & config",
        "Memory limit and eviction policy",
        r##"CONFIG GET maxmemory*"##,
    ),
    CheatEntry::new("Slow log & config", "Access control rules", r##"ACL LIST"##),
    // ── Keys ──
    CheatEntry::new(
        "Keys",
        "Find keys by pattern (SCAN, not KEYS)",
        r##"SCAN 0 MATCH ${pattern} COUNT 100"##,
    ),
    CheatEntry::new(
        "Keys",
        "Find keys of one type",
        r##"SCAN 0 TYPE ${type} COUNT 100"##,
    ),
    CheatEntry::new("Keys", "Does a key exist", r##"EXISTS ${key}"##),
    CheatEntry::new("Keys", "Type of a key", r##"TYPE ${key}"##),
    CheatEntry::new("Keys", "Time to live of a key (seconds)", r##"TTL ${key}"##),
    CheatEntry::new(
        "Keys",
        "Time to live of a key (milliseconds)",
        r##"PTTL ${key}"##,
    ),
    CheatEntry::new("Keys", "Memory used by a key", r##"MEMORY USAGE ${key}"##),
    CheatEntry::new(
        "Keys",
        "Internal encoding of a key",
        r##"OBJECT ENCODING ${key}"##,
    ),
    CheatEntry::new(
        "Keys",
        "How long a key has not been used",
        r##"OBJECT IDLETIME ${key}"##,
    ),
    CheatEntry::new("Keys", "A random key", r##"RANDOMKEY"##),
    // ── Read ──
    CheatEntry::new("Read", "String", r##"GET ${key}"##),
    CheatEntry::new("Read", "Several strings", r##"MGET ${key} ${other_key}"##),
    CheatEntry::new("Read", "Length of a string", r##"STRLEN ${key}"##),
    CheatEntry::new("Read", "Part of a string", r##"GETRANGE ${key} 0 99"##),
    CheatEntry::new("Read", "Hash (all fields)", r##"HGETALL ${key}"##),
    CheatEntry::new("Read", "One field of a hash", r##"HGET ${key} ${field}"##),
    CheatEntry::new("Read", "Field names of a hash", r##"HKEYS ${key}"##),
    CheatEntry::new("Read", "Number of fields in a hash", r##"HLEN ${key}"##),
    CheatEntry::new("Read", "List (all items)", r##"LRANGE ${key} 0 -1"##),
    CheatEntry::new("Read", "List (first 20 items)", r##"LRANGE ${key} 0 19"##),
    CheatEntry::new("Read", "Length of a list", r##"LLEN ${key}"##),
    CheatEntry::new("Read", "Set (all members)", r##"SMEMBERS ${key}"##),
    CheatEntry::new("Read", "Number of members in a set", r##"SCARD ${key}"##),
    CheatEntry::new(
        "Read",
        "Is a value in a set",
        r##"SISMEMBER ${key} ${member}"##,
    ),
    CheatEntry::new(
        "Read",
        "A few random members of a set",
        r##"SRANDMEMBER ${key} 5"##,
    ),
    CheatEntry::new(
        "Read",
        "Sorted set with scores",
        r##"ZRANGE ${key} 0 -1 WITHSCORES"##,
    ),
    CheatEntry::new(
        "Read",
        "Sorted set, top 10 by score",
        r##"ZRANGE ${key} 0 9 REV WITHSCORES"##,
    ),
    CheatEntry::new(
        "Read",
        "Sorted set, members within a score range",
        r##"ZRANGEBYSCORE ${key} -inf +inf WITHSCORES LIMIT 0 50"##,
    ),
    CheatEntry::new("Read", "Score of a member", r##"ZSCORE ${key} ${member}"##),
    CheatEntry::new(
        "Read",
        "Position of a member (lowest score is 0)",
        r##"ZRANK ${key} ${member}"##,
    ),
    CheatEntry::new(
        "Read",
        "Number of members in a sorted set",
        r##"ZCARD ${key}"##,
    ),
    CheatEntry::new(
        "Read",
        "Stream (first 10 entries)",
        r##"XRANGE ${key} - + COUNT 10"##,
    ),
    CheatEntry::new(
        "Read",
        "Stream (latest 10 entries)",
        r##"XREVRANGE ${key} + - COUNT 10"##,
    ),
    CheatEntry::new("Read", "Number of entries in a stream", r##"XLEN ${key}"##),
    CheatEntry::new("Read", "Stream details", r##"XINFO STREAM ${key}"##),
    CheatEntry::new(
        "Read",
        "Consumer groups of a stream",
        r##"XINFO GROUPS ${key}"##,
    ),
    CheatEntry::new(
        "Read",
        "Approximate unique count (HyperLogLog)",
        r##"PFCOUNT ${key}"##,
    ),
    // ── Write ──
    CheatEntry::new("Write", "Set a string", r##"SET ${key} ${value}"##),
    CheatEntry::new(
        "Write",
        "Set a string that expires",
        r##"SET ${key} ${value} EX 3600"##,
    ),
    CheatEntry::new(
        "Write",
        "Set a string only if the key is new",
        r##"SET ${key} ${value} NX"##,
    ),
    CheatEntry::new(
        "Write",
        "Set several strings",
        r##"MSET ${key} ${value} ${other_key} ${other_value}"##,
    ),
    CheatEntry::new("Write", "Add to a number", r##"INCRBY ${key} ${amount}"##),
    CheatEntry::new("Write", "Count up by one", r##"INCR ${key}"##),
    CheatEntry::new("Write", "Append to a string", r##"APPEND ${key} ${value}"##),
    CheatEntry::new(
        "Write",
        "Set a hash field",
        r##"HSET ${key} ${field} ${value}"##,
    ),
    CheatEntry::new("Write", "Remove a hash field", r##"HDEL ${key} ${field}"##),
    CheatEntry::new(
        "Write",
        "Add to a number in a hash",
        r##"HINCRBY ${key} ${field} ${amount}"##,
    ),
    CheatEntry::new(
        "Write",
        "Push onto the end of a list",
        r##"RPUSH ${key} ${value}"##,
    ),
    CheatEntry::new(
        "Write",
        "Push onto the start of a list",
        r##"LPUSH ${key} ${value}"##,
    ),
    CheatEntry::new("Write", "Pop from the start of a list", r##"LPOP ${key}"##),
    CheatEntry::new("Write", "Pop from the end of a list", r##"RPOP ${key}"##),
    CheatEntry::new("Write", "Add to a set", r##"SADD ${key} ${member}"##),
    CheatEntry::new("Write", "Remove from a set", r##"SREM ${key} ${member}"##),
    CheatEntry::new(
        "Write",
        "Add to a sorted set",
        r##"ZADD ${key} ${score} ${member}"##,
    ),
    CheatEntry::new(
        "Write",
        "Remove from a sorted set",
        r##"ZREM ${key} ${member}"##,
    ),
    CheatEntry::new(
        "Write",
        "Add to a score in a sorted set",
        r##"ZINCRBY ${key} ${amount} ${member}"##,
    ),
    CheatEntry::new(
        "Write",
        "Add an entry to a stream",
        r##"XADD ${key} * ${field} ${value}"##,
    ),
    CheatEntry::new(
        "Write",
        "Delete a stream entry",
        r##"XDEL ${key} ${entry_id}"##,
    ),
    CheatEntry::new(
        "Write",
        "Set a time to live (seconds)",
        r##"EXPIRE ${key} ${seconds}"##,
    ),
    CheatEntry::new("Write", "Remove the time to live", r##"PERSIST ${key}"##),
    CheatEntry::new("Write", "Rename a key", r##"RENAME ${key} ${new_key}"##),
    CheatEntry::new("Write", "Delete a key", r##"DEL ${key}"##),
    CheatEntry::new(
        "Write",
        "Delete a key without blocking (UNLINK)",
        r##"UNLINK ${key}"##,
    ),
    // ── Pub/Sub ──
    CheatEntry::new(
        "Pub/Sub",
        "Publish a message",
        r##"PUBLISH ${channel} ${message}"##,
    ),
    CheatEntry::new(
        "Pub/Sub",
        "Active channels",
        r##"PUBSUB CHANNELS ${pattern}"##,
    ),
    CheatEntry::new(
        "Pub/Sub",
        "Subscribers of a channel",
        r##"PUBSUB NUMSUB ${channel}"##,
    ),
    // ── Admin ──
    CheatEntry::new("Admin", "Save to disk in the background", r##"BGSAVE"##),
    CheatEntry::new("Admin", "Free unused memory", r##"MEMORY PURGE"##),
    CheatEntry::new(
        "Admin",
        "Delete every key of this database (asynchronously)",
        r##"FLUSHDB ASYNC"##,
    ),
];
