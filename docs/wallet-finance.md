# 钱包与金融体系设计

> raisfast 通用后端平台 — 钱包、余额、交易流水基础架构。
> 支持：电商支付、虚拟物品买卖、AI API 充值消费、积分系统。
>
> 本文档仅定义**金融基础层**（2 张核心表 + 4 个原子操作）。
> 充值订单、提现、订阅、冻结预授权等业务模块按需扩展，通过调用基础层接口完成资金操作。

---

## 1. 设计原则

| 原则 | 说明 |
|------|------|
| **不可变流水** | 交易流水只 INSERT，永不 UPDATE/DELETE |
| **幂等操作** | 每笔交易带 `transaction_no`，重复请求返回原结果 |
| **金额整数** | 所有金额以**最小单位**存储（人民币→分，USD→cents），避免浮点误差 |
| **乐观锁** | 余额更新使用 `version` CAS，防止并发覆盖 |
| **多币种** | 每用户可持有多种币种（CNY、USD、points、credits、ai_tokens） |
| **可冲正** | 任何错误交易可通过 reverse 撤销，不破坏不可变原则 |

---

## 2. 币种体系

### 2.1 `currencies` 配置表

币种通过 `currencies` 表管理，管理员可动态增减。钱包操作时校验币种存在且启用。

```sql
CREATE TABLE IF NOT EXISTS currencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    code TEXT NOT NULL UNIQUE CHECK(code = UPPER(code) AND LENGTH(code) BETWEEN 1 AND 10),
    name TEXT NOT NULL,
    decimals INTEGER NOT NULL DEFAULT 0 CHECK(decimals BETWEEN 0 AND 18),
    is_active INTEGER NOT NULL DEFAULT 1 CHECK(is_active IN (0, 1)),
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
```

**约束规则：**

- `decimals` 在创建时设定，**后续不可修改**（防止金额语义突变）
- `is_active = false` 停用后，钱包操作会被拒绝（`currency_not_active`）
- 有钱包引用的币种不可删除（`currency_in_use`）
- `code` 必须大写、1-10 字符，DB 层 `CHECK(UPPER)` 兜底

**预设币种：**

| currency | 精度 | 说明 | 示例 |
|----------|------|------|------|
| `CNY` | 2（分） | 人民币 | 10000 = ¥100.00 |
| `USD` | 2（cents） | 美元 | 10000 = $100.00 |
| `POINTS` | 0（整数） | 积分（签到/消费/活动获得） | 500 = 500 积分 |
| `CREDITS` | 0（整数） | AI 点数（现金充值购买） | 100 = 100 点 |
| `AI_TOKENS` | 0（整数） | AI token 额度（credits 兑换） | 1000000 = 100 万 token |

新增币种通过 `POST /admin/currencies` 创建，不改表结构。

**流转示例：**

```
积分系统:
  签到   → credit(points, +10, reference_type=checkin)
  消费返积分 → credit(points, +50, reference_type=order_reward)
  积分兑换 → debit(points, reference_type=points_mall)

AI 充值消费:
  微信支付 ¥50 → credit(CNY, +5000, reference_type=recharge)
  CNY 购买 credits → transfer(CNY → credits)
  credits 兑换 tokens → transfer(credits → ai_tokens)
  调用 GPT-4o → debit(ai_tokens, reference_type=api_usage)
```

---

## 3. 表结构（共 2 张核心表）

### 3.1 `wallets` — 用户钱包（每用户每币种一个）

```sql
CREATE TABLE IF NOT EXISTS wallets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    currency TEXT NOT NULL,
    balance INTEGER NOT NULL DEFAULT 0 CHECK(balance >= 0),
    version INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(user_id, currency)
);

CREATE INDEX IF NOT EXISTS idx_wallets_user ON wallets(user_id);
CREATE INDEX IF NOT EXISTS idx_wallets_currency ON wallets(currency);
```

**核心不变量：**

```
balance >= 0
balance = SUM(credit) - SUM(debit) — 此用户此币种所有流水
```

