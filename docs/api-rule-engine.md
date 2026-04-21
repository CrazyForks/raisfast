# API Rule 引擎 — 完整架构文档

> 版本：v1（当前实现）+ PocketBase 对标分析
> 最后更新：2026-04-21

## 一、概述

API Rule 引擎为 CMS 动态路由提供**表达式级别的行级访问控制**。不同于简单的 `public/member/admin` 三档权限，Rule 引擎允许在 TOML schema 中声明过滤表达式，实现：

- **匿名用户只看到已发布文章**（`filter`）
- **登录用户额外看到自己的草稿**（`filter_auth`）
- **只能修改/删除自己的记录**（运行时求值）
- **管理员专属操作**（角色检查）

## 二、整体架构

```
┌─────────────────────────────────────────────────────────────────────┐
│                        启动时（一次性）                              │
│                                                                     │
│  TOML Schema ──serde──▶ ContentTypeSchema                          │
│       │                      │                                      │
│       │                      ├─ cache_select_columns()              │
│       │                      └─ cache_rules()                       │
│       │                            │                                 │
│       │                            ▼                                 │
│       │                     Rule::parse(filter_str)                 │
│       │                     Rule::parse(filter_auth_str)            │
│       │                            │                                 │
│       │                            ▼                                 │
│       │                     CachedRules（AST）                      │
│       │                     存入 Arc<ContentTypeSchema>              │
└───────┼─────────────────────────────────────────────────────────────┘
        │
┌───────┼─────────────────────────────────────────────────────────────┐
│       │               请求时（每次请求）                              │
│       │                                                             │
│       ▼                                                             │
│  HTTP Request ──▶ Handler                                          │
│       │           │                                                 │
│       │           ├─ 1. check_api_access(access)  ← 粗粒度权限      │
│       │           │                                                 │
│       │           └─ 2. build_rule_sql() ← 细粒度过滤              │
│       │                  │                                          │
│       │                  ├─ 读取 ct.cached_rules.list               │
│       │                  ├─ compile_rule_sql(filter, auth)          │
│       │                  └─ compile_rule_sql(filter_auth, auth)     │
│       │                       │                                     │
│       │                       ▼                                     │
│       │              (sql_fragment, params)                         │
│       │                  │                                          │
│       │                  ▼                                          │
│       │           ContentQuery {                                    │
│       │               rule_where: Some("status = ?1"),              │
│       │               rule_params: vec!["published"],               │
│       │           }                                                 │
│       │                  │                                          │
│       │                  ▼                                          │
│       │           Repository::find()                                │
│       │               └─ WHERE ... AND (status = 'published')       │
│       │                                                            │
│       └─ 3. do_update/do_delete → evaluate(rule, record, ctx)      │
│                                └─ 运行时对记录求值 → 允许/拒绝      │
└────────────────────────────────────────────────────────────────────┘
```

## 三、TOML Schema 定义

### 3.1 结构

每个 content type 的 `[api]` 部分为 5 个端点独立配置：

```toml
[api.list]           # GET /cms/{plural}
[api.get]            # GET /cms/{plural}/{id}
[api.create]         # POST /cms/{plural}
[api.update]         # PUT /cms/{plural}/{id}
[api.delete]         # DELETE /cms/{plural}/{id}
```

每个端点有 3 个字段：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `access` | `"none"` / `"public"` / `"member"` / `"admin"` | 是（默认 `public`） | 粗粒度访问级别 |
| `filter` | 字符串（规则表达式） | 否 | 对所有请求生效的数据过滤 |
| `filter_auth` | 字符串（规则表达式） | 否 | 已登录用户的额外过滤（与 filter 取 OR） |

### 3.2 示例

**博客文章 — 匿名只看已发布，登录可创建**

```toml
[api.list]
access = "public"
filter = 'status = "published"'

[api.get]
access = "public"
filter = 'status = "published"'

[api.create]
access = "member"

[api.update]
access = "member"
filter = 'author_id = @request.auth.id'

[api.delete]
access = "admin"
```

**电商订单 — 用户只能看自己的订单**

```toml
[api.list]
access = "member"
filter_auth = 'user_id = @request.auth.id'

[api.get]
access = "member"
filter_auth = 'user_id = @request.auth.id'

[api.create]
access = "member"

[api.update]
access = "member"
filter = 'user_id = @request.auth.id'

[api.delete]
access = "admin"
```

**完全禁止外部访问（仅内部/插件使用）**

