#!/usr/bin/env bash
#
# QQ 机器人接入（OneBot v11，go-cqhttp 转发 WS）—— 复用 dispatch 帧 + 模板化出站。
#
# 入站: go-cqhttp 正向 WS（默认 ws://127.0.0.1:6700）推送 JSON 事件 →
#       dispatch 帧（post_type=message）→ 管道 → chat.ingress
# 出站: chat.egress kind=api → callApi(onebot.send_msg) → go-cqhttp HTTP
#       /send_msg（私聊 user_id / 群聊 group_id，空字段自动省略）
# 补全: enrich get_stranger_info → nickname
#
# 前置:
#   1. 本地跑 go-cqhttp（https://github.com/Mrs4s/go-cqhttp）：
#      - 正向 WS 端口 6700（默认），HTTP API 端口 5700（默认）
#      - 若设了 access_token，脚本会带；本地可不设
#   2. go-cqhttp 登录 QQ 号成功
#   3. 服务已启动
#
# 用法:
#   ADMIN_TOKEN=... [ONE_BOT_ACCESS_TOKEN=xxx] ./scripts/demo-onebot.sh
#   （ONE_BOT_HTTP_URL / ONE_BOT_WS_URL 可覆盖默认端口）
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
curl -sf "$BASE_URL/health" >/dev/null || die "服务未启动"

ONE_BOT_HTTP_URL="${ONE_BOT_HTTP_URL:-http://127.0.0.1:5700}"
ONE_BOT_WS_URL="${ONE_BOT_WS_URL:-ws://127.0.0.1:6700}"
# access_token 走 go-cqhttp 的 HTTP 头；WS 正向连接用 query 参数（go-cqhttp 约定）
WS_ENDPOINT="$ONE_BOT_WS_URL"
if [ -n "${ONE_BOT_ACCESS_TOKEN:-}" ]; then
  WS_ENDPOINT="$ONE_BOT_WS_URL?access_token=$ONE_BOT_ACCESS_TOKEN"
fi

req() { _RES=$(curl -s -w '\n%{http_code}' -X "${1}" "$BASE_URL${2}" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  ${3:+-d "$3"}); _CODE=$(echo "$_RES" | tail -1); _BODY=$(echo "$_RES" | sed '$d'); }

sec "1/4 api-client: onebot (go-cqhttp HTTP)"
req GET "/admin/integration/api-clients"
OB_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['client_key']=='onebot'), '')")
CREDS="{\"token\":\"${ONE_BOT_ACCESS_TOKEN:-}\"}"
OPS='{"send_msg":{"method":"POST","path":"/send_msg","output":{"message_id":"$.message_id"}},"get_stranger_info":{"method":"GET","path":"/get_stranger_info?user_id={user_id}","output":{"nickname":"$.nickname"}}}'
if [ -n "$OB_ID" ]; then
  req PUT "/admin/integration/api-clients/$OB_ID" "{\"credentials\":$CREDS,\"ops\":$OPS,\"enabled\":true}"
  [ "$_CODE" = "200" ] && ok "onebot client refreshed" || die "刷新失败: $_RES"
else
  req POST /admin/integration/api-clients "{\"client_key\":\"onebot\",\"base_url\":\"$ONE_BOT_HTTP_URL\",\"auth\":{\"kind\":\"bearer\"},\"credentials\":$CREDS,\"ops\":$OPS}"
  OB_ID=$(jget "$_BODY" 'd["data"]["id"]')
  [ "$_CODE" = "200" ] && [ -n "$OB_ID" ] && ok "onebot client created" || die "创建失败: $_RES"
fi

sec "2/4 验证 go-cqhttp 连通（get_login_info）"
ME=$(curl -s --max-time 5 -H "Authorization: Bearer ${ONE_BOT_ACCESS_TOKEN:-}" "$ONE_BOT_HTTP_URL/get_login_info")
case "$ME" in
  *'"user_id"'*) ok "go-cqhttp 连通: QQ $(jget "$ME" 'd["data"]["user_id"]')";;
  *) bad "get_login_info 失败: ${ME:0:150}（确认 go-cqhttp 已启动、端口正确）";;
esac

