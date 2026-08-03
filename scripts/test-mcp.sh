#!/bin/bash
# ── raisfast MCP curl 测试脚本 ──────────────────────────────────
# 用法: TOKEN="rf_mcp_xxx" bash test-mcp.sh
# ────────────────────────────────────────────────────────────────

TOKEN="${TOKEN:-rf_mcp_632e00b18ccafc6da296413872fd052f4abdc22edcfc3829}"
BASE="http://localhost:9898/api/v1/mcp"
PV="2026-07-28"

# 公共 _meta (modern 协议必填)
META='"io.modelcontextprotocol/protocolVersion": "'$PV'", "io.modelcontextprotocol/clientInfo": {"name": "curl-test", "version": "1.0"}, "io.modelcontextprotocol/clientCapabilities": {}'

mcp() {
  local id=$1 method=$2 params=$3
  local body
  if [ -z "$params" ]; then
    body="{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"$method\",\"params\":{$META}}"
  else
    body="{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"$method\",\"params\":{$params, $META}}"
  fi
  curl -s -X POST "$BASE" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -H "Accept: application/json" \
    -H "MCP-Protocol-Version: $PV" \
    -d "$body"
}

echo "═══ 1. server/discover ═══"
mcp 1 "server/discover" "" | jq '{capabilities: .result.capabilities, instructions: .result.instructions}'

echo
echo "═══ 2. ping ═══"
mcp 2 "ping" "" | jq '.result.resultType'

echo
echo "═══ 3. tools/list ═══"
mcp 3 "tools/list" "" | jq -r '.result.tools[] | "\(.name): \(.description[:60])..."'

echo
echo "═══ 4. list_content_types ═══"
mcp 4 "tools/call" '"name":"list_content_types","arguments":{}' | jq -r '.result.content[0].text' | jq '.[0].name' 2>/dev/null || echo "(no content types)"

echo
echo "═══ 5. list_posts ═══"
mcp 5 "tools/call" '"name":"list_posts","arguments":{"page":1,"page_size":3}' | jq -r '.result.content[0].text' | head -5

echo
echo "═══ 6. resources/list ═══"
mcp 6 "resources/list" "" | jq -r '.result.resources[] | "\(.uri): \(.name)"'

echo
echo "═══ 7. read schema guide ═══"
mcp 7 "resources/read" '"uri":"raisfast://content-type-schema-guide"' | jq -r '.result.contents[0].text' | head -5

echo
echo "═══ 8. prompts/list ═══"
mcp 8 "prompts/list" "" | jq -r '.result.prompts[] | "\(.name): \(.description[:50])..."'

echo
echo "═══ 9. prompts/get draft_post ═══"
mcp 9 "prompts/get" '"name":"draft_post","arguments":{"topic":"Why Rust wins in 2026","tone":"technical"}' | jq -r '.result.messages[0].content.text' | head -8

echo
echo "═══ 10. completion/complete ═══"
mcp 10 "completion/complete" '"ref":{"type":"ref/resource","uri":"raisfast://content-types/{key}"},"argument":{"name":"key","value":""}' | jq '.result.completion'

echo
echo "═══ 11. 错误: 不支持的协议版本 ═══"
# 直接用 mcp 函数但手动替换 header
curl -s -X POST "$BASE" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -H "MCP-Protocol-Version: 2099-01-01" \
  -d '{"jsonrpc":"2.0","id":99,"method":"ping","params":{"io.modelcontextprotocol/protocolVersion":"2099-01-01","io.modelcontextprotocol/clientInfo":{"name":"test","version":"1.0"},"io.modelcontextprotocol/clientCapabilities":{}}}' | jq '.error'
