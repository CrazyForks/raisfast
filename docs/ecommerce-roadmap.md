# 电商系统现状与路线图

> 最后更新：2026-05-18

## 一、已实现能力

### 数据表（14 张）

| 表 | 说明 | 状态 |
|---|------|------|
| `products` | 商品主表（含变体标记、库存、成本价、售价） | ✅ |
| `product_variants` | SKU/变体（独立价格、库存、属性） | ✅ |
| `categories` | 分类 | ✅ |
| `cart_items` | 购物车（支持变体级唯一） | ✅ |
| `orders` | 订单（含税、优惠券ID、地址ID） | ✅ |
| `order_items` | 订单明细（含变体、SKU、税额） | ✅ |
| `user_addresses` | 地址簿（收货/账单、默认地址） | ✅ |
| `payment_channels` | 支付渠道配置 | ✅ |
| `payment_orders` | 支付单 | ✅ |
| `payment_transactions` | 支付流水 | ✅ |
| `payment_refunds` | 退款单 | ✅ |
| `wallets` | 钱包 | ✅ |
| `wallet_transactions` | 钱包流水 | ✅ |
| `wallet_outbox` | 钱包事件发件箱 | ✅ |

### 功能覆盖

| 能力 | 说明 |
|------|------|
| 商品管理 | 多类型（custom/download）、多规格变体、分类、排序、上下架 |
| 购物车 | 变体级加购、数量管理、清空、一键结算 |
| 订单流程 | 完整生命周期（pending → paid → shipped → completed / refunded） |
| 支付 | 多渠道、钱包余额、第三方支付、退款 |
| 地址簿 | 收货/账单地址、默认地址、租户隔离 |

---

## 二、覆盖度评估

### 适用场景

- ✅ **虚拟商品电商**（充值卡、课程、数字下载）
- ✅ **小型 B2C**（简单实物，固定运费，无促销）
- ⚠️ **中型 B2C**（服装、3C）— 核心链路有，运营能力缺

### 关键缺失

| 缺失 | 影响程度 | 说明 |
|------|---------|------|
| **运费模板** | 🔴 高 | `orders.shipping_amount` 只是字段，无按重量/地区/件数计算的运费模板 |
| **优惠券/促销** | 🔴 高 | `orders.coupon_id` 有字段无表，无法满减、折扣码、限时促销 |
| **发货/物流** | 🔴 高 | `orders.tracking_no` 只是文本，无多包裹发货、物流商对接、物流追踪 |
| **退货/换货** | 🔴 高 | 退款有，退货流程无（买家寄回→商家验货→退款/换货） |
| **库存锁定** | 🔴 高 | `stock` 只是数字，下单时不预扣/锁定，并发超卖风险 |
| **评价系统** | 🟡 中 | 无商品评分/评价/追评，影响 SEO 和转化率 |
| **商品多分类** | 🟡 中 | products 只有单个 `category_id`，无法一品多类 |
| **商品图片库** | 🟡 中 | `image_ids` 是 JSON 字符串，无独立图片表（排序、裁剪、标签） |
| **税率配置** | 🟢 低 | `tax_amount` 有字段但无税率配置表，无法按地区/品类自动计算 |
| **秒杀/拼团** | 🟢 低 | 有 `min_purchase`/`max_purchase` 字段但无活动/库存抢占机制 |
| **供应商** | 🟢 低 | 无供应商管理，无法多供应商入驻 |
| **售后工单** | 🟢 低 | 无售后沟通记录，退款/退货缺乏买卖家交互流程 |

---

## 三、待建表规划

### Phase 2：核心运营能力（优先级 🔴）

#### 3.1 运费模板

```sql
-- 运费模板
shipping_methods (
    id, document_id, tenant_id,
    name,                -- "顺丰标快"
    provider,            -- "sf" / "yto" / "sto" / "custom"
    type,                -- "by_weight" / "by_piece" / "fixed" / "free"
    base_fee,            -- 首重/首件费用（分）
    base_unit,           -- 首重克数 / 首件数
    additional_fee,      -- 续重/续件费用（分）
    additional_unit,     -- 续重克数 / 续件数
    free_threshold,      -- 满额包邮（分），NULL 表示不包邮
    is_active,
    sort_order,
    created_at, updated_at
)

-- 运费区域规则（覆盖偏远地区加价等）
shipping_rates (
    id, document_id, tenant_id,
    shipping_method_id,
    region_type,         -- "province" / "city" / "district"
    region_code,         -- "XZ" / "540000"
    base_fee,            -- 覆盖模板默认值
    additional_fee,
    created_at
)
```

#### 3.2 优惠券

