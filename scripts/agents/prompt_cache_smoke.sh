#!/usr/bin/env bash
#
# Prompt-cache 观测冒烟：跑 5 轮短对话，从每轮 turn:meta.usage_total 读出
# cache_read/cache_write，验证 provider 前缀缓存是否生效（省钱可测）。
# 依赖：BASE_URL · RAISFAST_ADMIN_TOKEN · RAISFAST_AI_BASE_URL/API_KEY/MODEL
# 注：DeepSeek 自动 KV 缓存（首轮写缓存、后续命中）；provider 若字段缺省 → INFO 不判 FAIL。
set -euo pipefail
BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
AUTH="Authorization: Bearer ${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
JSON="Content-Type: application/json"

echo "== 1) 建 agent（简短助手）=="
AGENT=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" -H "$AUTH" -H "$JSON" \
  -d "{\"name\":\"sk-cache-$(date +%s)\",\"system_prompt\":\"你是算术助手，只回答简短数字。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\",\"tools\":[]}")
AGENT_ID=$(printf '%s' "$AGENT" | jq -r '.data.id')

S=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$AGENT_ID/sessions" -H "$AUTH" -H "$JSON" -d '{"title":"prompt-cache-smoke"}')
SID=$(printf '%s' "$S" | jq -r '.data.id')

echo "== 2) 连跑 5 轮短问答 =="
for i in 1 2 3 4 5; do
  curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
    -d "{\"content\":\"$i + $i 等于几？只答数字。\"}" > /dev/null || true
  echo "turn$i done"
done

echo "== 3) 回放并汇总每轮 usage_total =="
curl -fsS "$BASE_URL/api/v1/ai/sessions/$SID/messages?after_seq=0" -H "$AUTH" \
  | jq -r '.data[] | select(.kind=="turn:meta") | .content' \
  | jq -r '[.usage_total.input, .usage_total.output, .usage_total.cache_read, .usage_total.cache_write] | @tsv' \
  | tee /tmp/prompt_cache_usage.tsv

echo
echo "== 检查点 =="
TOTAL_HITS=$(awk -F'\t' '{s+=$3} END{print s+0}' /tmp/prompt_cache_usage.tsv)
WRITES=$(awk -F'\t' '{s+=$4} END{print s+0}' /tmp/prompt_cache_usage.tsv)
LATER_HITS=$(awk -F'\t' 'NR>1 && $3>0 {c++} END{print c+0}' /tmp/prompt_cache_usage.tsv)
echo "cache_read 合计=$TOTAL_HITS  cache_write 合计=$WRITES  (第2轮起有命中的轮数=$LATER_HITS)"

ok=1
if [ "$TOTAL_HITS" -gt 0 ]; then
  echo "PASS: provider 返回缓存命中（cache_read=$TOTAL_HITS）——前缀缓存生效"
else
  echo "INFO: 未观测到 cache_read（DeepSeek 对极短前缀/首个请求可不命中；多轮+固定 system 应会命中）"
  ok=0
fi
echo "提示：正常模式应呈现『第1轮 cache_read=0、后续轮 cache_read>0 且 input 基本稳定』"
exit $ok
