#!/usr/bin/env bash
#
# CMS API 测试脚本:覆盖 content type 记录里 blob / media / media_set 字段的
# 添加、修改、删除等所有场景。每个场景是一个独立函数,可单独运行。
#
# 用法:
#   scripts/test-cms-api.sh                 # 运行全部场景
#   scripts/test-cms-api.sh create          # 只跑 create
#   scripts/test-cms-api.sh update          # 只跑 update
#   scripts/test-cms-api.sh list            # 列出所有场景
#
# 环境变量:
#   BASE_URL    API 地址(默认 http://localhost:3000/api/v1)
#   TEST_EMAIL  登录邮箱(默认 api@test.com)
#   TEST_PASS   密码(默认 ApiPass123!)
#   PLURAL      内容类型复数(默认 some_tests)
#   BLOB_FIELD  blob 字段名(默认 bb)
#   MEDIA_SET_FIELD media_set 字段名(默认 gallery_test)
#
set -uo pipefail

BASE_URL="${BASE_URL:-http://localhost:3000/api/v1}"
EMAIL="${TEST_EMAIL:-api@test.com}"
PASS="${TEST_PASS:-ApiPass123!}"
PLURAL="${PLURAL:-some_tests}"
BLOB_FIELD="${BLOB_FIELD:-bb}"
MEDIA_SET_FIELD="${MEDIA_SET_FIELD:-gallery_test}"

TOKEN=""

# ── 公共辅助 ─────────────────────────────────────────────────────────

auth() {
  [ -n "$TOKEN" ] && return 0
  TOKEN=$(curl -s -X POST "$BASE_URL/auth/login" \
    -H "Content-Type: application/json" \
    -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}" \
    | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['data']['access_token'])" 2>/dev/null)
  if [ -z "$TOKEN" ] || [ "$TOKEN" = "null" ]; then
    echo "✗ 登录失败(EMAIL=$EMAIL)。请先注册该用户,或设置 TEST_EMAIL/TEST_PASS"
    exit 1
  fi
}

# 上传一个媒体文件,返回 media id
upload_media() {
  local file="$1"
  curl -s -X POST "$BASE_URL/admin/media/upload" \
    -H "Authorization: Bearer $TOKEN" \
    -F "file=@$file" \
    | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['id'])"
}

# 发起请求,返回 "HTTP_STATUS" 单独一行,前面是 body
api() {
  local method="$1" path="$2" data="${3:-}"
  local args=(-s -w '\n%{http_code}' -X "$method" "$BASE_URL$path" -H "Authorization: Bearer $TOKEN")
  if [ -n "$data" ]; then
    args+=(-H "Content-Type: application/json" -d "$data")
  fi
  curl "${args[@]}"
}

status_of() { echo "$1" | tail -n1; }
body_of()  { echo "$1" | sed '$d'; }

b64_file() { base64 -i "$1" 2>/dev/null | tr -d '\n'; }

# 创建一条带 blob + media_set 的记录,输出 id
create_record() {
  local f="/tmp/cms-api-blob-$$.json"
  printf '{"hello":"api test"}' > "$f"
  local b64; b64=$(b64_file "$f"); rm -f "$f"
  local resp
  resp=$(api POST "/cms/$PLURAL" "{\"title\":\"api-scenario\",\"$BLOB_FIELD\":{\"data\":\"$b64\",\"filename\":\"a.json\",\"mimetype\":\"application/json\"},\"$MEDIA_SET_FIELD\":[]}")
  local st; st=$(status_of "$resp")
  if [ "$st" = "201" ] || [ "$st" = "200" ]; then
    body_of "$resp" | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['id'])"
  else
    echo "" >&2
    echo "  ✗ 创建记录失败 HTTP=$st: $(body_of "$resp")" >&2
  fi
}

check() {
  local name="$1" ok="$2" detail="$3"
  if [ "$ok" = "0" ]; then
    echo "  ✓ $name"
  else
    echo "  ✗ $name $detail"
  fi
}

# ── 场景 ─────────────────────────────────────────────────────────────

scenario_create() {
  echo "▶ 添加:创建记录(blob 对象 + media_set 数组)"
  local f="/tmp/cms-api-create-$$.json"
  printf '{"x":1}' > "$f"
  local b64; b64=$(b64_file "$f"); rm -f "$f"
  local resp
  resp=$(api POST "/cms/$PLURAL" "{\"title\":\"create\",\"$BLOB_FIELD\":{\"data\":\"$b64\",\"filename\":\"a.json\",\"mimetype\":\"application/json\"},\"$MEDIA_SET_FIELD\":[\"10001\",\"10002\"]}")
  local st; st=$(status_of "$resp")
  local b; b=$(body_of "$resp")
  check "创建返回 2xx" "$([ "$st" = "201" ] || [ "$st" = "200" ]; echo $?)" "(HTTP=$st $b)"
  echo "$b" | python3 -c "import sys,json;d=json.load(sys.stdin)['data'];assert d['$BLOB_FIELD']['filename']=='a.json', 'blob filename mismatch';assert len(d['$MEDIA_SET_FIELD'])==2, 'media_set count mismatch';assert '_meta' not in json.dumps(d), 'companion meta leaked'" \
    && echo "  ✓ blob/media_set 值 + 伴生列不泄漏" \
    || echo "  ✗ 响应结构不正确"
}

