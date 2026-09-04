#!/usr/bin/env bash
#
# memory consolidation 选择性冒烟（真模型，两组对照）：
#   A) 无长期价值长对话（临时算术+填充）→ 折叠后 Core 应保持空（可空，不机械转储）
#   B) 有长期价值（重复确立退款政策）→ 折叠后该事实应写入 Core 并可 memory_recall
# 服务启动建议（把小模型窗口设 4096 以便几轮内触发折叠；reserve 地板使大窗口更难触发）：
#   RAISFAST_AI_MODEL_CONTEXT_JSON='{"deepseek-chat":4096}' RAISFAST_AI_MEMORY_CONSOLIDATE=true just dev
# 需 BASE_URL · RAISFAST_ADMIN_TOKEN · RAISFAST_AI_BASE_URL/API_KEY/MODEL · sqlite3
set -euo pipefail
BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
AUTH="Authorization: Bearer ${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
JSON="Content-Type: application/json"
DB_FILE="${RAISFAST_SQLITE_DB:-storage/db/raisfast.db}"
TS=$(date +%s)

FILL_A="这段填充叙述用于把单轮撑长以触发自动压缩，它本身没有任何长期价值，也不是规则，请不要把它当规则去记忆。"
FILL_B="此段为无关背景叙述，仅为撑长单轮触发自动压缩，并不修改或稀释上面提出的规则。"

rows_for() { # $1=name-prefix
  sqlite3 "$DB_FILE" "SELECT COUNT(*) FROM ai_memories WHERE category='core' AND agent_id IN (SELECT id FROM ai_agents WHERE name LIKE '$1%')" 2>/dev/null || echo -1
}

long_session() { # $1=prefix $2=fill $3=tpl(with %N)
  local agent session aid sid msg
  agent=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" -H "$AUTH" -H "$JSON" \
    -d "{\"name\":\"$1-$TS\",\"system_prompt\":\"你是记忆测试助手。系统会自动把值得长期记住的内容归纳进记忆，你无需也不要用 memory_store 主动存储；只用 memory_recall 查询。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\",\"tools\":[\"memory_recall\",\"today\"]}")
  aid=$(printf '%s' "$agent" | jq -r '.data.id')
  session=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$aid/sessions" -H "$AUTH" -H "$JSON" -d "{\"title\":\"$1-$TS\"}")
  sid=$(printf '%s' "$session" | jq -r '.data.id')
  for n in 1 2 3 4 5; do
    msg=$(printf '%s' "$3" | sed "s/%N/$n/g")
    msg="${msg}${2}${2}"
    curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$sid/turns" -H "$AUTH" -H "$JSON" \
      -d "{\"content\":\"$msg\"}" > /dev/null || true
  done
  printf '%s %s' "$sid" "$aid"
}

echo "== A) 无价值对话 =="
read -r SAID AID_A <<<"$(long_session "sk-cons-no" "$FILL_A" "第 %N 个临时问题：%N * %N = ？ 只答数字。这不需要记住。")"
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SAID/turns" -H "$AUTH" -H "$JSON" \
  -d '{"content":"刚才有值得长期记住的内容吗？没有就说没有。"}' > /tmp/cons_no_final.sse.txt || true

echo "== B) 有价值对话 =="
read -r SBID AID_B <<<"$(long_session "sk-cons-yes" "$FILL_B" "重要规则 ALPHA-%N：金额超过 1000 元的订单必须先人工确认再发货。这条规则请务必长期记住。")"
echo "   -- 显式 compact（强制折叠并触发 consolidation）--"
COMPACT=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/sessions/$SBID/compact" -H "$AUTH" || true)
echo "compact resp: $COMPACT"
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SBID/turns" -H "$AUTH" -H "$JSON" \
  -d '{"content":"金额超过多少需要人工确认？请先用 memory_recall 查找，再回答。"}' \
  | tee /tmp/cons_yes_final.sse.txt || true

echo
echo "== DB 核对 =="
NA=$(rows_for "sk-cons-no")
NY=$(rows_for "sk-cons-yes")
POLICY=$(sqlite3 "$DB_FILE" "SELECT content FROM ai_memories WHERE category='core' AND agent_id IN (SELECT id FROM ai_agents WHERE name LIKE 'sk-cons-yes%') AND content LIKE '%1000%' ORDER BY id DESC LIMIT 1" 2>/dev/null || true)
echo "no-value rows=$NA ; high-value rows=$NY ; policy行=${POLICY:0:80}"
echo "-- B 会话消息 kind 分布（应含 context:summary）--"
sqlite3 "$DB_FILE" "SELECT m.kind, COUNT(*) FROM ai_messages m JOIN ai_sessions s ON s.id=m.session_id AND s.title LIKE 'sk-cons-yes-%' GROUP BY m.kind" 2>/dev/null || echo "(无)"

echo
echo "== 检查点 =="
ok=1
if [ "$NA" -eq 0 ]; then echo "PASS: 无价值对话折叠后 Core 为空（可空，不机械转储）"; else echo "WARN: 无价值对话存了 $NA 行"; fi
if [ "$NY" -gt 0 ]; then echo "PASS: 有价值对话折叠后写入 Core（$NY 行）"; else echo "FAIL: 有价值对话未写入 Core"; ok=0; fi
if [ -n "$POLICY" ]; then echo "PASS: 政策(含1000)入库"; else echo "WARN: 未见含1000政策行"; fi
if rg -q '1000' /tmp/cons_yes_final.sse.txt; then echo "PASS: 最终回答引用 1000 元规则"; else echo "WARN: 最终回答未见1000（可能仍在摘要上下文，非记忆）"; fi
exit $ok
