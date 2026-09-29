#!/usr/bin/env bash
# MiniMax (海螺) 真机冒烟 — media-platform-roadmap.md §4 步骤 1/2/4
#
# ⚠️ 路径说明（2026-09 教训）：relay `/v1/videos` 是 OpenAI Videos 协议的
# 硬编码代理（POST {base}/videos），只适用于 OpenAI 协议上游；kling/minimax
# 等专有协议厂商必须走 **flows video 节点**（facade → provider_for → JS
# provider 扩展）。本脚本因此走 flows API。
#
# 全链路：登录 admin → 建渠道(provider=minimax) → 建模型(video/per_second)
#   → 建 flow(start→video→end) → run 实例 → 轮询 waiting→success → 验证成片
#
# 用法:
#   scripts/llm/smoke_minimax.sh
#   SMOKE_ADMIN_EMAIL=... SMOKE_ADMIN_PASSWORD=... scripts/llm/smoke_minimax.sh
#
# 环境变量（scripts/llm/.env.local 可覆盖默认值）:
#   MINIMAX_API_KEY        必填。格式 `api_key` 或 `api_key:group_id`
#   RAISFAST_BASE_URL      默认 http://localhost:9898
#   MINIMAX_BASE           默认 https://api.minimax.io（国际版）；CN=api.minimaxi.com
#   SMOKE_ADMIN_EMAIL/PASSWORD  管理台账号（必填）
#   SMOKE_MODEL            默认 MiniMax-H3-Max（V2 快速版，最省）
#   SMOKE_SECONDS          默认 "5"（H3-Max 5-15；H3 4-15）
#   SMOKE_POLL_SECS        实例轮询间隔，默认 20
#   SMOKE_TIMEOUT_SECS     总超时，默认 900

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=/dev/null
[ -f "$DIR/.env.local" ] && set -a && source "$DIR/.env.local" && set +a

BASE_URL="${RAISFAST_BASE_URL:-http://localhost:9898}"
API="$BASE_URL/api/v1"
MINIMAX_BASE="${MINIMAX_BASE:-https://api.minimax.io}"
MODEL="${SMOKE_MODEL:-MiniMax-H3-Max}"
SECONDS_ARG="${SMOKE_SECONDS:-5}"
PROMPT="${SMOKE_PROMPT:-夜晚的城市天际线航拍，霓虹灯倒映在湿漉漉的街道上，电影感，缓慢推进}"
POLL_SECS="${SMOKE_POLL_SECS:-20}"
TIMEOUT_SECS="${SMOKE_TIMEOUT_SECS:-900}"

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

KEY="${MINIMAX_API_KEY:-}"
[ -n "$KEY" ] || die "MINIMAX_API_KEY 未设置（scripts/llm/.env.local 或环境变量）"

# ── 直连探测（零成本）：查询不存在的任务 ──────────────────────────────
# GET 无副作用、不创建任务、不扣费。HTTP 401/403 或 base_resp 1004 = key 无效。
log "直连探测: GET ${MINIMAX_BASE}/v1/query/video_generation?task_id=__smoke_probe__"
P_CODE=$(curl -s -m 15 -o /tmp/minimax_probe.json -w '%{http_code}' \
  "${MINIMAX_BASE}/v1/query/video_generation?task_id=__smoke_probe__" \
  -H "Authorization: Bearer ${KEY%%:*}" || echo "000")
