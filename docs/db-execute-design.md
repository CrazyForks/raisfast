# db_execute 插件扩展设计

> 2026-04-16 · 为插件系统增加数据库写操作能力，解锁电商/工作流等业务场景。

---

## 1. 目标

在现有 `Host.dbQuery(sql)` (只读) 基础上，新增 `Host.dbExecute(sql)` (写入)，让插件可以执行 INSERT/UPDATE/DELETE。

**不做的事：**
- 不支持 DDL（CREATE/DROP/ALTER/TRUNCATE）
- 不支持事务（BEGIN/COMMIT/ROLLBACK）
- 不支持批量语句（只执行第一条）

---

## 2. API 设计

### 插件端调用

```javascript
// JS 插件
const result = Host.dbExecute("UPDATE products SET stock = stock - 1 WHERE id = 'abc'");
// result: '{"rows_affected":1}' 或 '{"error":"no write permission for table: products"}'
```

```lua
-- Lua 插件
local result = Host.dbExecute("INSERT INTO orders (id, status) VALUES ('ord-1', 'pending')")
-- result: '{"rows_affected":1}' 或 '{"error":"..."}'
```

```rust
// WASM 插件（预期签名）
fn host_db_execute(ptr: i32, len: i32) -> i32;
```

### 返回格式

```json
// 成功
{"rows_affected": 3}

// 失败
{"error": "only INSERT/UPDATE/DELETE are allowed"}
{"error": "no write permission for table: orders"}
{"error": "DDL operations are not allowed"}
```

---

## 3. 权限模型

### manifest.toml 声明

```toml
[permissions]
database = [
    "read:products",       # 只读 products 表
    "write:orders",        # 只写 orders 表
    "products",            # 读写 products 表（等同于 read + write）
    "read:order_items",
    "write:order_items",
]
```

### 权限规则（复用已有实现）

| 声明 | `db_query` (读) | `db_execute` (写) |
|---|---|---|
| `"*"` | ✅ 所有表 | ✅ 所有表 |
| `"products"` (裸表名) | ✅ | ✅ |
| `"read:products"` | ✅ | ❌ |
| `"write:products"` | ❌ | ✅ |

已有的 `is_table_writable()` (`src/plugins/permissions.rs:84-93`) 已覆盖以上逻辑。

---

## 4. 安全防护（4 层）

### Layer 1: SQL 类型白名单

```rust
fn is_write_query(sql: &str) -> bool {
    let trimmed = sql.trim().to_uppercase();
    trimmed.starts_with("INSERT")
        || trimmed.starts_with("UPDATE")
        || trimmed.starts_with("DELETE")
}
```

只允许 INSERT / UPDATE / DELETE，拒绝 SELECT / DDL / 其他。

### Layer 2: DDL 黑名单

```rust
fn is_ddl_query(sql: &str) -> bool {
    let trimmed = sql.trim().to_uppercase();
    trimmed.starts_with("CREATE")
        || trimmed.starts_with("DROP")
        || trimmed.starts_with("ALTER")
        || trimmed.starts_with("TRUNCATE")
        || trimmed.starts_with("ATTACH")
        || trimmed.starts_with("DETACH")
        || trimmed.starts_with("PRAGMA")
        || trimmed.starts_with("REINDEX")
        || trimmed.starts_with("ANALYZE")
        || trimmed.starts_with("VACUUM")
}
```

即使绕过 Layer 1，DDL 也会被拦截。

### Layer 3: 表级权限 RBAC

```rust
let table = extract_write_table_name(sql);
if !PermissionChecker::is_table_writable(&self.permissions, &table) {
    return r#"{"error":"no write permission for table: ..."}"#.to_string();
}
```

### Layer 4: 系统表保护

```rust
const PROTECTED_TABLES: &[&str] = &[
    "users", "roles", "permissions",
    "extensions", "audit_log",
    "plugin_storage", "options",
    "rbac_roles", "rbac_permissions", "rbac_role_permissions",
    "tenants",
];

fn is_protected_table(table: &str) -> bool {
    PROTECTED_TABLES.contains(&table.to_lowercase().as_str())
}
```

即使声明 `"write:users"` 或 `"*"`，系统核心表也拒绝写入。

---

## 5. 写 SQL 表名提取

现有 `extract_table_name` 只解析 `SELECT ... FROM table`。新增 `extract_write_table_name`：

```rust
fn extract_write_table_name(sql: &str) -> Option<String> {
    let upper = sql.trim().to_uppercase();
    if upper.starts_with("INSERT") {
        // INSERT INTO table_name ...
        extract_after_keyword(&upper, "INTO")
    } else if upper.starts_with("UPDATE") {
        // UPDATE table_name SET ...
        extract_first_identifier_after(&upper, "UPDATE")
    } else if upper.starts_with("DELETE") {
        // DELETE FROM table_name ...
        extract_after_keyword(&upper, "FROM")
    } else {
        None
    }
}
```

---

## 6. 实现变更清单

### `src/plugins/permissions.rs`

| 操作 | 说明 |
|---|---|
| 新增 `is_write_query(sql)` | 检查是否 INSERT/UPDATE/DELETE |
| 新增 `is_ddl_query(sql)` | DDL 黑名单检查 |
| 新增 `extract_write_table_name(sql)` | 提取写操作的目标表名 |
| 新增 `is_protected_table(table)` | 系统表保护 |
| 已有 `is_table_writable(permissions, table)` | 无需修改 |

