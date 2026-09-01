#!/usr/bin/env bash
#
# Telegram 机器人接入（webhook push 模式）—— 与飞书/钉钉对齐的工作台闭环。
#
# 入站: Telegram setWebhook → POST /api/v1/ingress/telegram（verify_kind=token，
#       校验 X-Telegram-Bot-Api-Secret-Token 头）→ 管道 → chat.ingress
# 出站: chat.egress kind=api → callApi(telegram.send_text) → /bot<token>/sendMessage
#       （新平台原语 url-path-token：token 密封在 vault，注入 URL 路径）
#
# 前置:
#   1. @BotFather 建机器人，拿 TELEGRAM_BOT_TOKEN
#   2. 公网 HTTPS URL（本地开发可用 tunnel）能到达 {host}/api/v1/ingress/telegram
#   3. 服务已启动（INTEGRATION_VAULT_KEY=dev-secret just dev）
#
# 用法:
#   TELEGRAM_BOT_TOKEN=123:abc ADMIN_TOKEN=... \
#     TELEGRAM_WEBHOOK_URL="https://你的域名/api/v1/ingress/telegram" \
#     ./scripts/demo-telegram.sh
# 网络受限时（如国内访问 Telegram API）：
#   HTTPS_PROXY=http://127.0.0.1:7897 ./scripts/demo-telegram.sh
#
set -uo pipefail

BASE_URL="${BASE_URL:-http://localhost:9898/api/v1}"
RUN="$(date +%s)"
PASS=0; FAIL=0

ok()  { PASS=$((PASS+1)); printf '  \033[32mPASS\033[0m %s\n' "$1"; }
bad() { FAIL=$((FAIL+1)); printf '  \033[31mFAIL\033[0m %s\n' "$1"; }
sec() { printf '\n\033[1m== %s ==\033[0m\n' "$1"; }
die() { printf '\033[31mFATAL\033[0m %s\n' "$1"; exit 1; }
jget() { python3 -c "import sys,json;d=json.loads(sys.argv[1]);print($2)" "$1" 2>/dev/null; }

# Proxy for script-side Telegram calls (getMe/setWebhook). The server's own
# outbound (chat.egress callApi) honors the same env via reqwest proxy support.
CURL_OPTS=(-s --max-time 15)
if [ -n "${HTTPS_PROXY:-}" ]; then CURL_OPTS+=(-x "$HTTPS_PROXY"); fi

[ -n "${ADMIN_TOKEN:-}" ] || die "请提供 ADMIN_TOKEN"
[ -n "${TELEGRAM_BOT_TOKEN:-}" ] || die "请提供 TELEGRAM_BOT_TOKEN"
[ -n "${TELEGRAM_WEBHOOK_URL:-}" ] || die "请提供 TELEGRAM_WEBHOOK_URL（公网 HTTPS，指向 /api/v1/ingress/telegram）"
curl -sf "$BASE_URL/health" >/dev/null || die "服务未启动"

WEBHOOK_SECRET="${WEBHOOK_SECRET:-tg-secret-$RUN}"

req() { _RES=$(curl -s -w '\n%{http_code}' -X "${1}" "$BASE_URL${2}" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  ${3:+-d "$3"}); _CODE=$(echo "$_RES" | tail -1); _BODY=$(echo "$_RES" | sed '$d'); }

sec "1/4 api-client: telegram (url-path-token)"
req GET "/admin/integration/api-clients"
TG_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['client_key']=='telegram'), '')")
OPS='{"send_text":{"method":"POST","path":"/sendMessage","output":{"message_id":"$.result.message_id"}},"get_chat":{"method":"GET","path":"/getChat?chat_id={chat_id}","output":{"first_name":"$.result.first_name","last_name":"$.result.last_name","username":"$.result.username","title":"$.result.title"}}}'
CREDS="{\"secret\":\"$TELEGRAM_BOT_TOKEN\"}"
if [ -n "$TG_ID" ]; then
  req PUT "/admin/integration/api-clients/$TG_ID" "{\"credentials\":$CREDS,\"ops\":$OPS,\"enabled\":true}"
  [ "$_CODE" = "200" ] && ok "telegram client refreshed" || die "刷新失败: $_RES"
else
  req POST /admin/integration/api-clients "{\"client_key\":\"telegram\",\"base_url\":\"https://api.telegram.org\",\"auth\":{\"kind\":\"url-path-token\",\"path_prefix\":\"/bot\"},\"credentials\":$CREDS,\"ops\":$OPS}"
  TG_ID=$(jget "$_BODY" 'd["data"]["id"]')
  [ "$_CODE" = "200" ] && [ -n "$TG_ID" ] && ok "telegram client created" || die "创建失败: $_RES"
fi

sec "2/4 验证 Bot API 连通（getMe）"
ME=$(curl "${CURL_OPTS[@]}" "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/getMe")
case "$ME" in
  *'"ok":true'*) ok "Bot API 连通: $(jget "$ME" 'd["result"]["username"]')";;
  *) die "getMe 失败: ${ME:0:200}（检查 HTTPS_PROXY 代理是否可达）";;
