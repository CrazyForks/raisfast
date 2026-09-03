#!/usr/bin/env bash
#
# Skills 端到端整体测试（真实 LLM + HTTP + 记忆 + 技能 Full 注入）
# 需先起服务并设好：
#   BASE_URL(默认 127.0.0.1:9898) · RAISFAST_ADMIN_TOKEN · RAISFAST_AI_BASE_URL/API_KEY/MODEL
# 前置：storage/skills/platform/{store-ops,response-style}/SKILL.md 已存在
set -euo pipefail
BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
AUTH="Authorization: Bearer ${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
JSON="Content-Type: application/json"

echo "== 1) 建 agent（启用 store-ops + response-style，工具给 today）=="
AGENT=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" -H "$AUTH" -H "$JSON" \
  -d "{\"name\":\"e2e-sk-$(date +%s)\",\"system_prompt\":\"你是店铺运营助手，按技能作答。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\",\"tools\":[\"today\"],\"params\":{\"skill_bundles\":[\"store-ops\",\"response-style\"]}}")
echo "$AGENT"
AGENT_ID=$(printf '%s' "$AGENT" | jq -r '.data.id')

echo "== 2) 开会话 =="
S=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$AGENT_ID/sessions" -H "$AUTH" -H "$JSON" -d '{"title":"skills-e2e"}')
SID=$(printf '%s' "$S" | jq -r '.data.id')
echo "$S"

echo "== 3) turn1（触发 store-ops：记忆政策 + 查日期；观察是否遵守 response-style 简洁/粗体）=="
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
  -d '{"content":"我们店售后：金额超1000的单要人工确认后再发货。请记住并用一句话+一个粗体词告诉我是几号？"}' \
  | tee /tmp/skills_turn1.sse.txt || true
echo

echo "== 4) turn2（跨回合：验证记忆 + 风格；今天几号）=="
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
  -d '{"content":"回到刚才的售后政策，按这个政策回应：客户想退一笔1200元的单，今天几号？"}' \
  | tee /tmp/skills_turn2.sse.txt || true
echo

echo "== 5) 回放（应见 memory_store/today tool 行 + turn:meta.system_hash/prompt_version）=="
curl -fsS "$BASE_URL/api/v1/ai/sessions/$SID/messages?after_seq=0" -H "$AUTH" \
  | jq -r '.data[] | "\(.seq) \(.role) \(.kind) :: \(.content)"'

echo
echo "检查点：turn1 应出现 today tool_call、memory_store；turn2 应引用刚记的政策（>1000 人工确认）；"
echo "回复应简短且带粗体（response-style）；messages 含 assistant(usage) 与 turn:meta。"
