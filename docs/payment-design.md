# Payment Integration Design

## Core Principle

Payment and Wallet are **independent modules**. The only relationship:

```
Provider webhook → payment_orders.status = paid
                 → INSERT wallet_transactions (credit)
```

Both operations happen in one DB transaction. That's it.

Payment does not know wallet internals. Wallet does not know which provider the money came from. They communicate through `wallet_transactions.reference_type = "payment"` + `reference_id = payment_order.document_id`.

## Architecture

```
┌────────────┐     ┌──────────────┐     ┌─────────────────┐     ┌───────────┐
│  Handler    │────>│   Service    │────>│ PaymentProvider  │────>│  Gateway  │
│ (thin API)  │     │ (business)   │     │    (trait)       │     │ (Stripe   │
└────────────┘     └──────┬───────┘     └─────────────────┘     │  PayPal   │
                          │                                     │  WxPay    │
                    ┌─────▼──────┐                               │  Alipay)  │
                    │  Model     │                               └───────────┘
                    │ (sqlx DB)  │
                    └────────────┘
```

## Database Schema

### payment_channels

```sql
CREATE TABLE IF NOT EXISTS payment_channels (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    provider TEXT NOT NULL,                    -- stripe/paypal/wxpay/alipay
    name TEXT NOT NULL,                        -- "Stripe USD"
    is_live INTEGER NOT NULL DEFAULT 0,
    credentials TEXT NOT NULL,                 -- AES-256-GCM encrypted JSON
    webhook_secret TEXT,                       -- AES-256-GCM encrypted
    settings TEXT,                             -- JSON: { currencies:["USD","CNY"], methods:["card","alipay_qr"], min_amount:100, max_amount:10000000 }
    is_active INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(provider, name)
);
```

### payment_orders

```sql
CREATE TABLE IF NOT EXISTS payment_orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,

    user_id INTEGER NOT NULL REFERENCES users(id),
    order_id TEXT,                             -- references orders.document_id (loose coupling)
    title TEXT NOT NULL,

    amount INTEGER NOT NULL CHECK(amount > 0), -- smallest unit (cents)
    currency TEXT NOT NULL DEFAULT 'USD',

    channel_id INTEGER NOT NULL REFERENCES payment_channels(id),
    provider TEXT NOT NULL,
    provider_order_id TEXT,                    -- Stripe: pi_xxx, Alipay: trade_no
    provider_method TEXT,                      -- card/alipay_qr/wechat_jsapi/paypal

    status TEXT NOT NULL DEFAULT 'pending',

    reference_type TEXT,                       -- order/subscription/wallet_topup
    reference_id TEXT,                         -- business entity ID
    return_url TEXT,

    idempotency_key TEXT NOT NULL UNIQUE,

    version INTEGER NOT NULL DEFAULT 1,        -- optimistic lock for concurrent refund

    provider_data TEXT,                        -- JSON: last provider response
    client_ip TEXT,
    metadata TEXT,

    paid_at TEXT,
    cancelled_at TEXT,
    expired_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_payment_orders_user ON payment_orders(user_id);
CREATE INDEX IF NOT EXISTS idx_payment_orders_status ON payment_orders(status);
CREATE INDEX IF NOT EXISTS idx_payment_orders_provider ON payment_orders(provider_order_id);
CREATE INDEX IF NOT EXISTS idx_payment_orders_order_id ON payment_orders(order_id);
```

### payment_transactions (immutable ledger, append-only)

Records every external provider event. Separate from wallet_transactions — this is the **provider-side** ledger.

```sql
CREATE TABLE IF NOT EXISTS payment_transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,

    payment_order_id INTEGER NOT NULL REFERENCES payment_orders(id),
    order_id TEXT,                             -- denormalized from payment_orders.order_id for direct query
    user_id INTEGER NOT NULL REFERENCES users(id),

    tx_type TEXT NOT NULL,                     -- charge / refund
    amount INTEGER NOT NULL CHECK(amount > 0),
    currency TEXT NOT NULL,

    provider_tx_id TEXT NOT NULL UNIQUE,       -- Stripe: ch_xxx / re_xxx, prevents duplicate webhook inserts
    status TEXT NOT NULL DEFAULT 'pending',    -- pending/succeeded/failed

    raw_payload TEXT,                          -- JSON: original webhook body

    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_payment_tx_order ON payment_transactions(payment_order_id);
CREATE INDEX IF NOT EXISTS idx_payment_tx_order_id ON payment_transactions(order_id);
```

