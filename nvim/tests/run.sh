#!/usr/bin/env bash
# Runs every nvim/tests/*_spec.lua in its own headless Neovim, each against
# its own tradar-server, config dir and empty SQLite database (connection
# "local"). Usage: nvim/tests/run.sh [name-filter]
# TRADAR_SERVER (default target/debug/tradar-server) and NVIM override the
# binaries. A spec may ask for server env with a `-- env: K=V ...` line.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SERVER=${TRADAR_SERVER:-$ROOT/target/debug/tradar-server}
NVIM=${NVIM:-nvim}
if [ ! -x "$SERVER" ]; then
  echo "no tradar-server at $SERVER -- cargo build -p tradar-server first" >&2
  exit 2
fi

filter=${1:-}
specs=0 failed=0
for spec in "$ROOT"/nvim/tests/*_spec.lua; do
  name=$(basename "$spec" .lua)
  if [ -n "$filter" ] && [[ "$name" != *"$filter"* ]]; then continue; fi
  specs=$((specs + 1))
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/tradar-test.XXXXXX")
  mkdir -p "$tmp/config/tradar"
  printf '[[connections]]\nname = "local"\ndriver = "sqlite"\ntarget = "%s"\n' "$tmp/t.db" \
    >"$tmp/config/tradar/connections.toml"
  read -r -a extra_env <<<"$(sed -n 's/^-- env: //p' "$spec" | head -1)"
  export_env=(XDG_CONFIG_HOME="$tmp/config" XDG_STATE_HOME="$tmp/state" TRADAR_LOG_FILE="$tmp/server.log")
  sock=$tmp/s.sock

  env "${export_env[@]}" "${extra_env[@]}" "$SERVER" "$sock" >"$tmp/server.out" 2>&1 &
  pid=$!
  for _ in $(seq 100); do [ -S "$sock" ] && break; sleep 0.05; done

  (cd "$ROOT" && env "${export_env[@]}" "${extra_env[@]}" TRADAR_SOCKET="$sock" TRADAR_TEST_TMP="$tmp" \
    timeout 180 "$NVIM" --headless -u NONE -i NONE \
    -c "lua local ok, e = pcall(dofile, '$spec') if not ok then io.stderr:write('spec error: ' .. tostring(e) .. '\n') vim.cmd('cquit 3') end" \
    </dev/null >"$tmp/nvim.out" 2>&1)
  code=$?
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null

  results=$(cat "$tmp/results" 2>/dev/null)
  sed -n 's/^\(PASS\|FAIL\) /  \1 /p' <<<"$results" | sed "s/^/$name:/"
  if [ $code -ne 0 ] || grep -q '^FAIL ' <<<"$results" || ! grep -q '^DONE$' <<<"$results"; then
    failed=$((failed + 1))
    echo "$name: FAILED (exit $code)"
    echo "--- nvim output"; tail -20 "$tmp/nvim.out"
    echo "--- server log"; tail -20 "$tmp/server.log" 2>/dev/null
  else
    rm -rf "$tmp"
  fi
done

echo "$((specs - failed))/$specs specs passed"
[ $specs -gt 0 ] && [ $failed -eq 0 ]
