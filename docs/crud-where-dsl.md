# CRUD Where DSL 设计

## 背景

当前 `crud_query_paged!` 要求用户手写原始 SQL：

```rust
crud_query_paged!(pool, Order,
    data_sql: "SELECT * FROM orders WHERE user_id = ?{tenant} ORDER BY created_at DESC",
    count_sql: "SELECT COUNT(*) FROM orders WHERE user_id = ?{tenant}",
    binds: [user_id],
    tenant: tenant_id,
    page: page,
    page_size: page_size
)
```

### 问题

1. **占位符不兼容** — `?` 在 PostgreSQL 下必须写成 `$N`，用户手写 `?` 会导致 PG 运行时报错
2. **`{tenant}` hack** — 字符串替换方式注入租户条件，脆弱且不优雅
3. **无编译时校验** — 列名拼写错误只能在运行时发现
4. **跨库语法陷阱** — 用户可能无意中写了 `ILIKE`、`::text`、`RETURNING` 等 PG 特有语法

## 设计目标

1. 用户**不再写原始 SQL 的 WHERE 子句**，改用结构化 DSL
2. 宏完全控制 SQL 生成，自动适配 `$N` / `?N` / `?`
3. 编译时校验列名
4. `{tenant}` 自动注入，用户不感知
5. 覆盖 80% 的常见查询场景，剩余 20% 用 `crud_query!` + `Driver::ph()` 手写

## DSL 语法

### 基本形式

```
("column", value)                    → col = ?
("column", OP, value)                → col OP ?
```

### 逻辑组合

```
AND( cond1, cond2, ... )             → (cond1 AND cond2 AND ...)
OR( cond1, cond2, ... )              → (cond1 OR cond2 OR ...)
```

可任意嵌套：

```
AND( ("status", 1), OR(("role", "admin"), ("role", "editor")) )
```

生成 SQL：

```sql
(status = ?) AND ((role = ?) OR (role = ?))
```

### 运算符

| 运算符 | SQL | 元素 | 示例 |
|---|---|---|---|
| `EQ`（默认，可省略） | `= ?` | 2-tuple 或 3-tuple | `("id", 1)` 或 `("id", EQ, 1)` |
| `NEQ` | `!= ?` | 3-tuple | `("status", NEQ, "deleted")` |
| `GT` / `GTE` | `> ?` / `>= ?` | 3-tuple | `("amount", GT, 100)` |
| `LT` / `LTE` | `< ?` / `<= ?` | 3-tuple | `("created_at", LTE, now)` |
| `LIKE` | `LIKE ?` | 3-tuple | `("title", LIKE, "%rust%")` |
| `NOT_LIKE` | `NOT LIKE ?` | 3-tuple | `("title", NOT_LIKE, "%spam%")` |
| `IN` | `IN (?,?,?)` | 3-tuple，value 为 Vec | `("status", IN, vec!["a","b"])` |
| `NOT_IN` | `NOT IN (?,?,?)` | 3-tuple，value 为 Vec | `("status", NOT_IN, vec!["x"])` |
| `IS_NULL` | `IS NULL` | 2-tuple，value 为 `()` | `("deleted_at", ())` |
| `NOT_NULL` | `IS NOT NULL` | 特殊标记 | 待定 |

### 可选条件（动态 WHERE）

用 `Option` 包装值，`None` 时跳过该条件：

```rust
// status 为 None 时不加入 WHERE
where: AND(("user_id", uid), opt!("status", status))
```

`opt!` 宏在编译时展开为条件绑定代码：

```rust
// 生成的代码
if let Some(ref __wv) = status {
    __ph_idx += 1;
    __where_sql.push_str(&format!(" AND status = {}", __ph(__ph_idx)));
}
```

### 完整调用示例

迁移前（手写 SQL）：

```rust
crud_query_paged!(pool, Comment,
    data_sql: "SELECT * FROM comments WHERE post_id = ? AND status = ?{tenant} ORDER BY created_at ASC",
    count_sql: "SELECT COUNT(*) FROM comments WHERE post_id = ? AND status = ?{tenant}",
    binds: [post_id, CommentStatus::Approved],
    tenant: tenant_id,
    page: page,
    page_size: page_size
)
```

迁移后（DSL）：

```rust
crud_query_paged!(pool, Comment,
    table: "comments",
    where: AND(("post_id", post_id), ("status", CommentStatus::Approved)),
    order_by: "created_at ASC",
    page: page,
    page_size: page_size,
    tenant: tenant_id
)
```

- 无 `data_sql` / `count_sql` / `binds` — 宏从 DSL 自动生成
- 无 `{tenant}` — `tenant:` 参数自动注入
- 无 `?` — 占位符由宏根据 dialect 自动生成

## 宏 API 对比

### 现有 API（保留）

```rust
crud_query_paged!(pool, Type,
    data_sql: "...",      // 用户手写 SQL，含 ? 和 {tenant}
    count_sql: "...",     // 用户手写 SQL
    binds: [val1, val2],  // 绑定值
    where: [...],         // 可选的动态条件
    tenant: expr,
    page: expr,
    page_size: expr
)
```

### 新 API（推荐）

```rust
crud_query_paged!(pool, Type,
    table: "orders",                              // 表名
    where: AND(("user_id", uid), opt!("status", status)),  // DSL 条件
    order_by: "created_at DESC",                  // 排序
    tenant: expr,                                 // 自动注入租户
    page: expr,
    page_size: expr
)
```

**解析策略**：宏检测第一个命名参数。如果是 `data_sql:` → 走旧路径；如果是 `table:` → 走新路径。向后兼容。

