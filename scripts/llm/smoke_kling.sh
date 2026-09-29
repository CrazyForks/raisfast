#!/usr/bin/env bash
# Kling (可灵) 真机冒烟 — media-platform-roadmap.md §4 步骤 1/2/4
#
# 全链路走本仓自己的栈（不是直连上游）：
#   登录 admin → 建渠道(provider=kling) → 建模型(video/per_second)
#   → 铸 relay token → POST /v1/videos 提交 → 轮询至终态 → 对账 llm_logs
#
# 用法:
#   scripts/llm/smoke_kling.sh                 # 读 scripts/llm/.env.local
#   SMOKE_ADMIN_EMAIL=... SMOKE_ADMIN_PASSWORD=... scripts/llm/smoke_kling.sh
#
# 环境变量（.env.local 可覆盖默认值）:
#   KLING_API_KEY          必填。官方台为 `access_key:secret_key` 双段；
#                          单段 key 无法签 JWT，脚本会降级为直连探测并给出指引
#   RAISFAST_BASE_URL      默认 http://localhost:9898
#   SMOKE_ADMIN_EMAIL/PASSWORD  管理台账号（全栈模式必填）
#   SMOKE_MODEL            默认 kling-v1-6
#   SMOKE_SECONDS          默认 "5"（可灵 wire 字符串，5/10）
#   SMOKE_PROMPT           默认一支城市夜景航拍
#   SMOKE_POLL_SECS        轮询间隔，默认 15
#   SMOKE_TIMEOUT_SECS     总超时，默认 600

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=/dev/null
[ -f "$DIR/.env.local" ] && set -a && source "$DIR/.env.local" && set +a

BASE_URL="${RAISFAST_BASE_URL:-http://localhost:9898}"
API="$BASE_URL/api/v1"
SECONDS_ARG="${SMOKE_SECONDS:-5}"
PROMPT="${SMOKE_PROMPT:-夜晚的城市天际线航拍，霓虹灯倒映在湿漉漉的街道上，电影感，缓慢推进}"
POLL_SECS="${SMOKE_POLL_SECS:-15}"
TIMEOUT_SECS="${SMOKE_TIMEOUT_SECS:-600}"