```sql
coupons (
    id, document_id, tenant_id,
    code,                -- "SUMMER2026"
    title,               -- "夏季满减"
    type,                -- "percent" / "fixed" / "shipping_free"
    value,               -- 折扣百分比 / 固定金额（分）
    min_order_amount,    -- 最低订单金额（分）
    max_discount,        -- 最大优惠上限（分），用于 percent 类型
    total_count,         -- 发放总量
    used_count,          -- 已使用量
    per_user_limit,      -- 每人限领
    starts_at, expires_at,
    is_active,
    created_at, updated_at
)

coupon_usages (
    id, tenant_id,
    coupon_id, user_id, order_id,
    used_at
)
```

#### 3.3 发货/物流

```sql
shipments (
    id, document_id, tenant_id,
    order_id,
    tracking_no,
    carrier,             -- "sf" / "yto" / "jd"
    status,              -- "pending" / "shipped" / "delivered"
    shipped_at, delivered_at,
    created_at, updated_at
)

shipment_items (
    id, shipment_id, order_item_id,
    quantity
)

-- 物流轨迹（可选，对接快递100等）
shipment_tracks (
    id, shipment_id,
    location, description,
    tracked_at
)
```

#### 3.4 退货/换货

```sql
order_returns (
    id, document_id, tenant_id,
    order_id, user_id,
    type,                -- "refund" / "exchange" / "refund_and_return"
    reason,              -- "defective" / "wrong_item" / "not_as_described" / "other"
    description,
    status,              -- "requested" / "approved" / "rejected" / "shipping_back" / "received" / "completed" / "cancelled"
    refund_amount,       -- 退款金额（分）
    tracking_no,         -- 买家退货物流单号
    carrier,
    admin_remark,
    requested_at, approved_at, received_at, completed_at,
    created_at, updated_at
)

order_return_items (
    id, order_return_id, order_item_id,
    quantity,
    reason
)

order_return_photos (
    id, order_return_id,
    url, sort_order
)
```

#### 3.5 库存锁定

不需要新表，在现有流程中增加：

- `product_variants.stock` 下单时预扣（`stock - locked`）
- 新增 `stock_locked` 字段到 `product_variants` 和 `products`
- 订单取消/超时时释放锁定
- 订单完成时确认扣减

```sql
-- products 和 product_variants 各加两列
ALTER TABLE products ADD COLUMN stock_locked INTEGER NOT NULL DEFAULT 0;
ALTER TABLE product_variants ADD COLUMN stock_locked INTEGER NOT NULL DEFAULT 0;
-- 可用库存 = stock - stock_locked
```

### Phase 3：增强运营能力（优先级 🟡）

#### 3.6 评价系统

```sql
product_reviews (
    id, document_id, tenant_id,
    product_id, variant_id, user_id, order_item_id,
    rating,              -- 1-5
    title,
    content,
    images,              -- JSON array of URLs
    is_anonymous,
    status,              -- "pending" / "approved" / "rejected" / "hidden"
    admin_reply,
    replied_at,
    created_at, updated_at
)
```

#### 3.7 商品多分类

```sql
-- junction table
product_category_junction (
    product_id,
    category_id,
    sort_order,
    PRIMARY KEY (product_id, category_id)
)
```

#### 3.8 商品图片库

```sql
product_images (
    id, document_id, tenant_id,
    product_id,
    url,
    alt,
    sort_order,
    is_cover,
    created_at
)
```

#### 3.9 税率配置

```sql
tax_rates (
    id, document_id, tenant_id,
    name,                -- "中国增值税"
    rate,                -- 0.13 (百分比，存 TEXT 精度)
    region_code,         -- 适用地区
    product_type,        -- 适用商品类型，NULL 表示全部
    is_active,
    created_at, updated_at
)
```

### Phase 4：营销与高级能力（优先级 🟢）

- 秒杀/限时购（`flash_sales` + `flash_sale_items`）
- 拼团（`group_buys` + `group_buy_participants`）
- 供应商入驻（`suppliers` + 供应商商品关联）
- 售后工单（`support_tickets` + `support_messages`）
- 商品收藏/浏览历史
- 积分系统

---

## 四、实施建议

### 优先级排序

```
Phase 2.5 库存锁定 → Phase 2.1 运费模板 → Phase 2.2 优惠券
→ Phase 2.3 发货物流 → Phase 2.4 退货换货 → Phase 3 按需
```

### 原则

1. **只建表不写死逻辑** — 表结构先行，业务逻辑渐进
2. **所有金额存分** — `INTEGER`，展示时 / 100
3. **无 ON DELETE CASCADE** — 应用层处理级联
4. **decimal 存 TEXT** — SQLite 下保持精度
5. **variant_id 可选** — 无变体商品 `variant_id = NULL`，有变体商品必须指定
6. **三套 schema 同步** — SQLite / PG / MySQL 结构对齐