### 3.2 `wallet_transactions` — 不可变交易流水

```sql
CREATE TABLE IF NOT EXISTS wallet_transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    wallet_id INTEGER NOT NULL REFERENCES wallets(id),
    user_id INTEGER NOT NULL REFERENCES users(id),
    entry_type TEXT NOT NULL,                  -- credit / debit
    amount INTEGER NOT NULL CHECK(amount > 0), -- 正整数，最小单位
    balance_after INTEGER NOT NULL CHECK(balance_after >= 0), -- 操作后余额快照
    tx_type TEXT NOT NULL,                     -- recharge / payment / refund / transfer_in / transfer_out
    currency TEXT NOT NULL,
    transaction_no TEXT NOT NULL UNIQUE,       -- 交易编号（幂等）
    related_tx_id INTEGER,                     -- 关联交易（refund→原payment, reversal→原交易）
    reference_type TEXT,                       -- order / recharge / checkin / order_reward / api_usage / points_mall / admin / ...
    reference_id TEXT,                         -- 业务单号
    counterparty_wallet_id INTEGER,            -- 转账对手钱包（仅 transfer）
    metadata TEXT,                             -- JSON 扩展
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_wallet_tx_wallet ON wallet_transactions(wallet_id);
CREATE INDEX IF NOT EXISTS idx_wallet_tx_user ON wallet_transactions(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_wallet_tx_transaction_no ON wallet_transactions(transaction_no);
CREATE INDEX IF NOT EXISTS idx_wallet_tx_tx_type ON wallet_transactions(tx_type);
CREATE INDEX IF NOT EXISTS idx_wallet_tx_reference ON wallet_transactions(reference_type, reference_id);
CREATE INDEX IF NOT EXISTS idx_wallet_tx_created ON wallet_transactions(created_at);
```

**tx_type 与 entry_type：**

| tx_type | entry_type | 说明 |
|---------|-----------|------|
| `recharge` | credit | 充值 |
| `payment` | debit | 支付 |
| `refund` | credit | 退款（related_tx_id → 原 payment） |
| `transfer_in` | credit | 转账收入 |
| `transfer_out` | debit | 转账支出 |

**业务语义由 `reference_type` 区分，不增加 tx_type：**

| reference_type | 用途 | 调用方式 |
|---------------|------|---------|
| `recharge` | 充值 | credit(tx_type=recharge) |
| `checkin` | 签到送积分 | credit(tx_type=recharge) |
| `order_reward` | 消费返积分 | credit(tx_type=recharge) |
| `api_usage` | AI API 消费 | debit(tx_type=payment) |
| `points_mall` | 积分商城兑换 | debit(tx_type=payment) |
| `order` | 电商支付 | debit(tx_type=payment) |
| `admin` | 管理员操作 | credit 或 debit |
| `expiry` | 积分过期 | debit(tx_type=payment) |

---

## 4. 4 个原子操作

### 4.1 credit — 加款

充值、奖励、退款、管理员加款。

```
事务:
   a. UPDATE wallets SET balance = balance + amount, version = version + 1
      WHERE id = ? AND version = ?
   b. INSERT wallet_transactions (entry_type=credit, balance_after=新余额)
```

### 4.2 debit — 扣款

支付、消费、过期扣减、管理员扣款。

```
事务:
   a. UPDATE wallets SET balance = balance - amount, version = version + 1
      WHERE id = ? AND balance >= amount AND version = ?
   b. INSERT wallet_transactions (entry_type=debit, balance_after=新余额)
   → affected_rows == 0 → 余额不足或版本冲突，重试
```

### 4.3 transfer — 转账

同一用户不同币种之间（CNY→credits），或不同用户之间（分账）。

```
事务:
   a. UPDATE wallet_a SET balance = balance - amount, version = version + 1
      WHERE balance >= amount AND version = ?
   b. INSERT wallet_transactions (wallet_a, debit, transfer_out, counterparty=wallet_b)
   c. UPDATE wallet_b SET balance = balance + amount, version = version + 1 WHERE version = ?
   d. INSERT wallet_transactions (wallet_b, credit, transfer_in, counterparty=wallet_a)
```

