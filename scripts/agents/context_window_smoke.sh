#!/usr/bin/env bash
#
# 真实模型冒烟：transcript 上下文折叠（LLM consolidation）
# 前提：
#   - 服务配置模型窗口以触发折叠：RAISFAST_AI_CONTEXT_WINDOW_FALLBACK=<窗口token，如 8192>（0=关）；可选 RAISFAST_AI_MODEL_CONTEXT_JSON='{"<model>": <窗口>}' 精确到模型、RAISFAST_AI_CONTEXT_OUTPUT_RESERVE=输出预留
#   - BASE_URL · RAISFAST_ADMIN_TOKEN · RAISFAST_AI_BASE_URL/API_KEY/MODEL
#   - (可选) RAISFAST_DB_URL：PG URL，用于断言 ai_sessions.meta.ctx 已持久化
# 观测点：多轮长对话后，模型仍能引用最早确立的事项（摘要保留事实）；meta.ctx 写入。
set -euo pipefail
BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
AUTH="Authorization: Bearer ${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
JSON="Content-Type: application/json"

echo "== 1) 建 agent（记忆工具可用，指示简短作答）=="
AGENT=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" -H "$AUTH" -H "$JSON" \
  -d "{\"name\":\"sk-ctx-$(date +%s)\",\"system_prompt\":\"你是长对话测试助手：用户每次会给出编号事项，请简短确认即可（一句话）。后续若被问早期事项请如实引用。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\",\"tools\":[\"memory_store\",\"memory_recall\",\"today\"]}")
echo "$AGENT"
AGENT_ID=$(printf '%s' "$AGENT" | jq -r '.data.id')

echo "== 2) 开会话（标题唯一便于 psql 断言）=="
TS=$(date +%s)
S=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$AGENT_ID/sessions" -H "$AUTH" -H "$JSON" -d "{\"title\":\"ctx-smoke-$TS\"}")
SID=$(printf '%s' "$S" | jq -r '.data.id')

# 每轮较长的填充文本（凑 token，触发折叠）；编号事项各不相同。
FILLER="注意：以下叙述用于凑足单轮长度以便触发自动摘要压缩逻辑，它本身不是需要长期记住的内容，请不要当真。这是一条测试性质的填充文本，用于把每轮对话撑到足够大，好让本会话尽早越过上下文预算从而观察窗口折叠是否正常工作。"

echo "== 3) 连续多轮：确立编号事项 =="
for i in 1 2 3 4 5; do
  curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
    -d "{\"content\":\"关键事项 ALPHA-$i 已确立：编号为 $i 的约定是产品只支持简体中文界面。$FILLER$FILLER\"}" \
    > /tmp/ctx_turn_$i.sse.txt || true
  echo "turn$i done"
done

echo "== 4) 提问最早事项（应能引用 ALPHA-1）=="
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
  -d '{"content":"请回顾本会话最早确立的事项：ALPHA-1 的编号是多少，约定了什么？请直接引用。"}' \
  | tee /tmp/ctx_final.sse.txt || true
echo

echo "== 5) 回放 messages =="
curl -fsS "$BASE_URL/api/v1/ai/sessions/$SID/messages?after_seq=0" -H "$AUTH" \
  | jq -r '.data[] | "\(.seq) \(.role) \(.kind) :: \(.content)"' | tee /tmp/ctx_replay.txt

echo
echo "== 检查点 =="
ok=1
if rg -q 'ALPHA-1' /tmp/ctx_replay.txt; then
  echo "PASS: 回放里可见 ALPHA-1 原文存在（可能未被折叠或仍保留）"
else
  echo "INFO: ALPHA-1 原文不在尾部回放里（符合折叠预期，看最终回答是否还能引用）"
fi
# 放宽措辞差异：要求回答确实提到 ALPHA-1 与"简体中文"，且没有声称"丢失/无法确认"。
if rg -q 'ALPHA-1' /tmp/ctx_final.sse.txt \
  && rg -q '简体中文' /tmp/ctx_final.sse.txt \
  && ! rg -q '没有找到|无法直接引用|不记得|丢失' /tmp/ctx_final.sse.txt; then
  echo "PASS: 模型引用最早事项 ALPHA-1（摘要/记忆保留事实）"
else
  echo "FAIL: 最终回答未正确引用 ALPHA-1（查看 /tmp/ctx_final.sse.txt；摘要指令已要求逐条保留编号）"; ok=0
fi
if rg -q 'ALPHA-5' /tmp/ctx_replay.txt; then
  echo "PASS: 最近事项 ALPHA-5 以原文保留在尾部"
else
  echo "WARN: 未见 ALPHA-5（若全被折叠需确认 select_cover 逻辑）"
fi

# 持久化断言（sqlite 优先；RAISFAST_SQLITE_DB 覆盖默认路径）
DB_FILE="${RAISFAST_SQLITE_DB:-storage/db/raisfast.db}"
if command -v sqlite3 >/dev/null 2>&1 && [ -f "$DB_FILE" ]; then
  CTX=$(sqlite3 "$DB_FILE" "SELECT m.content FROM ai_messages m JOIN ai_sessions s ON s.id=m.session_id WHERE m.kind='context:summary' AND s.title='ctx-smoke-$TS' ORDER BY m.seq DESC LIMIT 1" 2>/dev/null || true)
  if [ -n "$CTX" ] && [ "$CTX" != "null" ] && [ "$CTX" != "" ]; then
    COVER=$(printf '%s' "$CTX" | grep -oE '"cover_seq":[0-9]+' | head -1 | grep -oE '[0-9]+' || true)
    NCNT=$(printf '%s' "$CTX" | grep -o '编号[0-9]' | wc -l | tr -d ' ')
    echo "PASS: ai_sessions.meta.ctx 已持久化 (cover_seq=${COVER:-?}, 独立编号${NCNT:-0}条)"
    [ "${COVER:-0}" -gt 0 ] 2>/dev/null || { echo "  WARN: cover_seq 为 0（可能尚未折叠，检查预算/填充量）"; ok=0; }
    [ "${NCNT:-0}" -ge 2 ] 2>/dev/null || { echo "  WARN: ctx 文本中独立编号 <2（摘要可能仍合并编号）"; ok=0; }
  else
    echo "FAIL: 未读到 ai_sessions.meta.ctx（折叠未发生或 DB 路径错）"; ok=0
  fi
else
  echo "INFO: 无 sqlite3/$DB_FILE，跳过 ctx 持久化断言（设置 RAISFAST_SQLITE_DB 或启动参数）"
fi
exit $ok