### payment_refunds

```sql
CREATE TABLE IF NOT EXISTS payment_refunds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,

    payment_order_id INTEGER NOT NULL REFERENCES payment_orders(id),
    order_id TEXT,                             -- denormalized from payment_orders.order_id for direct query
    user_id INTEGER NOT NULL REFERENCES users(id),

    amount INTEGER NOT NULL CHECK(amount > 0),
    currency TEXT NOT NULL,
    reason TEXT,                               -- user_request/duplicate/fraud/other

    provider_refund_id TEXT,                   -- Stripe: re_xxx
    status TEXT NOT NULL DEFAULT 'pending',    -- pending/processing/succeeded/failed

    payment_tx_id INTEGER REFERENCES payment_transactions(id),

    metadata TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_payment_refunds_order ON payment_refunds(payment_order_id);
CREATE INDEX IF NOT EXISTS idx_payment_refunds_order_id ON payment_refunds(order_id);
```

## State Machine

```
                    ┌──────────┐
                    │ pending  │ ← create order
                    └────┬──┬──┘
                         │  │
         user cancel     │  │  provider confirms (webhook)
              ┌──────────┘  │
              ▼             ▼
        ┌──────────┐  ┌──────────┐
        │cancelled │  │   paid   │
        └──────────┘  └────┬─────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
         full refund   partial     expired
              │        refund          │
              ▼            │           ▼
        ┌──────────┐      │     ┌─────────┐
        │ refunded │◄─────┘     │ expired │
        └──────────┘            └─────────┘
```

## Sequence Diagrams

### 1. Create Payment

```
Client              Handler             Service              Provider           DB
  │                    │                    │                    │               │
  │ POST /orders       │                    │                    │               │
  │───────────────────>│                    │                    │               │
  │                    │ create_order()     │                    │               │
  │                    │───────────────────>│                    │               │
  │                    │                    │                    │  INSERT order │
  │                    │                    │                    │  status=pending
  │                    │                    │                    │<──────────────│
  │                    │                    │  create()          │               │
  │                    │                    │──────────────────>│               │
  │                    │                    │  ProviderResponse  │               │
  │                    │                    │<─────────────────│               │
  │                    │                    │                    │  UPDATE       │
  │                    │                    │                    │  provider_id  │
  │                    │                    │                    │──────────────>│
  │  {redirect_url,   │                    │                    │               │
  │   qr_code,        │                    │                    │               │
  │   client_secret}  │                    │                    │               │
  │<──────────────────│<───────────────────│                    │               │
  │                    │                    │                    │               │
```

### 2. Webhook Callback (Provider → RaisFast)

```
Provider          Handler             Service              Wallet           DB
  │                  │                    │                    │             │
  │ POST /callback   │                    │                    │             │
  │─────────────────>│                    │                    │             │
  │                  │ handle_callback()  │                    │             │
  │                  │───────────────────>│                    │             │
  │                  │                    │                    │             │
  │                  │                    │ ┌─ verify signature ──────────┐  │
  │                  │                    │ │ invalid → 400              │  │
  │                  │                    │ └────────────────────────────┘  │
  │                  │                    │                    │             │
  │                  │                    │ ┌─ find order ────────────────┐  │
  │                  │                    │ │ not found → 400            │  │
  │                  │                    │ └────────────────────────────┘  │
  │                  │                    │                    │             │
  │                  │                    │ ┌─ idempotency check ────────┐  │
  │                  │                    │ │ already paid → 200 OK      │  │
  │                  │                    │ └────────────────────────────┘  │
  │                  │                    │                    │             │
  │                  │                    │ ┌─ amount check ─────────────┐  │
  │                  │                    │ │ mismatch → 400 + alert     │  │
  │                  │                    │ └────────────────────────────┘  │
  │                  │                    │                    │             │
  │                  │                    │ ── atomic transaction ──────────>│
  │                  │                    │                    │             │
  │                  │                    │ 1. payment_orders  │             │
  │                  │                    │    status = paid   │             │
  │                  │                    │                    │             │
  │                  │                    │ 2. payment_transactions INSERT  │
  │                  │                    │    tx_type=charge  │             │
  │                  │                    │                    │             │
  │                  │                    │ 3. wallet credit   │             │
  │                  │                    │───────────────────>│             │
  │                  │                    │    wallet_tx INSERT│             │
  │                  │                    │<───────────────────│             │
  │                  │                    │                    │             │
  │                  │                    │ ── commit ──────────────────────>│
  │                  │                    │                    │             │
  │  200 OK          │                    │                    │             │
  │<─────────────────│<───────────────────│                    │             │
```