### 4.4 reverse — 冲正

管理员纠错、系统 bug 修复。生成一笔反向交易。

```
   1. 查找原交易 original_tx
   2. 生成反向 entry_type（credit↔debit）
   3. 事务:
      a. UPDATE wallets SET balance ± amount, version = version + 1 WHERE version = ?
      b. INSERT wallet_transactions (tx_type=与原交易相同, related_tx_id=original_tx.id)

   约束:
   - 每笔交易只能被冲正一次
   - 冲正不可再冲正
```

---

## 5. 幂等机制

```rust
pub async fn credit_wallet(
    repo: &dyn WalletRepository,
    pool: &Pool,
    user_id: i64,
    currency: &str,
    amount: i64,
    tx_type: WalletTxType,
    transaction_no: &str,
    reference_type: Option<WalletReferenceType>,
    reference_id: Option<&str>,
    metadata: Option<&str>,
) -> AppResult<WalletTransaction> {
    if amount <= 0 {
        return Err(AppError::BadRequest("amount_must_be_positive"));
    }

    // 快速路径：事务前用 pool 查（不占事务资源）
    if let Some(existing) = repo.find_tx_by_transaction_no(transaction_no).await? {
        return Ok(existing);
    }

    in_transaction!(pool, tx, {
        // 事务内二次确认（防 WAL 模式下读到旧数据）
        if let Some(existing) = tx_find_tx_by_transaction_no(&mut tx, transaction_no).await? {
            return Ok(existing);
        }

        // 币种白名单校验
        ensure_currency_active(&mut tx, currency).await?;

        // 乐观锁 + 余额更新
        let w = tx_find_or_create(&mut tx, user_id, currency).await?;
        apply_wallet_delta(&mut tx, w.id, w.version, amount, w.balance).await?;

        // 插入不可变流水
        let updated = tx_find_wallet_by_id(&mut tx, w.id).await?;
        insert_tx(&mut tx, updated.id, user_id, EntryType::Credit,
                  amount, updated.balance, tx_type, currency,
                  transaction_no, None, reference_type, reference_id, None, metadata).await
    })
}
```

---

## 6. API 端点

### 6.1 用户端（需认证）

| Method | Path | 说明 |
|--------|------|------|
| GET | `/wallets` | 列出当前用户所有钱包 |
| GET | `/wallets/{currency}` | 查询单个钱包详情 |
| GET | `/wallets/{currency}/transactions` | 查询交易流水（分页） |

### 6.2 管理端（需 admin）

| Method | Path | 说明 |
|--------|------|------|
| GET | `/admin/wallets` | 查看所有钱包 |
| GET | `/admin/wallets/{user_id}/{currency}/transactions` | 查看任意用户流水 |
| POST | `/admin/wallets/credit` | 管理员手动加款 |
| POST | `/admin/wallets/debit` | 管理员手动扣款 |
| POST | `/admin/wallets/{tx_id}/reversal` | 冲正指定交易 |
| GET | `/admin/currencies` | 列出所有币种 |
| GET | `/admin/currencies/{code}` | 查看单个币种 |
| POST | `/admin/currencies` | 创建币种 |
| PUT | `/admin/currencies/{code}` | 修改币种（name/is_active） |

### 6.3 内部接口（Service 层调用，不暴露 HTTP）

| 函数 | 说明 |
|------|------|
| `credit_wallet()` | 加款 |
| `debit_wallet()` | 扣款 |
| `transfer()` | 转账 |
| `reverse_transaction()` | 冲正 |

---

## 7. 金融级安全分析

### 7.1 资金安全

