#!/usr/bin/env bash
#
# HTTP 真模型冒烟：admin token 驱动完整链路
#   create agent → create session → POST /turns (SSE) → GET messages → 查 ai_messages/turn:meta
#
# 需要环境（不在本仓库落 key）：
#   BASE_URL             默认 http://127.0.0.1:8080
#   RAISFAST_ADMIN_TOKEN  管理员 Bearer（authed 与 admin 均可）
#   RAISFAST_AI_BASE_URL  OpenAI 兼容根，如 https://api.deepseek.com/v1
#   RAISFAST_AI_API_KEY
#   RAISFAST_AI_MODEL     默认 deepseek-chat
#
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
TOKEN="${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
AUTH="Authorization: Bearer $TOKEN"
JSON="Content-Type: application/json"

echo "== 1) admin 建 agent =="
AGENT=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" \
  -H "$AUTH" -H "$JSON" \
  -d "{\"name\":\"smoke-$(date +%s)\",\"system_prompt\":\"你是助手，记住用户昵称，回答简洁。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\"}")
echo "$AGENT"
AGENT_ID=$(printf '%s' "$AGENT" | jq -r '.data.id')

echo "== 2) 建会话 =="
SESSION=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$AGENT_ID/sessions" \
  -H "$AUTH" -H "$JSON" -d '{"title":"http-smoke"}')
echo "$SESSION"
SESSION_ID=$(printf '%s' "$SESSION" | jq -r '.data.id')

echo "== 3) SSE 回合（记住昵称 + 触发 memory_store 工具事件）=="
curl -fsSN --no-buffer -X POST "$BASE_URL/api/v1/ai/sessions/$SESSION_ID/turns" \
  -H "$AUTH" -H "$JSON" \
  -d '{"content":"请记住我的昵称叫「小明」。然后今天几号？用一句话收尾。"}' \
  | tee /tmp/ai_smoke_sse.txt || true
echo

echo "== 4) 历史回放 =="
curl -fsS "$BASE_URL/api/v1/ai/sessions/$SESSION_ID/messages?after_seq=0" -H "$AUTH" | jq -r '.data[] | "\(.seq) \(.role) \(.kind) :: \(.content)"'

echo
echo "SSE 应含事件：tool_call(memory_store) / tool_result / chunk / done；"
echo "messages 应含 user/assistant(usage)/turn:meta(system_hash+prompt_version)。"