### 3. Refund

```
Admin             Handler             Service              Provider          DB
  │                  │                    │                    │             │
  │ POST /refund     │                    │                    │             │
  │─────────────────>│                    │                    │             │
  │                  │ refund_order()     │                    │             │
  │                  │───────────────────>│                    │             │
  │                  │                    │                    │             │
  │                  │                    │ ┌─ validate ──────────────────┐  │
  │                  │                    │ │ order not paid → 400       │  │
  │                  │                    │ │ refund > paid → 400        │  │
  │                  │                    │ │ version mismatch → 409     │  │
  │                  │                    │ └────────────────────────────┘  │
  │                  │                    │                    │             │
  │                  │                    │ refund()           │             │
  │                  │                    │───────────────────>│             │
  │                  │                    │ RefundResponse     │             │
  │                  │                    │<───────────────────│             │
  │                  │                    │                    │             │
  │                  │                    │ ── atomic transaction ──────────>│
  │                  │                    │                    │             │
  │                  │                    │ 1. payment_orders  │             │
  │                  │                    │    update status   │             │
  │                  │                    │                    │             │
  │                  │                    │ 2. payment_transactions INSERT  │
  │                  │                    │    tx_type=refund  │             │
  │                  │                    │                    │             │
  │                  │                    │ 3. payment_refunds INSERT       │
  │                  │                    │                    │             │
  │                  │                    │ 4. wallet debit    │             │
  │                  │                    │───────────────────>│             │
  │                  │                    │    wallet_tx INSERT│             │
  │                  │                    │<───────────────────│             │
  │                  │                    │                    │             │
  │                  │                    │ ── commit ──────────────────────>│
  │  {refund_id}     │                    │                    │             │
  │<─────────────────│<───────────────────│                    │             │
```

## Connection Point: Payment ↔ Wallet

Payment callback handler does exactly two things that touch wallet:

**On paid (credit):**

```rust
crate::in_transaction!(pool, tx, {
    // 1. payment: update order + record transaction
    tx_update_order_paid(&mut tx, order.id, &callback.provider_tx_id).await?;
    tx_insert_payment_tx(&mut tx, order.id, "charge", order.amount, &callback).await?;

    // 2. wallet: insert credit record
    crate::services::wallet::credit_wallet(
        wallet_repo, &pool,
        order.user_id, &order.currency, order.amount,
        WalletTxType::Recharge,
        &format!("PAY-{}", order.document_id),
        Some(WalletReferenceType::Payment),
        Some(&order.document_id),
        None,
    ).await?;
});
```

**On refund (debit):**

```rust
crate::in_transaction!(pool, tx, {
    // 1. payment: update order + record refund (with version check)
    let order = tx_lock_order_for_update(&mut tx, order.id, order.version).await?; // optimistic lock
    if order.refund_amount + refund_amount > order.amount {
        return Err(AppError::BadRequest("refund exceeds payment amount"));
    }
    tx_insert_payment_refund(&mut tx, order.id, refund_amount, &result).await?;
    tx_insert_payment_tx(&mut tx, order.id, "refund", refund_amount, &result).await?;
    tx_update_order_status(&mut tx, order.id, is_full ? "refunded" : order.status).await?;

    // 2. wallet: insert debit record
    crate::services::wallet::debit_wallet(
        wallet_repo, &pool,
        order.user_id, &order.currency, refund_amount,
        WalletTxType::Refund,
        &format!("REFUND-{}", order.document_id),
        Some(WalletReferenceType::PaymentRefund),
        Some(&order.document_id),
        None,
    ).await?;
});
```

Wallet sees a credit/debit like any other. It doesn't care about payment providers. Payment doesn't touch wallet balance directly — it goes through `wallet::credit_wallet` / `wallet::debit_wallet` which handle optimistic locking, currency validation, frozen checks, and idempotency.

## Enum Definitions

### New enums (payment module)

