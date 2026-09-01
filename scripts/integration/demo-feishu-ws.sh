#!/usr/bin/env bash
#
# 真实例子 2：飞书机器人（WebSocket 长连接模式）实测
#
# 流程: 建 api-client + dispatch 渠道 → 等 connected（token+握手+心跳全过）
#       → 你在飞书里给机器人发消息 → receipt/envelope/CT 行落库
#       → 从原始帧提取 chat_id → test-call 回发一条消息（双向闭环）
#
# 前置:
#   1. 飞书开放平台 https://open.feishu.cn → 创建企业自建应用
#   2. 添加「机器人」能力；事件与回调 → 订阅方式选「使用长连接接收事件」
#   3. 订阅事件 im.message.receive_v1；发布应用（本企业可用）
#   4. 拿到 App ID (cli_xxx) / App Secret
#   5. 服务已启动（可出网访问 open.feishu.cn）:
#      INTEGRATION_VAULT_KEY=dev-secret just dev
#
# 用法:
#   FEISHU_APP_ID=cli_xxx FEISHU_APP_SECRET=xxx ADMIN_TOKEN=... \
#     ./scripts/demo-feishu-ws.sh
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
[ -n "${FEISHU_APP_ID:-}" ] && [ -n "${FEISHU_APP_SECRET:-}" ] \
  || die "请提供 FEISHU_APP_ID / FEISHU_APP_SECRET"
curl -sf "$BASE_URL/health" >/dev/null || die "服务未启动"

req() { _RES=$(curl -s -w '\n%{http_code}' -X "${1}" "$BASE_URL${2}" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  ${3:+-d "$3"}); _CODE=$(echo "$_RES" | tail -1); _BODY=$(echo "$_RES" | sed '$d'); }

CREDS="{\"kind\":\"oauth-cc\",\"token_url\":\"https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal\",\"grant\":{\"app_id\":\"$FEISHU_APP_ID\",\"app_secret\":\"$FEISHU_APP_SECRET\",\"AppID\":\"$FEISHU_APP_ID\",\"AppSecret\":\"$FEISHU_APP_SECRET\"},\"token_path\":\"tenant_access_token\",\"expire_path\":\"expire\"}"

sec "1/5 api-client: feishu (oauth-cc 动态 token)"
req GET "/admin/integration/api-clients"
FEISHU_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['client_key']=='feishu'), '')")
OPS='{"send_text":{"method":"POST","path":"/im/v1/messages?receive_id_type=chat_id","output":{"message_id":"$.data.message_id"}},"get_user":{"method":"GET","path":"/contact/v3/users/{user_id}?user_id_type=open_id","output":{"name":"$.data.name","avatar_url":"$.data.avatar.avatar_72"}}}'
if [ -n "$FEISHU_ID" ]; then
  req PUT "/admin/integration/api-clients/$FEISHU_ID" "{\"credentials\":$CREDS,\"ops\":$OPS,\"enabled\":true}"
  [ "$_CODE" = "200" ] && ok "feishu client refreshed" || die "刷新失败: $_RES"
else
  req POST /admin/integration/api-clients "{\"client_key\":\"feishu\",\"base_url\":\"https://open.feishu.cn/open-apis\",\"auth\":{\"kind\":\"bearer\"},\"credentials\":$CREDS,\"ops\":$OPS}"
  FEISHU_ID=$(jget "$_BODY" 'd["data"]["id"]')
  [ "$_CODE" = "200" ] && [ -n "$FEISHU_ID" ] && ok "feishu client created" || die "创建失败: $_RES"
fi
# token 端点连通性（真实出网验证）。非 2xx 一律被包装成 500，
# 真实原因在 egress-log 的 response_summary 里（token 失败 vs 无效 receive_id 一眼可辨）。
req POST "/admin/integration/api-clients/$FEISHU_ID/test-call" '{"op":"send_text","input":{"receive_id":"oc_demo","msg_type":"text","content":"{\"text\":\"ping\"}"}}'
if [ "$_CODE" = "200" ]; then
  ok "飞书 API 连通（token 获取 + 发送成功）"
