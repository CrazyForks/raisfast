# raisfast Roadmap

## Current Status: Early Alpha

Core API and architecture are stable. Completing production-readiness features before v1.0.

---

## Q2 2026 — Completeness

### Infrastructure

- [ ] Serverless adapters (AWS Lambda / Vercel / Cloudflare Workers)
- [ ] Redis cache store implementation (`cache-redis` feature)
- [ ] Redis rate limit store implementation (`rate-limit-redis` feature)
- [ ] External search engine support (Meilisearch / OpenSearch)
- [ ] Docker image optimization (multi-stage, <20MB)

### Performance

- [ ] Benchmark suite (k6 / wrk) with published results
- [ ] Cold start optimization for serverless (<100ms)
- [ ] Connection pooling tuning guide

### Developer Experience

- [ ] `create-raisfast` CLI scaffolding tool
- [ ] JavaScript/TypeScript SDK
- [ ] Python SDK
- [ ] API rate limiting per-role configuration
- [ ] Database migration CLI improvements

---

## Q3 2026 — v1.0 Stable Release

### Core

- [ ] API stability guarantee (no breaking changes post-v1.0)
- [ ] Comprehensive error codes documentation
- [ ] Request/response TypeScript types auto-generated
- [ ] GraphQL adapter (optional)

### Documentation

- [ ] Full API reference (OpenAPI)
- [ ] Getting started guide (5-minute tutorial)
- [ ] Deployment guides (Docker / AWS / Vercel / Cloudflare)
- [ ] Plugin development tutorial
- [ ] Content type system guide
- [ ] Multi-tenant setup guide
- [ ] Architecture deep-dive

### Testing

- [ ] 500+ API integration tests
- [ ] End-to-end test suite (Playwright for Admin UI)
- [ ] Load test scenarios
- [ ] CI/CD pipeline (GitHub Actions)

---

## Q4 2026 — Ecosystem

### Plugin Marketplace

- [ ] Plugin registry API
- [ ] Plugin publishing CLI (`raisfast plugin publish`)
- [ ] Plugin marketplace web UI
- [ ] Plugin discovery and search
- [ ] Plugin analytics (downloads, ratings)

### SaaS Platform (raisfast.cloud)

- [ ] Multi-tenant hosting platform
- [ ] Dashboard (project management, usage metrics)
- [ ] Automatic backups
- [ ] Custom domain support
- [ ] Free tier (suitable for personal blogs)

### Community

- [ ] Plugin starter templates (JS / Lua / WASM)
- [ ] Theme system for front-end
- [ ] Example projects (Blog / E-commerce / Documentation site / Portfolio)
- [ ] Contribution bounties

---

## 2027 — Enterprise

- [ ] SSO / SAML integration
- [ ] SOC 2 compliance
- [ ] Audit log retention policies
- [ ] Role-based field-level permissions
- [ ] Content versioning and approval workflows
- [ ] Multi-language content (i18n)
- [ ] Real-time collaboration (WebSocket / CRDT)
- [ ] Edge caching layer
- [ ] OpenTelemetry integration
- [ ] Kubernetes Helm chart

---

## Completed

- [x] Core REST API (posts, pages, categories, tags, comments, media, users)
- [x] JWT auth + refresh tokens + OAuth (GitHub)
- [x] RBAC permission system
- [x] Admin SPA (React + shadcn/ui)
- [x] Plugin engine (WASM / JavaScript / Lua)
- [x] Content type system with AOP aspects
- [x] Multi-database support (SQLite / PostgreSQL / MySQL)
- [x] SQL dialect abstraction layer
- [x] Tauri desktop mode
- [x] Embedded Admin UI (rust-embed)
- [x] Job queue + cron scheduler
- [x] Webhook system
- [x] Audit logging
- [x] Rate limiting
- [x] Swagger UI (OpenAPI)
- [x] Block editor / page builder
- [x] Media library with thumbnails
- [x] SMS login
- [x] Email verification
- [x] Password reset
- [x] Optional multi-tenant mode
- [x] RSS / Atom feeds
- [x] Sitemap generation
- [x] S3 storage backend
