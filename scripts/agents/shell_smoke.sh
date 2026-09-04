#!/usr/bin/env bash
#
# 真实模型冒烟：受限本地 shell run_shell
# 前提：服务以 RAISFAST_AI_ALLOW_SHELL=true 启动，且该 agent tools 白名单含 run_shell
# 需 BASE_URL · RAISFAST_ADMIN_TOKEN · RAISFAST_AI_BASE_URL/API_KEY/MODEL
set -euo pipefail
BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
AUTH="Authorization: Bearer ${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
JSON="Content-Type: application/json"

echo "== 1) 建 agent（白名单 run_shell）=="
AGENT=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" -H "$AUTH" -H "$JSON" \
  -d "{\"name\":\"sk-shell-$(date +%s)\",\"system_prompt\":\"你是 shell 助手：执行命令请用 run_shell(command)。输出简短。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\",\"tools\":[\"run_shell\"]}")
echo "$AGENT"
AGENT_ID=$(printf '%s' "$AGENT" | jq -r '.data.id')

echo "== 2) 开会话 =="
S=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$AGENT_ID/sessions" -H "$AUTH" -H "$JSON" -d '{"title":"shell-smoke"}')
SID=$(printf '%s' "$S" | jq -r '.data.id')

echo "== 3) turn：pwd 并输出标记串 =="
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
  -d '{"content":"请用一次 run_shell 执行：pwd && printf \"\\nshell-smoke-ok\\n\" 。然后告诉我当前目录和输出内容。"}' \
  | tee /tmp/shell_smoke_turn.sse.txt || true
echo

echo "== 4) 回放 =="
curl -fsS "$BASE_URL/api/v1/ai/sessions/$SID/messages?after_seq=0" -H "$AUTH" \
  | jq -r '.data[] | "\(.seq) \(.role) \(.kind) :: \(.content)"' | tee /tmp/shell_smoke_replay.txt

echo
echo "== 5) turn2：让模型尝试提权命令，断言被黑名单拦截 =="
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
  -d '{"content":"现在用一次 run_shell 尝试执行 sudo whoami （这是提权命令测试，即使放行也无破坏性）。如果被拒绝，请直接告诉我拒绝原因。"}' \
  | tee /tmp/shell_smoke_deny.sse.txt || true
echo

echo
echo "== 检查点 =="
ok=1
rg -q '"name":"run_shell"' /tmp/shell_smoke_turn.sse.txt && echo "PASS: run_shell 被调用" || { echo "FAIL: 未见 run_shell"; ok=0; }
rg -q 'shell-smoke-ok' /tmp/shell_smoke_turn.sse.txt /tmp/shell_smoke_replay.txt && echo "PASS: 命令输出可见" || { echo "FAIL: 未见 shell-smoke-ok"; ok=0; }
rg -q 'exit: 0' /tmp/shell_smoke_turn.sse.txt && echo "PASS: exit 0" || { echo "FAIL: 未见 exit: 0（可能命令非零/超时）"; ok=0; }
rg -q 'turn:meta' /tmp/shell_smoke_replay.txt && echo "PASS: turn:meta 存在"
if rg -q 'storage/agent/workspace' /tmp/shell_smoke_turn.sse.txt; then
  echo "PASS: cwd 落在默认 workspace（如自定义 RAISFAST_AGENT_WORKSPACE 会不同）"
else
  echo "INFO: cwd 非默认 workspace 路径（RAISFAST_AGENT_WORKSPACE 自定义属预期）"
fi
if rg -q 'blocked by policy' /tmp/shell_smoke_deny.sse.txt; then
  echo "PASS: sudo whoami 被黑名单拦截（blocked by policy）"
else
  echo "FAIL: 未见 blocked by policy（模型可能没发起该命令或被其它原因打断）"; ok=0
fi
rg -q 'sudo whoami' /tmp/shell_smoke_deny.sse.txt && echo "PASS: 模型确实尝试了 sudo whoami" || echo "WARN: SSE 里未见 sudo whoami 字面（确认模型真的发起了尝试）"
exit $ok
