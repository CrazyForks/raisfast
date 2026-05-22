# Production Readiness Assessment

> Last updated: 2026-05-22

## Summary

**Verdict: Ready for production deployment.** Core security (authentication, injection prevention, authorization) is solid. Remaining debt is test coverage and model-layer consistency — non-blocking, iterable post-launch.

- **1856 unit tests** passing, 0 failures
- **326 integration tests** covering 30 handler modules, content type, proxy
- **2 benchmark scripts**: `scripts/benchmark.js` (385 lines), `scripts/benchmark.py` (425 lines)
- **Clippy**: zero warnings
- **No `unsafe` code** in production paths

---

## Green (Production Ready)

### Authentication & Authorization

- **Handler layer**: unified `ensure_authenticated()` / `ensure_admin()` / `ensure_author()` as the sole auth gate
- **Service layer**: resource-level Policy checks (e.g. `PostPolicy::can_update`, `CommentPolicy::can_delete`) for owner-or-admin logic
- **Middleware layer**: `PermissionGuard` + `RbacService` for fine-grained RBAC
- **Route tags**: unified `system public` / `system authed` / `system admin` — all handlers correctly tagged and verified
- **No role string hardcoding**: all role checks use `auth.is_admin()` / `auth.is_author()` methods

### SQL Injection Prevention

- All queries use parameterized bindings via sqlx
- `is_safe_identifier()` validates table names, column names, ORDER BY, GROUP BY
- `build_where_clause` rejects raw SQL strings, only accepts JSON object/array with key validation
- Content type table names validated via `is_safe_identifier` at registration

### Protected Tables

- Startup: `fetch_table_names(pool)` queries all tables from database
- Subtracts content type tables (user-managed) → remaining = protected
- Cached in `OnceLock`, accessible via `get_protected_tables()`
- Plugin DB API checks permissions first, then protected table as friendlier error

### Reserved Route Segments

- Static list of all top-level route segments
- Always reserved regardless of `BUILTIN_*` flags (prevents conflict on re-enable)

### Infrastructure

- **Graceful shutdown**: implemented via `tokio::watch` channel
- **Rate limiting**: global + per-route via `RateLimiterSet`
- **CORS**: configurable via `CORS_ORIGINS` env var
- **Logging**: 294 `tracing` calls across codebase
- **Request ID**: per-request UUID in middleware

### Error Handling

- `thiserror` for `AppError` enum, implements `IntoResponse`
- Handler layer: no `unwrap()` / `expect()` in production code
- Validation errors: i18n-localized messages via `validator` crate
- `validation::validate()` at handler boundary before service calls

### ID Encoding

- Multiplicative inverse cipher (single u64 multiply) + base62 encoding
- Small IDs produce short output: ID 1 → 7 chars, ID 42 → 8 chars
- `validate_optional_id` only checks format (base62 chars), actual decode happens once in `parse_id()`

### DTO Layer

- All request/response/query structs live in `src/dto/`
- Handler files contain only routing logic + service calls
- Validation annotations (`#[validate(...)]`) on DTO fields

---

## Yellow (Known Debt, Non-Blocking)

| Issue | Impact | Count | Recommendation |
|---|---|---|---|
| Services with direct DB queries | Architecture inconsistency | 9 files | Gradually move to model layer |
| Services without unit tests | Regression risk | 6 (cart, audit, content_revision, currencies, product_variant, user_address) | Prioritize post-launch |
| `AppError::Internal(anyhow!)` for expected errors | Returns 500 instead of 404/400 | wallet 7, stats 8, oauth 3 | Use specific error variants |
| `write!(hex, ...).unwrap()` in connection.rs | Violates no-unwrap rule | 1 | Use `String` write (infallible) |
| `stats.rs` DB calls in service layer | No corresponding model | 8 calls | Create `models/stats.rs` |
| ID encoding key hardcoded | Cannot vary per deployment | 1 constant | Acceptable — key is for obfuscation only |

---

## Low Priority (Post-Launch Backlog)

| Issue | Location |
|---|---|
| CSP `unsafe-inline` in styles | HTML templates |
| Plugin `args.add().ok()` silently drops arguments | 15+ places in host_common.rs |
| `serde_json::to_string` failure produces empty string | payment service |
| `let _ = auth.ensure_authenticated()?` discards user_id | 5 places in service/payment.rs |
| `user_int_id` naming in framework internals | aspects.rs, protocols/, content_type/ |

---

## Build & Test Commands

```bash
# Build with all warnings
SQLX_OFFLINE=false DATABASE_URL="sqlite:./storage/db/raisfast.db?mode=rwc" \
  cargo clippy --tests --no-default-features \
  --features "db-sqlite,plugin-js,plugin-lua,plugin-rhai" -- -D warnings

# Run all tests
SQLX_OFFLINE=false DATABASE_URL="sqlite:./storage/db/raisfast.db?mode=rwc" \
  cargo test --no-default-features \
  --features "db-sqlite,plugin-js,plugin-lua,plugin-rhai"

# With ID encoding enabled
SQLX_OFFLINE=false DATABASE_URL="sqlite:./storage/db/raisfast.db?mode=rwc" ID_ENCODING=true \
  cargo test --no-default-features \
  --features "db-sqlite,plugin-js,plugin-lua,plugin-rhai"
```