```toml
[api.list]
access = "none"

[api.get]
access = "none"

[api.create]
access = "none"

[api.update]
access = "none"

[api.delete]
access = "none"
```

## 四、规则表达式语法

### 4.1 当前已实现（v1）

#### 比较运算符

| 运算符 | 含义 | 示例 |
|--------|------|------|
| `=` | 等于 | `status = "published"` |
| `!=` | 不等于 | `status != "draft"` |
| `>` | 大于 | `price > 100` |
| `>=` | 大于等于 | `stock >= 1` |
| `<` | 小于 | `view_count < 1000` |
| `<=` | 小于等于 | `created_at <= "2026-01-01"` |
| `~` | LIKE 模糊匹配 | `title ~ "%rust%"` |
| `!~` | NOT LIKE | `title !~ "%spam%"` |

#### 逻辑组合

| 运算符 | 含义 | 示例 |
|--------|------|------|
| `&&` | 逻辑与 | `status = "published" && author_id = @request.auth.id` |
| `\|\|` | 逻辑或 | `status = "published" \|\| status = "draft"` |
| `()` | 优先级分组 | `(a = 1 \|\| b = 2) && c = 3` |

#### 特殊变量

| 变量 | 含义 | 示例 |
|------|------|------|
| `@request.auth.id` | 当前登录用户 ID | `author_id = @request.auth.id` |
| `@request.auth.role` | 当前登录用户角色 | `@request.auth.role = "admin"` |

#### 字面量

| 类型 | 示例 |
|------|------|
| 字符串 | `"published"`、`"admin"` |
| 数字 | `100`、`3.14` |
| 布尔 | `true`、`false` |
| 空值 | `null` |

### 4.2 filter 与 filter_auth 的组合逻辑

```
请求到达
    │
    ├─ 未登录 → 只用 filter
    │            filter 存在 → 编译为 SQL WHERE
    │            filter 不存在 → 不过滤
    │
    └─ 已登录 → filter OR filter_auth
                 两者都有 → WHERE (filter_sql OR filter_auth_sql)
                 只有 filter → WHERE filter_sql
                 只有 filter_auth → WHERE filter_auth_sql
                 都没有 → 不过滤
```

### 4.3 已实现（Phase 1）

| 功能 | 语法 | SQL 编译 | 运行时求值 | 状态 |
|------|------|----------|-----------|------|
| 请求体引用 | `@request.body.title` | 占位符（仅运行时） | ✅ 从 RuleContext.body 取值 | ✅ |
| 查询参数引用 | `@request.query.category` | 占位符（仅运行时） | ✅ 从 RuleContext.query_params 取值 | ✅ |
| 当前时间 | `@now` | `datetime('now')`（可配置） | ✅ ISO 8601 UTC | ✅ |
| 字段存在检查 | `title:isset` | `IS NOT NULL`（可配置） | ✅ `!is_null()` | ✅ |
| 长度检查 | `tags:length > 0` | `LENGTH(field)`（可配置） | ✅ String/Array/Object | ✅ |

### 4.4 未实现（对标 PocketBase）

| 功能 | PocketBase 语法 | rust-blog 语法 | 用途 | 优先级 |
|------|----------------|---------------|------|--------|
| 字段变更检查 | `title:changed` | `title:changed` | update 时判断字段是否修改 | P2 |
| 数组遍历 | `tags:each = "rust"` | `tags:each = "rust"` | 检查数组元素 | P2 |
| 多值 ANY 操作 | `status ?= "published\|draft"` | `status ?= "published\|draft"` | 字段匹配任意一个值 | P2 |
| 多值 NONE 操作 | `status ?!= "deleted"` | `status ?!= "deleted"` | 字段不匹配任何值 | P2 |
| 多值 LIKE | `title ?~ "%rust%\|%.js"` | `title ?~ "%rust%\|%.js"` | 模糊匹配任意一个模式 | P2 |
| 跨表引用 | `@collection.posts.author` | `@table.posts.author` | 关联查询 | P3 |
| 时间格式化 | `strftime("%Y", created_at)` | `strftime("%Y", created_at)` | 提取时间部分 | P3 |
| 地理距离 | `geoDistance(lat, lng, 40.7, -74.0)` | `geoDistance(lat, lng, 40.7, -74.0)` | LBS 过滤 | P3 |

## 五、核心代码结构

### 5.1 文件组织

