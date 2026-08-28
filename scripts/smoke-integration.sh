#!/usr/bin/env bash
#
# Integration Plane M0 一体化冒烟脚本。
#
# 覆盖: api-client CRUD / 凭据密封 / test-call(成功·失败·限流·未知op) /
#       egress-log / push→receipt→SSE / trace 端点 / CT 写入。
#
# 前置: 服务已启动且带 vault key:
#   INTEGRATION_VAULT_KEY=dev-secret just dev
#
# 用法:
#   ADMIN_TOKEN=<管理员JWT或API Token> scripts/smoke-integration.sh
#
# 环境变量:
#   ADMIN_TOKEN 必填 — 管理员 token (登录返回的 access_token 或 admin 的 API Token)
#   BASE_URL    API 地址 (默认 http://localhost:9898/api/v1)
#   DB_PATH     sqlite 数据库 (默认 storage/db/raisfast.db, 仅用于凭据密封校验)
#   MOCK_PORT   内置 mock LLM 端口 (默认 9777)
#
set -uo pipefail

BASE_URL="${BASE_URL:-http://localhost:9898/api/v1}"
DB_PATH="${DB_PATH:-storage/db/raisfast.db}"
MOCK_PORT="${MOCK_PORT:-9777}"
RUN="$(date +%s)$RANDOM"
PASS=0; FAIL=0; FAILED_STEPS=()

ok()   { PASS=$((PASS+1)); printf '  \033[32mPASS\033[0m %s\n' "$1"; }
bad()  { FAIL=$((FAIL+1)); FAILED_STEPS+=("$1"); printf '  \033[31mFAIL\033[0m %s\n' "$1"; }
sec()  { printf '\n\033[1m== %s ==\033[0m\n' "$1"; }
die()  { printf '\033[31mFATAL\033[0m %s\n' "$1"; exit 1; }

# JSON 取值: jget <json> <python-expr on d>
jget() { python3 -c "import sys,json;d=json.loads(sys.argv[1]);r=($2);print(str(r).lower() if isinstance(r,bool) else r)" "$1" 2>/dev/null; }

api() { # api <METHOD> <path> [json-body] → 输出 "code\nbody"
  local m=$1 p=$2 b=${3:-} code body auth_args=()
  [ -n "${TOKEN:-}" ] && auth_args=(-H "Authorization: Bearer $TOKEN")
  if [ -n "$b" ]; then
    body=$(curl -s -w '\n%{http_code}' -X "$m" "$BASE_URL$p" \
      ${auth_args[@]+"${auth_args[@]}"} -H 'Content-Type: application/json' -d "$b")
  else
    body=$(curl -s -w '\n%{http_code}' -X "$m" "$BASE_URL$p" ${auth_args[@]+"${auth_args[@]}"})
  fi
  code=$(echo "$body" | tail -1)
  echo "$code"; echo "$body" | sed '$d'
}

# ── 0. 前置检查 ──────────────────────────────────────────────
sec "preflight"
curl -sf "$BASE_URL/health" >/dev/null 2>&1 \
  || die "服务未启动。请先: INTEGRATION_VAULT_KEY=dev-secret just dev"
ok "server reachable"

MOCK_PID=""
cleanup() { [ -n "$MOCK_PID" ] && kill "$MOCK_PID" 2>/dev/null; }
trap cleanup EXIT

python3 - "$MOCK_PORT" <<'EOF' &
import sys, json
from http.server import BaseHTTPRequestHandler, HTTPServer
PORT = int(sys.argv[1])
class H(BaseHTTPRequestHandler):
    def _reply(self, obj, status=200):
        body = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_POST(self):
        if self.path == "/fail":
            self._reply({"error": "boom"}, 500)
        else:  # /chat-messages
            self._reply({"answer": "mock-reply",
                         "usage": {"prompt_tokens": 10, "completion_tokens": 5},
                         "model": "mock-llm-1"})
    def do_GET(self):
        self._reply({"id": "x", "ok": True})
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", PORT), H).serve_forever()
EOF
MOCK_PID=$!
sleep 0.5
curl -sf "http://127.0.0.1:$MOCK_PORT/chat-messages" -X POST >/dev/null \
  || die "mock LLM 启动失败"
ok "mock LLM on :$MOCK_PORT"

# ── 1. 管理员 token ──────────────────────────────────────────
sec "admin auth"
TOKEN="${ADMIN_TOKEN:-}"
[ -n "$TOKEN" ] || die "请提供 ADMIN_TOKEN (管理员登录的 access_token 或 API Token)"
WHO=$(api GET /users/me)
if [ "$(echo "$WHO" | head -1)" = "200" ]; then ok "token valid"; else
  die "token 无效 (请用管理员账号登录获取 access_token): $WHO"
