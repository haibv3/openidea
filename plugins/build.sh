#!/usr/bin/env bash
# Builds Rust plugins into WebAssembly components (wasm32-wasip2 emits a
# component directly; no cargo-component / wasm-tools needed).
#
#   plugins/build.sh            # all plugins with a Cargo.toml
#   plugins/build.sh hello      # one plugin, by name under plugins/
#   plugins/build.sh crates/ide-plugin/tests/fixtures/preview   # by path
#
# Output: <plugin>/dist/plugin.wasm (referenced by `wasm = ...` in plugin.toml).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
rustup target list --installed | grep -qx wasm32-wasip2 || rustup target add wasm32-wasip2

build() {
  local dir="$1"
  local name
  name="$(sed -n 's/^name *= *"\(.*\)"/\1/p' "$dir/Cargo.toml" | head -1)"
  echo "building $name ($dir)"
  CARGO_TARGET_DIR="$dir/target" cargo build --manifest-path "$dir/Cargo.toml" --release --target wasm32-wasip2
  mkdir -p "$dir/dist"
  cp "$dir/target/wasm32-wasip2/release/${name//-/_}.wasm" "$dir/dist/plugin.wasm"
  echo "  -> $dir/dist/plugin.wasm ($(wc -c < "$dir/dist/plugin.wasm" | tr -d " ") bytes)"
}

if [ $# -gt 0 ]; then
  for p in "$@"; do
    case "$p" in
      /*) build "$p" ;;
      # A name under plugins/, else a path relative to the working directory.
      "$here"/*|*/*) build "$p" ;;
      *) build "$here/$p" ;;
    esac
  done
else
  for manifest in "$here"/*/Cargo.toml; do build "$(dirname "$manifest")"; done
fi
