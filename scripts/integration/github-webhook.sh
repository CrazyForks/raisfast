#!/usr/bin/env bash
#
# 真实例子 4：GitHub Webhook（HMAC-SHA256 前缀形态）实测
#
# 链路: ngrok/cloudflared 隧道 → GitHub 仓库事件 push 进来 →
#       hmac-sha256 验签（x-hub-signature-256, sha256= 前缀, hex）→
#       mapping 落库 → receipts/trace → （可选）SSE
#
# 前置:
#   1. 隧道工具（ngrok 或 cloudflared），本脚本会自动探测：
#      - ngrok:  ngrok http 9898  （另一个终端）
#      - cloudflared: cloudflared tunnel --url http://localhost:9898
#   2. GitHub 仓库的管理员权限（设置 webhook）
#   3. 服务已启动: INTEGRATION_VAULT_KEY=dev-secret just dev
#
# 用法:
#   GITHUB_SECRET=whsec_xxx GITHUB_PUBLIC_URL=https://xxxx.ngrok.app \
#     ADMIN_TOKEN=... ./github-webhook.sh setup     # 建渠道+给出 GitHub 配置指引
#   GITHUB_SECRET=whsec_xxx ADMIN_TOKEN=... ./github-webhook.sh wait  # 等事件
#   GITHUB_SECRET=whsec_xxx ADMIN_TOKEN=... ./github-webhook.sh ping  # 用 API 发 ping 事件
#
set -uo pipefail

BASE_URL="${BASE_URL:-http://localhost:9898/api/v1}"
CMD="${1:-setup}"

ok()  { printf '  \033[32mPASS\033[0m %s\n' "$1"; }
bad() { printf '  \033[31mFAIL\033[0m %s\n' "$1"; }
sec() { printf '\n\033[1m== %s ==\033[0m\n' "$1"; }
die() { printf '\033[31mFATAL\033[0m %s\n' "$1"; exit 1; }
jget() { python3 -c "import sys,json;d=json.loads(sys.argv[1]);print($2)" "$1" 2>/dev/null; }

[ -n "${ADMIN_TOKEN:-}" ] || die "请提供 ADMIN_TOKEN"
[ -n "${GITHUB_SECRET:-}" ] || die "请提供 GITHUB_SECRET（GitHub webhook 的 Secret）"
curl -sf "$BASE_URL/health" >/dev/null || die "服务未启动"

req() { _RES=$(curl -s -w '\n%{http_code}' -X "${1}" "$BASE_URL${2}" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  ${3:+-d "$3"}); _CODE=$(echo "$_RES" | tail -1); _BODY=$(echo "$_RES" | sed '$d'); }