```
src/content_type/
├── rule_engine.rs      # 表达式引擎核心（Lexer + Parser + AST + SQL/Evaluate）
├── schema.rs           # ApiConfig + ApiEndpointConfig + CachedRules
├── handler.rs          # build_rule_sql() + do_list/do_get/do_update/do_delete 集成
├── repository.rs       # ContentQuery.rule_where/rule_params + find() 注入
└── ...

plugins-protocol/wit/plugin.wit   # WIT 接口（含 filter/filter_auth 类型）
```

### 5.2 rule_engine.rs 内部结构

```
rule_engine.rs
│
├── RuleContext          # 求值上下文（auth_user_id, auth_role, body, query_params）
│
├── Lexer                # 词法分析：字符串 → Vec<Token>
│   ├── Token::Identifier   # 字段名、@request.auth.*
│   ├── Token::StringLit    # "字符串"
│   ├── Token::NumberLit    # 数字
│   ├── Token::BoolLit      # true/false
│   ├── Token::NullLit      # null
│   ├── Token::Eq/Neq/...   # 比较运算符
│   ├── Token::And/Or       # 逻辑运算符
│   └── Token::LParen/RParen
│
├── Parser               # 语法分析：Vec<Token> → Expr (AST)
│   ├── parse_or()           # || 优先级最低
│   ├── parse_and()          # && 优先级中间
│   ├── parse_comparison()   # 比较运算符优先级最高
│   └── parse_atom()         # 操作数（字段名/字面量/特殊变量）
│
├── Expr (AST)           # 抽象语法树
│   ├── Compare { left, op, right }  # 比较表达式
│   ├── And(lhs, rhs)                 # 逻辑与
│   └── Or(lhs, rhs)                  # 逻辑或
│
├── Operand              # 操作数
│   ├── Field(String)              # 数据库字段
│   ├── AuthId                     # @request.auth.id
│   ├── AuthRole                   # @request.auth.role
│   ├── StringLit/NumberLit/...    # 字面量
│   └── Null                       # null
│
├── Rule                 # 解析后的规则
│   ├── parse(source) → Rule       # 入口：字符串 → AST
│   ├── to_sql(offset) → (sql, params)  # 编译为 SQL
│   └── evaluate(record, ctx) → bool    # 运行时求值
│
└── compile_rule_sql()   # 带 auth 替换的 SQL 编译（公共接口）
```

### 5.3 schema.rs 相关类型

```rust
// 访问级别
enum ApiAccess { None, Public, Member, Admin }

// 单个端点配置
struct ApiEndpointConfig {
    access: ApiAccess,
    filter: Option<String>,        // 原始表达式字符串
    filter_auth: Option<String>,   // 原始表达式字符串
}

// 5 个端点
struct ApiConfig {
    list: ApiEndpointConfig,
    get: ApiEndpointConfig,
    create: ApiEndpointConfig,
    update: ApiEndpointConfig,
    delete: ApiEndpointConfig,
}

// 预解析缓存（启动时填充，不序列化）
struct CachedEndpointRules {
    filter: Option<Rule>,       // 已解析的 AST
    filter_auth: Option<Rule>,  // 已解析的 AST
}

struct CachedRules {
    list: CachedEndpointRules,
    get: CachedEndpointRules,
    create: CachedEndpointRules,
    update: CachedEndpointRules,
    delete: CachedEndpointRules,
}

struct ContentTypeSchema {
    // ...
    api: ApiConfig,
    cached_rules: Option<CachedRules>,  // 注册时 cache_rules() 填充
}
```

### 5.4 handler.rs 集成点

```
请求到达
    │
    ├─ check_api_access(endpoint.access)     ← 粗粒度：None/Public/Member/Admin
    │
    ├─ build_rule_sql(&endpoint, auth)       ← 细粒度：编译规则为 SQL
    │   ├─ filter 编译为 SQL（参数化）
    │   ├─ filter_auth 编译为 SQL（注入 auth.id/role）
    │   └─ 组合：(filter_sql OR filter_auth_sql)
    │
    ├─ [list/get] ContentQuery.rule_where 注入 → Repository::find()
    │                                       → SQL WHERE ... AND (rule)
    │
    └─ [update/delete] 先 find_by_id → Rule::evaluate(record, ctx)
                                         → false → 403 Forbidden
```

### 5.5 repository.rs 查询注入

