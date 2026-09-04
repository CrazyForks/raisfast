#!/usr/bin/env bash
#
# 真实模型冒烟：沙箱代码执行 run_js / run_rhai（复用 PluginManager）
# 需先起服务并设好 BASE_URL · RAISFAST_ADMIN_TOKEN · RAISFAST_AI_BASE_URL/API_KEY/MODEL
set -euo pipefail
BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
AUTH="Authorization: Bearer ${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
JSON="Content-Type: application/json"

echo "== 1) 建 agent（白名单 run_js run_rhai；system_prompt 带调用约定）=="
AGENT=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" -H "$AUTH" -H "$JSON" \
  -d "{\"name\":\"sk-script-$(date +%s)\",\"system_prompt\":\"你是脚本执行助手。run_js 要求 ESM：export function main(__in){const a=JSON.parse(__in); return ...}；run_rhai 要求顶层 fn main(input) { ... }。输出简短中文说明。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\",\"tools\":[\"run_js\",\"run_rhai\"]}")
echo "$AGENT"
AGENT_ID=$(printf '%s' "$AGENT" | jq -r '.data.id')

echo "== 2) 开会话 =="
S=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$AGENT_ID/sessions" -H "$AUTH" -H "$JSON" -d '{"title":"sandbox-code-smoke"}')
SID=$(printf '%s' "$S" | jq -r '.data.id')

echo "== 3) turn：先用 run_js 求和 6+7，再用 run_rhai 求积 3*4，汇总 =="
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
  -d '{"content":"请分两次调用：① run_js，code=export function main(__in){const a=JSON.parse(__in); return {sum:a.a+a.b}}，args={\"a\":6,\"b\":7}；② run_rhai，code=fn main(input){input.a*input.b}，args={\"a\":3,\"b\":4}。最后用一句话告诉我两次结果。"}' \
  | tee /tmp/sandbox_code_turn.sse.txt || true
echo

echo "== 4) 回放 =="
curl -fsS "$BASE_URL/api/v1/ai/sessions/$SID/messages?after_seq=0" -H "$AUTH" \
  | jq -r '.data[] | "\(.seq) \(.role) \(.kind) :: \(.content)"' | tee /tmp/sandbox_code_replay.txt

echo
echo "== 检查点 =="
ok=1
rg -q '"name":"run_js"' /tmp/sandbox_code_turn.sse.txt && echo "PASS: run_js 被调用" || { echo "FAIL: 未见 run_js"; ok=0; }
rg -q '"name":"run_rhai"' /tmp/sandbox_code_turn.sse.txt && echo "PASS: run_rhai 被调用" || { echo "FAIL: 未见 run_rhai"; ok=0; }
rg -q '"success":true' /tmp/sandbox_code_turn.sse.txt && echo "PASS: 工具均 success" || echo "WARN: 存在非 success 工具行"
rg -q '13|12' /tmp/sandbox_code_turn.sse.txt /tmp/sandbox_code_replay.txt && echo "PASS: 输出含 13(和) 与 12(积) 之一" || { echo "FAIL: 未在输出中找到 13/12"; ok=0; }
rg -q 'turn:meta' /tmp/sandbox_code_replay.txt && echo "PASS: turn:meta 存在"
exit $ok