else
  req GET "/admin/integration/egress-log?client_key=feishu"
  SUMMARY=$(jget "$_BODY" "d['data']['items'][0].get('response_summary','') if d['data']['items'] else ''")
  HTTP_S=$(jget "$_BODY" "d['data']['items'][0].get('http_status','') if d['data']['items'] else ''")
  printf '  note: 试调 http=%s resp=%s\n' "${HTTP_S:-?}" "${SUMMARY:0:200}"
  case "$SUMMARY" in
    *receive_id*|*invalid*|*not[[:space:]]found*|*40000*|*"code":99991*)
      ok "token 正常（飞书拒绝了演示用的假 receive_id——符合预期）";;
    *token*|*app_access*|*"code":99992*|"")
      die "token/凭据问题: ${SUMMARY:-无响应}";;
    *)
      printf '  [33mWARN[0m 未能归类——请把上面 resp 贴给开发者\n';;
  esac
fi

sec "2/5 dispatch 渠道: feishu (ws 长连接)"
req GET "/admin/integration/channels"
CH_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['channel_key']=='feishu'), '')")
CH_BODY=$(python3 -c '
import json,sys
print(json.dumps({
  "provider":"feishu","display_name":"飞书机器人",
  "mode":"stream","transport":"ws","framing":"pb-frame","codec":"json",
  "endpoint":"wss://placeholder.invalid",
  "verify_kind":"none",
  "credentials":json.loads(sys.argv[1]),
  "mapping":{
    "external_id":"$.header.event_id",
    "sender":"$.event.sender.sender_id.open_id",
    "payload":{
      "body":"$.event.message.content | as_json($.text)",
      "reply_chat_id":"$.event.message.chat_id"
    }
  },
  "target_type":"chat/chat_messages",
  "route_extra":{"jobs":[{"job_type":"chat.ingress","max_attempts":1}]},
  "stream_config":{
    "pre_connect":{
      "url":"https://open.feishu.cn/callback/ws/endpoint",
      "body":{"AppID":"{{AppID}}","AppSecret":"{{AppSecret}}"},
      "headers":{"User-Agent":"oapi-sdk-python/v1.7.3","locale":"zh"},
      "code_path":"$.code","ok_code":0,
      "url_path":"$.data.URL"
    },
    "pb_frame":{
      "ping_interval_secs":25,
      "events":{"equals":"event"},
      "fragment":{"id_header":"message_id","sum_header":"sum","seq_header":"seq"},
      "ack":True,"ack_code":200
    }
  },
  "enabled":True
}, ensure_ascii=False))' "$CREDS")
if [ -n "$CH_ID" ]; then
  # Protocol config is replaced wholesale: delete + recreate.
  req DELETE "/admin/integration/channels/$CH_ID"
  [ "$_CODE" = "200" ] && ok "old channel removed" || die "旧渠道删除失败: $_RES"
fi
if [ -z "$CH_ID" ] || true; then
  BODY=$(python3 -c 'import json,sys;b=json.loads(sys.argv[1]);b["channel_key"]="feishu";print(json.dumps(b))' "$CH_BODY")
  req POST /admin/integration/channels "$BODY"
  CH_ID=$(jget "$_BODY" 'd["data"]["id"]')
  [ "$_CODE" = "200" ] && [ -n "$CH_ID" ] && ok "channel created" || die "渠道创建失败: $_RES"
fi

sec "3/5 等待 connected（换URL→连ws→心跳, 30s）"
STATE=""; CONNECTED=""
for i in $(seq 1 30); do
  req GET "/admin/integration/channels/health"
  STATE=$(jget "$_BODY" "next((h['status'] for h in d['data'] if h['channel_key']=='feishu'), '?')")
  case "$STATE" in
    connected) CONNECTED=1; break;;
    *) sleep 1;;
  esac
