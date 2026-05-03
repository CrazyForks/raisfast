#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

TSC="${TSC:-}"
if [ -z "$TSC" ]; then
  for candidate in \
    "$ROOT_DIR/frontend/web/node_modules/.bin/tsc" \
    "$ROOT_DIR/frontend/sdk/node_modules/.bin/tsc" \
    "$ROOT_DIR/node_modules/.bin/tsc"; do
    if [ -x "$candidate" ]; then
      TSC="$candidate"
      break
    fi
  done
fi

if [ -z "$TSC" ]; then
  echo "error: tsc not found. Install typescript or set TSC=/path/to/tsc"
  exit 1
fi

SDK_DIR="$ROOT_DIR/plugin-sdk/js"

echo "Compiling js_plugin_v1.ts → .js + .d.ts ..."
rm -rf "$SDK_DIR/dist"
"$TSC" --project "$SDK_DIR/tsconfig.json"

cp "$SDK_DIR/dist/js_plugin_v1.js" "$SDK_DIR/js_plugin_v1.js"
echo "  → plugin-sdk/js/js_plugin_v1.js"

for dir in "$ROOT_DIR"/extensions/plugins/*/; do
  cp "$SDK_DIR/dist/js_plugin_v1.d.ts" "${dir}sdk.d.ts"
  echo "  → ${dir}sdk.d.ts"
done

rm -rf "$SDK_DIR/dist"
echo "done"
