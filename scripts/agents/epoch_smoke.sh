#!/usr/bin/env bash
#
# Mini-Epoch 冒烟：验证 system 前缀定妆 + rebuild 归因。
# 前置：服务需带模型窗口启动使 epoch 生效，例如：
#   RAISFAST_AI_CONTEXT_WINDOW_FALLBACK=8192 just dev
# 需 BASE_URL · RAISFAST_ADMIN_TOKEN · RAISFAST_AI_BASE_URL/API_KEY/MODEL
# 流程：3 轮（reuse）→ PATCH 改 system_prompt → 1 轮应 rebuild(agent) → 再 1 轮恢复 reuse。
set -euo pipefail
BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
AUTH="Authorization: Bearer ${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
JSON="Content-Type: application/json"

echo "== 1) 建 agent（tools: today）=="
AGENT=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" -H "$AUTH" -H "$JSON" \
  -d "{\"name\":\"sk-epoch-$(date +%s)\",\"system_prompt\":\"你是版本一的助手。请保持简短。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\",\"tools\":[\"today\"]}")
AGENT_ID=$(printf '%s' "$AGENT" | jq -r '.data.id')
S=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$AGENT_ID/sessions" -H "$AUTH" -H "$JSON" -d '{"title":"epoch-smoke"}')
SID=$(printf '%s' "$S" | jq -r '.data.id')

ask() { curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" -d "{\"content\":\"$1\"}" > /dev/null || true; }

echo "== 2) 3 轮（应 reuse）=="
ask "1+1?"
ask "2+3?"
ask "5+5?"

echo "== 3) PATCH 修改 system_prompt（应触发 rebuild: agent）=="
curl -fsS -X PUT "$BASE_URL/api/v1/admin/ai/agents/$AGENT_ID" -H "$AUTH" -H "$JSON" \
  -d '{"system_prompt":"你是版本二的助手，多聊几句。保持简短。"}' > /dev/null

ask "8+8?"

echo "== 4) 再 1 轮（应恢复 reuse）=="
ask "9+9?"

echo "== 5) 汇总每轮 epoch/cache =="
curl -fsS "$BASE_URL/api/v1/ai/sessions/$SID/messages?after_seq=0" -H "$AUTH" \
  | jq -r '.data[] | select(.kind=="turn:meta") | .content' \
  | jq -r '[.usage_total.cache_read // 0, (.epoch.reused|tostring), .epoch.reason] | @tsv' \
  | nl -v1 | tee /tmp/epoch_summary.tsv

echo
echo "== 检查点 =="
ok=1
LINE1=$(sed -n '1p' /tmp/epoch_summary.tsv)
if printf '%s' "$LINE1" | rg -q 'false' && printf '%s' "$LINE1" | rg -q 'first'; then
  echo "PASS: turn1 首次 rebuild（reason=first）"
else echo "FAIL: turn1 应 first rebuild: $LINE1"; ok=0; fi

for n in 2 3; do
  L=$(sed -n "${n}p" /tmp/epoch_summary.tsv)
  if printf '%s' "$L" | rg -q $'\ttrue\t'; then echo "PASS: turn$n epoch reused"; else echo "FAIL: turn$n 应 reused: $L"; ok=0; fi
done

L4=$(sed -n '4p' /tmp/epoch_summary.tsv)
if printf '%s' "$L4" | rg -q 'false' && printf '%s' "$L4" | rg -q 'agent'; then
  echo "PASS: PATCH 后 rebuild（reason=agent）"
else echo "FAIL: turn4 应 rebuild agent: $L4"; ok=0; fi

L5=$(sed -n '5p' /tmp/epoch_summary.tsv)
if printf '%s' "$L5" | rg -q $'\ttrue\t'; then echo "PASS: turn5 恢复 reuse"; else echo "FAIL: turn5 应 reuse: $L5"; ok=0; fi

HITS=$(awk -F'\t' '{s+=$2} END{print s+0}' /tmp/epoch_summary.tsv)
[ "$HITS" -gt 0 ] && echo "PASS: cache_read 合计=$HITS" || { echo "INFO: 未见 cache_read（查看 provider 是否返回）"; }
exit $ok