done
if [ -n "$CONNECTED" ]; then ok "channel connected（长连接握手+心跳全通）"; else
  req GET "/admin/integration/channels/health"
  ERR=$(jget "$_BODY" "next((str(h.get('last_error')) for h in d['data'] if h['channel_key']=='feishu'), '?')")
  printf '  服务端错误: %s\n' "${ERR:0:200}"
  # curl 对照: 分辨「实现差异」vs「env 值问题」(同 body/headers 直打飞书)
  CRES=$(curl -s --max-time 10 -X POST https://open.feishu.cn/callback/ws/endpoint \
    -H 'Content-Type: application/json' \
    -H 'User-Agent: oapi-sdk-python/v1.7.3' -H 'locale: zh' \
    -d "{\"AppID\":\"$FEISHU_APP_ID\",\"AppSecret\":\"$FEISHU_APP_SECRET\"}")
  case "$CRES" in
    *'"code":0'*) die "curl 直连成功但服务端被拒 — 实现差异，把两条错误贴给开发者";;
    *invalid*) die "curl 直连也报 app_id invalid — FEISHU_APP_ID/SECRET 环境变量值有问题（检查空格/引号/复制完整性）";;
    *) die "curl 直连: ${CRES:0:200}";;
  esac
fi

sec "4/5 在飞书里给机器人发一条消息（120s 内）"
printf '  打开飞书 → 找到你的机器人（应用名）→ 发送任意文字…\n'
GOT=""
for i in $(seq 1 120); do
  req GET "/admin/integration/receipts?channel_id=$CH_ID"
  N=$(jget "$_BODY" 'len(d["data"]["items"])')
  if [ "${N:-0}" -ge 1 ]; then GOT=1; break; fi
  sleep 1
done
[ -n "$GOT" ] || die "120s 未收到事件 — 检查: 应用是否发布/机器人能力/事件订阅 im.message.receive_v1/订阅方式=长连接"

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
  die "按 steps 里的错误修正（多为 mapping 字段名与真实帧不符——看下方 raw 帧校准）"
fi
# 从 VFS 原始归档提取 chat_id（回发要用）
RAW_REF=$(jget "$_BODY" '(d["data"].get("envelope") or {}).get("raw_ref") or ""')
CHAT_ID=""
if [ -n "$RAW_REF" ] && [ -f "$RAW_REF" ]; then
  CHAT_ID=$(python3 -c "
import json,sys
frame=json.load(open('$RAW_REF'))
print(frame.get('event',{}).get('message',{}).get('chat_id',''))" 2>/dev/null)
  printf '  raw frame keys: %s\n' "$(python3 -c "
import json
f=json.load(open('$RAW_REF'));print(list(f.keys()), list(f.get('headers',{}).keys())[:4])" 2>/dev/null)"
fi
[ -n "$CHAT_ID" ] && ok "chat_id=${CHAT_ID} (raw 帧提取)" || printf '  \033[33mSKIP\033[0m chat_id 提取失败（raw_ref=%s）\n' "$RAW_REF"

sec "5/5 回发消息（test-call，双向闭环）"
if [ -n "$CHAT_ID" ]; then
  req POST "/admin/integration/api-clients/$FEISHU_ID/test-call" \
    "{\"op\":\"send_text\",\"input\":{\"receive_id\":\"$CHAT_ID\",\"msg_type\":\"text\",\"content\":\"{\\\"text\\\":\\\"✅ RaisFast 已收到你的消息（dispatch 渠道 + oauth-cc 全链路）\\\"}\"}}"
  if [ "$_CODE" = "200" ]; then
    MID=$(jget "$_BODY" 'd["data"]["output"]["message_id"]')
    ok "已回发 message_id=$MID — 去飞书看机器人的回复！"
  else
    bad "回发失败: $_RES"
  fi
else
  printf '  \033[33mSKIP\033[0m 无 chat_id，跳过回发（可在飞书消息里长按复制 chat_id 手动试）\n'
fi

# trace 对账
req GET "/admin/integration/receipts/$TRACE_ID/trace"
EGRESS=$(jget "$_BODY" 'len(d["data"]["egress"])')
printf '\n  trace egress 调用数: %s（GET /admin/integration/egress-log?client_key=feishu 查详情）\n' "${EGRESS:-0}"
printf '\n\033[1m结果: %d passed, %d failed\033[0m\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