scenario_read() {
  echo "▶ 读取:创建后按 id 读取"
  local id; id=$(create_record)
  [ -z "$id" ] && { echo "  ✗ 前置创建失败"; return; }
  local resp; resp=$(api GET "/cms/$PLURAL/$id")
  local st; st=$(status_of "$resp")
  check "读取返回 200" "$([ "$st" = "200" ]; echo $?)" "(HTTP=$st)"
  body_of "$resp" | python3 -c "import sys,json;d=json.load(sys.stdin)['data'];assert d['$BLOB_FIELD']['filename']=='a.json', 'blob mismatch'" \
    && echo "  ✓ blob 值往返正确" \
    || echo "  ✗ 读取内容不正确"
}

scenario_update() {
  echo "▶ 修改:替换 blob(data/filename/mimetype)+ media_set"
  local id; id=$(create_record)
  [ -z "$id" ] && { echo "  ✗ 前置创建失败"; return; }
  local f="/tmp/cms-api-update-$$.bin"
  printf 'updated-bytes' > "$f"
  local b64; b64=$(b64_file "$f"); rm -f "$f"
  local resp
  resp=$(api PUT "/cms/$PLURAL/$id" "{\"$BLOB_FIELD\":{\"data\":\"$b64\",\"filename\":\"b.bin\",\"mimetype\":\"application/octet-stream\"},\"$MEDIA_SET_FIELD\":[\"10009\"]}")
  local st; st=$(status_of "$resp")
  local b; b=$(body_of "$resp")
  check "更新返回 2xx" "$([ "$st" = "200" ] || [ "$st" = "201" ]; echo $?)" "(HTTP=$st $b)"
  echo "$b" | python3 -c "import sys,json;d=json.load(sys.stdin)['data'];assert d['$BLOB_FIELD']['filename']=='b.bin', 'blob not updated';assert len(d['$MEDIA_SET_FIELD'])==1, 'media_set not updated'" \
    && echo "  ✓ blob/media_set 均已更新" \
    || echo "  ✗ 更新内容不正确"
}

scenario_media_set_clear() {
  echo "▶ media_set 清空:设为 []"
  local id; id=$(create_record)
  [ -z "$id" ] && { echo "  ✗ 前置创建失败"; return; }
  local resp; resp=$(api PUT "/cms/$PLURAL/$id" "{\"$MEDIA_SET_FIELD\":[]}")
  local st; st=$(status_of "$resp")
  check "清空返回 2xx" "$([ "$st" = "200" ] || [ "$st" = "201" ]; echo $?)" "(HTTP=$st)"
  body_of "$resp" | python3 -c "import sys,json;assert json.load(sys.stdin)['data']['$MEDIA_SET_FIELD']==[], 'not cleared'" \
    && echo "  ✓ media_set 已清空" \
    || echo "  ✗ 清空失败"
}

scenario_media_set_keep_add() {
  echo "▶ media_set 保留原有 + 追加新文件"
  local id; id=$(create_record)
  [ -z "$id" ] && { echo "  ✗ 前置创建失败"; return; }
  local f="/tmp/cms-api-new-$$.png"
  printf 'newfile' > "$f"
  local mid; mid=$(upload_media "$f"); rm -f "$f"
  [ -z "$mid" ] && { echo "  ✗ 上传媒体失败"; return; }
  local resp; resp=$(api PUT "/cms/$PLURAL/$id" "{\"$MEDIA_SET_FIELD\":[\"10001\",\"$mid\"]}")
  local st; st=$(status_of "$resp")
  check "追加返回 2xx" "$([ "$st" = "200" ] || [ "$st" = "201" ]; echo $?)" "(HTTP=$st)"
  body_of "$resp" | python3 -c "import sys,json;assert '$mid' in json.load(sys.stdin)['data']['$MEDIA_SET_FIELD'], 'new id missing'" \
    && echo "  ✓ 原 ID + 新 ID 都在" \
    || echo "  ✗ 追加后缺少新 ID"
}

