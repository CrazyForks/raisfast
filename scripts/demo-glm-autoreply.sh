#!/usr/bin/env bash
#
# 真实例子 1：智谱 GLM 自动回复（Real-world e2e demo）
#
# 链路: push(widget 渠道, token 验签) → 管道落库+入队 → worker 自动消费
#       support.autoreply → GLM chat completions(真实 LLM) → assistant 落库
#       + SSE integration.message → 全程 trace(receipts → egress_log tokens)
#
# 前置:
#   1. INTEGRATION_VAULT_KEY=dev-secret just dev   (需含本脚本新代码的构建)
#   2. GLM_API_KEY — 智谱开放平台 API key
#   3. ADMIN_TOKEN  — 管理员 token
#
# 用法:
#   GLM_API_KEY=xxx.yyy ADMIN_TOKEN=... scripts/demo-glm-autoreply.sh
#
set -uo pipefail

BASE_URL="${BASE_URL:-http://localhost:9898/api/v1}"
DB_PATH="${DB_PATH:-storage/db/raisfast.db}"
RUN="$(date +%s)$RANDOM"
PASS=0; FAIL=0

ok()  { PASS=$((PASS+1)); printf '  \033[32mPASS\033[0m %s\n' "$1"; }
bad() { FAIL=$((FAIL+1)); printf '  \033[31mFAIL\033[0m %s\n' "$1"; }
sec() { printf '\n\033[1m== %s ==\033[0m\n' "$1"; }
die() { printf '\033[31mFATAL\033[0m %s\n' "$1"; exit 1; }
jget() { python3 -c "import sys,json;d=json.loads(sys.argv[1]);print($2)" "$1" 2>/dev/null; }

[ -n "${ADMIN_TOKEN:-}" ] || die "请提供 ADMIN_TOKEN"
[ -n "${GLM_API_KEY:-}" ] || die "请提供 GLM_API_KEY (open.bigmodel.cn)"
curl -sf "$BASE_URL/health" >/dev/null || die "服务未启动"

