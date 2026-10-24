-- Which tab a statement lands in, for what each connector writes (its CRUD
-- snippets and browse commands -- see the `crud_snippet` tests in the
-- connector crates) and for hand-written ones.
local H = dofile('nvim/tests/helpers.lua')
H.setup({})
local slots = require('tradar.slots')

local function target(name, query, want)
  local got = slots.target_of(query)
  H.check(name, got == want, ('%q -> %s (want %s)'):format(query, tostring(got), tostring(want)))
end

-- SQL: crud snippets of postgres / mysql / sqlite / clickhouse / cassandra
target('SQL: quoted table', 'SELECT * FROM "users" LIMIT 100;', 'users')
target('SQL: quoted schema.table', 'SELECT * FROM "public"."users" LIMIT 100;', 'public.users')
target('SQL: cassandra keyspace.table', 'SELECT * FROM "demo"."events" LIMIT 100;', 'demo.events')
target('SQL: backticks (clickhouse / mysql)', 'SELECT * FROM `db`.`events` LIMIT 100', 'db.events')
target('SQL: lower case, newline before FROM', 'select a,\n b\nfrom Orders o\nwhere o.id = 1', 'Orders')
target('SQL: a subquery has no single table', 'select * from (select 1) t', nil)
target('SQL: INSERT / UPDATE / DELETE are not tabs of a table', "UPDATE users SET a = 1 WHERE id = 2", nil)
target('SQL: SHOW / EXPLAIN / PRAGMA have none', 'SHOW TABLES', nil)
target('SQL: a CTE has none', 'with x as (select 1) select * from x', nil)

-- Mongo: crud_snippet and shell calls
target('Mongo: find', 'db.users.find({})', 'users')
target('Mongo: insertOne', 'db.users.insertOne({})', 'users')
target('Mongo: updateOne', 'db.users.updateOne({_id: ObjectId("<id>")}, {$set: {}})', 'users')
target('Mongo: aggregate over lines', 'db.orders.aggregate([\n  {$match: {}}\n])', 'orders')
target('Mongo: countDocuments', 'db.events.countDocuments({})', 'events')
target('Mongo: db.stats() has none', 'db.stats()', nil)
target('Mongo: db.getCollectionNames() has none', 'db.getCollectionNames()', nil)
target('Mongo: runCommand has none', 'db.runCommand({ ping: 1 })', nil)
target('Mongo: a comment line first', '// recent\ndb.users.find({})', 'users')

-- Elasticsearch: crud_snippet writes no leading slash
target('ES: GET index/_search', 'GET my-index/_search\n{\n  "query": {}\n}', 'my-index')
target('ES: POST index/_doc', 'POST my-index/_doc\n{\n}', 'my-index')
target('ES: DELETE index/_doc/id', 'DELETE my-index/_doc/<id>', 'my-index')
target('ES: leading slash', 'GET /logs-2024.01/_count', 'logs-2024.01')
target('ES: wildcard / list of indices', 'GET logs-*,metrics/_search', 'logs-*,metrics')
target('ES: _cat has none', 'GET _cat/indices?v', nil)
target('ES: cluster health has none', 'GET /_cluster/health', nil)
target('ES: a bare slash has none', 'GET /', nil)

-- Redis: browse commands and snippets
target('Redis: GET', 'GET session:42', 'session:42')
target('Redis: HGETALL', 'HGETALL user:1', 'user:1')
target('Redis: LRANGE', 'LRANGE queue 0 -1', 'queue')
target('Redis: ZRANGE WITHSCORES', 'ZRANGE board 0 -1 WITHSCORES', 'board')
target('Redis: a key with a space is quoted by the connector', 'GET "has space"', 'has space')
target('Redis: ...and with a quote', 'HGETALL "quo\\"te"', 'quo"te')
target('Redis: lower case', 'smembers tags', 'tags')
target('Redis: KEYS has none', 'KEYS user:*', nil)
target('Redis: PING has none', 'PING', nil)
target('Redis: writes have none', 'HSET user:1 a b', nil)

-- the key joins connection and target
H.check('same target on two connections: two tabs', slots.key('a', 'GET k') ~= slots.key('b', 'GET k'))
H.check('Redis key and Mongo collection of one name do not collide per connection', slots.key('r', 'GET users') ~= slots.key('m', 'db.users.find({})'))
H.check('a statement without a target is its own tab', slots.key('c', 'SHOW TABLES') ~= slots.key('c', 'SHOW DATABASES'))
H.check('...and rerunning it lands in the same one', slots.key('c', 'SHOW  TABLES') == slots.key('c', 'SHOW TABLES'))
H.done()
