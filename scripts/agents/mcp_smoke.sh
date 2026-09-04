#!/usr/bin/env bash
#
# 通用 MCP 冒烟：验证 admin 配置的外部 MCP server（stdio 或 streamable-HTTP）工具
# 能被 agent 端到端调用（发现→组合名 mcp__{server}__{tool}→调用→审计/回灌）。
#
# 服务需以带 MCP server 配置启动，例如：
#   stdio: RAISFAST_AI_MCP_SERVERS='[{"name":"echo","command":"python3","args":["scripts/agents/mcp_echo_server.py"]}]' just dev
#   http : （另起 bun+hono fixture）PORT=9899 bun run scripts/agents/mcp_http_server.ts
#          RAISFAST_AI_MCP_SERVERS='[{"name":"echo","url":"http://127.0.0.1:9899"}]' just dev
# 需 BASE_URL · RAISFAST_ADMIN_TOKEN · RAISFAST_AI_BASE_URL/API_KEY/MODEL
# 可选：MCP_SERVER=组合名中的 server（默认 echo）、MCP_MSG=回显文本（默认 hello-mcp）
set -euo pipefail
BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
MCP_SERVER="${MCP_SERVER:-echo}"
MCP_MSG="${MCP_MSG:-hello-mcp}"
TOOL="mcp__${MCP_SERVER}__echo"
AUTH="Authorization: Bearer ${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
JSON="Content-Type: application/json"

echo "== 1) 建 agent（tools '*' 以启用含 ${TOOL} 的域工具）=="
AGENT=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" -H "$AUTH" -H "$JSON" \
  -d "{\"name\":\"sk-mcp-$(date +%s)\",\"system_prompt\":\"你是外部 MCP 工具测试助手：需要回显时调用 echo 工具。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\",\"tools\":[\"*\"]}")
AGENT_ID=$(printf '%s' "$AGENT" | jq -r '.data.id')
S=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$AGENT_ID/sessions" -H "$AUTH" -H "$JSON" -d '{"title":"mcp-smoke"}')
SID=$(printf '%s' "$S" | jq -r '.data.id')

echo "== 2) turn：让模型调用 ${TOOL} 回显 ${MCP_MSG} =="
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
  -d "{\"content\":\"请调用 echo 工具把 ${MCP_MSG} 回显回来，并告诉我它返回了什么。\"}" \
  | tee /tmp/mcp_turn.sse.txt || true

echo
echo "== 3) 回放 =="
curl -fsS "$BASE_URL/api/v1/ai/sessions/$SID/messages?after_seq=0" -H "$AUTH" \
  | jq -r '.data[] | "\(.seq) \(.role) \(.kind) \(.tool_name // "") :: \(.content)"' | tee /tmp/mcp_replay.txt

echo
echo "== 检查点 =="
ok=1
if rg -q "${TOOL}" /tmp/mcp_turn.sse.txt /tmp/mcp_replay.txt; then
  echo "PASS: 组合工具 ${TOOL} 被调用"
else
  echo "FAIL: 未见 ${TOOL}（检查服务 RAISFAST_AI_MCP_SERVERS 的 name 是否为 ${MCP_SERVER}，及服务日志 registered MCP tools）"; ok=0
fi
if rg -q "${MCP_MSG}" /tmp/mcp_turn.sse.txt; then
  echo "PASS: 回显内容 ${MCP_MSG} 返回并进入上下文"
else
  echo "WARN: 未见 ${MCP_MSG} 回显（看 /tmp/mcp_turn.sse.txt）"
fi
exit $ok
