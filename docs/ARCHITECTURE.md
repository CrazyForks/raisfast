# Architecture

> This document provides a comprehensive overview of the raisfast system architecture for developers and contributors.

---

## Table of Contents

- [Overview](#overview)
- [Technology Stack](#technology-stack)
- [Feature Flags](#feature-flags)
- [Project Structure](#project-structure)
- [Three-Layer Architecture](#three-layer-architecture)
- [Application State](#application-state)
- [Request Lifecycle](#request-lifecycle)
- [Database Layer](#database-layer)
- [Auth System](#auth-system)
- [Plugin Engine](#plugin-engine)
- [Content Type System](#content-type-system)
- [AOP & Protocols](#aop--protocols)
- [Worker & Job Queue](#worker--job-queue)
- [Event System](#event-system)
- [Search Engine](#search-engine)
- [Storage](#storage)
- [Payment](#payment)
- [Multi-Tenancy](#multi-tenancy)
- [Configuration](#configuration)
- [Error Handling](#error-handling)
- [CLI](#cli)
- [Key Design Decisions](#key-design-decisions)

---

## Overview

raisfast is a Rust-powered high-performance BaaS and headless CMS. It compiles to a single binary with zero runtime dependencies, providing blog, ecommerce, wallet, payment, and multi-tenant SaaS capabilities out of the box.

```
                    ┌──────────────────────────────────┐
                    │           raisfast binary         │
                    │  ┌────────┐  ┌────────────────┐  │
    HTTP ─────────► │  │  Axum  │  │  Admin SPA     │  │
    (9898)          │  │ Router │  │  (rust-embed)  │  │
                    │  └───┬────┘  └────────────────┘  │
                    │      │                            │
                    │  ┌───▼────────────────────────┐  │
                    │  │     Middleware Stack         │  │
                    │  │  Auth / CORS / RateLimit /   │  │
                    │  │  Metrics / Locale / AOP      │  │
                    │  └───────────┬─────────────────┘  │
                    │              │                     │
                    │  ┌───────────▼─────────────────┐  │
                    │  │     Handler → Service → Model │  │
                    │  └──┬──────┬──────┬──────┬─────┘  │
                    │     │      │      │      │        │
                    │  ┌──▼──┐┌──▼──┐┌──▼──┐┌──▼───┐   │
                    │  │ DB  ││Cache││Search││Storage│  │
                    │  └─────┘└─────┘└─────┘└──────┘   │
                    │  ┌──────┐ ┌─────────┐ ┌───────┐  │
                    │  │Plugin│ │  Worker  │ │EventBus│ │
                    │  │Engine│ │Job+Queue │ │Pub/Sub │ │
                    │  └──────┘ └─────────┘ └───────┘  │
                    └──────────────────────────────────┘
```

---

## Technology Stack

| Layer | Technology |
|-------|-----------|
| Language | Rust (edition 2024) |
| HTTP Framework | Axum 0.8 |
| Database | SQLx 0.8 (SQLite / PostgreSQL / MySQL) |
| Auth | JWT (HS256) + Argon2 |
| Search | Tantivy |
| Plugin Runtime | wasmtime / rquickjs / mlua / rhai |
| Cache | moka (in-memory, TinyLFU + LRU) |
| Admin UI | React 19 + Vite + shadcn/ui (embedded via rust-embed) |
| Desktop | Tauri (optional) |
| ID Generation | ferroid (Snowflake + multiplicative inverse cipher + base62) |
| CLI | clap |
| Logging | tracing + tracing-subscriber |
| Metrics | Prometheus |
| API Docs | utoipa + Swagger UI |

---

## Feature Flags

Feature flags control which components are compiled. Only the needed features are included.

### Database (pick one)

| Flag | Description |
|------|-------------|
| `db-sqlite` | SQLite backend |
| `db-postgres` | PostgreSQL backend |
| `db-mysql` | MySQL backend |

### Plugin Engines

| Flag | Description |
|------|-------------|
| `plugin-js` | JavaScript (QuickJS) |
| `plugin-lua` | Lua 5.4 (mlua) |
| `plugin-rhai` | Rhai scripting |
| `plugin-wasm` | WebAssembly (wasmtime) |
| `plugin-all` | All four engines |

### Search

| Flag | Description |
|------|-------------|
| `search-tantivy` | Full-text search via Tantivy |

### Storage

| Flag | Description |
|------|-------------|
| `storage-s3` | S3-compatible object storage |

### Payment Providers

| Flag | Description |
|------|-------------|
| `payment-alipay` | Alipay |
| `payment-wechat` | WeChat Pay |
| `payment-stripe` | Stripe |
| `payment-dodo` | Dodo Payments |
| `payment-creem` | Creem |
| `payment-all` | All payment providers |

### Other

| Flag | Description |
|------|-------------|
| `tls` | HTTPS via rustls |
| `openapi` | Swagger UI |
| `proxy` | Multi-tenant reverse proxy |
| `tauri` | Tauri desktop mode |
| `export-types` | TypeScript type generation |

### Example Build

```bash
# Minimal: SQLite + JS plugins + search
cargo build --no-default-features --features "db-sqlite,plugin-js,search-tantivy"

# Full: all databases, all plugins, all payments
cargo build --features "db-sqlite,plugin-all,search-tantivy,payment-all,openapi"
```

---

## Project Structure

```
src/
├── main.rs                 # CLI entry point (clap)
├── lib.rs                  # AppState composition + module declarations
├── server.rs               # HTTP server + route registration
├── app.rs                  # ServiceRegistry (type-safe service locator)
├── cli/                    # CLI sub-commands (server/db/plugin/route/doctor/codegen)
├── config/                 # Environment-based configuration loading
├── handlers/               # Axum route handlers (31 modules)
├── services/               # Business logic layer (28 modules)
├── models/                 # Data structures + SQL queries (35 modules)
├── middleware/              # Auth, rate limiting, CORS, metrics, locale, AOP
├── dto/                    # Request/response DTOs with validation
├── db/                     # Connection pool, SQL dialect, schema, write lock
├── plugins/                # 4-engine plugin system (15 files)
├── content_type/           # Dynamic content type system (7 files)
├── protocols/              # AOP protocol definitions (11 protocols)
├── aspects/                # AOP aspect engine (auto-slug, auto-excerpt, etc.)
├── worker/                 # Job queue + cron scheduler (14 built-in handlers)
├── workflow/               # State machine workflow engine
├── event/                  # Event definitions (~40 event types)
├── eventbus.rs             # tokio::sync::broadcast pub/sub
├── search/                 # Full-text search (Tantivy / noop)
├── storage/                # File storage (local / S3)
├── cache.rs                # In-memory cache (moka)
├── oauth/                  # OAuth providers (GitHub / Google / WeChat)
├── notifier/               # Email + SMS senders (SMTP/SendGrid/Resend/Aliyun/Twilio)
├── payment/                # Payment provider integrations
├── webhook/                # Webhook delivery (HMAC-SHA256)
├── errors/                 # Unified AppError (thiserror + IntoResponse)
├── types/                  # Snowflake ID (ferroid + base62)
├── policy.rs               # Resource-level authorization
├── audit.rs                # Audit logging
├── graphql/                # GraphQL endpoint (async-graphql)
├── proxy/                  # Multi-tenant reverse proxy
├── tauri/                  # Tauri desktop commands
├── admin_spa.rs            # Embedded Admin UI (rust-embed)
├── constants.rs            # Path prefixes and constants
├── macros.rs               # Internal macros
└── utils/                  # Timezone, ID generation utilities
```

---

## Three-Layer Architecture

All business logic follows a strict **Handler → Service → Model** layering:

```
┌─────────────────────────────────────────────────┐
│                   Handler                        │
│  - Extract HTTP params (Path/Query/Json)         │
│  - Authentication check (ensure_authenticated)   │
│  - Input validation                              │
│  - Call service                                  │
│  - Return ApiResponse<T>                         │
└──────────────────────┬──────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────┐
│                   Service                         │
│  - Business logic orchestration                   │
│  - Policy checks (resource ownership)             │
│  - EventBus emission                              │
│  - Cache invalidation                             │
│  - Never calls ensure_* (auth is handler-only)    │
└──────────────────────┬──────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────┐
│                   Model                           │
│  - Data structures (DTOs / DB rows)               │
│  - SQL queries (sqlx + CRUD macros)               │
│  - Transaction support (tx_* variants)            │
│  - No business logic                              │
└──────────────────────────────────────────────────┘
```

**Rules:**

- **Handler** is the only layer that calls `ensure_authenticated()` / `ensure_admin()`
- **Service** does policy checks (resource ownership) but never auth
- **Model** is pure data + SQL, no business logic
- Dependencies flow downward only: Handler → Service → Model

---

## Application State

`AppState` is the global shared state injected into all Axum handlers via `State`:

```rust
pub struct AppState {
    pub pool: Pool,                            // DB connection pool
    pub config: Arc<AppConfig>,                // Application configuration
    pub jwt_decoding_key: DecodingKey,         // JWT verification key
    pub plugins: Arc<PluginManager>,           // Plugin engine manager
    pub eventbus: EventBus,                    // Pub/sub event bus
    pub search: Arc<dyn SearchEngine>,         // Full-text search
    pub storage: Arc<dyn Storage>,             // File storage (local/S3)
    pub cache: Arc<dyn CacheStore>,            // In-memory cache
    pub content_type_registry: Arc<ContentTypeRegistry>,  // Dynamic CT registry
    pub aspect_engine: Arc<AspectEngine>,      // AOP aspect engine
    pub protocol_registry: Arc<ProtocolRegistry>,  // Protocol declarations
    pub options: Arc<OptionsService>,          // Key-value options
    pub rbac: Arc<RbacService>,                // RBAC permission service
    pub tenant: Arc<TenantService>,            // Multi-tenant service
    pub audit: Arc<AuditService>,              // Audit logging
    pub webhook: Arc<WebhookService>,          // Webhook delivery
    pub workflow: Arc<WorkflowService>,        // Workflow engine
    pub oauth_registry: Arc<OAuthProviderRegistry>,  // OAuth providers
    pub email_sender: Arc<dyn EmailSender>,    // Email sender
    pub sms_sender: Arc<dyn SmsSender>,        // SMS sender
    pub services: ServiceRegistry,             // Type-safe service locator
    // ... domain services (post, page, category, tag, comment,
    //     user, wallet, product, order, cart, payment, etc.)
}
```

Built by `build_app_state()` which initializes all components in dependency order, spawns background subscribers (audit, webhook), and wires the service registry.

---

## Request Lifecycle

```
HTTP Request
    │
    ▼
┌──────────────────────┐
│  Security Headers     │  X-Content-Type-Options, X-Frame-Options
├──────────────────────┤
│  CORS Layer           │  Configurable origins
├──────────────────────┤
│  TraceLayer           │  Request/response logging (tracing)
├──────────────────────┤
│  Request ID           │  Unique ID per request
├──────────────────────┤
│  Metrics              │  Prometheus counters
├──────────────────────┤
│  Locale Detection     │  Accept-Language / query param
├──────────────────────┤
│  Rate Limiter         │  IP-based sliding window
├──────────────────────┤
│  AOP HTTP Layer       │  Before/After request aspects
├──────────────────────┤
│  Auth Middleware       │  JWT / API Token resolution
├──────────────────────┤
│  Handler              │  Extract → validate → service → respond
└──────────────────────┘
    │
    ▼
JSON Response: { "code": 0, "message": "ok", "data": { ... } }
```

---

## Database Layer

### Multi-Database Support

Zero code changes to switch between SQLite, PostgreSQL, and MySQL. SQL dialect differences are abstracted via `DbDriver` trait:

```rust
trait DbDriver {
    fn now_fn(&self) -> &str;       // datetime('now') / NOW() / NOW()
    fn placeholder(&self, n: usize) -> String;  // $1 / ? / ?
    // ...
}
```

### Write Lock (SQLite)

All write transactions go through `acquire_write()` — a tokio Mutex that serializes SQLite writes to eliminate `SQLITE_BUSY` errors:

```rust
// All mutations use this pattern:
in_transaction!(pool, |tx| {
    // ... SQL operations ...
})
```

### CRUD Macro System

All database operations use the `raisfast-derive` macro DSL:

| Macro | Purpose |
|-------|---------|
| `crud_insert!` | INSERT with auto-generated columns |
| `crud_update!` | UPDATE with dynamic SET clauses |
| `crud_delete!` | DELETE with WHERE conditions |
| `crud_find!` | SELECT with dynamic WHERE + pagination |
| `crud_find_one!` | SELECT single row |
| `crud_find_all!` | SELECT all matching rows |
| `crud_find_page!` | Paginated SELECT |
| `crud_join_paged!` | Paginated SELECT with JOINs |
| `crud_resolve_id!` | Resolve encoded ID to internal ID |
| `crud_resolve_ids!` | Batch ID resolution |
| `in_transaction!` | Transaction wrapper (auto write lock) |

### Schema

- All timestamps stored as `TEXT` in ISO 8601 format
- Primary keys: Snowflake ID with multiplicative inverse cipher + base62 encoding
- Schema defined in `src/db/schema.rs`, auto-created on first run

---

## Auth System

### JWT Authentication

```
┌──────────┐    POST /auth/login     ┌──────────┐
│  Client  │ ──────────────────────► │  Server  │
│          │ ◄────────────────────── │          │
│          │  { access_token,        │          │
│          │    refresh_token,       │          │
│          │    expires_in }         │          │
└──────────┘                         └──────────┘
```

- **Access token**: JWT (HS256), default 15 minutes
- **Refresh token**: Stored in DB, default 7 days
- **API Token**: Long-lived, scope-based permissions

### Auth Flow in Handlers

```rust
// AuthUser is extracted from JWT or API Token + X-Tenant-ID header
async fn my_handler(auth: AuthUser, State(state): State<AppState>) -> AppResult<...> {
    auth.ensure_authenticated()?;  // Reject 401 if anonymous
    auth.ensure_admin()?;         // Reject 403 if not admin
    // ... business logic
}
```

### OAuth Providers

| Provider | Flow |
|----------|------|
| GitHub | OAuth2 + PKCE |
| Google | OpenID Connect + PKCE |
| WeChat | QR code / MP |

### RBAC

Four built-in roles: `admin`, `editor`, `author`, `reader`. Each role has fine-grained permissions (action + subject + fields + conditions).

---

## Plugin Engine

### Architecture

```
┌─────────────────────────────────────────────┐
│              PluginManager                   │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐       │
│  │ JS Pool │ │ Lua Pool│ │Rhai Pool│  ...   │
│  └────┬────┘ └────┬────┘ └────┬────┘       │
│       └───────────┼───────────┘             │
│              Host Functions                  │
│  ┌───────────────────────────────────────┐  │
│  │  DB / HTTP / FS / Cache / Config      │  │
│  │  (all permission-gated)               │  │
│  └───────────────────────────────────────┘  │
│                                              │
│  Hook System: Filter / Action / RenderOverride│
│  Route System: Declarative HTTP routes       │
│  Cron System: Plugin-defined cron schedules  │
│  VFS: Isolated virtual filesystem per plugin │
│  Health: Auto-disable after 5 consecutive errors│
│  Metrics: Call count, errors, duration        │
└─────────────────────────────────────────────┘
```

### Plugin Manifest

```toml
[plugin]
name = "my-plugin"
version = "0.1.0"
entry = "main.js"          # .js / .lua / .rhai / .wasm
runtime = "js"

[permissions]
http = ["GET", "POST"]
db = ["read", "write"]
filesystem = ["read"]
hooks = ["post_created", "comment_created"]

[[hooks]]
event = "post_created"
priority = 10

[[routes]]
method = "GET"
path = "/my-plugin/hello"

[[cron]]
label = "cleanup"
cron_expr = "0 */6 * * *"
```

### Hook Types

| Type | Behavior |
|------|----------|
| **Filter** | Chain: each hook transforms data, passes to next |
| **Action** | Sequential: fire-and-forget side effects |
| **RenderOverride** | Override response rendering |

### Hot Reload

File watcher (debounced) monitors the plugin directory. Changed plugins are automatically reloaded without server restart.

---

## Content Type System

Dynamic schema definition via TOML files, generating CRUD API automatically.

```toml
# extensions/content_types/portfolio.toml
[content_type]
name = "Portfolio"
singular = "portfolio"
plural = "portfolios"
table = "portfolios"
kind = "collection"
description = "Portfolio items"

[[fields]]
name = "title"
field_type = "text"
required = true

[[fields]]
name = "image"
field_type = "media"
media_config = { accept = ["image/*"], max_count = 1 }

[[fields]]
name = "category"
field_type = "relation"
relation = { relation_type = "many_to_one", target = "category" }

[content_type.implements]
protocols = ["timestampable", "soft_deletable", "sortable"]
```

### Generated API

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/cms/portfolios` | List (paginated, filterable) |
| `GET` | `/cms/portfolios/:id` | Get single |
| `POST` | `/cms/portfolios` | Create |
| `PUT` | `/cms/portfolios/:id` | Update |
| `DELETE` | `/cms/portfolios/:id` | Delete |

### API Access Control

Each content type can configure per-endpoint access:

```toml
[content_type.api.list]
access = "public"       # none / public / member / admin
cache = true
fields = ["title", "image"]

[content_type.api.create]
access = "admin"
```

### Rule Engine

Dynamic data filtering via expressions:

- `@request.auth.id` — current user ID
- `@request.body.*` — request body fields
- `@now` — current timestamp
- Compiles to SQL WHERE clauses

---

## AOP & Protocols

### Protocol System

Protocols are composable, declarative behaviors that can be mixed into any content type:

| Protocol | Adds | Behavior |
|----------|------|----------|
| `timestampable` | `created_at`, `updated_at` | Auto-managed timestamps |
| `ownable` | `created_by`, `updated_by` | Auto-set from auth user |
| `soft_deletable` | `deleted_at` | Logical delete, filter `IS NULL` |
| `versionable` | Revision tracking | Snapshot before update |
| `tenantable` | `tenant_id` | Multi-tenant isolation |
| `lockable` | `lock_version` | Optimistic locking |
| `sortable` | `sort_order` | Default ordering |
| `statusable` | Status field | State machine with allowed transitions |
| `expirable` | `expires_at` | Expiration dates |
| `nestable` | `parent_id` | Parent-child hierarchy |
| `metaable` | `metadata` (JSON) | Arbitrary metadata |

### AOP Layers

```
┌──────────────────────────────────┐
│  HTTP Layer    — Before/After    │  Request/response interception
│  Access Layer  — Check/Filter    │  Route/data-level permissions
│  Data Layer    — Before/After    │  Pre/post CRUD hooks
│  Event Layer   — Consume         │  Event interception
└──────────────────────────────────┘
```

### Built-in Aspects

- **Auto-slug**: Generate URL-safe slug from title field
- **Auto-excerpt**: Generate excerpt from content body
- Protocol-generated aspects run automatically when content type declares `implements`

---

## Worker & Job Queue

### Architecture

```
┌────────────┐     ┌───────────┐     ┌──────────────┐
│  EventBus  │────►│ JobEnqueuer│────►│  Job Queue   │
│  (pub/sub) │     │ (mapper)  │     │  (SQLite)    │
└────────────┘     └───────────┘     └──────┬───────┘
                                              │
┌────────────┐     ┌───────────┐     ┌──────▼───────┐
│    Cron    │────►│Scheduler  │     │WorkerRunner  │
│  Scheduler │     │(poll DB)  │     │(poll queue)  │
└────────────┘     └───────────┘     └──────┬───────┘
                                              │
                                     ┌────────▼────────┐
                                     │ JobHandlerRegistry│
                                     │ (14 built-in +   │
                                     │  plugin cron)    │
                                     └─────────────────┘
```

### Built-in Job Handlers

| Handler | Trigger |
|---------|---------|
| `SendWelcomeEmail` | User registered |
| `GenerateThumbnail` | Media uploaded |
| `ScheduledPublish` | Post/Page publish time reached |
| `WebhookNotify` | Event emitted |
| `RebuildSearchIndex` | Content changed |
| `InvalidateCache` | Content changed |
| `GenerateSitemap` | Cron (every 6h) |
| `SendPasswordResetEmail` | Password reset requested |
| `SendSmsCode` | SMS verification |
| `SendEmailVerification` | Email verification |
| `ExpirePaymentOrders` | Cron (every 5min) |
| `RetryPaymentCallback` | Payment callback failed |
| `ReconcilePayments` | Cron (daily 4am) |
| `ProcessWalletOutbox` | Cron (every 10min) |

### Job Lifecycle

```
Pending → Running → Completed
                  → Failed (retry with exponential backoff + jitter)
                  → Dead (max attempts exceeded)
```

---

## Event System

### EventBus

`tokio::sync::broadcast`-based pub/sub with 256 capacity. Used to decouple business logic:

```
Service → eventbus.emit(Event::PostCreated { ... })
              │
              ├──► AuditService (log event)
              ├──► WebhookService (deliver to subscribers)
              ├──► JobEnqueuer (enqueue side-effect jobs)
              ├──► PluginManager (dispatch hooks)
              └──► SearchEngine (update index)
```

### Event Types (~40)

| Category | Events |
|----------|--------|
| Post | Creating, Created, Updating, Updated, Deleted |
| Comment | Created, Updated, Deleted |
| Page | Created, Updated, Deleted |
| Product | Created, Updated, Deleted |
| Order | Created, Paid, Shipped, Completed, Cancelled |
| Payment | Created, Paid, Refunded |
| Wallet | Credited, Debited |
| User | Registered, LoggedIn |
| Media | Uploaded, Deleted |
| CMS | Generic content CRUD |
| Utility | RenderMarkdown, FilterHtml, OnLogin, CronTick |

---

## Search Engine

`SearchEngine` trait with two implementations:

| Implementation | Description |
|----------------|-------------|
| `TantivyEngine` | Full-text search with keyword highlighting |
| `NoopSearchEngine` | No-op fallback |

**Features:**
- Chinese-aware tokenization
- Keyword highlighting (`<em>` tags)
- Auto-excerpt generation around matched keywords
- Background index rebuild via worker jobs

---

## Storage

`Storage` trait with two implementations:

| Implementation | Feature Flag | Description |
|----------------|-------------|-------------|
| `LocalStorage` | (default) | Filesystem storage under `upload_dir` |
| `S3Storage` | `storage-s3` | S3-compatible object storage with presigned URLs |

**Operations:** `put()`, `get()`, `delete()`, `url()`, `presigned_upload()`

---

## Payment

### Architecture

```
┌──────────┐  POST /payment/orders  ┌──────────────┐
│  Client  │ ─────────────────────► │ PaymentService│
│          │ ◄───────────────────── │              │
│          │  { payment_url }       │  ┌────────┐  │
│          │                        │  │ Router │  │
│          │  Callback (async)      │  │  Alipay│  │
│          │ ◄───────────────────── │  │ Stripe │  │
│          │                        │  │ WeChat │  │
│          │                        │  │ Dodo   │  │
│          │                        │  │ Creem  │  │
│          │                        │  └────────┘  │
└──────────┘                        └──────────────┘
```

### Providers

| Provider | Feature Flag | Description |
|----------|-------------|-------------|
| Alipay | `payment-alipay` | Alipay web/mobile payment |
| WeChat Pay | `payment-wechat` | WeChat Pay (JSAPI/Native) |
| Stripe | `payment-stripe` | Stripe Checkout |
| Dodo | `payment-dodo` | Dodo Payments |
| Creem | `payment-creem` | Creem |
| All | `payment-all` | Enable all providers |

### Payment Flow

1. Client creates payment order → get payment URL
2. User pays via provider
3. Provider sends async callback → verify signature → update order status
4. EventBus emits `PaymentPaid` → triggers webhook, wallet operations
5. Failed callbacks retried via worker jobs, reconciled daily

---

## Multi-Tenancy

Optional tenant isolation for SaaS deployments:

- **Header-based**: `X-Tenant-ID` header resolves tenant
- **Domain-based**: Reverse proxy (feature `proxy`) routes by domain
- **Data isolation**: `tenant_id` column on all tenantable tables
- **Protocol**: `tenantable` protocol auto-injects tenant filtering

### Tenant Resolution Order

1. `X-Tenant-ID` header (if `builtin_tenantable` enabled)
2. Domain matching (if `proxy` feature enabled)
3. Default tenant (built-in)

---

## Configuration

All configuration via environment variables or `.env` file:

### Core

| Variable | Default | Description |
|----------|---------|-------------|
| `HOST` | `0.0.0.0` | Listen address |
| `PORT` | `9898` | Listen port |
| `DATABASE_URL` | — | Database connection string |
| `JWT_SECRET` | — | JWT signing key (required in production) |
| `APP_KEY` | auto | 32-byte base64 key for encryption |

### Built-in Modules

| Variable | Default | Description |
|----------|---------|-------------|
| `BUILTIN_BLOG` | `true` | Enable blog module |
| `BUILTIN_PAGES` | `true` | Enable pages module |
| `BUILTIN_MEDIA` | `true` | Enable media module |
| `BUILTIN_ECOMMERCE` | `true` | Enable ecommerce module |
| `BUILTIN_PAYMENT` | `true` | Enable payment module |
| `BUILTIN_WALLET` | `true` | Enable wallet module |
| `BUILTIN_WORKFLOW` | `true` | Enable workflow module |

### Plugin

| Variable | Default | Description |
|----------|---------|-------------|
| `PLUGIN_DIR` | `./plugins` | Plugin directory |
| `PLUGIN_HOT_RELOAD` | `true` | Auto-reload on file change |
| `PLUGIN_MAX_MEMORY_MB` | `32` | Per-plugin memory limit |
| `PLUGIN_DEFAULT_TIMEOUT_MS` | `5000` | Execution timeout |

### Worker

| Variable | Default | Description |
|----------|---------|-------------|
| `WORKER_ENABLED` | `true` | Enable background worker |
| `WORKER_CONCURRENCY` | `2` | Concurrent job workers |
| `WORKER_POLL_INTERVAL_MS` | `500` | Queue poll interval |

### Rate Limiting

| Variable | Default | Description |
|----------|---------|-------------|
| `RATE_LIMIT_ENABLED` | `true` | Enable rate limiting |
| `RATE_LIMIT_GLOBAL_MAX` | `100` | Global requests per window |
| `RATE_LIMIT_REGISTER_MAX` | `5` | Register requests per window |
| `RATE_LIMIT_LOGIN_MAX` | `10` | Login requests per window |

### Storage

| Variable | Default | Description |
|----------|---------|-------------|
| `STORAGE_DRIVER` | `local` | `local` or `s3` |
| `UPLOAD_DIR` | `./storage/uploads` | Upload directory |
| `MAX_UPLOAD_SIZE` | `104857600` | Max file size (100MB) |

### Email

| Variable | Default | Description |
|----------|---------|-------------|
| `EMAIL_PROVIDER` | `log` | `log`/`smtp`/`sendgrid`/`resend`/`aliyun`/`tencent` |
| `EMAIL_FROM` | — | Sender address |

### SMS

| Variable | Default | Description |
|----------|---------|-------------|
| `SMS_PROVIDER` | `log` | `log`/`aliyun`/`twilio` |

---

## Error Handling

### Unified Error Type

```rust
enum AppError {
    BadRequest(String),          // 400
    Unauthorized(String),        // 401
    Forbidden(String),           // 403
    NotFound(String),            // 404
    MethodNotAllowed,            // 405
    Conflict(String),            // 409
    PayloadTooLarge,             // 413
    TooManyRequests,             // 429
    Internal(anyhow::Error),     // 500
    ServiceUnavailable(String),  // 503
}
```

### Unified Response Format

```json
{
  "code": 0,
  "message": "ok",
  "data": { ... }
}
```

Paginated responses:

```json
{
  "code": 0,
  "message": "ok",
  "data": {
    "items": [...],
    "total": 100,
    "page": 1,
    "page_size": 20
  }
}
```

### Rules

- `AppError` (thiserror) at handler boundaries
- `anyhow` for internal service propagation
- No `unwrap()` / `expect()` in non-test code
- Auto-maps `sqlx::Error` (RowNotFound → 404, UNIQUE violation → 409)
- i18n error messages via `rust_i18n`

---

## CLI

```bash
raisfast <command> [options]

# Commands:
raisfast server start        # Start HTTP server
raisfast server stop         # Stop server
raisfast server restart      # Restart server
raisfast db migrate          # Run database migrations
raisfast db rollback         # Rollback migration
raisfast db backup           # Backup database
raisfast db seed <email> <username> <password>  # Seed admin user
raisfast plugin list         # List loaded plugins
raisfast plugin reload <id>  # Reload plugin
raisfast route list          # List all routes
raisfast ct list             # List content types
raisfast ct create <name>    # Create content type
raisfast doctor              # System diagnostics
raisfast codegen             # Code generation
```

---

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **Single binary** | Zero deployment friction, no runtime dependencies |
| **`unsafe` banned** | `#![deny(unsafe_code)]` — memory safety guaranteed |
| **No `unwrap()` in prod** | All errors propagated via `?` or explicit handling |
| **Write lock for SQLite** | Serialize writes via tokio Mutex to eliminate `SQLITE_BUSY` |
| **Snowflake ID + cipher** | Globally unique, time-sortable, non-sequential (security) |
| **Handler-only auth** | `ensure_*` calls only in handlers, services do policy only |
| **Feature flags** | Compile only what you need, minimal binary size |
| **Embedded admin** | `rust-embed` serves SPA from binary, no separate deployment |
| **EventBus decoupling** | Services emit events, subscribers handle side effects |
| **AOP protocols** | Composable behaviors, no code duplication across content types |
| **Plugin VFS** | Isolated filesystem per plugin with size limits |
| **moka cache** | High-performance concurrent cache, no external dependency |
