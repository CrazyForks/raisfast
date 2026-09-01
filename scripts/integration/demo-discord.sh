#!/usr/bin/env bash
#
# Discord 机器人接入（Gateway WS / dispatch 帧）—— 与飞书/钉钉/Telegram 对齐。
#
# 入站: wss://gateway.discord.gg 长连接 → Identify(op2, intents) →
#       HELLO(op10) 下发心跳间隔 → 客户端周期心跳(op1) → MESSAGE_CREATE 事件
#       → 管道 → chat.ingress（新平台原语 client_heartbeat：间隔由服务端帧下发）
# 出站: chat.egress kind=api → callApi(discord.send_text) → POST /channels/{id}/messages
# 补全: enrich get_user → global_name/username
#
# 前置:
#   1. Discord Developer Portal → New Application → Bot → 拿 TOKEN
#   2. 勾选 Intents: MESSAGE CONTENT INTENT（否则收不到正文）
#   3. 把机器人加进服务器，或在 DM 里直接发消息测试
#   4. 服务已启动
#
# 用法:
#   DISCORD_BOT_TOKEN=xxx ADMIN_TOKEN=... ./scripts/demo-discord.sh
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

[ -n "${ADMIN_TOKEN:-}" ] || die "请提供 ADMIN_TOKEN"
[ -n "${DISCORD_BOT_TOKEN:-}" ] || die "请提供 DISCORD_BOT_TOKEN"
curl -sf "$BASE_URL/health" >/dev/null || die "服务未启动"

CURL_OPTS=(-s --max-time 15)
if [ -n "${HTTPS_PROXY:-}" ]; then CURL_OPTS+=(-x "$HTTPS_PROXY"); fi

req() { _RES=$(curl -s -w '\n%{http_code}' -X "${1}" "$BASE_URL${2}" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  ${3:+-d "$3"}); _CODE=$(echo "$_RES" | tail -1); _BODY=$(echo "$_RES" | sed '$d'); }

# Discord intents: GUILDS(1) | GUILD_MESSAGES(512) | DIRECT_MESSAGES(4096) | MESSAGE_CONTENT(32768)
INTENTS=$((1 | 512 | 4096 | 32768))

sec "1/4 api-client: discord (bearer Bot token)"
req GET "/admin/integration/api-clients"
DC_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['client_key']=='discord'), '')")
CREDS="{\"token\":\"$DISCORD_BOT_TOKEN\"}"
OPS='{"send_text":{"method":"POST","path":"/channels/{channel_id}/messages","output":{"message_id":"$.id"}},"get_user":{"method":"GET","path":"/users/{user_id}","output":{"username":"$.username","global_name":"$.global_name"}}}'
if [ -n "$DC_ID" ]; then
  req PUT "/admin/integration/api-clients/$DC_ID" "{\"credentials\":$CREDS,\"ops\":$OPS,\"enabled\":true}"
  [ "$_CODE" = "200" ] && ok "discord client refreshed" || die "刷新失败: $_RES"
else
  req POST /admin/integration/api-clients "{\"client_key\":\"discord\",\"base_url\":\"https://discord.com/api/v10\",\"auth\":{\"kind\":\"bearer\",\"prefix\":\"Bot\"},\"credentials\":$CREDS,\"ops\":$OPS}"
  DC_ID=$(jget "$_BODY" 'd["data"]["id"]')
  [ "$_CODE" = "200" ] && [ -n "$DC_ID" ] && ok "discord client created" || die "创建失败: $_RES"
fi

sec "2/4 验证 Bot API 连通（get current user）"
ME=$(curl "${CURL_OPTS[@]}" -H "Authorization: Bot $DISCORD_BOT_TOKEN" "https://discord.com/api/v10/users/@me")
case "$ME" in
  *'"username"'*) ok "Bot API 连通: $(jget "$ME" 'd["username"]')";;
  *) die "get @me 失败: ${ME:0:200}（检查 TOKEN / 网络代理）";;
esac