| # | 维度 | 措施 | 保护层数 |
|---|------|------|---------|
| 1 | **原子性** | `credit`/`debit`/`transfer`/`reverse` 全部在 `in_transaction!` 内，任何一步失败整体回滚 | 1 |
| 2 | **乐观锁** | `UPDATE ... WHERE version = ?`，`affected == 0` 检测并发冲突，覆盖 wallet 和 currencies 表 | 1 |
| 3 | **余额非负** | ① Rust `checked_add` 预检查 ② SQL `WHERE balance >= abs` ③ DB `CHECK(balance >= 0)` | 3 |
| 4 | **金额正数** | ① Rust `amount <= 0` 拒绝 ② DB `CHECK(amount > 0)` | 2 |
| 5 | **溢出保护** | Rust `checked_add` 在 SQL UPDATE 前预检查，`balance + delta > i64::MAX` 返回 `balance_overflow` | 1 |
| 6 | **不可变流水** | `wallet_transactions` 代码中无 UPDATE/DELETE 路径，只有 INSERT | 1 |
| 7 | **幂等性** | ① 事务前 `repo` 快速检查 ② 事务内 `tx` 函数二次检查 ③ DB `transaction_no UNIQUE` 兜底 | 3 |
| 8 | **余额快照** | 每笔流水记录 `balance_after`，可对账验证 `SUM(credit) - SUM(debit) = wallet.balance` | 1 |
| 9 | **冻结保护** | 所有操作入口检查 `wallet.status == Active`，`Frozen` 状态拒绝一切资金变动 | 1 |
| 10 | **冲正安全** | ① 禁止冲正一笔冲正 ② 每笔交易只能被冲正一次 ③ 转账冲正自动双向冲正在同一事务内 | 3 |

### 7.2 币种安全

| # | 维度 | 措施 |
|---|------|------|
| 1 | **格式校验** | DTO 层 `validate_currency_code` — 大写 ASCII 1-10 字符 |
| 2 | **白名单** | 事务内 `ensure_currency_active()` 查 `currencies` 表确认 `is_active = 1` |
| 3 | **DB 约束** | `CHECK(code = UPPER(code) AND LENGTH(code) BETWEEN 1 AND 10)` |
| 4 | **decimals 不可变** | `create` 时设定，`update` API 不暴露 `decimals` 字段 |
| 5 | **删除保护** | `delete_by_code` 查 `wallets` 表有无引用，有则返回 `currency_in_use` |
| 6 | **并发修改** | `currencies.update` 使用 `version` 乐观锁 |

### 7.3 类型安全

| # | 维度 | 措施 |
|---|------|------|
| 1 | **Enum 全覆盖** | `WalletStatus`、`WalletEntryType`、`WalletTxType`、`WalletReferenceType` 替代所有魔法字符串 |
| 2 | **DB → Rust** | Model 字段保持 `String`（兼容 sqlx FromRow），通过 `status_enum()` / `tx_type_enum()` 等方法获取类型安全枚举 |
| 3 | **脏数据保护** | 所有 enum accessor 返回 `Result<T, String>`，非法值转为 `AppError::Internal` 返回 500，不 panic 不静默 |
| 4 | **DTO 类型** | Response 中 `status`/`entry_type`/`tx_type`/`reference_type` 使用 enum 类型，OpenAPI schema 自动生成枚举值 |
| 5 | **Serde 一致** | `define_enum!` 宏为每个 variant 生成 `#[serde(rename = $value)]`，JSON 序列化值与 DB 存储一致 |

### 7.4 SQL 安全

| # | 措施 |
|---|------|
| 1 | 全部 SQL 使用 `ph()` 占位符 + `.bind()` 参数绑定，零字符串拼接 |
| 2 | `define_enum!` 宏自动 `#[derive(utoipa::ToSchema)]`，OpenAPI 文档自动同步 |
| 3 | Model 层 `has_reversal_for` 等查询也使用 bind param 传入 enum 值 |

### 7.5 操作流程安全图