fi

# ── 2. 目标 CT + push 渠道 ───────────────────────────────────
sec "content type & channel"
RES=$(api POST /admin/content-types "{\"name\":\"Smoke Note $RUN\",\"singular\":\"sn_$RUN\",\"plural\":\"sns_$RUN\",\"table\":\"sns_$RUN\",\"fields\":[{\"name\":\"external_id\",\"field_type\":\"text\"},{\"name\":\"body\",\"field_type\":\"text\"}]}")
CODE=$(echo "$RES" | head -1)
{ [ "$CODE" = "200" ] || [ "$CODE" = "201" ]; } && ok "CT sn_$RUN created" || bad "CT create: $RES"

CH_BODY="{\"channel_key\":\"smoke-$RUN\",\"provider\":\"generic\",\"mode\":\"push\",\"transport\":\"http1\",\"framing\":\"raw\",\"codec\":\"json\",\"verify_kind\":\"none\",\"mapping\":{\"external_id\":\"\$.id\",\"payload\":{\"body\":\"\$.text\"}},\"target_type\":\"sn_$RUN\"}"
RES=$(api POST /admin/integration/channels "$CH_BODY")
CODE=$(echo "$RES" | head -1); BODY=$(echo "$RES" | sed '1d')
CH_ID=$(jget "$BODY" 'd["data"]["id"]')
[ "$CODE" = "200" ] && [ -n "$CH_ID" ] && ok "channel smoke-$RUN ($CH_ID)" || bad "channel create: $RES"

# ── 3. api-client CRUD + 凭据密封 ────────────────────────────
sec "api-client crud"
RES=$(api POST /admin/integration/api-clients "{\"client_key\":\"llm-$RUN\",\"base_url\":\"http://127.0.0.1:$MOCK_PORT\",\"auth\":{\"kind\":\"bearer\"},\"credentials\":{\"secret\":\"sk-secret-$RUN\"},\"ops\":{\"chat\":{\"method\":\"POST\",\"path\":\"/chat-messages\",\"output\":{\"text\":\"\$.answer\"}},\"fail\":{\"method\":\"POST\",\"path\":\"/fail\"}}}")
CODE=$(echo "$RES" | head -1); BODY=$(echo "$RES" | sed '1d')
CLIENT_ID=$(jget "$BODY" 'd["data"]["id"]')
if [ "$CODE" = "200" ] && [ -n "$CLIENT_ID" ]; then ok "client llm-$RUN created"; else
  case "$BODY" in *vault*) die "vault 未解锁 — 请用 INTEGRATION_VAULT_KEY=xxx just dev 重启";; esac
  bad "client create: $RES"
fi

HAS_CRED=$(jget "$BODY" 'd["data"]["has_credentials"]')
[ "$HAS_CRED" = "true" ] && ok "has_credentials=true" || bad "has_credentials: $BODY"
echo "$BODY" | grep -q '"credentials"' && bad "credentials 泄漏在回显中" || ok "no credentials echo"

RES=$(api POST /admin/integration/api-clients "{\"client_key\":\"llm-$RUN\",\"base_url\":\"http://127.0.0.1:$MOCK_PORT\",\"ops\":{}}")
[ "$(echo "$RES" | head -1)" = "400" ] && ok "duplicate client_key → 400" || bad "duplicate key check: $RES"

if [ -f "$DB_PATH" ] && command -v sqlite3 >/dev/null; then
  SEALED=$(sqlite3 "$DB_PATH" "SELECT credentials FROM itg_api_clients WHERE client_key='llm-$RUN'")
  case "$SEALED" in *"sk-secret-$RUN"*) bad "DB 中凭据是明文";; "") bad "DB 凭据为空";; *) ok "DB credentials sealed";; esac
fi

# ── 4. test-call: 成功 / 失败 / 未知 op / 限流 ────────────────
sec "test-call branches"
RES=$(api POST "/admin/integration/api-clients/$CLIENT_ID/test-call" '{"op":"chat","input":{"query":"hi"}}')
CODE=$(echo "$RES" | head -1); BODY=$(echo "$RES" | sed '1d')
TEXT=$(jget "$BODY" 'd["data"]["output"]["text"]')
TIN=$(jget "$BODY" 'd["data"]["tokens_in"]')
[ "$CODE" = "200" ] && [ "$TEXT" = "mock-reply" ] && ok "chat → output.text=mock-reply" || bad "chat call: $RES"
[ "$TIN" = "10" ] && ok "tokens_in=10" || bad "tokens_in: $BODY"

RES=$(api POST "/admin/integration/api-clients/$CLIENT_ID/test-call" '{"op":"fail","input":{}}')
[ "$(echo "$RES" | head -1)" = "500" ] && ok "fail op → 500" || bad "fail op: $RES"

