# Payment Routing Strategy

> Status: Draft  
> Last updated: 2026-05-14

## Background

Currently `CreatePaymentOrderRequest` requires the frontend to pass `channel_id`, meaning the frontend must decide which payment channel to use. This design adds automatic channel routing based on client context (country / language / IP), while keeping manual selection as an option.

## Goals

1. **Plan B** — Backend recommends channels via a new endpoint, frontend presents choices to the user
2. **Plan A** — When `channel_id` is not provided, backend auto-selects the best channel
3. Record client context (IP, language, country, user agent) on each `PaymentOrder` for analytics
4. Zero new tables — use existing `payment_channels.settings` JSON for routing metadata

## Channel Settings Convention

Each `PaymentChannel.settings` JSON may include routing fields:

```json
{
  "product_id": "prod_xxx",
  "countries": ["CN", "HK"],
  "currencies": ["CNY", "HKD"],
  "languages": ["zh", "zh-CN"],
  "priority": 100
}
```

| Field | Type | Description |
|---|---|---|
| `product_id` | string | Provider-specific product ID (already used by Creem, Dodo) |
| `countries` | string[] | ISO 3166-1 alpha-2 codes. `["*"]` = global fallback |
| `currencies` | string[] | ISO 4217 currency codes this channel supports |
| `languages` | string[] | BCP 47 language tags (prefix-matched, e.g. `"zh"` matches `"zh-CN"`) |
| `priority` | integer | Higher = preferred. Default 0 if omitted |

**Example configurations:**

```json
// Alipay — China focused
{
  "countries": ["CN"],
  "currencies": ["CNY"],
  "languages": ["zh"],
  "priority": 100
}

// Stripe — Global
{
  "countries": ["US", "GB", "DE", "FR", "JP", "AU"],
  "currencies": ["USD", "EUR", "GBP", "JPY"],
  "languages": ["en", "de", "fr", "ja"],
  "priority": 50
}

// Dodo Payments — Global fallback
{
  "product_id": "prod_yyy",
  "countries": ["*"],
  "currencies": ["USD", "EUR", "GBP"],
  "priority": 1
}
```

## Schema Changes

### `payment_orders` — 4 new columns

| Column | SQLite | PostgreSQL | MySQL |
|---|---|---|---|
| `client_language` | TEXT | TEXT | VARCHAR(50) |
| `client_country` | TEXT | TEXT | VARCHAR(2) |
| `client_user_agent` | TEXT | TEXT | VARCHAR(512) |
| `channel_selected_by` | TEXT | TEXT | VARCHAR(20) |

`channel_selected_by` values:
- `"manual"` — frontend explicitly chose `channel_id`
- `"auto"` — backend routing selected the channel
- `"fallback"` — routing had no match, used first active channel

## New API Endpoints

### `GET /payment/channels/available`

Returns ranked channels matching the client's context.

**Query params:**

| Param | Required | Description |
|---|---|---|
| `order_id` | yes | Order to pay for (used to determine currency) |
| `country` | no | ISO 3166-1 alpha-2. If omitted, inferred from IP or Accept-Language |
| `language` | no | BCP 47 tag. If omitted, read from Accept-Language header |

**Response:**

```json
{
  "success": true,
  "data": {
    "recommended_channel_id": "ch_alipay",
    "channels": [
      {
        "channel_id": "ch_alipay",
        "provider": "alipay",
        "name": "Alipay",
        "is_recommended": true,
        "sort_order": 0
      },
      {
        "channel_id": "ch_wechat",
        "provider": "wechat",
        "name": "WeChat Pay",
        "is_recommended": false,
        "sort_order": 1
      }
    ]
  }
}
```

### `POST /payment/orders` (modified)

`channel_id` becomes **optional**.

- If provided → Plan B (manual), uses specified channel, sets `channel_selected_by = "manual"`
- If not provided → Plan A (auto), backend routes, sets `channel_selected_by = "auto"`

**Request:**

```json
{
  "order_id": "ord_xxx",
  "channel_id": "ch_alipay",
  "country": "CN",
  "language": "zh-CN",
  "return_url": "https://...",
  "metadata": "{}"
}
```

**Response** (unchanged, plus new fields):

```json
{
  "success": true,
  "data": {
    "id": "pay_xxx",
    "channel_id": "ch_alipay",
    "provider": "alipay",
    "client_ip": "120.xxx.xxx.xxx",
    "client_language": "zh-CN",
    "client_country": "CN",
    "client_user_agent": "Mozilla/5.0...",
    "channel_selected_by": "auto",
    "redirect_url": "https://alipay.com/...",
    "..."
  }
}
```

## Routing Algorithm

```
Input:  channels[], currency, country?, language?
Output: RankedChannel[]

1. Filter is_active = 1
2. Currency hard match (channel must support order's currency)
3. Country match:
   - Exact match (countries contains country)  → keep priority
   - Wildcard (countries contains "*")         → priority / 10
   - No match                                  → exclude
4. Language bonus:
   - Exact match    → effective_priority += 50
   - Prefix match   → effective_priority += 25
   - No match       → +0
5. Sort by effective_priority DESC, then sort_order ASC
6. Return ordered list
```

**Example — user from China, CNY:**

| Channel | countries | priority | language match | effective | Rank |
|---|---|---|---|---|---|
| Alipay | ["CN"] | 100 | zh exact | 150 | 1st |
| WeChat | ["CN"] | 90 | zh exact | 140 | 2nd |
| Dodo | ["*"] | 1 | no | 0.1 | 3rd (fallback) |
| Stripe | ["US","GB",...] | 50 | — | excluded | — |

## Files to Change

| File | Change |
|---|---|
| `migrations/sqlite/schema.sqlite.sql` | Add 4 columns to `payment_orders` |
| `migrations/postgres/schema.postgres.sql` | Add 4 columns to `payment_orders` |
| `migrations/mysql/schema.mysql.sql` | Add 4 columns to `payment_orders` |
| `src/payment/routing.rs` | **New** — routing algorithm |
| `src/payment/mod.rs` | Add `pub mod routing` |
| `src/models/payment_order.rs` | Add 4 fields to struct + `impl_from_row` + `insert` |
| `src/dto/payment.rs` | `channel_id` optional + new fields + `AvailableChannelResponse` |
| `src/services/payment.rs` | New `select_channels()`, `list_available_channels()`, modify `create_payment_order` |
| `src/handlers/payment.rs` | New handler, modify create handler to extract language/UA, add route |

## Files NOT Changed

- Provider implementations (dodo/stripe/alipay/wechat/creem) — no impact
- Workers (retry/expire/reconcile) — read `channel_id` as int, unaffected
- Webhook callback flow — no routing involved
- `PaymentChannelRepository` trait — `find_all_active` is sufficient

## Future Enhancements

- **GeoIP** — Add `maxminddb` crate + GeoLite2 database for IP → country inference. Avoids requiring frontend to pass `country`.
- **Amount-based routing** — Some channels have min/max amounts. Add `min_amount` / `max_amount` to settings.
- **Routing analytics** — Track conversion rates per channel/country to auto-tune priority.
- **A/B testing** — Split traffic across channels with similar priority to measure performance.