### `src/plugins/host_common.rs`

| 操作 | 说明 |
|---|---|
| 新增 `HostContext::db_execute(&self, sql: &str) -> String` | 核心实现：4 层安全检查 + 执行 + 返回 JSON |

### `src/plugins/js_host.rs`

| 操作 | 说明 |
|---|---|
| 新增 `dbExecute` 函数注册 | `Host.dbExecute(sql: string) -> string` |

### `src/plugins/lua_host.rs`

| 操作 | 说明 |
|---|---|
| 新增 `dbExecute` 函数注册 | `Host.dbExecute(sql) -> string` |

### `src/plugins/host.rs`

| 操作 | 说明 |
|---|---|
| 新增 `host_db_execute` 函数 | WASM ABI：`(ptr, len) -> ptr` |

### `src/plugins/mod.rs`

| 操作 | 说明 |
|---|---|
| 无修改 | `rows_to_json` 不适用于写操作，新增 `execute_result_to_json` |

---

## 7. db_execute 核心实现

```rust
// host_common.rs
impl HostContext {
    pub fn db_execute(&self, sql: &str) -> String {
        // Layer 1: 只允许 INSERT/UPDATE/DELETE
        if !PermissionChecker::is_write_query(sql) {
            return r#"{"error":"only INSERT/UPDATE/DELETE are allowed"}"#.to_string();
        }

        // Layer 2: 拒绝 DDL（双重保险）
        if PermissionChecker::is_ddl_query(sql) {
            return r#"{"error":"DDL operations are not allowed"}"#.to_string();
        }

        // Layer 3: 表级权限
        let table = match PermissionChecker::extract_write_table_name(sql) {
            Some(t) => t,
            None => return r#"{"error":"cannot parse table name from SQL"}"#.to_string(),
        };

        if PermissionChecker::is_protected_table(&table) {
            return format!(r#"{{"error":"table '{table}' is protected and cannot be modified by plugins"}}"#);
        }

        if !PermissionChecker::is_table_writable(&self.permissions, &table) {
            return format!(r#"{{"error":"no write permission for table: {table}"}}"#);
        }

        // 执行
        let Some(pool) = &self.pool else {
            return r#"{"error":"no database access"}"#.to_string();
        };

        let handle = tokio::runtime::Handle::current();
        let sql = crate::db::dialect::translate(sql).into_owned();

        tokio::task::block_in_place(|| {
            match handle.block_on(async {
                let result = sqlx::query(&sql).execute(pool).await?;
                Ok::<_, sqlx::Error>(result)
            }) {
                Ok(r) => format!(r#"{{"rows_affected":{}}}"#, r.rows_affected()),
                Err(e) => format!(r#"{{"error":"{e}"}}"#),
            }
        })
    }
}
```

---

## 8. 测试计划

### 单元测试 (`permissions.rs`)

| 测试 | 输入 | 预期 |
|---|---|---|
| `is_write_query("INSERT INTO ...")` | INSERT | true |
| `is_write_query("UPDATE ...")` | UPDATE | true |
| `is_write_query("DELETE FROM ...")` | DELETE | true |
| `is_write_query("SELECT ...")` | SELECT | false |
| `is_write_query("DROP TABLE ...")` | DDL | false |
| `is_ddl_query("CREATE TABLE ...")` | CREATE | true |
| `is_ddl_query("ALTER TABLE ...")` | ALTER | true |
| `is_ddl_query("INSERT INTO ...")` | INSERT | false |
| `extract_write_table_name("INSERT INTO orders ...")` | INSERT | Some("orders") |
| `extract_write_table_name("UPDATE products SET ...")` | UPDATE | Some("products") |
| `extract_write_table_name("DELETE FROM cart_items ...")` | DELETE | Some("cart_items") |
| `extract_write_table_name("SELECT * FROM orders")` | SELECT | None |
| `is_protected_table("users")` | users | true |
| `is_protected_table("orders")` | orders | false |
| `is_table_writable(["write:orders"], "orders")` | — | true |
| `is_table_writable(["read:orders"], "orders")` | — | false |
| `is_table_writable(["orders"], "orders")` | — | true |

### 集成测试

| 测试 | 说明 |
|---|---|
| 插件写入有权限的表 | 成功，返回 rows_affected |
| 插件写入无权限的表 | 返回 error |
| 插件写入系统表 | 返回 protected error |
| 插件执行 SELECT | 返回 "only INSERT/UPDATE/DELETE allowed" |
| 插件执行 DDL | 返回 "DDL not allowed" |

---

## 9. 工作量

| 文件 | 操作 | 时间 |
|---|---|---|
| `permissions.rs` | 新增 4 个函数 + 15 个单元测试 | 2h |
| `host_common.rs` | 新增 `db_execute` 方法 | 1h |
| `js_host.rs` | 注册 `dbExecute` | 30min |
| `lua_host.rs` | 注册 `dbExecute` | 30min |
| `host.rs` | 注册 `host_db_execute` | 30min |
| 集成测试 | API 端到端测试 | 1h |
| **合计** | | **~5-6h** |