```rust
struct ContentQuery {
    // ... 原有字段 ...
    pub rule_where: Option<String>,    // API Rule 编译的 SQL 片段
    pub rule_params: Vec<String>,      // 对应参数
}

// find() 中：
// 1. 构建 WHERE 子句（status、filters、tenant_id...）
// 2. 追加 rule_where：AND (rule_sql)
// 3. 追加 rule_params 到参数列表
// 4. 绑定参数执行查询
```

## 六、性能设计

| 环节 | 策略 | 说明 |
|------|------|------|
| 规则解析 | **启动时一次** | `cache_rules()` 在 schema 注册时解析，请求时零解析开销 |
| SQL 编译 | **每次请求** | `compile_rule_sql()` 生成 WHERE 片段 + 参数列表，开销 < 1μs |
| 运行时求值 | **CUD 时** | `Rule::evaluate()` 遍历 AST 树，开销 < 1μs |
| 参数绑定 | **参数化** | 规则编译为 `?N` 占位符 + 参数列表，防 SQL 注入 |
| LIKE 实现 | **SQL LIKE** | 使用 SQL 原生 `LIKE`，非 Regex 编译 |

## 七、安全设计

| 威胁 | 防护 |
|------|------|
| SQL 注入 | 规则编译为参数化查询，用户输入不拼入 SQL |
| 越权访问 | `check_api_access` + rule evaluate 双重检查 |
| auth 伪造 | auth.id/role 从 JWT 解析，非客户端传入 |
| 规则语法错误 | `Rule::parse()` 返回 Err，schema 加载时 log warn 并跳过该规则 |
| 未认证请求 | `compile_rule_sql` 对需要 auth 的规则返回 None → 拒绝请求 |

## 八、请求处理流程（完整示例）

### 场景：匿名用户 GET /cms/articles

```
1. Handler 收到请求
2. check_api_access(Public) → ✅ 通过
3. build_rule_sql(list, auth=None)
   ├─ filter = Rule('status = "published"')
   ├─ compile_rule_sql(rule, auth=None)
   │   ├─ to_sql(0) → ("status = ?1", ["published"])
   │   └─ 无 auth 替换 → 返回 ("status = ?1", ["published"])
   └─ filter_auth 存在但 auth=None → 忽略
4. ContentQuery { rule_where: "status = ?1", rule_params: ["published"] }
5. Repository::find()
   SQL: SELECT ... FROM articles WHERE ... AND (status = ?1)
   Bind: ["published"]
6. 返回已发布文章列表
```

### 场景：作者 GET /cms/articles（已登录 user_id=abc）

```
1. Handler 收到请求
2. check_api_access(Public) → ✅ 通过
3. build_rule_sql(list, auth=Some(abc, member))
   ├─ filter = Rule('status = "published"')
   │   → compile → ("status = ?1", ["published"])
   ├─ filter_auth = Rule('author_id = @request.auth.id')
   │   → compile with auth → ("author_id = 'abc'", [])
   └─ 组合 → ("(status = ?1 OR author_id = 'abc')", ["published"])
4. ContentQuery { rule_where: "(status = ?1 OR author_id = 'abc')", ... }
5. SQL: WHERE ... AND ((status = 'published' OR author_id = 'abc'))
6. 返回已发布文章 + 作者 abc 的草稿
```

### 场景：非作者 PUT /cms/articles/123（user_id=xyz, author_id=abc）

```
1. Handler 收到请求
2. check_api_access(Member) → ✅ 通过（已登录）
3. do_update 中 evaluate:
   ├─ 先 find_by_id → record = { author_id: "abc", ... }
   ├─ rule = Rule('author_id = @request.auth.id')
   ├─ evaluate(record, ctx={ auth_user_id: "xyz" })
   │   → "abc" == "xyz" → false
   └─ 返回 403 Forbidden
```

## 九、与 PocketBase 对比

