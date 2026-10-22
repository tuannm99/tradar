#!/usr/bin/env bash
# Plugins the specs load besides tradar itself, pinned and cloned into
# <repo>/.deps (gitignored) the first time. Idempotent; a failed clone is
# reported here and the specs that need the plugin then fail loudly.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DEPS="$ROOT/.deps"
mkdir -p "$DEPS"

fetch() { # name url tag
  [ -d "$DEPS/$1/.git" ] && return 0
  echo "fetching $1 $3" >&2
  git clone -q --depth 1 --branch "$3" -c advice.detachedHead=false "$2" "$DEPS/$1" || {
    rm -rf "$DEPS/$1"
    echo "could not fetch $1 (specs that load it will fail)" >&2
  }
}

fetch plenary.nvim https://github.com/nvim-lua/plenary.nvim.git v0.1.4
fetch telescope.nvim https://github.com/nvim-telescope/telescope.nvim.git 0.1.8
