# Order System Design

## Positioning

SaaS order system — supports both digital goods (auto-delivery) and physical goods (shipping). Initially focused on digital products (subscriptions, memberships, credits, content paywalls, license keys). Physical goods shipping ready when needed.

The relationship with payment:

```
orders (business)              payment_orders (payment intent)
  买了什么、多少钱               怎么付的、渠道回调
  1 ──────────────── N          (一次下单可能多次支付尝试)
  │                              │
  └── order_id ──────────────────┘
```

## Database Schema

### orders

```sql
CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,

    user_id INTEGER NOT NULL REFERENCES users(id),
    order_no TEXT NOT NULL UNIQUE,             -- business order number: ORD-20260513-xxxx

    -- pricing (smallest unit)
    subtotal INTEGER NOT NULL DEFAULT 0,       -- sum of items unit_price * quantity
    discount_amount INTEGER NOT NULL DEFAULT 0,
    shipping_amount INTEGER NOT NULL DEFAULT 0,
    total_amount INTEGER NOT NULL CHECK(total_amount >= 0),

    currency TEXT NOT NULL DEFAULT 'USD',

    -- status
    status TEXT NOT NULL DEFAULT 'pending',

    -- buyer info
    buyer_name TEXT,
    buyer_phone TEXT,
    buyer_email TEXT,
    shipping_address TEXT,                    -- JSON: { country, province, city, district, street, zip, name, phone }

    -- shipping (physical goods)
    tracking_no TEXT,
    carrier TEXT,                             -- e.g. "sf_express", "yto", "fedex"

    -- remark
    remark TEXT,
    admin_remark TEXT,

    -- delivery (for digital goods, e.g. license key, download link)
    delivery_data TEXT,                        -- JSON: auto-populated after paid

    -- timestamps
    paid_at TEXT,
    completed_at TEXT,
    cancelled_at TEXT,
    refunding_at TEXT,
    refunded_at TEXT,
    expired_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_orders_user ON orders(user_id);
CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
CREATE INDEX IF NOT EXISTS idx_orders_order_no ON orders(order_no);
```

### products

```sql
CREATE TABLE IF NOT EXISTS products (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,

    category_id INTEGER REFERENCES categories(id),  -- product classification
    title TEXT NOT NULL,
    description TEXT,
    cover_url TEXT,

    -- type & delivery
    product_type TEXT NOT NULL DEFAULT 'custom',  -- virtual_credit/membership/content_paywall/license/download/physical/custom
    fulfillment_type TEXT NOT NULL DEFAULT 'digital', -- digital/physical
    delivery_hook TEXT,                            -- plugin hook name for digital delivery (e.g. "deliver_license")

    -- physical goods
    weight INTEGER,                               -- grams
    shipping_template_id INTEGER,                 -- reference to shipping template (future table)

    -- pricing (smallest unit)
    price INTEGER NOT NULL CHECK(price >= 0),     -- 0 = free
    currency TEXT NOT NULL DEFAULT 'USD',

    -- status
    status TEXT NOT NULL DEFAULT 'draft',          -- draft/active/archived

    -- extensible
    attributes TEXT,                               -- JSON: arbitrary product-specific data (duration, access_level, etc.)

    sort_order INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_products_status ON products(status);
CREATE INDEX IF NOT EXISTS idx_products_type ON products(product_type);
```

### order_items

```sql
CREATE TABLE IF NOT EXISTS order_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,

    order_id INTEGER NOT NULL REFERENCES orders(id),

    -- what was bought (snapshot, immutable once order is paid)
    product_id INTEGER REFERENCES products(id),  -- NULL if product deleted
    title TEXT NOT NULL,                          -- snapshot at purchase time
    description TEXT,

    -- pricing (smallest unit)
    unit_price INTEGER NOT NULL CHECK(unit_price >= 0),
    quantity INTEGER NOT NULL CHECK(quantity > 0),
    subtotal INTEGER NOT NULL,                 -- unit_price * quantity

    -- snapshot
    cover_url TEXT,                            -- product image
    attributes TEXT,                           -- JSON: variant info (color, size, etc.)

    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_order_items_order ON order_items(order_id);
```

## State Machine

```
┌──────────┐
│ pending  │ ← create order
└────┬──┬──┘
     │  │
     │  └──────────────→ cancelled (buyer cancel or expire)
     ▼
┌──────────┐
│   paid   │ ← payment callback confirmed
└────┬──┬──┘
     │  │
     │  └──────────────→ refunding → refunded
     │
     ├──→ completed (digital: auto)
     │
     ▼
┌───────────┐
│  shipped  │ ← admin marks shipped (physical only)
└─────┬─────┘
      │
      ▼
┌───────────┐
│ completed  │ ← buyer confirms receipt (or auto after timeout)
└───────────┘
```

```rust
define_enum!(
    OrderStatus {
        Pending = "pending",
        Paid = "paid",
        Shipped = "shipped",
        Completed = "completed",
        Cancelled = "cancelled",
        Refunding = "refunding",
        Refunded = "refunded",
        Expired = "expired",
    }
);

define_enum!(
    ProductType {
        VirtualCredit = "virtual_credit",
        Membership = "membership",
        ContentPaywall = "content_paywall",
        License = "license",
        Download = "download",
        Physical = "physical",
        Custom = "custom",
    }
);

define_enum!(
    FulfillmentType {
        Digital = "digital",
        Physical = "physical",
    }
);

define_enum!(
    ProductStatus {
        Draft = "draft",
        Active = "active",
        Archived = "archived",
    }
);
```

## Sequence: Order → Payment → Delivery