log()  { printf '\033[1;34m[smoke]\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m[ ok ]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[warn]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[fail]\033[0m %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null || die "curl 未安装"
command -v python3 >/dev/null || die "python3 未安装"

# JSON 取值 helper: jqj <json> <filter>
jqj() { python3 -c 'import json,sys;d=json.load(sys.stdin);
for k in sys.argv[1].split("."):
    d = d[int(k)] if isinstance(d, list) else (d or {}).get(k)
print("" if d is None else d)' "$1" 2>/dev/null; }

KEY="${KLING_API_KEY:-}"
[ -n "$KEY" ] || die "KLING_API_KEY 未设置（scripts/llm/.env.local 或环境变量）"

# ── key 形态即协议选择器（kling.js 双协议，2026-09）───────────────────
# - `ak:sk` 双段 → 旧版 API：base=api.klingai.com，模型 kling-v1-6
# - 单段 API Key（klingai.com/dev 新版平台）→ 新版 API：
#   base=api-beijing.klingai.com，模型 kling-3.0（路径内嵌 /text-to-video/{model}）
if [[ "$KEY" != *:* ]]; then
  KLING_BASE="${KLING_BASE:-https://api-beijing.klingai.com}"
  MODEL="${SMOKE_MODEL:-kling-3.0}"
  warn "单段 API Key（klingai.com/dev 新版平台）→ 新版协议 ${KLING_BASE}，模型 $MODEL"
  echo
  probe() { # $1=path $2=body — 诊断走 stderr，stdout 仅回传 HTTP code
    local code
    code=$(curl -s -m 15 -o /tmp/kling_probe.json -w '%{http_code}' -X POST "$1" \
      -H "Authorization: Bearer $KEY" \
      -H "Content-Type: application/json" \
      -d "$2" || echo "000")
    python3 -c 'import json,sys
try: print("    ", json.dumps(json.load(open(sys.argv[1])), ensure_ascii=False)[:220])
except Exception: pass' /tmp/kling_probe.json >&2
    printf '%s' "$code"
  }
  log "直连探测: POST $KLING_BASE/text-to-video/${MODEL}（Bearer）"
  C1=$(probe "$KLING_BASE/text-to-video/$MODEL" \
    '{"prompt":"smoke probe","settings":{"duration":5}}')
  if [ "$C1" = "401" ] || [ "$C1" = "403" ] || [ "$C1" = "000" ]; then
    warn "未通过鉴权 — key 在上游即无效，全栈无意义。核对清单："
    warn "  1. klingai.com/dev/api-key 页 key 是否完整复制（无多余空格/换行）"
    warn "  2. key 是否已启用/绑定计费（资源包/试用包，见 klingai.com/dev/pricing）"
    warn "  3. 接口文档: https://klingai.com/document-api/guides/get-started/quick-start"
    exit 2
  fi
  ok "探测非鉴权类响应（key 有效）— 继续全栈链路"
else
  KLING_BASE="${KLING_BASE:-https://api.klingai.com}"
  MODEL="${SMOKE_MODEL:-kling-v1-6}"
  log "ak:sk 双段 key → 旧版协议 ${KLING_BASE}（JWT），模型 $MODEL"
fi

# ── 全栈模式 ─────────────────────────────────────────────────────────
log "目标: $BASE_URL · 模型: $MODEL · seconds: $SECONDS_ARG"
curl -sf "$BASE_URL/health" >/dev/null || die "服务不可达: $BASE_URL/health（先 just dev）"

ADMIN_EMAIL="${SMOKE_ADMIN_EMAIL:-}"
ADMIN_PASSWORD="${SMOKE_ADMIN_PASSWORD:-}"
[ -n "$ADMIN_EMAIL" ] && [ -n "$ADMIN_PASSWORD" ] || die "全栈模式需要 SMOKE_ADMIN_EMAIL / SMOKE_ADMIN_PASSWORD"

log "登录 admin → token"
LOGIN=$(curl -sf -X POST "$API/auth/login" -H "Content-Type: application/json" \
  -d "{\"email\":\"$ADMIN_EMAIL\",\"password\":\"$ADMIN_PASSWORD\"}") || die "登录失败"
ACCESS=$(printf '%s' "$LOGIN" | jqj data.access_token)
[ -n "$ACCESS" ] || die "登录响应无 access_token"
AUTH="Authorization: Bearer $ACCESS"

# 渠道：存在即复用，否则创建
log "渠道 provider=kling"
CHANNELS=$(curl -sf "$API/admin/llm/channels" -H "$AUTH")
CH_ID=$(printf '%s' "$CHANNELS" | KLING_BASE="$KLING_BASE" python3 -c '
import json,sys,os
rows=json.load(sys.stdin).get("data") or []
hit=[c for c in rows if c.get("provider")=="kling" and c.get("base_url")==os.environ["KLING_BASE"]]
print(hit[0]["id"] if hit else "")')
if [ -n "$CH_ID" ]; then
  ok "复用既有 kling 渠道 id=$CH_ID"
else
  CH=$(curl -sf -X POST "$API/admin/llm/channels" -H "$AUTH" -H "Content-Type: application/json" -d '{
    "name": "kling-smoke",
    "provider": "kling",
    "base_url": "'"$KLING_BASE"'",
    "models": "'"$MODEL"'",
    "auto_ban": false,
    "groups": "default",
    "initial_keys": [{"key": "'"$KEY"'"}]
  }') || die "建渠道失败"
  CH_ID=$(printf '%s' "$CH" | jqj data.id)
  ok "渠道已建 id=$CH_ID"
fi
# 幂等启用 key[0]：上游 4xx（余额/风控）会触发 auto_ban 禁 key，每轮冒烟
# 前重置，避免 "no available channel" 假故障。
curl -s -X POST "$API/admin/llm/channels/$CH_ID/keys/0/enable" -H "$AUTH" >/dev/null 2>&1 || true
# 幂等同步 key（换 key 后无需删渠道重建）：replace_keys 全量替换
curl -s -X PUT "$API/admin/llm/channels/$CH_ID/keys" -H "$AUTH" -H "Content-Type: application/json" \
  -d '{"keys": [{"key": "'"$KEY"'"}]}' >/dev/null 2>&1 || true

# 模型目录行：缺则建（video / per_second）
log "模型目录: $MODEL (video / per_second)"
MODELS=$(curl -sf "$API/admin/llm/models" -H "$AUTH")
HAS_MODEL=$(printf '%s' "$MODELS" | MODEL="$MODEL" python3 -c '
import json,sys,os
rows=json.load(sys.stdin).get("data") or []
hit=[m for m in rows if m.get("name")==os.environ["MODEL"]]
print(hit[0]["price_mode"] if hit else "")')
if [ -n "$HAS_MODEL" ]; then
  ok "模型行已存在 (price_mode=$HAS_MODEL)"
  [ "$HAS_MODEL" = "per_second" ] || warn "price_mode=$HAS_MODEL ≠ per_second — 计价口径不是每秒，账单对账会失真"
else
  curl -sf -X POST "$API/admin/llm/models" -H "$AUTH" -H "Content-Type: application/json" -d '{
    "name": "'"$MODEL"'",
    "model_type": "video",
    "price_mode": "per_second",
    "input_price": 0,
    "output_price": 0.5,
    "status": "active"
  }' >/dev/null || die "建模型失败"
  ok "模型已建（per_second: \$0.5/s，录错请到管理台改）"
fi

# relay token（明文只出现一次 → 每轮新铸，跑完自行到管理台清理）
log "铸 relay token (smoke-kling-<ts>, unlimited)"
SK=$(curl -sf -X POST "$API/admin/llm/tokens" -H "$AUTH" -H "Content-Type: application/json" -d '{
  "name": "smoke-kling-'"$(date +%s)"'",
  "unlimited_quota": true
}' | jqj data.key)
[ -n "$SK" ] || die "铸 token 失败"
ok "token 就绪 (${SK:0:8}…)"

# 提交
log "POST /v1/videos （per_second 预扣: 0 + ${SECONDS_ARG}×\$0.5）"
SUBMIT=$(curl -s -X POST "$BASE_URL/v1/videos" \
  -H "Authorization: Bearer $SK" -H "Content-Type: application/json" \
  -d "{\"model\":\"$MODEL\",\"prompt\":\"$PROMPT\",\"seconds\":\"$SECONDS_ARG\"}") \
  || die "提交失败: $(printf '%s' "$SUBMIT" | head -c 300)"
[ -n "$SUBMIT" ] || die "提交失败：空响应（服务日志可能有详情）"
if printf '%s' "$SUBMIT" | grep -q '"error"'; then
  die "提交被拒: $(printf '%s' "$SUBMIT" | head -c 400)"
fi
TASK_ID=$(printf '%s' "$SUBMIT" | jqj task_id)
[ -n "$TASK_ID" ] || TASK_ID=$(printf '%s' "$SUBMIT" | jqj id)
[ -n "$TASK_ID" ] || die "提交响应无任务 id: $(printf '%s' "$SUBMIT" | head -c 300)"
ok "任务已受理 id=$TASK_ID"

# 轮询至终态
DEADLINE=$(( $(date +%s) + TIMEOUT_SECS ))
STATUS="queued"
while :; do
  sleep "$POLL_SECS"
  RESP=$(curl -sf "$BASE_URL/v1/videos/$TASK_ID" -H "Authorization: Bearer $SK") \
    || { warn "查询瞬时失败，继续轮询"; continue; }
  STATUS=$(printf '%s' "$RESP" | jqj status)
  echo "  ${STATUS:-?}"
  case "$STATUS" in
    completed|failed|expired) break ;;
  esac
  [ "$(date +%s)" -lt "$DEADLINE" ] || die "超 ${TIMEOUT_SECS}s 未终态（任务 TTL 30min，可稍后 GET /v1/videos/$TASK_ID 重查）"
done

if [ "$STATUS" != "completed" ]; then
  die "任务终态=${STATUS}（预期 completed）。详情: GET /v1/videos/$TASK_ID"
fi

VIDEO_URL=$(printf '%s' "$RESP" | python3 -c 'import json,sys
d=json.load(sys.stdin)
def find_url(o):
    if isinstance(o, dict):
        for k,v in o.items():
            if k=="url" and isinstance(v,str) and v.startswith("http"): return v
            r=find_url(v)
            if r: return r
    if isinstance(o, list):
        for x in o:
            r=find_url(x)
            if r: return r
    return ""
print(find_url(d))')
ok "成片: ${VIDEO_URL:-（响应内未找到 url，请看完整响应）}"

# 对账：llm_logs 最新一行（quota 应 ≈ ceil(0 + N×0.5×1e6)）
log "对账 llm_logs（per_second: 预期 quota = ceil(${SECONDS_ARG}×0.5×1e6) = $((SECONDS_ARG * 500000))）"
LOG=$(curl -sf "$API/admin/llm/logs?model_name=$MODEL&page_size=1" -H "$AUTH" | jqj data.0)
printf '  quota=%s cost_quota=%s completion_tokens(=秒)=%s status=%s\n' \
  "$(printf '%s' "$LOG" | jqj quota)" \
  "$(printf '%s' "$LOG" | jqj cost_quota)" \
  "$(printf '%s' "$LOG" | jqj completion_tokens)" \
  "$(printf '%s' "$LOG" | jqj status_code)"

ok "冒烟通过 ✅  渠道/模型/日志落库；token 名 smoke-kling-* 用完可删"
