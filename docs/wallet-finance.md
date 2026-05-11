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

| currency | 精度 | 说明 | 示例 |
|----------|------|------|------|
| `CNY` | 2（分） | 人民币 | 10000 = ¥100.00 |
| `USD` | 2（cents） | 美元 | 10000 = $100.00 |
| `points` | 0（整数） | 积分（签到/消费/活动获得） | 500 = 500 积分 |
| `credits` | 0（整数） | AI 点数（现金充值购买） | 100 = 100 点 |
| `ai_tokens` | 0（整数） | AI token 额度（credits 兑换） | 1000000 = 100 万 token |

新增币种只需在应用层注册，不改表结构。

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
    balance INTEGER NOT NULL DEFAULT 0,        -- 可用余额（最小单位）
    version INTEGER NOT NULL DEFAULT 1,        -- 乐观锁版本号
    status TEXT NOT NULL DEFAULT 'active',     -- active / frozen / closed
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
    amount INTEGER NOT NULL,                   -- 正整数，最小单位
    balance_after INTEGER NOT NULL,            -- 操作后余额快照
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
    pool: &Pool,
    wallet_id: i64,
    amount: i64,
    tx_type: &str,
    transaction_no: &str,
    reference: Option<(&str, &str)>,
) -> AppResult<WalletTransaction> {
    if let Some(existing) = find_tx_by_transaction_no(pool, transaction_no).await? {
        return Ok(existing);
    }

    in_transaction!(pool, tx, {
        if let Some(existing) = find_tx_by_transaction_no(&mut *tx, transaction_no).await? {
            return Ok(existing);
        }

        let affected = update_balance_cas(&mut *tx, wallet_id, amount, 1).await?;
        if affected == 0 {
            return Err(AppError::Conflict("concurrent_wallet_update"));
        }

        let entry = insert_transaction(&mut *tx, ...).await?;
        Ok(entry)
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

### 6.3 内部接口（Service 层调用，不暴露 HTTP）

| 函数 | 说明 |
|------|------|
| `credit_wallet()` | 加款 |
| `debit_wallet()` | 扣款 |
| `transfer()` | 转账 |
| `reverse_transaction()` | 冲正 |

---

## 7. 安全策略

| 策略 | 说明 |
|------|------|
| 乐观锁 | `UPDATE ... WHERE version = ?`，affected_rows == 0 时重试（最多 3 次） |
| 幂等 | 所有写操作必须有 `transaction_no`，由调用方生成（如 `order_123_payment`） |
| 金额整数 | 金额一律为 `i64`，展示时除以精度（CNY/100，credits/1） |
| 余额非负 | `balance >= 0` 由 `WHERE balance >= amount` 保证 |
| 流水对账 | `SUM(credit) - SUM(debit)` 应等于 `wallets.balance` |
| 冲正约束 | 每笔交易只能被冲正一次，冲正不可再冲正 |
| 管理员审计 | 管理员手动操作写入 `audit_log` |
| 钱包状态 | `status = frozen` 时禁止所有操作（司法冻结等场景） |

---

## 8. 文件变更清单

| 文件 | 变更 |
|------|------|
| `migrations/sqlite/schema.sqlite.sql` | 新增 wallets / wallet_transactions |
| `src/models/wallet.rs` | Wallet / WalletTransaction 结构体 + CRUD |
| `src/services/wallet.rs` | credit / debit / transfer / reverse |
| `src/handlers/wallet.rs` | 用户端 API handler |
| `src/handlers/admin_wallet.rs` | 管理端 API handler |
| `src/dto/wallet.rs` | 请求/响应 DTO |

---

## 9. 实施步骤

```
Phase 1: Schema + Model
  1. 两张表写入 schema
  2. Wallet / WalletTransaction model + CRUD
  3. 编译通过

Phase 2: Core Service（4 个原子操作）
  4. credit_wallet / debit_wallet（乐观锁 + 幂等 + 流水）
  5. transfer
  6. reverse_transaction

Phase 3: Handler + DTO
  7. 用户端 3 个端点
  8. 管理端 5 个端点

Phase 4: 测试
  9. 单元测试（并发扣款、幂等、余额非负、冲正）
  10. 对账定时任务
```

---

## 10. 未来按需扩展

| 功能 | 扩展方式 |
|------|---------|
| **冻结预授权** | 新增 `wallet_freezes` 表 + freeze/unfreeze/deduct_frozen 操作（电商下单场景） |
| **第三方支付** | 业务层建 `recharge_orders` 表 + `PaymentGateway` trait，回调调 `credit_wallet()` |
| **提现** | 业务层建 `withdrawal_orders` 表，审核通过调 `debit_wallet()` |
| **订阅** | 业务层建 `subscriptions` 表，定时调 `debit_wallet()` |
| **积分过期** | 业务层记录过期时间，定时任务调 `debit_wallet(reference_type=expiry)` |
| **多商户分账** | `transfer()` 从买家钱包到卖家钱包 |
| **对账报表** | 聚合 `wallet_transactions` 按日/月/币种统计 |