sec "3/4 dispatch 渠道: onebot（转发 WS，post_type=message）"
req GET "/admin/integration/channels"
CH_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['channel_key']=='onebot'), '')")
CH_BODY=$(WS_ENDPOINT="$WS_ENDPOINT" python3 -c '
import json,os
print(json.dumps({
  "provider":"onebot","display_name":"QQ 机器人",
  "mode":"stream","transport":"ws","framing":"dispatch","codec":"json",
  "endpoint":os.environ["WS_ENDPOINT"],
  "verify_kind":"none",
  "mapping":{
    "external_id":"$.message_id | as_str",
    "sender":"$.user_id | as_str",
    "payload":{
      "body":"$.raw_message",
      "reply_chat_id":"$.user_id | as_str",
      "reply_group_id":"$.group_id | as_str",
      "reply_message_type":"$.message_type"
    }
  },
  "target_type":"chat/chat_messages",
  "route_extra":{"jobs":[{"job_type":"chat.ingress","max_attempts":1}]},
  "stream_config":{
    "events":{"match":{"path":"$.post_type","equals":"message"},"payload_path":"$"},
    "ws_keepalive":True
  },
  "enabled":True
}, ensure_ascii=False))')
if [ -n "$CH_ID" ]; then
  req DELETE "/admin/integration/channels/$CH_ID"
  [ "$_CODE" = "200" ] && ok "old channel removed" || die "旧渠道删除失败: $_RES"
fi
BODY=$(python3 -c 'import json,sys;b=json.loads(sys.argv[1]);b["channel_key"]="onebot";print(json.dumps(b))' "$CH_BODY")
req POST /admin/integration/channels "$BODY"
CH_ID=$(jget "$_BODY" 'd["data"]["id"]')
[ "$_CODE" = "200" ] && [ -n "$CH_ID" ] && ok "channel created" || die "渠道创建失败: $_RES"

sec "3.5 绑定 chat_inbox（egress + enrich）"
# 注意：admin 创建的渠道默认平台级，工作台「会话→渠道」只显示 app 自有渠道，
# dev 目录模式请手动标记：
#   sqlite3 storage/db/raisfast.db "UPDATE itg_channels SET app_id='chat' WHERE channel_key='onebot';"
req GET "/admin/cms/chat/chat_inboxes?channel_id=$CH_ID"
INBOX_ID=$(jget "$_BODY" "next((r['id'] for r in d['data'] if str(r.get('channel_id'))=='$CH_ID'), '')")
if [ -z "$INBOX_ID" ]; then
  req POST /admin/cms/chat/chat_inboxes "{\"channel_id\":\"$CH_ID\",\"name\":\"QQ 机器人\",\"egress\":{\"kind\":\"api\",\"client\":\"onebot\",\"op\":\"send_msg\",\"input\":{\"message_type\":\"{reply.message_type}\",\"user_id\":\"{reply.chat_id}\",\"group_id\":\"{reply.group_id}\",\"message\":\"{msg.body}\"}},\"enrich\":{\"client\":\"onebot\",\"op\":\"get_stranger_info\",\"input\":{\"user_id\":\"{sender}\"},\"name\":\"nickname\"}}"
  INBOX_ID=$(jget "$_BODY" 'd["data"]["id"]')
  if [ "$_CODE" = "200" ] || [ "$_CODE" = "201" ]; then
    [ -n "$INBOX_ID" ] && ok "chat_inbox created" || die "收件箱创建失败: $_RES"
  else
    die "收件箱创建失败: $_RES"
  fi
else
  req PUT "/admin/cms/chat/chat_inboxes/$INBOX_ID" "{\"egress\":{\"kind\":\"api\",\"client\":\"onebot\",\"op\":\"send_msg\",\"input\":{\"message_type\":\"{reply.message_type}\",\"user_id\":\"{reply.chat_id}\",\"group_id\":\"{reply.group_id}\",\"message\":\"{msg.body}\"}},\"enrich\":{\"client\":\"onebot\",\"op\":\"get_stranger_info\",\"input\":{\"user_id\":\"{sender}\"},\"name\":\"nickname\"}}" || true
  ok "chat_inbox exists (egress/enrich ensured)"
fi

sec "4/4 等待 connected（转发 WS 接通，30s）"
STATE=""; CONNECTED=""
for i in $(seq 1 30); do
  req GET "/admin/integration/channels/health"
  STATE=$(jget "$_BODY" "next((h['status'] for h in d['data'] if h['channel_key']=='onebot'), '?')")
  case "$STATE" in
    connected) CONNECTED=1; break;;
    *) sleep 1;;
  esac
done
if [ -n "$CONNECTED" ]; then ok "channel connected"; else
  req GET "/admin/integration/channels/health"
  ERR=$(jget "$_BODY" "next((str(h.get('last_error')) for h in d['data'] if h['channel_key']=='onebot'), '?')")
  printf '  服务端错误: %s\n' "${ERR:0:250}"
  die "未连上 — 确认 go-cqhttp 正向 WS 开启、access_token 一致"
fi

printf '\n  现在 QQ 私聊机器人 / 群里 @机器人 发消息 → 工作台出现会话（SSE 实时）\n'
printf '  坐席回复 → send_msg 回发给私聊/群聊（空字段自动省略）。\n'
printf '\n\033[1m结果: %d passed, %d failed\033[0m\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