```
HTTP 请求
  │
  ├─ Handler 层
  │   ├─ auth.ensure_admin() / auth.ensure_authenticated()
  │   ├─ validation::validate(&req) — DTO 格式校验（含 currency 格式）
  │   └─ 调用 service
  │
  ├─ Service 层（in_transaction! 内）
  │   ├─ 幂等检查：repo.find_tx_by_transaction_no() 快速路径
  │   ├─ 开启事务
  │   ├─ 幂等检查：tx_find_tx_by_transaction_no() 二次确认
  │   ├─ 币种验证：ensure_currency_active() 查表白名单
  │   ├─ 钱包状态：w.status_enum() != Active → 拒绝
  │   ├─ 溢出检查：checked_add(delta) → 拒绝
  │   ├─ 余额更新：UPDATE ... WHERE version = ? AND balance >= ? → CAS
  │   ├─ 余额快照：SELECT updated balance
  │   ├─ 流水记录：INSERT wallet_transactions → 不可变
  │   └─ 事务提交（任一步失败整体回滚）
  │
  └─ Response
      └─ DTO enum 字段 → JSON 枚举值
```

---

## 8. 文件变更清单

| 文件 | 变更 |
|------|------|
| `migrations/sqlite/schema.sqlite.sql` | 新增 currencies / wallets / wallet_transactions |
| `migrations/postgres/schema.postgres.sql` | 同上 |
| `migrations/mysql/schema.mysql.sql` | 同上 |
| `src/models/currencies.rs` | Currency 结构体 + CRUD + 币种约束 |
| `src/models/wallet.rs` | Wallet 结构体 + CRUD + WalletStatus enum |
| `src/models/wallet_transaction.rs` | WalletTransaction 结构体 + CRUD + 3 个 enum |
| `src/services/wallet.rs` | credit / debit / transfer / reverse + 币种验证 + 溢出保护 |
| `src/handlers/wallet.rs` | 用户端 + 管理端 handler |
| `src/handlers/currencies.rs` | 币种管理 handler（list/get/create/update） |
| `src/dto/wallet.rs` | 钱包 DTO（enum 类型响应） |
| `src/dto/currencies.rs` | 币种 DTO |
| `src/macros.rs` | `define_enum!` 宏 |

---

## 9. 实施步骤

```
Phase 1: Schema + Model ✅
   1. currencies / wallets / wallet_transactions 三张表写入 schema
   2. Currency / Wallet / WalletTransaction model + CRUD
   3. define_enum! 宏 + 4 个钱包 enum

Phase 2: Core Service ✅
   4. credit_wallet / debit_wallet（乐观锁 + 幂等 + 流水 + 币种验证）
   5. transfer（转账 + 自转拒绝）
   6. reverse_transaction（冲正 + 转账双向冲正）
   7. 溢出保护 + 余额非负三重校验

Phase 3: Handler + DTO ✅
   8. 用户端 3 个端点 + 管理端 7 个端点
   9. 币种管理 4 个端点（list/get/create/update）
   10. DTO enum 类型响应 + OpenAPI schema

Phase 4: 测试 ✅
   11. 57 wallet 测试 + 7 currencies 测试 + 全量 1609 测试通过
   12. 0 clippy warnings
```

---

## 10. 与专业金融系统的差距

当前实现在 CMS/低代码平台内置钱包中属于顶级，但与 Kill Bill、Stripe 等专业金融系统相比，存在三个架构级差距。

### 10.1 复式记账（Double-Entry Bookkeeping）

**现状：** 单式记账 + `balance_after` 快照。校验依赖 `SUM(credit) - SUM(debit) = wallet.balance`。

**差距：** 专业金融系统使用严格复式记账 — 每笔操作产生 >= 2 条 entry，`SUM(all entries) = 0` 是数学不变量，无需额外对账即可自证明正确性。

**示例：** 用户充值 ¥100

```
当前（单式）：
  wallet_transactions: [credit ¥100, balance_after=100]

复式记账：
  ledger_entries: [
    { account: "wallet:user:1",   debit:  100 },  // 资产增加
    { account: "revenue:deposit", credit: 100 },   // 负债增加
  ]
  // SUM(debit) - SUM(credit) = 100 - 100 = 0  ← 自证明
```

**改造方案：**