api() {
  local m=$1 p=$2 b=${3:-} out
  if [ -n "$b" ]; then
    out=$(curl -s -w '\n%{http_code}' -X "$m" "$BASE_URL$p" \
      -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' -d "$b")
  else
    out=$(curl -s -w '\n%{http_code}' -X "$m" "$BASE_URL$p" -H "Authorization: Bearer $ADMIN_TOKEN")
  fi
  echo "$out" | sed '$d'; echo "$out" | tail -1
}
# api_code / api_body helpers: last api() call leaves _RES
req() { _RES=$(api "$@"); _CODE=$(echo "$_RES" | tail -1); _BODY=$(echo "$_RES" | sed '$d'); }

sec "1/6 content types (幂等)"
req POST /admin/content-types "{\"name\":\"Contact\",\"singular\":\"sc_contact\",\"plural\":\"sc_contacts\",\"table\":\"sc_contacts\",\"fields\":[{\"name\":\"channel\",\"field_type\":\"text\"},{\"name\":\"sender\",\"field_type\":\"text\"}]}"
[ "$_CODE" = "200" ] || [ "$_CODE" = "201" ] || [ "$_CODE" = "409" ] \
  && ok "sc_contact ready ($_CODE)" || die "sc_contact 创建失败: $_RES"
req POST /admin/content-types "{\"name\":\"Conversation\",\"singular\":\"sc_conversation\",\"plural\":\"sc_conversations\",\"table\":\"sc_conversations\",\"fields\":[{\"name\":\"contact_id\",\"field_type\":\"big_int\"},{\"name\":\"status\",\"field_type\":\"text\"}]}"
[ "$_CODE" = "200" ] || [ "$_CODE" = "201" ] || [ "$_CODE" = "409" ] \
  && ok "sc_conversation ready ($_CODE)" || die "sc_conversation 创建失败: $_RES"
req POST /admin/content-types "{\"name\":\"Message\",\"singular\":\"sc_message\",\"plural\":\"sc_messages\",\"table\":\"sc_messages\",\"fields\":[{\"name\":\"conversation_id\",\"field_type\":\"big_int\"},{\"name\":\"role\",\"field_type\":\"text\"},{\"name\":\"body\",\"field_type\":\"text\"},{\"name\":\"external_id\",\"field_type\":\"text\"}]}"
[ "$_CODE" = "200" ] || [ "$_CODE" = "201" ] || [ "$_CODE" = "409" ] \
  && ok "sc_message ready ($_CODE)" || die "sc_message 创建失败: $_RES"

sec "2/6 api-client: glm (bearer, vault 密封)"
# 幂等: 已存在则更新凭据
req GET "/admin/integration/api-clients"
GLM_ID=$(jget "$_BODY" "next((c[\"id\"] for c in d[\"data\"] if c[\"client_key\"]==\"glm\"), \"\")")
if [ -n "$GLM_ID" ]; then
  req POST "/admin/integration/api-clients/$GLM_ID/update" "{\"credentials\":{\"secret\":\"$GLM_API_KEY\"},\"enabled\":true}"
  [ "$_CODE" = "200" ] && ok "glm credentials refreshed" || bad "refresh: $_RES"
else
  req POST /admin/integration/api-clients "{\"client_key\":\"glm\",\"base_url\":\"https://open.bigmodel.cn/api/coding/paas/v4\",\"auth\":{\"kind\":\"bearer\"},\"credentials\":{\"secret\":\"$GLM_API_KEY\"},\"ops\":{\"chat\":{\"method\":\"POST\",\"path\":\"/chat/completions\",\"output\":{\"text\":\"choices.0.message.content\"}}}}"
  GLM_ID=$(jget "$_BODY" 'd["data"]["id"]')
  [ "$_CODE" = "200" ] && [ -n "$GLM_ID" ] && ok "glm client created" || die "api-client 创建失败: $_RES"
fi

sec "3/6 连通性: test-call 直打 GLM"
req POST "/admin/integration/api-clients/$GLM_ID/test-call" '{"op":"chat","input":{"model":"glm-4-flash","messages":[{"role":"user","content":"只回复两个字: 收到"}]}}'
if [ "$_CODE" = "200" ]; then
  REPLY=$(jget "$_BODY" 'd["data"]["output"]["text"]')
  MODEL=$(jget "$_BODY" 'd["data"]["model"]')
  TIN=$(jget "$_BODY" 'd["data"]["tokens_in"]')
  ok "GLM 在线: model=$MODEL tokens_in=$TIN reply=\"$REPLY\""
else
  die "GLM 调用失败 (key 无效/网络不通?): $_RES"
fi

sec "4/6 widget 渠道 (token 验签 + autoreply)"
WIDGET_TOKEN="widget-secret-$RUN"
req GET "/admin/integration/channels"
CH_ID=$(jget "$_BODY" "next((c[\"id\"] for c in d[\"data\"] if c[\"channel_key\"]==\"glm-widget\"), \"\")")
CH_BODY="{\"provider\":\"widget\",\"mode\":\"push\",\"transport\":\"http1\",\"framing\":\"raw\",\"codec\":\"json\",\"verify_kind\":\"token\",\"verify_config\":{\"header\":\"x-widget-token\"},\"credentials\":{\"token\":\"$WIDGET_TOKEN\"},\"mapping\":{\"external_id\":\"\$.id\",\"sender\":\"\$.user\",\"payload\":{\"body\":\"\$.text\"}},\"target_type\":\"sc_message\",\"route_extra\":{\"jobs\":[{\"job_type\":\"support.autoreply\",\"max_attempts\":1}],\"autoreply\":{\"client\":\"glm\",\"op\":\"chat\",\"input_style\":\"openai\",\"model\":\"glm-4-flash\",\"system_prompt\":\"你是 RaisFast 的演示客服，用中文简短回答（一两句话）。\",\"context_window\":10,\"output_field\":\"text\"}}}"
if [ -n "$CH_ID" ]; then
  UPD_BODY=$(CH_BODY="$CH_BODY" WIDGET_TOKEN="$WIDGET_TOKEN" python3 -c 'import os,json;print(json.dumps({"credentials":{"token":os.environ["WIDGET_TOKEN"]},"route_extra":json.loads(os.environ["CH_BODY"])["route_extra"]}))')
  req POST "/admin/integration/channels/$CH_ID/update" "$UPD_BODY"
  [ "$_CODE" = "200" ] && ok "glm-widget refreshed" || bad "channel refresh: $_RES"
else
  BODY=$(python3 -c "import json,sys;b=json.loads(sys.argv[1]);b['channel_key']='glm-widget';b['display_name']='GLM Widget';print(json.dumps(b))" "$CH_BODY")
  req POST /admin/integration/channels "$BODY"
  CH_ID=$(jget "$_BODY" 'd["data"]["id"]')
  [ "$_CODE" = "200" ] && [ -n "$CH_ID" ] && ok "glm-widget created" || die "渠道创建失败: $_RES"
fi

sec "5/6 push 两条真实消息 (worker 自动消费)"
push() { # push <id> <text>
  curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE_URL/ingress/glm-widget" \
    -H 'Content-Type: application/json' -H "x-widget-token: $WIDGET_TOKEN" \
    -d "{\"id\":\"$1\",\"user\":\"demo_user\",\"text\":\"$2\"}"
}
CODE=$(push "demo-$RUN-1" "你好，请用一句话介绍你自己")
[ "$CODE" = "200" ] && ok "push 1 acked" || bad "push 1: $CODE"
sleep 2
CODE=$(push "demo-$RUN-2" "我刚才问了你什么？")
[ "$CODE" = "200" ] && ok "push 2 acked (上下文记忆测试)" || bad "push 2: $CODE"

sec "6/6 等待自动回复 (轮询 30s)"
REPLY1=""; REPLY2=""
for i in $(seq 1 30); do
  TRACE=$(sqlite3 "$DB_PATH" "SELECT id FROM itg_receipts WHERE external_id='demo-$RUN-1'" 2>/dev/null)
  TRACE2=$(sqlite3 "$DB_PATH" "SELECT id FROM itg_receipts WHERE external_id='demo-$RUN-2'" 2>/dev/null)
  [ -n "$TRACE" ] && REPLY1=$(sqlite3 "$DB_PATH" "SELECT body FROM sc_messages WHERE external_id='reply-$TRACE'" 2>/dev/null)
  [ -n "$TRACE2" ] && REPLY2=$(sqlite3 "$DB_PATH" "SELECT body FROM sc_messages WHERE external_id='reply-$TRACE2'" 2>/dev/null)
  if [ -n "$REPLY1" ] && [ -n "$REPLY2" ]; then break; fi
  sleep 1
done

if [ -n "$REPLY1" ]; then
  ok "回复1: $REPLY1"
else
  bad "回复1 未出现 — 查: GET /admin/integration/receipts/{trace}/trace 与 GET /admin/integration/egress-log?client_key=glm"
fi
if [ -n "$REPLY2" ]; then
  ok "回复2: $REPLY2"
else
  bad "回复2 未出现"
fi

# trace 对账（receipts API 返回编码 id；裸数字在编码模式下 parse_id 已兼容）
if [ -n "${TRACE:-}" ]; then
  req GET "/admin/integration/receipts?trace_id=$TRACE"
  ENC_TRACE=$(jget "$_BODY" 'd["data"]["items"][0]["id"]')
  req GET "/admin/integration/receipts/${ENC_TRACE:-$TRACE}/trace"
  STEPS=$(jget "$_BODY" '",".join(s["step"]+":"+s["status"] for s in d["data"]["first_pass"])')
  EGRESS=$(jget "$_BODY" '",".join(f\"{r[\"op\"]}:{r[\"status\"]}:{r[\"tokens_in\"]}in/{r[\"tokens_out\"]}out\" for r in d[\"data\"][\"egress\"])')
  printf '  trace: %s\n  egress: %s\n' "$STEPS" "$EGRESS"
fi
CONV=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM sc_conversations" 2>/dev/null)
CONTACT=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM sc_contacts WHERE sender='demo_user'" 2>/dev/null)
printf '  会话: %s 个 / 联系人: %s 个 (同 sender 归并)\n' "${CONV:-?}" "${CONTACT:-?}"

printf '\n\033[1m结果: %d passed, %d failed\033[0m\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
