<p align="center">
  <h1 align="center">raisfast</h1>
  <p align="center">
    <strong>Rust-powered Headless CMS · Serverless · Desktop</strong>
  </p>
  <p align="center">
    <em>One CMS. Three deployments. Zero compromises.</em>
  </p>
</p>

---

> **⚠️ Early Alpha — Not ready for production use.**
>
> This project is under active development. APIs may change without notice.
> We're open-sourcing early to establish provenance and gather feedback.
> A stable v1.0 release is targeted for Q3 2026.

---

## What is raisfast?

raisfast is a **high-performance Headless CMS and API engine** built entirely in Rust. It runs in three modes from a single codebase:

| Mode | Use Case | Database | Storage |
|------|----------|----------|---------|
| **Desktop** (Tauri) | Personal blogs, local development | SQLite (embedded) | Local filesystem |
| **Serverless** | Team collaboration, zero-ops | PostgreSQL / MySQL / D1 | S3 / R2 |
| **Self-hosted** | Enterprise, full control | SQLite / PostgreSQL / MySQL | Local / S3 |

**No other CMS does all three from one codebase.**

### Why Rust?

| Metric | raisfast (Rust) | Node.js CMS (Strapi/Payload) |
|--------|-----------------|------------------------------|
| Cold start | <5ms | ~500ms |
| Memory usage | <50MB | ~300MB |
| Binary size | ~15MB | ~200MB (node_modules) |
| Single-instance RPS | 50K+ | ~2K |
| Plugin sandbox | ✅ WASM + JS + Lua | JS only |

---

## Features

### Core
- **REST API** — Full CRUD for posts, pages, categories, tags, comments, media
- **Admin SPA** — Modern React dashboard (embedded in binary, zero config)
- **Auth** — JWT (HS256) + refresh tokens + OAuth (GitHub) + SMS login
- **RBAC** — Role-based access control with fine-grained permissions
- **Multi-tenant** — Optional `BUILTIN_TENANTABLE` mode for SaaS

### Content Management
- **Content Type System** — Define custom content types via TOML schemas
- **AOP Aspects** — Timestampable, soft-deletable, ownable, lockable, publishable, orderable, sluggable, and more
- **Block Editor** — Page builder with reusable blocks
- **Media Library** — Upload, thumbnails, dimensions detection
- **RSS/Atom** — Auto-generated feeds
- **Sitemap** — Auto-generated sitemap.xml

### Extensibility
- **Plugin Engine** — Three runtimes: WASM (wasmtime), JavaScript (QuickJS), Lua (mlua)
- **Hook System** — Lifecycle hooks for posts, comments, media, users, and custom events
- **Event Bus** — In-process event system for plugin coordination
- **Custom Cron Jobs** — Plugins can register scheduled tasks

### Infrastructure
- **Multi-database** — Switch between SQLite / PostgreSQL / MySQL with zero code changes
- **SQL Dialect Layer** — Automatic placeholder translation (`?` → `$1`), date functions, UPSERT syntax
- **Job Queue** — Built-in SQLite-backed job queue with retry, dead-letter, and cron scheduling
- **Webhooks** — Subscribe to events via HTTP webhooks
- **Audit Log** — Track all admin actions
- **Rate Limiting** — Configurable per-endpoint rate limits
- **Swagger UI** — Auto-generated OpenAPI documentation

---

## Quick Start

### Prerequisites

- Rust 1.85+ (edition 2024)
- SQLite 3.x (default), or PostgreSQL / MySQL

### Build & Run

```bash
# Clone
git clone https://github.com/snkzhong/raisfast.git
cd raisfast

# Build and run (SQLite, default)
cargo run --features "db-sqlite plugin-all search-tantivy"

# Server starts at http://localhost:9898
# Admin UI at http://localhost:9898/admin
# Swagger at http://localhost:9898/swagger-ui
```

### First run

On first startup, raisfast automatically:
1. Creates all database tables (`schema.sqlite.sql`)
2. Seeds default roles, permissions, and site options
3. Starts serving API + Admin UI

Create an admin user:

```bash
cargo run -- db seed admin@example.com admin your-password
```

### Docker

```bash
docker build -t raisfast .
docker run -p 9898:9898 -v ./data:/app/data raisfast
```

---

## Architecture

