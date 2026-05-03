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

# ── 1. 校验：JS host 函数 vs TS SDK ──

JS_HOST="$ROOT_DIR/src/plugins/js_host.rs"
TS_SDK="$ROOT_DIR/plugin-sdk/js/js_plugin_v1.ts"
CONSTANTS="$ROOT_DIR/src/constants.rs"

# 从 js_host.rs 提取 host.set("functionName", ...) 的函数名
js_host_fns=$(grep -oE 'host\.set\("([a-zA-Z]+)"' "$JS_HOST" | sed 's/host\.set("//;s/"//' | sort -u)

# 从 TS SDK 提取 RaisFastHost.functionName 调用
ts_sdk_fns=$(grep -oE 'RaisFastHost\.([a-zA-Z]+)\(' "$TS_SDK" | sed 's/RaisFastHost\.//;s/(//' | sort -u)

# 从 constants.rs 提取全局名
global_name=$(grep 'PLUGIN_HOST_GLOBAL' "$CONSTANTS" | grep -oE '"([^"]+)"' | sed 's/"//g')

# 从 TS SDK 提取 declare const 行
ts_global_name=$(grep -oE 'declare const ([A-Za-z]+):' "$TS_SDK" | sed 's/declare const //;s/://')

echo "Checking JS host ($JS_HOST) vs TS SDK ($TS_SDK) ..."

errors=0

# 检查全局名一致性
if [ "$global_name" != "$ts_global_name" ]; then
  echo "  error: global name mismatch: constants.rs='$global_name', ts_sdk='$ts_global_name'"
  errors=$((errors + 1))
else
  echo "  ok: global name = $global_name"
fi

# 检查 Rust 端有但 TS 端没有的函数
missing_in_ts=""
for fn in $js_host_fns; do
  if ! echo "$ts_sdk_fns" | grep -qx "$fn"; then
    missing_in_ts="$missing_in_ts $fn"
  fi
done
if [ -n "$missing_in_ts" ]; then
  echo "  error: functions in js_host.rs but NOT in TS SDK:$missing_in_ts"
  errors=$((errors + 1))
fi

# 检查 TS 端有但 Rust 端没有的函数
missing_in_rs=""
for fn in $ts_sdk_fns; do
  if ! echo "$js_host_fns" | grep -qx "$fn"; then
    missing_in_rs="$missing_in_rs $fn"
  fi
done
if [ -n "$missing_in_rs" ]; then
  echo "  error: functions in TS SDK but NOT in js_host.rs:$missing_in_rs"
  errors=$((errors + 1))
fi

host_fn_count=$(echo "$js_host_fns" | wc -l | tr -d ' ')
sdk_fn_count=$(echo "$ts_sdk_fns" | wc -l | tr -d ' ')
echo "  js_host.rs: $host_fn_count functions, TS SDK calls: $sdk_fn_count unique"

# ── 2. 校验：Lua SDK vs JS host 函数 ──

LUA_SDK="$ROOT_DIR/plugin-sdk/lua/lua_plugin_v1.lua"

# 从 Lua SDK 提取全局名 (找 Host.jsonEncode 模式)
lua_global_name=$(grep -o '[A-Za-z]*\.jsonEncode' "$LUA_SDK" | head -1 | sed 's/\.jsonEncode//')

# 提取 GlobalName.function( 调用（任意位置）
lua_sdk_fns=$(grep -o "${lua_global_name}"'\.[a-zA-Z]*(' "$LUA_SDK" | sed "s/${lua_global_name}//" | sed 's/^\.//' | sed 's/($//' | sort -u)

echo ""
echo "Checking Lua SDK ($LUA_SDK) ..."

if [ "$global_name" != "$lua_global_name" ]; then
  echo "  error: global name mismatch: constants.rs='$global_name', lua_sdk='$lua_global_name'"
  errors=$((errors + 1))
else
  echo "  ok: global name = $lua_global_name"
fi

# Lua 可以调用 Host 上没有直接注册的函数（jsonEncode/jsonDecode 是 Lua-only），
# 但 Host 上注册的函数不应该在 Lua SDK 中缺失
missing_in_lua=""
for fn in $js_host_fns; do
  if ! echo "$lua_sdk_fns" | grep -qx "$fn"; then
    missing_in_lua="$missing_in_lua $fn"
  fi
done
if [ -n "$missing_in_lua" ]; then
  echo "  warn: functions in js_host.rs but NOT in Lua SDK:$missing_in_lua"
fi

lua_fn_count=$(echo "$lua_sdk_fns" | wc -l | tr -d ' ')
echo "  Lua SDK calls: $lua_fn_count unique"

# ── 3. 生成 ──

if [ $errors -gt 0 ]; then
  echo ""
  echo "error: $errors check(s) failed. Fix the mismatches above before generating."
  exit 1
fi

echo ""
echo "All checks passed. Generating ..."

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
