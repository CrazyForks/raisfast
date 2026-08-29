#!/usr/bin/env bash
#
# 真实例子 3：钉钉机器人（Stream 模式）实测
#
# 流程: 建 dispatch 渠道 → 等 connected（换URL拼ticket→连ws→WS心跳）
#       → 你在钉钉里 @机器人 发消息 → receipt/envelope/CT 行落库（JSON 帧 + as_json）
#
# 前置:
#   1. 钉钉开放平台 https://open-dev.dingtalk.com → 创建企业内部应用
#   2. 添加「机器人」能力；消息接收模式选「Stream 模式」；发布应用
#   3. 拿到 Client ID (ding_xxx/AppKey) / Client Secret (AppSecret)
#   4. 服务已启动（可出网访问 api.dingtalk.com）:
#      INTEGRATION_VAULT_KEY=dev-secret just dev
#
# 用法:
#   DING_APP_ID=ding_xxx DING_APP_SECRET=xxx ADMIN_TOKEN=... \
#     ./scripts/demo-dingtalk-stream.sh
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
[ -n "${ADMIN_TOKEN:-}" ] || die "请提供 ADMIN_TOKEN"
[ -n "${DING_APP_ID:-}" ] && [ -n "${DING_APP_SECRET:-}" ] \
  || die "请提供 DING_APP_ID / DING_APP_SECRET"
curl -sf "$BASE_URL/health" >/dev/null || die "服务未启动"

req() { _RES=$(curl -s -w '\n%{http_code}' -X "${1}" "$BASE_URL${2}" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  ${3:+-d "$3"}); _CODE=$(echo "$_RES" | tail -1); _BODY=$(echo "$_RES" | sed '$d'); }

sec "1/3 连通性: 钉钉 connections/open（真实换址）"
PC_RESP=$(curl -s --max-time 10 -X POST https://api.dingtalk.com/v1.0/gateway/connections/open \
  -H 'Content-Type: application/json' -H 'Accept: application/json' \
  -d "{\"clientId\":\"$DING_APP_ID\",\"clientSecret\":\"$DING_APP_SECRET\",\"subscriptions\":[{\"type\":\"CALLBACK\",\"topic\":\"/v1.0/im/bot/messages/get\"}],\"ua\":\"dingtalk-sdk-python/v0.20-union\",\"localIp\":\"127.0.0.1\"}")
case "$PC_RESP" in
  *ticket*) ok "connections/open 通（endpoint+ticket 已签发）";;
  *) die "换址失败: ${PC_RESP:0:200}";;
esac

sec "2/3 dispatch 渠道: dingtalk（Stream 模式）"
req GET "/admin/integration/channels"
CH_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['channel_key']=='dingtalk'), '')")
CH_BODY=$(DING_APP_ID="$DING_APP_ID" DING_APP_SECRET="$DING_APP_SECRET" python3 -c '
import json,os
print(json.dumps({
  "provider":"dingtalk","display_name":"钉钉机器人",
  "mode":"stream","transport":"ws","framing":"dispatch","codec":"json",
  "endpoint":"wss://placeholder.invalid",
  "verify_kind":"none",
  "credentials":{"grant":{"clientId":os.environ["DING_APP_ID"],"clientSecret":os.environ["DING_APP_SECRET"]}},
  "mapping":{
    "external_id":"$.headers.messageId",
    "sender":"$.data | as_json($.senderStaffId)",
    "payload":{"body":"$.data | as_json($.text.content)"}
  },
  "target_type":"sc_message",
  "stream_config":{
    "pre_connect":{
      "url":"https://api.dingtalk.com/v1.0/gateway/connections/open",
      "body":{
        "clientId":"{{clientId}}","clientSecret":"{{clientSecret}}",
        "subscriptions":[{"type":"CALLBACK","topic":"/v1.0/im/bot/messages/get"}],
        "ua":"dingtalk-sdk-python/v0.20-union","localIp":"127.0.0.1"
      },
      "headers":{"Accept":"application/json"},
      "url_template":"{{endpoint}}?ticket={{ticket}}"
    },
    "ws_keepalive":True,
    "heartbeat_secs":60,
    "events":{"match":{"path":"$.type","equals":"CALLBACK"},"payload_path":"$"},
    "ack_reply":{"code":200,"headers":{"messageId":"{{id}}"},"message":"ok","data":"{}"},
    "ack_reply_id_path":"$.headers.messageId"
  },
  "enabled":True
}, ensure_ascii=False))')
if [ -n "$CH_ID" ]; then
  req POST "/admin/integration/channels/$CH_ID/delete" "{}"
  [ "$_CODE" = "200" ] && ok "old channel removed" || die "旧渠道删除失败: $_RES"
fi
BODY=$(python3 -c 'import json,sys;b=json.loads(sys.argv[1]);b["channel_key"]="dingtalk";print(json.dumps(b))' "$CH_BODY")
req POST /admin/integration/channels "$BODY"
CH_ID=$(jget "$_BODY" 'd["data"]["id"]')
[ "$_CODE" = "200" ] && [ -n "$CH_ID" ] && ok "channel created" || die "渠道创建失败: $_RES"

sec "3/3 等待 connected → 在钉钉里 @机器人 发消息（120s）"
STATE=""; CONNECTED=""
for i in $(seq 1 30); do
  req GET "/admin/integration/channels/health"
  STATE=$(jget "$_BODY" "next((h['status'] for h in d['data'] if h['channel_key']=='dingtalk'), '?')")
  case "$STATE" in
    connected) CONNECTED=1; break;;
    *) sleep 1;;
  esac