```rust
define_enum!(
    PaymentProviderName {
        Stripe = "stripe",
        Paypal = "paypal",
        Wxpay = "wxpay",
        Alipay = "alipay",
    }
);

define_enum!(
    PaymentStatus {
        Pending = "pending",
        Paid = "paid",
        Failed = "failed",
        Cancelled = "cancelled",
        Refunded = "refunded",
        Expired = "expired",
    }
);

define_enum!(
    PaymentTxType {
        Charge = "charge",
        Refund = "refund",
    }
);

define_enum!(
    PaymentRefundStatus {
        Pending = "pending",
        Processing = "processing",
        Succeeded = "succeeded",
        Failed = "failed",
    }
);

define_enum!(
    PaymentMethod {
        Card = "card",
        AlipayQr = "alipay_qr",
        AlipayWap = "alipay_wap",
        AlipayPage = "alipay_page",
        WechatJsapi = "wechat_jsapi",
        WechatNative = "wechat_native",
        WechatH5 = "wechat_h5",
        Paypal = "paypal",
    }
);
```

### Existing enum additions

```rust
// Add to WalletReferenceType:
Payment = "payment",
PaymentRefund = "payment_refund",
```

## Provider Trait

```rust
#[async_trait]
pub trait PaymentProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn create(
        &self,
        channel: &PaymentChannel,
        order: &PaymentOrder,
        return_url: Option<&str>,
    ) -> AppResult<ProviderResponse>;

    async fn query(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
    ) -> AppResult<ProviderStatus>;

    async fn cancel(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
    ) -> AppResult<()>;

    async fn refund(
        &self,
        channel: &PaymentChannel,
        provider_order_id: &str,
        amount: i64,
        reason: Option<&str>,
    ) -> AppResult<RefundResponse>;

    async fn verify_callback(
        &self,
        channel: &PaymentChannel,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<CallbackData>;
}

pub struct ProviderResponse {
    pub provider_order_id: String,
    pub redirect_url: Option<String>,
    pub qr_code: Option<String>,
    pub client_secret: Option<String>,
}

pub struct ProviderStatus {
    pub status: PaymentStatus,
    pub provider_tx_id: Option<String>,
    pub paid_at: Option<String>,
}

pub struct RefundResponse {
    pub provider_refund_id: String,
}

pub struct CallbackData {
    pub provider_order_id: String,
    pub status: PaymentStatus,
    pub amount: i64,
    pub provider_tx_id: Option<String>,
    pub paid_at: Option<String>,
}
```

## File Structure

```
src/payment/
  mod.rs              -- pub mod + factory
  provider.rs         -- trait + shared structs
  crypto.rs           -- AES-256-GCM for credentials
  model.rs            -- DB queries (all 4 tables)
  service.rs          -- create_order / handle_callback / refund_order
  handler.rs          -- HTTP handlers
  dto.rs              -- request/response types

  providers/
    mod.rs            -- #[cfg(feature)] re-exports
    stripe.rs         -- impl PaymentProvider
    paypal.rs         -- impl PaymentProvider
    wxpay.rs          -- impl PaymentProvider
    alipay.rs         -- impl PaymentProvider
```

## Feature Gates

```toml
payment-stripe  = ["async-stripe"]
payment-paypal  = []
payment-wxpay   = []
payment-alipay  = []
payment-all     = ["payment-stripe", "payment-paypal", "payment-wxpay", "payment-alipay"]
```

## API

```
# Public (authenticated)
POST   /api/v1/payment/orders                        # Create → redirect_url / qr_code
GET    /api/v1/payment/orders/:id                    # Query
POST   /api/v1/payment/orders/:id/cancel             # Cancel pending
GET    /api/v1/payment/orders/:id/transactions       # List provider events
GET    /api/v1/payment/orders/:id/refunds            # List refunds

# Webhook (no auth, provider signature verified)
POST   /api/v1/payment/callback/:channel_doc_id      # Unified callback

# Admin
GET/POST/PUT/DELETE  /api/v1/admin/payment/channels[/:id]
GET                  /api/v1/admin/payment/orders[/:id]    # List/filter + detail
POST                 /api/v1/admin/payment/orders/:id/refund
GET                  /api/v1/admin/payment/transactions     # All provider events
GET                  /api/v1/admin/payment/refunds          # All refunds
```

## Credential Formats (AES-256-GCM encrypted in DB)

