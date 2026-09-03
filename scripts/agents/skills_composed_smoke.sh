#!/usr/bin/env bash
#
# M5-B 真实模型冒烟：skill__<tool> 组合注册 + availability（tools:/disallowed-tools: frontmatter）
#
# 需先起服务并设好：
#   BASE_URL(默认 127.0.0.1:9898) · RAISFAST_ADMIN_TOKEN · RAISFAST_AI_BASE_URL/API_KEY/MODEL
# 说明：平台工具 today 必须也列入 agent.tools 白名单 —— availability 判定基于
# allowlist 之后的注册表；组合壳 skill__today 随即可用，技能正文引导模型只调组合名。
set -euo pipefail
BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
AUTH="Authorization: Bearer ${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
JSON="Content-Type: application/json"
SKILL_DIR="storage/skills/platform/date-comp"

mkdir -p "$SKILL_DIR"
cat > "$SKILL_DIR/SKILL.md" <<'EOF'
---
name: date-comp
description: 组合工具冒烟技能：通过声明 tools 暴露 datenow__today，禁止调用裸 today。
tools:
  - today
disallowed-tools:
  - memory_store
---
# 日期组合工具
- 用户问日期/星期/今天几号这类问题时，必须调用工具列表里名字带 `date-comp__today` 的组合工具，
  绝不要调用裸的 `today`。
- 用其返回的日期作答，结尾追加一句"（via date-comp）"。
EOF

echo "== 1) 建 agent（启用 date-comp；白名单 today；Full 注入）=="
AGENT=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" -H "$AUTH" -H "$JSON" \
  -d "{\"name\":\"sk-cmp-$(date +%s)\",\"system_prompt\":\"按技能指示调用组合工具。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\",\"tools\":[\"today\"],\"params\":{\"skill_bundles\":[\"date-comp\"]}}")
echo "$AGENT"
AGENT_ID=$(printf '%s' "$AGENT" | jq -r '.data.id')

echo "== 2) 开会话 =="
S=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$AGENT_ID/sessions" -H "$AUTH" -H "$JSON" -d '{"title":"composed-smoke"}')
SID=$(printf '%s' "$S" | jq -r '.data.id')

echo "== 3) turn：问日期（应触发 date-comp__today 而非裸 today）=="
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
  -d '{"content":"今天几号？请严格用技能声明过的组合工具取日期，不要用裸 today。"}' \
  | tee /tmp/skills_composed_turn.sse.txt || true
echo

echo "== 4) 回放：断言组合工具行进 tool 审计 =="
curl -fsS "$BASE_URL/api/v1/ai/sessions/$SID/messages?after_seq=0" -H "$AUTH" \
  | jq -r '.data[] | "\(.seq) \(.role) \(.kind) :: \(.content)"' | tee /tmp/skills_composed_replay.txt

echo
echo "== 检查点 =="
if rg -q 'date-comp__today' /tmp/skills_composed_turn.sse.txt /tmp/skills_composed_replay.txt; then
  echo "PASS: 出现 date-comp__today 组合工具调用"
else
  echo "FAIL: 未见 date-comp__today（模型可能调了裸 today —— SSE/回放里查 name 确认，可重试换措辞）"
fi
if rg -q '"name":"today"' /tmp/skills_composed_turn.sse.txt; then
  echo "WARN: SSE 里同时出现裸 today 调用（非致命，但说明引导不充分）"
else
  echo "PASS: 未见裸 today 调用"
fi
rg -q 'turn:meta' /tmp/skills_composed_replay.txt && echo "PASS: turn:meta 存在（system_hash/prompt 预算信号可查）"

echo
echo "清理：删除技能目录与 agent（agent 需 DELETE 路由，暂无则保留）"
rm -rf "$SKILL_DIR"
curl -sS -X DELETE "$BASE_URL/api/v1/admin/ai/agents/$AGENT_ID" -H "$AUTH" >/dev/null 2>&1 \
  || echo "(无 DELETE /agents 路由，agent 保留；技能目录已删)"