done
if [ -n "$CONNECTED" ]; then ok "channel connected（换址→拼ticket→WS心跳全通）"; else
  req GET "/admin/integration/channels/health"
  ERR=$(jget "$_BODY" "next((str(h.get('last_error')) for h in d['data'] if h['channel_key']=='dingtalk'), '?')")
  printf '  服务端错误: %s\n' "${ERR:0:250}"
  die "未连上 — 检查应用凭据/Stream 模式是否开启/出网"
fi

printf '  打开钉钉 → 进入与机器人的会话（或群里 @机器人）→ 发送任意文字…\n'
GOT=""
for i in $(seq 1 120); do
  req GET "/admin/integration/receipts?channel_id=$CH_ID"
  N=$(jget "$_BODY" 'len(d["data"]["items"])')
  if [ "${N:-0}" -ge 1 ]; then GOT=1; break; fi
  sleep 1
done
[ -n "$GOT" ] || die "120s 未收到事件 — 检查: 应用是否发布/机器人能力/消息接收模式=Stream"

req GET "/admin/integration/receipts?channel_id=$CH_ID"
TRACE_ID=$(jget "$_BODY" 'd["data"]["items"][0]["id"]')
R_STATUS=$(jget "$_BODY" 'd["data"]["items"][0]["status"]')
req GET "/admin/integration/receipts/$TRACE_ID"
ENVELOPE=$(jget "$_BODY" 'json.dumps(d["data"]["envelope"],ensure_ascii=False)')
if [ "$R_STATUS" = "delivered" ]; then
  ok "消息已路由（receipt delivered）"
  printf '  envelope: %s\n' "$(echo "$ENVELOPE" | head -c 300)"
else
  bad "receipt 状态 $R_STATUS — steps:"
  jget "$_BODY" 'json.dumps(d["data"]["steps"],ensure_ascii=False)'
  die "按 steps 修正（多为 mapping 字段名与真实帧不符）"
fi

# 回发: 从归档原始帧提取 sessionWebhook（会话级临时 URL，POST 即回）
RAW_REF=$(jget "$_BODY" '(d["data"].get("envelope") or {}).get("raw_ref") or ""')
if [ -n "$RAW_REF" ] && [ -f "$RAW_REF" ]; then
  REPLY=$(python3 -c "
import json, urllib.request
f = json.load(open('$RAW_REF'))
d = json.loads(f['data']) if isinstance(f['data'], str) else f['data']
body = json.dumps({'msgtype':'text','text':{'content':'✅ RaisFast 已收到（dispatch+pre_connect+as_json 全链路）'},'at':{'atUserIds':[d['senderStaffId']]}}).encode()
req = urllib.request.Request(d['sessionWebhook'], data=body, headers={'Content-Type':'application/json'})
print(urllib.request.urlopen(req).read().decode())" 2>/dev/null)
  case "$REPLY" in
    *errcode*) if echo "$REPLY" | grep -q '"errcode":0'; then
                 ok "已回复 (errcode 0), 去钉钉看机器人的消息!"
               else
                 bad "回发失败: ${REPLY:-无响应}"
               fi ;;
    *) bad "回发失败: ${REPLY:-无响应}";;
  esac
else
  printf '  \033[33mSKIP\033[0m 无归档帧，跳过回发\n'
fi

printf '\n\033[1m结果: %d passed, %d failed\033[0m\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
# trace 对账
req GET "/admin/integration/receipts/$TRACE_ID/trace"
EGRESS=$(jget "$_BODY" 'len(d["data"]["egress"])')
printf '\n  trace egress 调用数: %s（GET /admin/integration/egress-log?client_key=feishu 查详情）\n' "${EGRESS:-0}"
# 回发: 从归档原始帧提取 sessionWebhook（会话级临时 URL，POST 即回）
RAW_REF=$(jget "$_BODY" '(d["data"].get("envelope") or {}).get("raw_ref") or ""')
if [ -n "$RAW_REF" ] && [ -f "$RAW_REF" ]; then
  REPLY=$(python3 -c "
import json, urllib.request
f = json.load(open('$RAW_REF'))
d = json.loads(f['data']) if isinstance(f['data'], str) else f['data']
body = json.dumps({'msgtype':'text','text':{'content':'✅ RaisFast 已收到（dispatch+pre_connect+as_json 全链路）'},'at':{'atUserIds':[d['senderStaffId']]}}).encode()
req = urllib.request.Request(d['sessionWebhook'], data=body, headers={'Content-Type':'application/json'})
print(urllib.request.urlopen(req).read().decode())" 2>/dev/null)
  case "$REPLY" in
    *errcode*) if echo "$REPLY" | grep -q '"errcode":0'; then
                 ok "已回复 (errcode 0), 去钉钉看机器人的消息!"
               else
                 bad "回发失败: ${REPLY:-无响应}"
               fi ;;
    *) bad "回发失败: ${REPLY:-无响应}";;
  esac
else
  printf '  \033[33mSKIP\033[0m 无归档帧，跳过回发\n'
fi

printf '\n\033[1m结果: %d passed, %d failed\033[0m\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