```
Buyer            Order Service       Payment Service      Provider        Wallet
  │                   │                    │                  │              │
  │ POST /orders      │                    │                  │              │
  │──────────────────>│                    │                  │              │
  │                   │ INSERT order       │                  │              │
  │                   │ status=pending     │                  │              │
  │                   │ INSERT order_items │                  │              │
  │<──────────────────│ {order_id}         │                  │              │
  │                   │                    │                  │              │
  │ POST /payment/orders                   │                  │              │
  │  {order_id, amount, channel}           │                  │              │
  │───────────────────────────────────────>│                  │              │
  │                   │                    │ create payment   │              │
  │                   │                    │─────────────────>│              │
  │  {redirect_url}   │                    │<────────────────│              │
  │<───────────────────────────────────────│                  │              │
  │                   │                    │                  │              │
  │ ... buyer pays on provider page ...    │                  │              │
  │                   │                    │                  │              │
  │                   │                    │  webhook callback│              │
  │                   │                    │<─────────────────│              │
  │                   │                    │                  │              │
  │                   │                    │ ── atomic tx ──────────────────>│
  │                   │                    │ 1. payment_order=paid          │
  │                   │                    │ 2. payment_tx INSERT           │
  │                   │                    │ 3. wallet credit               │
  │                   │                    │ ── commit ─────────────────────>│
  │                   │                    │                  │              │
  │                   │ order paid         │                  │              │
  │                   │<───────────────────│                  │              │
  │                   │ UPDATE orders      │                  │              │
  │                   │ status=paid        │                  │              │
  │                   │ paid_at=now        │                  │              │
  │                   │                    │                  │              │
  │                   │ deliver digital goods (auto-complete) │              │
  │                   │ UPDATE orders      │                  │              │
  │                   │ status=completed   │                  │              │
  │                   │ delivery_data=...  │                  │              │
  │<──────────────────│ {delivery_data}    │                  │              │
  │                   │                    │                  │              │
```

## Order ↔ Payment Connection

Payment callback, after its own atomic transaction, notifies the order system:

```rust
// In payment callback handler, after payment + wallet atomic commit:
if let Some(order_id) = &payment_order.order_id {
    crate::services::order::mark_paid(pool, order_id).await?;
}
```

```rust
// src/services/order.rs
pub async fn mark_paid(pool: &Pool, order_doc_id: &str) -> AppResult<()> {
    let order = model::find_by_doc_id(pool, order_doc_id).await?
        .ok_or_else(|| AppError::not_found("order"))?;

    if order.status != OrderStatus::Pending {
        return Ok(()); // idempotent
    }

    model::update_status(pool, order.id, OrderStatus::Paid, Some("paid_at")).await?;

    // Digital: auto-deliver → auto-complete
    // Physical: stay at paid, wait for admin to ship
    if order.is_digital() {
        deliver_and_complete(pool, order.id).await?;
    }

    Ok(())
}

async fn deliver_and_complete(pool: &Pool, order_id: i64) -> AppResult<()> {
    // Plugin hook: on-order-paid → generate license key / grant access / send download link
    let delivery = crate::services::delivery::process(pool, order_id).await?;

    model::update_delivery(pool, order_id, &delivery).await?;
    model::update_status(pool, order_id, OrderStatus::Completed, Some("completed_at")).await?;
    Ok(())
}
```

Payment does not know order internals. Order does not know which provider was used. They communicate through `payment_orders.order_id → orders.document_id`.

## API

```
# Public (authenticated)
GET    /api/v1/products                            # List active products
GET    /api/v1/products/:id                        # Product detail
POST   /api/v1/orders                              # Create order + items
GET    /api/v1/orders                              # My orders (paginated)
GET    /api/v1/orders/:id                          # Order detail + items
POST   /api/v1/orders/:id/cancel                   # Cancel pending order
POST   /api/v1/orders/:id/confirm                  # Buyer confirms receipt (shipped → completed)

# Admin
GET/POST/PUT/DELETE  /api/v1/admin/products[/:id]  # CRUD products
GET                  /api/v1/admin/orders[/:id]    # All orders (paginated + filter)
POST                 /api/v1/admin/orders/:id/ship  # Mark shipped (physical only)
POST                 /api/v1/admin/orders/:id/cancel
POST                 /api/v1/admin/orders/:id/refund
PUT                  /api/v1/admin/orders/:id/admin-remark
GET                  /api/v1/admin/orders/stats
```

## Digital Delivery

After order is paid, digital goods auto-deliver based on product type:

| Product Type | Delivery Action | delivery_data Example |
|---|---|---|
| virtual_credit | Credit wallet | `{"wallet_tx_id": "..."}` |
| membership | Activate subscription | `{"plan": "premium", "expires_at": "2027-05-13"}` |
| content_paywall | Grant access | `{"content_ids": ["post-123"]}` |
| license | Generate key | `{"license_key": "XXXX-XXXX-XXXX"}` |
| download | Generate signed URL | `{"download_url": "https://..."}` |
| custom | Plugin `delivery_hook` | depends on plugin |

Plugin registers `on-order-paid` hook (or product's `delivery_hook`), receives order data, returns delivery result. Core order system just stores the result in `delivery_data` and marks `completed`.

## File Structure

```
src/order/
  mod.rs              -- pub mod
  model.rs            -- DB queries (products + orders + order_items)
  service.rs          -- product CRUD / create order / cancel / mark_paid / deliver
  handler.rs          -- HTTP handlers
  dto.rs              -- request/response types
```

No provider trait needed — order service is pure business logic, payment integration is the thin bridge layer.
