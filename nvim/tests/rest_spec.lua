-- The request workbench's text format (reqparse) and the JSON re-indenter (jsonfmt).
local H = dofile('nvim/tests/helpers.lua')
H.setup()
local rp = require('tradar.reqparse')
local jf = require('tradar.jsonfmt')

local function parse(text, row)
  local lines = vim.split(text, '\n', { plain = true })
  return rp.parse(lines, rp.block_at(lines, row or 1))
end

-- blocks
local lines = vim.split('# c\n@a = 1\n### one\nGET /a\n\n### two\nPOST /b\n', '\n', { plain = true })
local blocks = rp.blocks(lines)
H.check('### splits blocks; the text before the first is a block', #blocks == 3 and blocks[1].first == 1 and blocks[1].last == 2, vim.inspect(blocks))
H.check('a block starts on its ### line and carries the title', blocks[2].first == 3 and blocks[2].last == 5 and blocks[2].title == 'one' and blocks[3].title == 'two', vim.inspect(blocks))
H.check('block_at finds the block of a line', rp.block_at(lines, 4).title == 'one' and rp.block_at(lines, 6).title == 'two')
H.check('a file starting with ### has no leading empty block', #rp.blocks({ '### x', 'GET /' }) == 1)

-- http
local r = parse('### list\nGET /users?limit=10\nAccept: application/json\nX-A:  b c \n\n{"a": 1}\n\n')
H.check('http: method, url, title', r and r.kind == 'http' and r.method == 'GET' and r.url == '/users?limit=10' and r.title == 'list', vim.inspect(r))
H.check('http: headers keep their order and trim values', r.headers[1][1] == 'Accept' and r.headers[2][1] == 'X-A' and r.headers[2][2] == 'b c', vim.inspect(r.headers))
H.check('http: body without trailing blank lines', r.body == '{"a": 1}', vim.inspect(r.body))
r = parse('/users')
H.check('http: a bare url is a GET', r and r.method == 'GET' and r.url == '/users')
r = parse('post /x HTTP/1.1')
H.check('http: verb is case-insensitive and HTTP/1.1 is dropped', r and r.method == 'POST' and r.url == '/x', vim.inspect(r))
r = parse('# note\n// other\n@v = 1\nDELETE /x/1')
H.check('http: comments and variables before the request line are skipped', r and r.method == 'DELETE' and r.line == 4, vim.inspect(r))
r = parse('POST /x\n\n# not a comment\n// nor this\nline')
H.check('http: the body is verbatim', r.body == '# not a comment\n// nor this\nline', vim.inspect(r.body))
local bad, why = parse('POST /x\n{"a":1}')
H.check('http: a body glued to the request line says to add a blank line', bad == nil and why:find('blank line', 1, true) ~= nil and why:find('line 2', 1, true) ~= nil, why)
bad, why = parse('# only a comment\n@a = 1')
H.check('a block without a request says so', bad == nil and why:find('no request', 1, true) ~= nil, why)

-- grpc
r = parse('### get\nGRPC users.UserService/GetUser\nx-Token: abc\ndeadline: 5s\n\n// hint\n{"id": "42"}')
H.check('grpc: service and method', r and r.kind == 'grpc' and r.service == 'users.UserService' and r.method == 'GetUser', vim.inspect(r))
H.check('grpc: metadata is lowercased, deadline is not metadata', r.metadata['x-token'] == 'abc' and r.metadata.deadline == nil and r.timeout_ms == 5000, vim.inspect(r))
H.check('grpc: the body is decoded and // lines are dropped', r.body.id == '42', vim.inspect(r.body))
r = parse('GRPC a.B/C\ndeadline: 500ms\n')
H.check('grpc: no body is an empty message; 500ms deadline', r and r.timeout_ms == 500 and next(r.body) == nil, vim.inspect(r))
bad, why = parse('GRPC a.B/C\ndeadline: soon')
H.check('grpc: a bad deadline is explained', bad == nil and why:find('bad deadline', 1, true) ~= nil, why)
bad, why = parse('GRPC a.B/C\n\n{nope')
H.check('grpc: invalid JSON is explained', bad == nil and why:find('not valid JSON', 1, true) ~= nil, why)
bad, why = parse('GRPC GetUser')
H.check('grpc: GRPC needs service/Method', bad == nil and why:find('service/Method', 1, true) ~= nil, why)

-- variables
local text = '@host = example.org\n@url = https://{{host}}/v1\n@token = t0k\n\nGET {{url}}/users\nAuthorization: Bearer {{token}}\n\n{"id": "{{$uuid}}", "t": {{$timestamp}}}'
r = parse(text, 5)
H.check('variables: expand in the url (also through other variables), headers and body', r and r.url == 'https://example.org/v1/users' and r.headers[1][2] == 'Bearer t0k', vim.inspect(r))
H.check('variables: $uuid and $timestamp expand', r.body:match('"id": "%x%x%x%x%x%x%x%x%-%x%x%x%x%-4%x%x%x%-[89ab]%x%x%x%-%x+"') ~= nil and r.body:match('"t": %d+}') ~= nil, r.body)
bad, why = parse('GET /{{nope}}')
H.check('variables: an undefined one is named', bad == nil and why:find('{{nope}}', 1, true) ~= nil, why)
vim.env.TRADAR_SPEC_VAR = 'from-env'
r = parse('GET /{{$env.TRADAR_SPEC_VAR}}')
H.check('variables: $env.NAME reads the environment', r and r.url == '/from-env', vim.inspect(r))
bad, why = parse('GET /{{$env.TRADAR_SPEC_UNSET_VAR}}')
H.check('variables: an unset environment variable is named', bad == nil and why:find('TRADAR_SPEC_UNSET_VAR', 1, true) ~= nil, why)

-- tabs
local key1, label1 = rp.slot(parse('GET /users?limit=5'), 'web')
local key2 = rp.slot(parse('GET https://h.io/users'), 'web')
local key3 = rp.slot(parse('POST /users'), 'web')
H.check('same method and path share a tab, a query string or host does not split it', key1 == key2 and label1 == 'GET /users', key1 .. ' / ' .. key2)
H.check('another method has its own tab', key3 ~= key1)
local gkey, glabel = rp.slot(parse('GRPC a.B/C'), 'g')
H.check('a gRPC method names its tab', glabel == 'C' and gkey:find('a.B/C', 1, true) ~= nil, gkey)

-- copy as
local curl = rp.as_curl(parse('POST /users\nContent-Type: application/json\n\n{"a": "it\'s"}'), 'https://api.io/')
H.check('curl: method, url joined to the base, header, body', curl:find("-X POST", 1, true) and curl:find("'https://api.io/users'", 1, true) and curl:find("-H 'Content-Type: application/json'", 1, true) and curl:find("--data-raw '{\"a\": \"it'\\''s\"}'", 1, true), curl)
H.check('curl: a GET has no -X', not rp.as_curl(parse('GET /x'), ''):find('-X', 1, true))
local gc = rp.as_grpcurl(parse('GRPC a.B/C\nx-t: 1\ndeadline: 2s\n\n{"id": 7}'), 'localhost:50051')
H.check('grpcurl: plaintext, metadata, deadline, body, target, method', gc:find('-plaintext', 1, true) and gc:find("-H 'x-t: 1'", 1, true) and gc:find('-max-time 2', 1, true) and gc:find('{"id":7}', 1, true) and gc:find('localhost:50051', 1, true) and gc:find('a.B/C', 1, true), gc)
H.check('grpcurl: https is not plaintext', not rp.as_grpcurl(parse('GRPC a.B/C'), 'https://h:443'):find('-plaintext', 1, true))

-- jsonfmt
H.check('jsonfmt keeps key order and big numbers', jf.pretty('{"b":1,"a":12345678901234567890,"c":[1,2,{"d":null}],"e":{},"f":[]}') ==
  '{\n  "b": 1,\n  "a": 12345678901234567890,\n  "c": [\n    1,\n    2,\n    {\n      "d": null\n    }\n  ],\n  "e": {},\n  "f": []\n}', jf.pretty('{"b":1,"a":12345678901234567890,"c":[1,2,{"d":null}],"e":{},"f":[]}'))
H.check('jsonfmt leaves strings alone', jf.pretty('{"s":"a, b: {c} \\" \\\\"}') == '{\n  "s": "a, b: {c} \\" \\\\"\n}', jf.pretty('{"s":"a, b: {c} \\" \\\\"}'))
H.check('jsonfmt: a scalar', jf.pretty(' 42 ') == '42')
H.check('jsonfmt: not JSON is nil', jf.pretty('<html>') == nil and jf.pretty('') == nil and jf.pretty('{"a":') == nil)
H.done()