P_BODY=$(python3 -c 'import json,sys
try: print(json.dumps(json.load(open(sys.argv[1])), ensure_ascii=False)[:220])
except Exception: pass' /tmp/minimax_probe.json)
echo "  HTTP ${P_CODE}  ${P_BODY}"
if [ "$P_CODE" = "401" ] || [ "$P_CODE" = "403" ] || [ "$P_CODE" = "000" ]; then
  warn "key 在上游即无效，全栈无意义。核对清单："
  warn "  1. key 是否完整复制；视频仅需 api_key 段（:group_id 可选）"
  warn "  2. 账号区域与端点匹配：国际=api.minimax.io，CN=api.minimaxi.com（MINIMAX_BASE 覆盖）"
  warn "  3. 账户是否有余额/credits"
  exit 2
fi
SC=$(python3 -c 'import json,sys
try: print(json.load(open(sys.argv[1])).get("base_resp", {}).get("status_code", ""))
except Exception: print("")' /tmp/minimax_probe.json)
if [ "$SC" = "1004" ]; then
  warn "base_resp.status_code=1004（鉴权失败）— key 无效，请核对后重试"
  exit 2
fi
ok "探测非鉴权类响应（key 有效，业务层可达）— 继续全栈链路"

# ── 全栈模式（flows video 节点路径）──────────────────────────────────
log "目标: $BASE_URL · 模型: $MODEL · seconds: $SECONDS_ARG · 上游: $MINIMAX_BASE"
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

# 渠道：同 provider+base 已存在即复用，否则创建
log "渠道 provider=minimax（${MINIMAX_BASE}）"
CHANNELS=$(curl -sf "$API/admin/llm/channels" -H "$AUTH")
CH_ID=$(printf '%s' "$CHANNELS" | MINIMAX_BASE="$MINIMAX_BASE" python3 -c '
import json,sys,os
rows=json.load(sys.stdin).get("data") or []
hit=[c for c in rows if c.get("provider")=="minimax" and c.get("base_url")==os.environ["MINIMAX_BASE"]]
print(hit[0]["id"] if hit else "")')
if [ -n "$CH_ID" ]; then
  ok "复用既有 minimax 渠道 id=$CH_ID"
else
  CH=$(curl -sf -X POST "$API/admin/llm/channels" -H "$AUTH" -H "Content-Type: application/json" -d '{
    "name": "minimax-smoke",
    "provider": "minimax",
    "base_url": "'"$MINIMAX_BASE"'",
    "models": "'"$MODEL"'",
    "auto_ban": false,
    "groups": "default",
    "initial_keys": [{"key": "'"$KEY"'"}]
  }') || die "建渠道失败"
  CH_ID=$(printf '%s' "$CH" | jqj data.id)
  ok "渠道已建 id=$CH_ID"
fi
# 幂等启用 key[0]：余额不足 402 会触发 auto_ban 永久禁 key（存储于
# disabled_reason），每轮冒烟前重置，避免 "no available channel" 假故障。
curl -s -X POST "$API/admin/llm/channels/$CH_ID/keys/0/enable" -H "$AUTH" >/dev/null 2>&1 || true
# 幂等同步 key（换 key 后无需删渠道重建）：replace_keys 全量替换
curl -s -X PUT "$API/admin/llm/channels/$CH_ID/keys" -H "$AUTH" -H "Content-Type: application/json" \
  -d '{"keys": [{"key": "'"$KEY"'"}]}' >/dev/null 2>&1 || true

# 模型目录行：缺则建（video / per_second；单价占位，录错请改管理台）
log "模型目录: $MODEL (video / per_second)"
MODELS=$(curl -sf "$API/admin/llm/models" -H "$AUTH")
HAS_MODEL=$(printf '%s' "$MODELS" | MODEL="$MODEL" python3 -c '
import json,sys,os
rows=json.load(sys.stdin).get("data") or []
hit=[m for m in rows if m.get("name")==os.environ["MODEL"]]
print(hit[0]["price_mode"] if hit else "")')
if [ -n "$HAS_MODEL" ]; then
  ok "模型行已存在 (price_mode=$HAS_MODEL)"
else
  curl -sf -X POST "$API/admin/llm/models" -H "$AUTH" -H "Content-Type: application/json" -d '{
    "name": "'"$MODEL"'",
    "model_type": "video",
    "price_mode": "per_second",
    "input_price": 0,
    "output_price": 0.3,
    "status": "active"
  }' >/dev/null || die "建模型失败"
  ok "模型已建（per_second: \$0.3/s 占位价，请按官方牌价到管理台修正）"
fi

# 建 flow：start → video → end（flows 内部消费不计费 §3.4）
log "建 flow: smoke-minimax-<ts>（start → video($MODEL) → end）"
FLOW=$(curl -sf -X POST "$API/admin/flows" -H "$AUTH" -H "Content-Type: application/json" -d '{
  "name": "smoke-minimax-'"$(date +%s)"'",
  "description": "minimax 真机冒烟（脚本生成，可删）",
  "definition": {
    "name": "smoke-minimax",
    "graph": {
      "nodes": [
        {"id": "start", "data": {"type": "start", "version": 1, "config": {"params": []}}},
        {"id": "v1", "data": {"type": "video", "version": 1, "config": {"model": "'"$MODEL"'", "prompt": "'"$PROMPT"'", "seconds": "'"$SECONDS_ARG"'"}}},
        {"id": "end", "data": {"type": "end", "version": 1, "config": {"outputs": [{"key": "video", "value": {"ref": ["v1", "resume"]}}]}}}
      ],
      "edges": [
        {"source": "start", "sourceHandle": "out", "target": "v1"},
        {"source": "v1", "sourceHandle": "out", "target": "end"}
      ]
    }
  }
}') || die "建 flow 失败（definition 校验拒绝？看响应体）"
FLOW_ID=$(printf '%s' "$FLOW" | jqj data.flow_id)
[ -n "$FLOW_ID" ] || die "建 flow 响应无 flow_id: $(printf '%s' "$FLOW" | head -c 300)"
ok "flow 已建 id=$FLOW_ID"

# 运行实例（video 节点提交后 park → 等待轮询聚合器唤醒）
log "run 实例"
RUN=$(curl -sf -X POST "$API/admin/flows/$FLOW_ID/run" -H "$AUTH" -H "Content-Type: application/json" -d '{"inputs": {}}') \
  || die "run 失败"
INSTANCE_ID=$(printf '%s' "$RUN" | jqj data.id)
[ -n "$INSTANCE_ID" ] || die "run 响应无实例 id: $(printf '%s' "$RUN" | head -c 300)"
ok "实例已受理 id=${INSTANCE_ID}（video 节点将 waiting → 轮询聚合器唤醒）"

# 轮询实例至终态
DEADLINE=$(( $(date +%s) + TIMEOUT_SECS ))
STATUS="running"
while :; do
  sleep "$POLL_SECS"
  INST=$(curl -sf "$API/admin/flows/instances/$INSTANCE_ID" -H "$AUTH") \
    || { warn "查询瞬时失败，继续轮询"; continue; }
  STATUS=$(printf '%s' "$INST" | jqj data.status)
  echo "  ${STATUS:-?}"
  case "$STATUS" in
    success|failed|canceled) break ;;
  esac
  [ "$(date +%s)" -lt "$DEADLINE" ] || die "超 ${TIMEOUT_SECS}s 未终态（video 节点默认时限 30min）— 查实例: $API/admin/flows/instances/$INSTANCE_ID"
done

if [ "$STATUS" != "success" ]; then
  ERR=$(printf '%s' "$INST" | jqj data.error)
  warn "实例终态=${STATUS}。error=${ERR:-—}"
  # auto_ban 诊断：上游失败（如 402 余额不足）会自动禁 key → 后续 attempt
  # 报 "no channel" 假象。读渠道 key 的 disabled_reason 给出真实原因。
  KEY_REASON=$(curl -sf "$API/admin/llm/channels" -H "$AUTH" | CH_ID="$CH_ID" python3 -c '
import json,sys,os
rows=json.load(sys.stdin).get("data") or []
for c in rows:
    if c.get("id")==os.environ["CH_ID"]:
        for k in (c.get("keys") or []):
            if k.get("status")!="active":
                print(k.get("disabled_reason") or "(unknown)")
                break
' 2>/dev/null)
  if [ -n "$KEY_REASON" ]; then
    warn "key 已被 auto_ban 禁用，真实原因: $KEY_REASON"
    warn "已自动重新启用（脚本每轮幂等 enable）；如为 402 余额不足请先充值再重跑"
  fi
  exit 1
fi

VIDEO_URL=$(printf '%s' "$INST" | python3 -c 'import json,sys
d=json.load(sys.stdin)
def find_url(o):
    if isinstance(o, dict):
        for k,v in o.items():
            if k=="url" and isinstance(v,str): return v
            r=find_url(v)
            if r: return r
    if isinstance(o, list):
        for x in o:
            r=find_url(x)
            if r: return r
    return ""
print(find_url(d))')
ok "成片: ${VIDEO_URL:-（outputs 内未找到 url，请看完整实例）}"

ok "冒烟通过 ✅  渠道/模型/flow 落库（成片已转存 storage，gen/flows/…）；flow 名 smoke-minimax-* 用完可删"