case "$CMD" in
setup)
  sec "1/2 渠道: github（hmac-sha256, GitHub 形态）"
  req GET "/admin/integration/channels"
  CH_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['channel_key']=='github'), '')")
  CH_BODY=$(GITHUB_SECRET="$GITHUB_SECRET" python3 -c '
import json,os
print(json.dumps({
  "provider":"github","display_name":"GitHub Webhook",
  "mode":"push","transport":"http1","framing":"raw","codec":"json",
  "endpoint":None,"verify_kind":"hmac-sha256",
  "verify_config":{"header":"x-hub-signature-256","scheme":"sha256=","encoding":"hex"},
  "credentials":{"secret":os.environ["GITHUB_SECRET"]},
  "mapping":{
    "external_id":"$._headers.x-github-delivery",
    "payload":{"body":"$.action | default(\"ping\")"}
  },
  "target_type":"ingress_note",
  "enabled":True
}, ensure_ascii=False))')
  if [ -n "$CH_ID" ]; then
    req POST "/admin/integration/channels/$CH_ID/delete" "{}"
    ok "old channel removed"
  fi
  BODY=$(python3 -c 'import json,sys;b=json.loads(sys.argv[1]);b["channel_key"]="github";print(json.dumps(b))' "$CH_BODY")
  req POST /admin/integration/channels "$BODY"
  CH_ID=$(jget "$_BODY" 'd["data"]["id"]')
  [ "$_CODE" = "200" ] && [ -n "$CH_ID" ] && ok "channel github created" || die "创建失败: $_RES"

  # CT target
  req POST /admin/content-types '{"name":"Ingress Note","singular":"ingress_note","plural":"ingress_notes","table":"ingress_notes","fields":[{"name":"external_id","field_type":"text"},{"name":"body","field_type":"text"}]}'
  case "$_CODE" in 200|201|409) ok "CT ingress_note ready ($_CODE)";; *) die "CT: $_RES";; esac

  sec "2/2 在 GitHub 上配置 webhook"
  PUBLIC="${GITHUB_PUBLIC_URL:-}"
  if [ -z "$PUBLIC" ]; then
    printf '  未提供 GITHUB_PUBLIC_URL — 探测本机隧道:\n'
    PUBLIC=$(curl -s --max-time 3 http://127.0.0.1:4040/api/tunnels 2>/dev/null | jget "$(curl -s --max-time 3 http://127.0.0.1:4040/api/tunnels 2>/dev/null)" 'd["tunnels"][0]["public_url"]' 2>/dev/null)
  fi
  cat <<EOS
  GitHub → 你的仓库 → Settings → Webhooks → Add webhook:
    Payload URL : \${PUBLIC}/api/v1/ingress/github${PUBLIC:+  （探测到: ${PUBLIC}/api/v1/ingress/github）}
    Content type: application/json
    Secret      : 你的 GITHUB_SECRET 同值
    Events      : 选 "Let me select individual events" → Issues / Push / 或 "*"（send everything）
  保存后 GitHub 会发 ping（delivery id 形如 xxxx-xxxx）。

  然后跑:  GITHUB_SECRET=... ADMIN_TOKEN=... $0 wait
  或自动触发: GITHUB_SECRET=... ADMIN_TOKEN=... $0 ping（经 GitHub API 发 ping，需 GH_TOKEN）
EOS
  ;;

wait)
  sec "等待 GitHub 事件（180s）— 去 GitHub 触发（提 issue/push/ping）"
  req GET "/admin/integration/channels"
  CH_ID=$(jget "$_BODY" "next((c['id'] for c in d['data'] if c['channel_key']=='github'), '')")
  [ -n "$CH_ID" ] || die "渠道不存在，先跑 setup"
  GOT=""
  for i in $(seq 1 180); do
    req GET "/admin/integration/receipts?channel_id=$CH_ID"
    N=$(jget "$_BODY" 'len(d["data"]["items"])')
    if [ "${N:-0}" -ge 1 ]; then GOT=1; break; fi
    sleep 1
  done
  [ -n "$GOT" ] || die "180s 无事件 — 检查: GitHub webhook 配置/隧道在线/Recent Deliveries 状态"
  req GET "/admin/integration/receipts?channel_id=$CH_ID"
  TRACE_ID=$(jget "$_BODY" 'd["data"]["items"][0]["id"]')
  R_STATUS=$(jget "$_BODY" 'd["data"]["items"][0]["status"]')
  req GET "/admin/integration/receipts/$TRACE_ID"
  ENVELOPE=$(jget "$_BODY" 'json.dumps(d["data"]["envelope"],ensure_ascii=False)')
  if [ "$R_STATUS" = "delivered" ]; then
    ok "GitHub 事件已验签+路由（receipt delivered）"
    printf '  envelope: %s\n' "$(echo "$ENVELOPE" | head -c 300)"
  else
    bad "receipt $R_STATUS — steps:"
    jget "$_BODY" 'json.dumps(d["data"]["steps"],ensure_ascii=False)'
    exit 1
  fi
  ;;

ping)
  # 用 GH_TOKEN 调 GitHub API 给 webhook 发 ping（需要仓库写权限）
  [ -n "${GH_REPO:-}" ] && [ -n "${GH_TOKEN:-}" ] || die "ping 需要 GH_REPO（owner/name）与 GH_TOKEN"
  req GET "/admin/integration/channels"
  HOOK_URL="https://api.github.com/repos/${GH_REPO}/hooks"
  HOOKS=$(curl -s -H "Authorization: Bearer $GH_TOKEN" "$HOOK_URL")
  HOOK_ID=$(jget "$HOOKS" "next((h['id'] for h in d if '/ingress/github' in h.get('config',{}).get('url','')), None)")
  [ -n "$HOOK_ID" ] || die "仓库没有指向 /ingress/github 的 webhook — 先 setup + GitHub 配置"
  RESP=$(curl -s -w '%{http_code}' -X POST -H "Authorization: Bearer $GH_TOKEN" "$HOOK_URL/$HOOK_ID/pings")
  case "$RESP" in 202*) ok "ping 已发（202），跑 wait 等收据";; *) die "ping 失败: $RESP";; esac
  ;;
*)
  die "用法: $0 setup|wait|ping"
  ;;
esac