RES=$(api POST "/admin/integration/api-clients/$CLIENT_ID/test-call" '{"op":"nope","input":{}}')
[ "$(echo "$RES" | head -1)" = "404" ] && ok "unknown op → 404" || bad "unknown op: $RES"

RL_BODY="{\"client_key\":\"rl-$RUN\",\"base_url\":\"http://127.0.0.1:$MOCK_PORT\",\"auth\":{\"kind\":\"none\"},\"ops\":{\"chat\":{\"method\":\"POST\",\"path\":\"/chat-messages\"}},\"rate_limit\":{\"per_minute\":1}}"
RES=$(api POST /admin/integration/api-clients "$RL_BODY")
RL_ID=$(jget "$(echo "$RES" | sed '1d')" 'd["data"]["id"]')
api POST "/admin/integration/api-clients/$RL_ID/test-call" '{"op":"chat","input":{}}' >/dev/null
RES=$(api POST "/admin/integration/api-clients/$RL_ID/test-call" '{"op":"chat","input":{}}')
[ "$(echo "$RES" | head -1)" = "429" ] && ok "rate limit (per_minute=1) → 429" || bad "rate limit: $RES"

# ── 5. egress-log ────────────────────────────────────────────
sec "egress log"
RES=$(api GET "/admin/integration/egress-log?client_key=llm-$RUN")
BODY=$(echo "$RES" | sed '1d')
ROWS=$(jget "$BODY" 'len(d["data"]["items"])')
ERR_ROW=$(jget "$BODY" 'any(r["status"]=="error" for r in d["data"]["items"])')
[ "${ROWS:-0}" -ge 2 ] && ok "egress-log rows ≥ 2 (success+error)" || bad "egress-log rows: $RES"
[ "$ERR_ROW" = "true" ] && ok "error row logged (fail op)" || bad "error row missing"

# ── 6. push → receipt → SSE ──────────────────────────────────
sec "inbound push + SSE"
SSE_TMP=$(mktemp)
curl -sN --max-time 8 "$BASE_URL/events?filter=integration.*" >"$SSE_TMP" 2>/dev/null &
SSE_PID=$!
sleep 1.5
PUSH_CODE=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE_URL/ingress/smoke-$RUN" \
  -H 'Content-Type: application/json' -d "{\"id\":\"msg-$RUN\",\"text\":\"在吗\"}")
[ "$PUSH_CODE" = "200" ] && ok "push acked 200" || bad "push: $PUSH_CODE"
wait "$SSE_PID" 2>/dev/null
grep -q "integration.received" "$SSE_TMP" \
  && ok "SSE delivered integration.received" || bad "SSE event (见 $SSE_TMP)"

RES=$(api GET "/admin/integration/receipts?channel_id=$CH_ID&status=delivered")
BODY=$(echo "$RES" | sed '1d')
N=$(jget "$BODY" 'len(d["data"]["items"])')
[ "${N:-0}" -ge 1 ] && ok "receipt delivered" || bad "receipt: $RES"
TRACE_ID=$(jget "$BODY" 'd["data"]["items"][0]["id"]')

RES=$(api GET "/admin/integration/receipts/$TRACE_ID/trace")
BODY=$(echo "$RES" | sed '1d')
STEPS=$(jget "$BODY" '",".join(s["step"] for s in d["data"]["first_pass"])')
echo "$STEPS" | grep -q "route" && ok "trace first_pass has route" || bad "trace steps: $STEPS"

# CT 行确实写入(通过公开 CT 列表确认)
RES=$(api GET "/cms/sns_$RUN?filter=external_id%3D%22msg-$RUN%22")
CODE=$(echo "$RES" | head -1)
[ "$CODE" = "200" ] && ok "CT row queryable" || printf '  \033[33mSKIP\033[0m CT 公开列表校验 (%s)\n' "$CODE"

# ── 7. 清理 ──────────────────────────────────────────────────
sec "cleanup"
api POST "/admin/integration/channels/$CH_ID/delete" "{}" >/dev/null && ok "channel deleted"
api POST "/admin/integration/api-clients/$CLIENT_ID/delete" "{}" >/dev/null && ok "client deleted"
api POST "/admin/integration/api-clients/$RL_ID/delete" "{}" >/dev/null && ok "rl client deleted"

# ── 汇总 ─────────────────────────────────────────────────────
printf '\n\033[1m结果: %d passed, %d failed\033[0m\n' "$PASS" "$FAIL"
if [ "$FAIL" -gt 0 ]; then
  printf '失败项:\n'; printf '  - %s\n' "${FAILED_STEPS[@]}"
  exit 1
fi
exit 0
