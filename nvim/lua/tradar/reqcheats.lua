-- Ready-made request blocks for the workbench (`:TradarCheatsheet` /
-- `<leader>rk` inside it): the shapes of request people keep looking up.
-- `{{name}}` in a block is a variable (see reqparse.lua); the ones that need
-- a value come with an `@name = …` line.
local M = {}

M.http = {
  { category = 'Basics', title = 'GET with query string', text = 'GET /items?limit=10&offset=0\nAccept: application/json' },
  { category = 'Basics', title = 'POST a JSON body', text = 'POST /items\nContent-Type: application/json\nAccept: application/json\n\n{\n  "name": "example"\n}' },
  { category = 'Basics', title = 'PUT (replace)', text = 'PUT /items/{{id}}\nContent-Type: application/json\n\n{\n  "name": "example"\n}' },
  { category = 'Basics', title = 'PATCH (partial update)', text = 'PATCH /items/{{id}}\nContent-Type: application/json\n\n{\n  "name": "renamed"\n}' },
  { category = 'Basics', title = 'DELETE', text = 'DELETE /items/{{id}}' },
  { category = 'Basics', title = 'HEAD (headers only)', text = 'HEAD /items/{{id}}' },
  { category = 'Basics', title = 'OPTIONS (what is allowed)', text = 'OPTIONS /items' },
  { category = 'Auth', title = 'Bearer token', text = '@token = \nGET /me\nAuthorization: Bearer {{token}}' },
  { category = 'Auth', title = 'Basic auth', text = '# the value is base64("user:password"): echo -n user:password | base64\nGET /me\nAuthorization: Basic dXNlcjpwYXNzd29yZA==' },
  { category = 'Auth', title = 'API key header', text = '@key = \nGET /items\nX-API-Key: {{key}}' },
  { category = 'Auth', title = 'Token from the environment', text = 'GET /me\nAuthorization: Bearer {{$env.API_TOKEN}}' },
  { category = 'Bodies', title = 'Form (urlencoded)', text = 'POST /login\nContent-Type: application/x-www-form-urlencoded\n\nusername=ann&password=secret' },
  { category = 'Bodies', title = 'Multipart form', text = 'POST /upload\nContent-Type: multipart/form-data; boundary=tradar\n\n--tradar\nContent-Disposition: form-data; name="note"\n\nhello\n--tradar--' },
  { category = 'Bodies', title = 'Plain text', text = 'POST /echo\nContent-Type: text/plain\n\nhello' },
  { category = 'Bodies', title = 'Unique id and timestamp', text = 'POST /events\nContent-Type: application/json\n\n{\n  "id": "{{$uuid}}",\n  "at": "{{$isoTimestamp}}"\n}' },
  { category = 'Caching', title = 'Conditional GET (ETag)', text = 'GET /items/{{id}}\nIf-None-Match: "etag-from-the-last-response"' },
  { category = 'Caching', title = 'Range request', text = 'GET /big-file\nRange: bytes=0-1023' },
  { category = 'Caching', title = 'No cache', text = 'GET /items\nCache-Control: no-cache' },
  { category = 'Protocols', title = 'GraphQL query', text = 'POST /graphql\nContent-Type: application/json\n\n{\n  "query": "{ viewer { id name } }",\n  "variables": {}\n}' },
  { category = 'Protocols', title = 'JSON-RPC 2.0 call', text = 'POST /rpc\nContent-Type: application/json\n\n{\n  "jsonrpc": "2.0",\n  "id": 1,\n  "method": "ping",\n  "params": []\n}' },
  { category = 'Protocols', title = 'CORS preflight', text = 'OPTIONS /items\nOrigin: https://example.org\nAccess-Control-Request-Method: POST\nAccess-Control-Request-Headers: content-type' },
  { category = 'Health', title = 'Health check', text = 'GET /health' },
  { category = 'Health', title = 'OpenAPI document', text = 'GET /openapi.json' },
}

M.grpc = {
  { category = 'Health', title = 'Standard health check', text = '### Health.Check\nGRPC grpc.health.v1.Health/Check\ndeadline: 3s\n\n{\n  "service": ""\n}' },
  { category = 'Metadata', title = 'Bearer token in metadata', text = '@token = \n### with auth\nGRPC pkg.Service/Method\nauthorization: Bearer {{token}}\n\n{}' },
  { category = 'Metadata', title = 'Custom metadata', text = '### with metadata\nGRPC pkg.Service/Method\nx-request-id: {{$uuid}}\nx-tenant: acme\n\n{}' },
  { category = 'Metadata', title = 'Token from the environment', text = '### with auth\nGRPC pkg.Service/Method\nauthorization: Bearer {{$env.API_TOKEN}}\n\n{}' },
  { category = 'Deadlines', title = 'Deadline of 5 seconds', text = '### with deadline\nGRPC pkg.Service/Method\ndeadline: 5s\n\n{}' },
  { category = 'Deadlines', title = 'Deadline of 500 ms', text = '### with deadline\nGRPC pkg.Service/Method\ndeadline: 500ms\n\n{}' },
  { category = 'Bodies', title = 'Well-known types', text = '### well-known types\nGRPC pkg.Service/Method\n\n{\n  "created_at": "2026-01-01T00:00:00Z",\n  "timeout": "30s",\n  "mask": "name,email",\n  "labels": {"env": "dev"}\n}' },
  { category = 'Bodies', title = 'Repeated field and enum', text = '### repeated and enum\nGRPC pkg.Service/Method\n\n{\n  "ids": ["1", "2"],\n  "level": "LEVEL_HIGH"\n}' },
  { category = 'Streams', title = 'Server stream (watch)', text = '# rr starts it; the replies fill the result tab. rx stops it.\n### Watch\nGRPC pkg.Service/Watch\n\n{}' },
  { category = 'Streams', title = 'Client / bidi stream', text = '# rr starts the call and sends this message; rr again sends the next one,\n# rc closes the send side, rx stops.\n### Chat\nGRPC pkg.Service/Chat\n\n{\n  "text": "hello"\n}' },
}

--- The entries for a driver, in the shape the cheat sheet pickers use
--- (`category`, `title`, `text`).
function M.for_driver(driver)
  return M[driver] or {}
end

return M