```json
// Stripe
{ "secret_key": "sk_live_...", "publishable_key": "pk_live_..." }

// PayPal
{ "client_id": "...", "client_secret": "...", "sandbox": false }

// WxPay
{ "app_id": "...", "mch_id": "...", "api_key": "...",
  "cert_pem": "base64...", "cert_key": "base64..." }

// Alipay
{ "app_id": "...", "private_key": "...", "alipay_public_key": "...",
  "is_sandbox": false }
```

## Channel Validation

`payment_channels.settings` stores constraints enforced at service layer:

```json
{
  "currencies": ["USD", "EUR"],
  "methods": ["card", "alipay_qr"],
  "min_amount": 100,
  "max_amount": 10000000
}
```

```rust
// service.rs — validate before creating order
fn validate_channel(channel: &PaymentChannel, currency: &str, method: Option<&str>, amount: i64) -> AppResult<()> {
    let settings: ChannelSettings = parse_settings(&channel.settings)?;
    if !settings.currencies.contains(&currency.to_uppercase()) {
        return Err(AppError::BadRequest("currency not supported by this channel"));
    }
    if let Some(m) = method {
        if !settings.methods.contains(&m.to_string()) {
            return Err(AppError::BadRequest("payment method not supported by this channel"));
        }
    }
    if let Some(min) = settings.min_amount {
        if amount < min { return Err(AppError::BadRequest("amount below minimum")); }
    }
    if let Some(max) = settings.max_amount {
        if amount > max { return Err(AppError::BadRequest("amount exceeds maximum")); }
    }
    Ok(())
}
```

## Reconciliation

No dedicated table. A worker runs daily via the existing worker system:

1. Query all `payment_orders` with `status = paid` from the previous day
2. For each, call `provider.query()` to get the latest status from the gateway
3. Compare: status, amount, provider_tx_id
4. Mismatches written to `audit_log` with `action: "payment_reconcile_mismatch"`, including both local and provider data
5. Admin dashboard queries `audit_log` filtered by action to display discrepancies

This is intentionally lightweight — `payment_transactions.raw_payload` already stores every provider event, so the worker only needs to detect drift, not rebuild state.

## Security Checklist

- [x] Credentials AES-256-GCM encrypted at rest (env key `PAYMENT_ENCRYPT_KEY`)
- [x] Webhook signature verification per-provider (HMAC-SHA256 for Stripe/WxPay, RSA2 for Alipay)
- [x] Idempotency key on payment_orders (double-pay prevention)
- [x] Amount verification on callback (provider amount must match order amount)
- [x] Optimistic locking on wallet balance (existing)
- [x] Optimistic locking on payment_orders.version for concurrent refund protection
- [x] Atomic order + payment_tx + wallet_tx updates in single DB transaction
- [x] Raw provider payloads stored immutably in payment_transactions.raw_payload
- [x] payment_transactions.provider_tx_id UNIQUE prevents duplicate webhook processing
- [x] Rate limiting on callback endpoint (existing middleware)
- [x] Partial refund tracking (payment_refunds accumulates, cannot exceed order amount)

## Milestones

| Phase | Scope | Days |
|-------|-------|------|
| **P0** | Schema + trait + model + service + handler | 2 |
| **P1** | Stripe (`async-stripe`) | 2 |
| **P2** | Alipay (RSA2 + QR/Page/WAP) | 4 |
| **P3** | WxPay (HMAC-SHA256 + JSAPI/Native/H5) | 4 |
| **P4** | PayPal (OAuth2 + Orders API) | 3 |
| **P5** | Admin UI + SDK | 3 |

**Total: ~18 days. Stripe usable after P0+P1 (4 days).**

## Future: Plugin-based Payment Providers

Currently all providers are Rust built-in implementations. Future channels (GrabPay, MercadoPago, crypto, etc.) can be added via plugins with minimal core changes:

**What plugins need from the sandbox (3 new Host APIs):**
- `Host.cryptoHmac(algo, key, data)` — HMAC-SHA256/SHA512
- `Host.cryptoRsaSign(algo, key, data)` / `Host.cryptoRsaVerify(...)` — RSA2
- `Host.getSecret(key)` / `Host.setSecret(key, value)` — encrypted KV storage

**Architecture (callback goes through core for atomicity):**
```
Plugin: verify_callback() → CallbackData (signature verify + parse only)
Core:   atomic_confirm_payment() → update order + wallet_tx in one transaction
```

Plugins only handle crypto + data parsing. The atomic transaction (order status + wallet credit) is always handled by core to guarantee consistency. No plugin can bypass this.
