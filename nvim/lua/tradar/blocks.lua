-- Fenced blocks inside a `.tdb` buffer ("Markdown"-shaped: ``` opens/closes
-- a block, the info string after the opening fence names the dialect and,
-- optionally, which saved connection to run it against) -- see the ".tdb"
-- section in nvim/README.md. Pure line-table parsing, no treesitter: the
-- same lexer-not-parser tradeoff `query_driver.rs`'s own tokenizers make on
-- the Rust side, and it has to run without a tree-sitter-markdown parser
-- installed (highlighting is a separate, independent concern -- see
-- `plugin/tradar.lua`).
local M = {}

--- Every fenced block in `lines` (1-based, matching `nvim_buf_get_lines`'s
--- own Lua array convention), in order. Only a plain ``` fence at column 0
--- counts -- no ~~~~, no indented fences, same restriction real Markdown
--- fences have anyway. An info string is `<dialect>` or `<dialect>
--- <connection>`; `conn` is `nil` when only the dialect is given (the block
--- then falls back to the buffer's own connection -- see
--- `M.connection_for`). A fence left open at EOF (never closed) runs to the
--- last line, same as how an unterminated string is still read to EOF
--- rather than dropped.
---
--- `lang`/`conn` are returned exactly as written, case preserved -- callers
--- that compare them do so case-insensitively if that matters.
function M.parse(lines)
  local out = {}
  local open
  for i, line in ipairs(lines) do
    if open then
      if line:match('^```%s*$') then
        open.finish = i - 1
        out[#out + 1] = open
        open = nil
      end
    else
      local info = line:match('^```(.+)$')
      if info then
        local lang, conn = info:match('^(%S+)%s*(%S*)')
        open = { lang = lang, conn = (conn ~= '' and conn or nil), header_line = i, start = i + 1 }
      end
    end
  end
  if open then
    open.finish = #lines
    out[#out + 1] = open
  end
  return out
end

--- The block whose body (not its opening/closing fence lines) contains
--- 1-based line `line` -- `nil` when `line` is outside every block (plain
--- prose between blocks, or a fence line itself).
function M.at(lines, line)
  for _, b in ipairs(M.parse(lines)) do
    if line >= b.start and line <= b.finish then return b end
  end
end

return M