```
src/
├── main.rs              # CLI entry point
├── server.rs            # HTTP server + route registration
├── lib.rs               # AppState composition
├── handlers/            # Route handlers (thin: extract → service → respond)
├── services/            # Business logic layer
├── models/              # Data structures + SQL queries
├── repositories/        # Repository pattern (trait + Sqlx impl)
├── middleware/           # Auth, rate limiting, CORS, metrics
├── plugins/             # Plugin engine (WASM/JS/Lua)
├── content_type/        # Dynamic content type system
├── worker/              # Job queue + cron scheduler
├── db/                  # Connection pool, dialect, schema
├── config/              # Environment-based configuration
├── errors/              # Unified AppError (thiserror)
├── storage/             # File storage (local / S3)
├── search/              # Full-text search (Tantivy)
├── oauth/               # OAuth providers
├── protocols/           # AOP protocol definitions
├── aspects/             # AOP aspect engine
└── admin_spa.rs         # Embedded Admin UI (rust-embed)
```

### Layering

```
Handler → Service → Repository → Model (SQL)
                  ↘ External: Storage / Cache / Search / EventBus
```

Handlers contain **no business logic**. Services orchestrate repositories and external services. Models contain only data structures and SQL queries.

---

## Switching Databases

Zero code changes. Just change the feature flag:

```bash
# SQLite (default)
cargo build --features "db-sqlite"

# PostgreSQL
cargo build --features "db-postgres"

# MySQL
cargo build --features "db-mysql"
```

The SQL dialect layer (`src/db/dialect.rs`) handles:
- Placeholder translation: `?` → `$1, $2, ...` (PostgreSQL)
- Time functions: `datetime('now')` → `NOW()`
- Date arithmetic: `datetime('now', '-N days')` → `NOW() - INTERVAL 'N days'`
- UPSERT: `ON CONFLICT ... DO UPDATE` → `ON DUPLICATE KEY UPDATE` (MySQL)
- RETURNING: `RETURNING *` → disabled for MySQL

---

## Plugin System

Plugins can be written in three languages:

```bash
plugins/
├── my-plugin/
│   ├── plugin.toml      # Manifest
│   ├── main.js          # JavaScript (QuickJS)
│   ├── main.lua         # Lua (mlua)
│   └── main.wasm        # WASM (wasmtime)
```

Example `plugin.toml`:

```toml
[plugin]
name = "my-plugin"
version = "0.1.0"
entry = "main.js"

[permissions]
http = ["GET"]
db = ["read"]
hooks = ["post_created", "comment_created"]
```

---

## Configuration

All configuration via environment variables:

```bash
# Database
DATABASE_URL=sqlite:./data/raisfast.db

# Server
PORT=9898
HOST=0.0.0.0

# Auth
JWT_SECRET=your-secret-key
JWT_ACCESS_TTL=900          # 15 minutes
JWT_REFRESH_TTL=604800      # 7 days

# Storage
STORAGE_DRIVER=local         # local | s3
UPLOAD_DIR=./uploads

# Multi-tenant
BUILTIN_TENANTABLE=false     # Enable tenant_id on all tables

# Search
SEARCH_DRIVER=tantivy        # tantivy | noop

# Plugins
PLUGIN_DIR=./plugins
PLUGIN_HOT_RELOAD=true
```

---

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Language | Rust (edition 2024) |
| HTTP Framework | Axum 0.8 |
| Database | SQLx 0.8 (SQLite / PostgreSQL / MySQL) |
| Auth | JWT (HS256) + Argon2 |
| Search | Tantivy |
| Plugin Runtime | wasmtime / rquickjs / mlua |
| Admin UI | React 19 + Vite + shadcn/ui |
| Desktop | Tauri |
| Embedded Assets | rust-embed |

---

## Project Status

| Component | Status |
|-----------|--------|
| Core API | ✅ Working |
| Admin UI | ✅ Working |
| Auth (JWT + OAuth) | ✅ Working |
| Multi-database | ✅ Working |
| Plugin engine (JS/Lua) | ✅ Working |
| Plugin engine (WASM) | ✅ Working |
| Content Type system | ✅ Working |
| Tauri desktop | ✅ Working |
| Job queue + Cron | ✅ Working |
| Serverless adapter | 🔧 In design |
| Redis cache | 🔧 Planned |
| Plugin marketplace | 📋 Planned |
| SDK (JS/Python) | 📋 Planned |

---

## License

raisfast is dual-licensed:

- **Core framework**: [MIT License](LICENSE)
- **Commercial modules** (SaaS hosting, plugin marketplace, enterprise features): [BSL 1.1](LICENSE-COMMERCIAL)

See [LICENSE](LICENSE) for details.

---

## Contributing

We welcome contributions! Please read [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

Note: This project is in early alpha. The API surface may change significantly before v1.0.

---

<p align="center">
  Built with ❤️ and Rust
</p>