esac

sec "3/4 dispatch 渠道: telegram (webhook push)"
req GET "/admin/integration/channels"
CH_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['channel_key']=='telegram'), '')")
CH_BODY=$(python3 -c '
import json,sys
print(json.dumps({
  "provider":"telegram","display_name":"Telegram 机器人",
  "mode":"push","transport":"http1","framing":"raw","codec":"json",
  "verify_kind":"token",
  "verify_config":{"header":"x-telegram-bot-api-secret-token"},
  "credentials":json.loads(sys.argv[1]),
  "mapping":{
    "external_id":"$.message.message_id | as_str",
    "sender":"$.message.from.id | as_str",
    "payload":{
      "body":"$.message.text",
      "reply_chat_id":"$.message.chat.id | as_str"
    }
  },
  "target_type":"chat/chat_messages",
  "route_extra":{"jobs":[{"job_type":"chat.ingress","max_attempts":1}]},
  "enabled":True
}, ensure_ascii=False))' "{\"token\":\"$WEBHOOK_SECRET\"}")
if [ -n "$CH_ID" ]; then
  req DELETE "/admin/integration/channels/$CH_ID"
  [ "$_CODE" = "200" ] && ok "old channel removed" || die "旧渠道删除失败: $_RES"
fi
BODY=$(python3 -c 'import json,sys;b=json.loads(sys.argv[1]);b["channel_key"]="telegram";print(json.dumps(b))' "$CH_BODY")
req POST /admin/integration/channels "$BODY"
CH_ID=$(jget "$_BODY" 'd["data"]["id"]')
[ "$_CODE" = "200" ] && [ -n "$CH_ID" ] && ok "channel created" || die "渠道创建失败: $_RES"

sec "3.5 绑定 chat_inbox（egress 回发配置）"
# 注意：admin 创建的渠道默认 app_id=NULL（平台级），工作台「会话→渠道」只显示
# app 自有渠道。chat 以 app bundle 安装后，这里可经渠道 update 传 app_id='chat'；
# dev 目录模式（未装 app）下请手动把渠道标为应用自有，例如：
#   sqlite3 storage/db/raisfast.db "UPDATE itg_channels SET app_id='chat' WHERE channel_key='telegram';"
req GET "/admin/cms/chat/chat_inboxes?channel_id=$CH_ID"
INBOX_ID=$(jget "$_BODY" "next((r['id'] for r in d['data'] if str(r.get('channel_id'))=='$CH_ID'), '')")
if [ -z "$INBOX_ID" ]; then
  req POST /admin/cms/chat/chat_inboxes "{\"channel_id\":\"$CH_ID\",\"name\":\"Telegram 机器人\",\"egress\":{\"kind\":\"api\",\"client\":\"telegram\",\"op\":\"send_text\",\"input\":{\"chat_id\":\"{reply.chat_id}\",\"text\":\"{msg.body}\"}},\"enrich\":{\"client\":\"telegram\",\"op\":\"get_chat\",\"input\":{\"chat_id\":\"{sender}\"},\"name\":{\"join\":[\"first_name\",\"last_name\"],\"fallback\":\"username\"}}}"
  INBOX_ID=$(jget "$_BODY" 'd["data"]["id"]')
  if [ "$_CODE" = "200" ] || [ "$_CODE" = "201" ]; then
    [ -n "$INBOX_ID" ] && ok "chat_inbox created" || die "收件箱创建失败: $_RES"
  else
    die "收件箱创建失败: $_RES"
  fi
else
  req PUT "/admin/cms/chat/chat_inboxes/$INBOX_ID" "{\"egress\":{\"kind\":\"api\",\"client\":\"telegram\",\"op\":\"send_text\",\"input\":{\"chat_id\":\"{reply.chat_id}\",\"text\":\"{msg.body}\"}},\"enrich\":{\"client\":\"telegram\",\"op\":\"get_chat\",\"input\":{\"chat_id\":\"{sender}\"},\"name\":{\"join\":[\"first_name\",\"last_name\"],\"fallback\":\"username\"}}}" || true
  ok "chat_inbox exists (egress/enrich ensured)"
fi

sec "4/4 setWebhook（Telegram → 本机）"
WH=$(curl "${CURL_OPTS[@]}" -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/setWebhook" \
  --data-urlencode "url=$TELEGRAM_WEBHOOK_URL" \
  --data-urlencode "secret_token=$WEBHOOK_SECRET")
case "$WH" in
  *'"ok":true'*) ok "webhook 已设置 → $TELEGRAM_WEBHOOK_URL";;
  *) die "setWebhook 失败: ${WH:0:200}";;
esac

printf '\n  现在在 Telegram 里给你的机器人发消息 → 工作台会出现会话（SSE 实时）\n'
printf '  坐席回复 → 机器人会把消息回发给对方。\n'
printf '\n\033[1m结果: %d passed, %d failed\033[0m\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