## 现有调用迁移分析

| # | 文件 | 现有 WHERE | DSL 等价 |
|---|---|---|---|
| 1 | `wallet_transaction.rs` | `wallet_id = ?` | `("wallet_id", wallet_id)` |
| 2 | `wallet_transaction.rs` | `user_id = ?` | `("user_id", user_id)` |
| 3 | `wallet_transaction.rs` | `1=1{tenant}` | 无 where，仅 tenant |
| 4 | `page.rs` | `status = ?{tenant}` | `("status", status)` |
| 5 | `order.rs` | `user_id = ?{tenant}` | `("user_id", user_id)` |
| 6 | `media.rs` | `user_id = ?{tenant}` | `("user_id", user_id)` |
| 7 | `payment_order.rs` | `user_id = ?{tenant}` | `("user_id", user_id)` |
| 8 | `comment.rs` | `post_id = ? AND status = ?{tenant}` | `AND(("post_id", post_id), ("status", status))` |
| 9-28 | 其他 20 处 | `1=1{tenant}` + 可选 where | 无 where + `where: [...]` |

**100% 现有用法可迁移。**

## 不适合 DSL 的场景

以下场景继续使用 `crud_query!` + `Driver::ph()` 手写：

| 场景 | 原因 | 示例 |
|---|---|---|
| 子查询 | `WHERE id IN (SELECT ...)` | `worker/job_queue.rs` dequeue |
| 聚合 | `GROUP BY` / `HAVING` / `SUM()` | `models/payment_refund.rs` sum_refunded |
| CASE 表达式 | `CASE WHEN ... THEN ... END` | `models/wallet_outbox.rs` mark_failed |
| 动态表名 | `FROM {dynamic_table}` | `services/stats.rs` |
| 动态列名 | `SET {col} = ?` | `models/order.rs` timestamp_col |
| 复杂 JOIN | 多表 JOIN + 动态 WHERE | `models/post.rs` |
| CAS 更新 | `version = version + 1` | `models/order.rs` tx_update_status_cas |

## 实现计划

### Phase 1: 解析器

在 `raisfast-derive/src/crud.rs` 中新增 DSL 解析：

- `WhereExpr` 枚举：`Condition` / `And` / `Or` / `Optional`
- `Operator` 枚举：`EQ` / `NEQ` / `GT` / `GTE` / `LT` / `LTE` / `LIKE` / `IN` / `IS_NULL` ...
- `Condition` 结构体：`(col, [op,] value)`

### Phase 2: SQL 生成

`WhereExpr` → SQL 字符串 + 绑定参数列表：

- 递归遍历 AST，生成 `col = ?N` / `col > ?N` 等
- 自动追踪 `__ph_idx`
- tenant 条件自动追加

### Phase 3: 编译时校验

对 DSL 中的每个列名调用 `validate_column(table, col)`，编译时报错拼写错误。

### Phase 4: 旧 API 兼容

- `data_sql:` 路径保留，加编译警告建议迁移
- `crud_join_paged!` 同步支持 DSL

### Phase 5: 迁移现有调用

将 28 处 `crud_query_paged!` 从手写 SQL 迁移到 DSL。

## Rust 类型表达（待定）

在 proc-macro 中解析 DSL 有几种方案：

### 方案 A: Token 解析

```rust
// 用户写法
where: AND(("post_id", post_id), ("status", status))

// proc-macro 解析 TokenStream
// 识别 AND(...) / OR(...) / (...) 三种模式
```

优点：语法简洁，接近 SQL 思维。
难点：Rust 宏里 `(expr, expr)` 是 tuple 字面量，需要自定义解析器。

### 方案 B: 数组语法

```rust
// 用户写法
where: [AND, [("post_id", post_id), ("status", status)]]

// proc-macro 更容易解析
```

优点：解析简单。
缺点：不直观。

### 方案 C: 关键字语法

```rust
// 用户写法
where: "post_id" => post_id, and: ["status" => status]

// 与现有 crud_find! 语法一致
```

优点：已有先例，解析器已存在。
缺点：不支持 OR / 嵌套 / 运算符。

**推荐方案 A**，因为它最接近 SQL 表达力，且 proc-macro 有完整的 TokenStream 解析能力。

## 占位符生成规则

| Dialect | `EQ` | `GT` | `IN(vec![a,b,c])` | `IS_NULL` | `LIKE` |
|---|---|---|---|---|---|
| SQLite | `?1` | `?2` | `IN (?3,?4,?5)` | `IS NULL` | `LIKE ?6` |
| PostgreSQL | `$1` | `$2` | `IN ($3,$4,$5)` | `IS NULL` | `LIKE $6` |
| MySQL | `?` | `?` | `IN (?,?,?)` | `IS NULL` | `LIKE ?` |

所有占位符由 `Dialect::ph(idx)` 生成，idx 由宏在遍历 AST 时自动递增。

## 与 `crud_join_paged!` 的关系

`crud_join_paged!` 已自行生成 WHERE SQL（不依赖用户手写），但只有 `=` 运算符。
DSL 可同时应用于两个宏，统一条件表达方式。

## 风险和缓解

| 风险 | 缓解 |
|---|---|
| DSL 解析复杂度 | 递归下降解析器，参考 `expand_find` 现有的 `and:` 解析 |
| proc-macro 编译变慢 | DSL 只在 `crud_query_paged!` 中使用，影响面小 |
| 用户学习成本 | DSL 元素与 SQL 一一对应，直觉易学 |
| 不支持的 SQL 回退 | 保留 `crud_query!` + `Driver::ph()` 手写路径 |