| 维度 | PocketBase | rust-blog（当前） | rust-blog（计划） |
|------|-----------|------------------|------------------|
| 配置格式 | listRule/viewRule/... 字符串 | `[api.list] access + filter + filter_auth` | 同 |
| 表达式解析 | `fexpr`（自研） | 自研 Lexer + Parser | 同 |
| SQL 编译 | filter → WHERE | Rule::to_sql() + RuleEngineConfig | 同 |
| 运行时求值 | Go 原生 | Rust serde_json + RuleEngineConfig | 同 |
| 环境变量配置 | ❌ | ✅ RuleEngineConfig（14 项） | 同 |
| `@request.auth.*` | ✅ | ✅ | — |
| `@request.body.*` | ✅ | ✅ | — |
| `@request.query.*` | ✅ | ✅ | — |
| `@now` | ✅ | ✅ | — |
| `:isset` | ✅ | ✅ | — |
| `:length` | ✅ | ✅ | — |
| `:changed` | ✅ | ❌ | Phase 2 |
| `:each` | ✅ | ❌ | Phase 2 |
| `?= ?!= ?~` | ✅ | ❌ | Phase 2 |
| `@table.*`（跨表） | ✅ | ❌ | Phase 3 |
| `geoDistance()` | ✅ | ❌ | Phase 3 |
| `strftime()` | ✅ | ❌ | Phase 3 |
| **覆盖率** | **100%** | **~30%** | **~85%**（Phase 1-3 后） |

## 十、扩展路线图

### Phase 1（1 天，覆盖 60% 场景）

```
新增 Operand 变体：
  - RequestBody(String)     ← @request.body.field_name
  - RequestQuery(String)    ← @request.query.param_name
  - Now                     ← @now

新增 AST 节点：
  - IsSet(Operand)          ← field:isset
  - Length(Operand)          ← field:length

新增 SQL 编译：
  - Now → datetime('now')
  - IsSet → field IS NOT NULL
  - Length → LENGTH(field)
```

### Phase 2（1 天，覆盖 75% 场景）

```
新增 CmpOp：
  - AnyEq / AnyNeq / AnyLike   ← ?= ?!= ?~

新增后缀解析：
  - :changed                    ← update 前后对比

新增 AST 节点：
  - Each(Operand, Expr)         ← field:each = "value"

修改 evaluate 签名：
  - 改为 async（为 @collection 准备）
```

### Phase 3（2 天，覆盖 85% 场景）

```
新增跨表查询：
  - @table.posts.author         ← async DB 查询（前缀可配置 RULE_PREFIX_CROSS_TABLE）

新增函数：
  - strftime(fmt, field)       ← 时间格式化
  - geoDistance(lat, lng, lat2, lng2) ← 地理距离
```

## 十一、环境变量配置

所有规则引擎中的硬编码值均可通过环境变量覆盖，便于适配不同数据库后端（SQLite / PostgreSQL / MySQL）。

在 `AppConfig.rule_engine: RuleEngineConfig` 中定义，通过 `.env` 或环境变量设置：

### 表达式前缀

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `RULE_PREFIX_AUTH_ID` | `@request.auth.id` | 认证用户 ID 前缀 |
| `RULE_PREFIX_AUTH_ROLE` | `@request.auth.role` | 认证用户角色前缀 |
| `RULE_PREFIX_REQUEST_BODY` | `@request.body.` | 请求体字段前缀 |
| `RULE_PREFIX_REQUEST_QUERY` | `@request.query.` | URL 查询参数前缀 |
| `RULE_PREFIX_NOW` | `@now` | 当前时间前缀 |
| `RULE_PREFIX_CROSS_TABLE` | `@table.` | 跨表引用前缀（Phase 3） |

### SQL 编译

| 环境变量 | 默认值 | PostgreSQL 替代 | 说明 |
|---------|--------|---------------|------|
| `RULE_SQL_NOW_FN` | `datetime('now')` | `NOW()` | @now 编译为的 SQL 函数 |
| `RULE_SQL_ISSET_OP` | `IS NOT NULL` | `IS NOT NULL` | :isset 编译为的操作符 |
| `RULE_SQL_LENGTH_FN` | `LENGTH` | `CHAR_LENGTH` | :length 编译为的函数名 |

### LIKE 通配符

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `RULE_SQL_LIKE_WILDCARD` | `%` | SQL LIKE 通配符 |
| `RULE_SQL_LIKE_SINGLE_CHAR` | `_` | SQL LIKE 单字符通配符 |
| `RULE_REGEX_LIKE_WILDCARD` | `.*` | LIKE 通配符对应的正则 |
| `RULE_REGEX_LIKE_SINGLE_CHAR` | `.` | LIKE 单字符对应的正则 |

### 缓存

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `CMS_CACHE_TTL` | `30` | CMS 列表/详情缓存 TTL（秒） |

### 配置示例

**PostgreSQL 适配：**

```env
RULE_SQL_NOW_FN=NOW()
RULE_SQL_LENGTH_FN=CHAR_LENGTH
CMS_CACHE_TTL=60
```