sec "3/4 dispatch 渠道: discord（Gateway WS，dispatch 帧 + 动态心跳）"
req GET "/admin/integration/channels"
CH_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['channel_key']=='discord'), '')")
CH_BODY=$(DISCORD_BOT_TOKEN="$DISCORD_BOT_TOKEN" INTENTS="$INTENTS" python3 -c '
import json,os
identify = "{\"op\":2,\"d\":{\"token\":\"{{token}}\",\"intents\":" + os.environ["INTENTS"] + ",\"properties\":{\"os\":\"raisfast\",\"browser\":\"raisfast\",\"device\":\"raisfast\"}}}"
print(json.dumps({
  "provider":"discord","display_name":"Discord 机器人",
  "mode":"stream","transport":"ws","framing":"dispatch","codec":"json",
  "endpoint":"wss://gateway.discord.gg/?v=10&encoding=json",
  "verify_kind":"none",
  "credentials":{"token":os.environ["DISCORD_BOT_TOKEN"]},
  "mapping":{
    "external_id":"$.id",
    "sender":"$.author.id",
    "payload":{
      "body":"$.content",
      "reply_chat_id":"$.channel_id"
    }
  },
  "target_type":"chat/chat_messages",
  "route_extra":{"jobs":[{"job_type":"chat.ingress","max_attempts":1}]},
  "stream_config":{
    "handshake":{
      "frames":[identify]
    },
    "events":{"match":{"path":"$.t","equals":"MESSAGE_CREATE"},"payload_path":"$.d"},
    "client_heartbeat":{
      "match":{"path":"$.op","equals":10},
      "interval_path":"$.d.heartbeat_interval",
      "frame":{"op":1,"d":0}
    },
    "ws_keepalive":False
  },
  "enabled":True
}, ensure_ascii=False))')
if [ -n "$CH_ID" ]; then
  req DELETE "/admin/integration/channels/$CH_ID"
  [ "$_CODE" = "200" ] && ok "old channel removed" || die "旧渠道删除失败: $_RES"
fi
BODY=$(python3 -c 'import json,sys;b=json.loads(sys.argv[1]);b["channel_key"]="discord";print(json.dumps(b))' "$CH_BODY")
req POST /admin/integration/channels "$BODY"
CH_ID=$(jget "$_BODY" 'd["data"]["id"]')
[ "$_CODE" = "200" ] && [ -n "$CH_ID" ] && ok "channel created" || die "渠道创建失败: $_RES"

sec "3.5 绑定 chat_inbox（egress + enrich）"
# 注意：admin 创建的渠道默认 app_id=NULL（平台级），工作台「会话→渠道」只显示
# app 自有渠道。dev 目录模式（chat 未装 app bundle）请手动标记：
#   sqlite3 storage/db/raisfast.db "UPDATE itg_channels SET app_id='chat' WHERE channel_key='discord';"
req GET "/admin/cms/chat/chat_inboxes?channel_id=$CH_ID"
INBOX_ID=$(jget "$_BODY" "next((r['id'] for r in d['data'] if str(r.get('channel_id'))=='$CH_ID'), '')")
if [ -z "$INBOX_ID" ]; then
  req POST /admin/cms/chat/chat_inboxes "{\"channel_id\":\"$CH_ID\",\"name\":\"Discord 机器人\",\"egress\":{\"kind\":\"api\",\"client\":\"discord\",\"op\":\"send_text\",\"input\":{\"channel_id\":\"{reply.chat_id}\",\"content\":\"{msg.body}\"}},\"enrich\":{\"client\":\"discord\",\"op\":\"get_user\",\"input\":{\"user_id\":\"{sender}\"},\"name\":{\"join\":[\"global_name\"],\"fallback\":\"username\"}}}"
  INBOX_ID=$(jget "$_BODY" 'd["data"]["id"]')
  if [ "$_CODE" = "200" ] || [ "$_CODE" = "201" ]; then
    [ -n "$INBOX_ID" ] && ok "chat_inbox created" || die "收件箱创建失败: $_RES"
  else
    die "收件箱创建失败: $_RES"
  fi
else
  req PUT "/admin/cms/chat/chat_inboxes/$INBOX_ID" "{\"egress\":{\"kind\":\"api\",\"client\":\"discord\",\"op\":\"send_text\",\"input\":{\"channel_id\":\"{reply.chat_id}\",\"content\":\"{msg.body}\"}},\"enrich\":{\"client\":\"discord\",\"op\":\"get_user\",\"input\":{\"user_id\":\"{sender}\"},\"name\":{\"join\":[\"global_name\"],\"fallback\":\"username\"}}}" || true
  ok "chat_inbox exists (egress/enrich ensured)"
fi

sec "4/4 等待 connected（Gateway 握手 + 动态心跳，45s）"
STATE=""; CONNECTED=""
for i in $(seq 1 45); do
  req GET "/admin/integration/channels/health"
  STATE=$(jget "$_BODY" "next((h['status'] for h in d['data'] if h['channel_key']=='discord'), '?')")
  case "$STATE" in
    connected) CONNECTED=1; break;;
    *) sleep 1;;
  esac
done
if [ -n "$CONNECTED" ]; then ok "channel connected（Identify→HELLO→心跳 全通）"; else
  req GET "/admin/integration/channels/health"
  ERR=$(jget "$_BODY" "next((str(h.get('last_error')) for h in d['data'] if h['channel_key']=='discord'), '?')")
  printf '  服务端错误: %s\n' "${ERR:0:250}"
  die "未连上 — 检查 TOKEN/Intents/出网（Telegram 若需代理，Discord 同样需要）"
fi

printf '\n  现在 Discord 里给机器人发消息（服务器或 DM）→ 工作台出现会话（SSE 实时）\n'
printf '  坐席回复 → 机器人回发给对方（content 到 /channels/{id}/messages）。\n'
printf '\n\033[1m结果: %d passed, %d failed\033[0m\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
