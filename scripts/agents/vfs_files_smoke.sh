#!/usr/bin/env bash
#
# 真实模型冒烟：托管文件工具 file_write/file_read/file_list/file_delete
# （复用 VirtualFs，工作区默认 storage/agent/workspace/{tenant}/）
# 需先起服务并设好 BASE_URL · RAISFAST_ADMIN_TOKEN · RAISFAST_AI_BASE_URL/API_KEY/MODEL
set -euo pipefail
BASE_URL="${BASE_URL:-http://127.0.0.1:9898}"
MODEL="${RAISFAST_AI_MODEL:-deepseek-chat}"
AUTH="Authorization: Bearer ${RAISFAST_ADMIN_TOKEN:?set RAISFAST_ADMIN_TOKEN}"
JSON="Content-Type: application/json"
WS="${RAISFAST_AGENT_WORKSPACE:-storage/agent/workspace}"
TENANT="default"   # admin 冒烟租户
FILE="notes/2026-09-04.md"

echo "== 1) 建 agent（白名单 file_write/file_read/file_list/file_delete）=="
AGENT=$(curl -fsS -X POST "$BASE_URL/api/v1/admin/ai/agents" -H "$AUTH" -H "$JSON" \
  -d "{\"name\":\"sk-files-$(date +%s)\",\"system_prompt\":\"你是工作区文件助手：写文件用 file_write(path/content)、读用 file_read(path)、列目录用 file_list(dir 可空)。输出简短。\",\"provider\":\"openai_compat\",\"model\":\"$MODEL\",\"tools\":[\"file_write\",\"file_read\",\"file_list\",\"file_delete\"]}")
echo "$AGENT"
AGENT_ID=$(printf '%s' "$AGENT" | jq -r '.data.id')

echo "== 2) 开会话 =="
S=$(curl -fsS -X POST "$BASE_URL/api/v1/ai/agents/$AGENT_ID/sessions" -H "$AUTH" -H "$JSON" -d '{"title":"vfs-files-smoke"}')
SID=$(printf '%s' "$S" | jq -r '.data.id')

echo "== 3) turn：写一份 md → file_list 确认 → file_read 回来 =="
curl -fsSN -N -X POST "$BASE_URL/api/v1/ai/sessions/$SID/turns" -H "$AUTH" -H "$JSON" \
  -d "{\"content\":\"请依次调用：file_write path='$FILE'、content='# 今日\n- 上午写接口\n- 下午修 bug'；然后 file_list（dir 空）看看有没有 notes/；再 file_read path='$FILE'。最后告诉我文件内容的一句摘要。\"}" \
  | tee /tmp/vfs_files_turn.sse.txt || true
echo

echo "== 4) 回放 =="
curl -fsS "$BASE_URL/api/v1/ai/sessions/$SID/messages?after_seq=0" -H "$AUTH" \
  | jq -r '.data[] | "\(.seq) \(.role) \(.kind) :: \(.content)"' | tee /tmp/vfs_files_replay.txt

echo
echo "== 检查点 =="
ok=1
rg -q '"name":"file_write"' /tmp/vfs_files_turn.sse.txt && echo "PASS: file_write 被调用" || { echo "FAIL: 未见 file_write"; ok=0; }
rg -q '"name":"file_read"' /tmp/vfs_files_turn.sse.txt && echo "PASS: file_read 被调用" || { echo "FAIL: 未见 file_read"; ok=0; }
rg -q '"name":"file_list"' /tmp/vfs_files_turn.sse.txt && echo "PASS: file_list 被调用" || echo "WARN: 未见 file_list（不影响写入断言）"
if [ -f "$WS/$TENANT/$FILE" ] && rg -q '上午写接口' "$WS/$TENANT/$FILE"; then
  echo "PASS: 文件已落盘到 $WS/$TENANT/$FILE 且内容正确"
else
  echo "FAIL: 文件不存在或内容不符（$WS/$TENANT/$FILE）"; ok=0
fi
rg -q 'turn:meta' /tmp/vfs_files_replay.txt && echo "PASS: turn:meta 存在"
exit $ok