```
1. 新增 ledger_accounts 表（账户树：wallet:user:1, revenue:deposit, fee:transfer 等）
2. 新增 ledger_entries 表（每笔操作写入 >= 2 条 entry）
3. 保留 wallet_transactions 作为用户视角流水（从 ledger 聚合生成）
4. 不变量：SUM(ledger_entries.debit) = SUM(ledger_entries.credit)（按 transaction_no 分组）
```

**优先级：** 中。当前单式记账 + `balance_after` 快照足以覆盖大部分场景，复式记账主要在需要合规审计（如持牌支付）时才必须。

### 10.2 对账任务（Reconciliation）

**现状：** 没有定时任务验证数据一致性。依赖代码正确性保证 `balance_after` 连续性和 `wallets.balance` 准确性。

**差距：** 金融系统必须有自动对账机制，及时发现代码 bug、数据库损坏、手动改库等异常。

**改造方案：**

```
对账维度：
1. 钱包余额对账：
   wallet.balance = SUM(CASE entry_type='credit' THEN amount ELSE -amount END)
   WHERE wallet_id = ?

2. balance_after 连续性：
   按 created_at 排序，第 N 笔的 balance_after = 第 N-1 笔的 balance_after ± amount

3. 转账配对完整性：
   每笔 transfer_out 必须有对应的 transfer_in（by transaction_no）

4. 冲正一致性：
   refund 交易的 related_tx_id 必须存在且非 refund 类型

实施：
   - 新建 src/services/wallet_reconciliation.rs
   - 定时任务（cron）每小时/每日执行
   - 不一致时写入 wallet_reconciliation_reports 表 + 告警
```

**优先级：** 高。即使单式记账，对账也是金融系统必备的安全网。

### 10.3 事件通知（Event / Webhook）

**现状：** 钱包变动后无 event/webhook 通知下游。充值成功后订单系统只能轮询或直接耦合调用。

**差距：** 专业系统中，钱包状态变更是事件源，下游系统（订单、通知、BI）通过订阅事件响应，实现解耦。

**改造方案：**

```
方案 A：进程内事件总线（简单）
   - wallet 操作成功后 emit WalletEvent::Credited / Debited / Transferred / Reversed
   - 下游模块注册 handler（如订单模块监听 Credited 后更新订单状态）
   - 利用现有 tokio mpsc 或 Event trait

方案 B：持久化事件表 + Webhook（完整）
   - 新增 wallet_events 表（不可变事件流）
   - 新增 wallet_event_subscriptions 表（URL + 事件类型过滤）
   - 事件写入后异步 HTTP POST 到订阅者
   - 失败重试（指数退避）+ 死信队列
   - 利用现有 webhook_subscriptions 基础设施

推荐：先做方案 A（成本低），业务复杂后再升级方案 B。
```

**优先级：** 中高。直接决定钱包能否作为独立模块被其他业务集成。

### 10.4 差距优先级总结

| 差距 | 优先级 | 改动量 | 前置条件 |
|------|--------|--------|---------|
| **对账任务** | 高 | 小（新增 1 个 service + 1 张表） | cron 定时任务框架已有 |
| **事件通知** | 中高 | 中（方案 A 小，方案 B 中） | 无 |
| **复式记账** | 中 | 大（2 张新表 + 重写核心逻辑） | 建议先稳定单式记账再迁移 |

---

## 11. 未来按需扩展

| 功能 | 扩展方式 |
|------|---------|
| **冻结预授权** | 新增 `wallet_freezes` 表 + freeze/unfreeze/deduct_frozen 操作（电商下单场景） |
| **第三方支付** | 业务层建 `recharge_orders` 表 + `PaymentGateway` trait，回调调 `credit_wallet()` |
| **提现** | 业务层建 `withdrawal_orders` 表，审核通过调 `debit_wallet()` |
| **订阅** | 业务层建 `subscriptions` 表，定时调 `debit_wallet()` |
| **积分过期** | 业务层记录过期时间，定时任务调 `debit_wallet(reference_type=expiry)` |
| **多商户分账** | `transfer()` 从买家钱包到卖家钱包 |
| **对账报表** | 聚合 `wallet_transactions` 按日/月/币种统计 |