scenario_media_set_delete_some() {
  echo "▶ media_set 删除其中几个"
  local id; id=$(create_record)
  [ -z "$id" ] && { echo "  ✗ 前置创建失败"; return; }
  local resp; resp=$(api PUT "/cms/$PLURAL/$id" "{\"$MEDIA_SET_FIELD\":[\"10001\",\"10002\",\"10003\"]}")
  local st; st=$(status_of "$resp")
  [ "$st" = "200" ] || [ "$st" = "201" ] || { echo "  ✗ 前置设置失败 HTTP=$st"; return; }
  resp=$(api PUT "/cms/$PLURAL/$id" "{\"$MEDIA_SET_FIELD\":[\"10002\"]}")
  st=$(status_of "$resp")
  check "删除部分返回 2xx" "$([ "$st" = "200" ] || [ "$st" = "201" ]; echo $?)" "(HTTP=$st)"
  body_of "$resp" | python3 -c "import sys,json;assert json.load(sys.stdin)['data']['$MEDIA_SET_FIELD']==['10002'], 'wrong remaining'" \
    && echo "  ✓ 只剩保留的 ID" \
    || echo "  ✗ 删除部分后结果不对"
}

scenario_blob_null() {
  echo "▶ blob 清空:设为 null"
  local id; id=$(create_record)
  [ -z "$id" ] && { echo "  ✗ 前置创建失败"; return; }
  local resp; resp=$(api PUT "/cms/$PLURAL/$id" "{\"$BLOB_FIELD\":null}")
  local st; st=$(status_of "$resp")
  check "blob null 返回 2xx" "$([ "$st" = "200" ] || [ "$st" = "201" ]; echo $?)" "(HTTP=$st)"
  body_of "$resp" | python3 -c "import sys,json;assert json.load(sys.stdin)['data']['$BLOB_FIELD'] is None, 'not null'" \
    && echo "  ✓ blob 已清空" \
    || echo "  ✗ 清空失败"
}

scenario_blob_oversize() {
  echo "▶ blob 超限:>512KB 应 400"
  local f="/tmp/cms-api-big-$$.bin"
  dd if=/dev/zero bs=1024 count=600 of="$f" 2>/dev/null
  local b64; b64=$(b64_file "$f"); rm -f "$f"
  local resp; resp=$(api POST "/cms/$PLURAL" "{\"title\":\"big\",\"$BLOB_FIELD\":{\"data\":\"$b64\",\"filename\":\"big.bin\",\"mimetype\":\"application/octet-stream\"}}")
  local st; st=$(status_of "$resp")
  check "超限返回 400" "$([ "$st" = "400" ]; echo $?)" "(HTTP=$st)"
}

scenario_blob_invalid() {
  echo "▶ blob 非法 base64 应 400"
  local resp; resp=$(api POST "/cms/$PLURAL" "{\"title\":\"bad\",\"$BLOB_FIELD\":{\"data\":\"!!!not-base64!!!\",\"filename\":\"x\",\"mimetype\":\"application/json\"}}")
  local st; st=$(status_of "$resp")
  check "非法 base64 返回 400" "$([ "$st" = "400" ]; echo $?)" "(HTTP=$st)"
}

scenario_media_set_invalid() {
  echo "▶ media_set 非法值(非数组/含非字符串)应 400"
  local resp; resp=$(api POST "/cms/$PLURAL" "{\"title\":\"bad\",\"$MEDIA_SET_FIELD\":\"oops\"}")
  local st; st=$(status_of "$resp")
  check "非数组返回 400" "$([ "$st" = "400" ]; echo $?)" "(HTTP=$st)"
}

scenario_delete() {
  echo "▶ 删除:删除后读取应 404"
  local id; id=$(create_record)
  [ -z "$id" ] && { echo "  ✗ 前置创建失败"; return; }
  local resp; resp=$(api DELETE "/cms/$PLURAL/$id")
  local st; st=$(status_of "$resp")
  check "删除返回 2xx" "$([ "$st" = "200" ] || [ "$st" = "204" ]; echo $?)" "(HTTP=$st)"
  resp=$(api GET "/cms/$PLURAL/$id")
  st=$(status_of "$resp")
  check "删除后读取 404" "$([ "$st" = "404" ]; echo $?)" "(HTTP=$st)"
}

# ── 调度 ─────────────────────────────────────────────────────────────

ALL="create read update media_set_clear media_set_keep_add media_set_delete_some blob_null blob_oversize blob_invalid media_set_invalid delete"

usage() {
  echo "用法: $0 [场景名]"
  echo "场景:"
  echo "  $ALL" | tr ' ' '\n' | sed 's/^/    /'
  echo "不传参数 = 运行全部"
}

if [ "${1:-}" = "list" ]; then usage; exit 0; fi

auth

if [ $# -ge 1 ]; then
  case " $ALL " in
    *" $1 "*)
      echo "==> 场景: $1"
      "scenario_$1"
      ;;
    *)
      echo "未知场景: $1"; usage; exit 1
      ;;
  esac
else
  for s in $ALL; do
    echo "==> 场景: $s"
    "scenario_$s"
  done
fi
