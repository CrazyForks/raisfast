# Contributing to raisfast

Thank you for your interest in contributing to raisfast!

## Status

raisfast is in **early alpha**. The codebase evolves rapidly and APIs may change without notice. We appreciate your patience and willingness to contribute at this stage.

## How to Contribute

### Bug Reports

- Open a [GitHub Issue](https://github.com/raisfast/raisfast/issues/new)
- Include: Rust version, OS, feature flags used, steps to reproduce, expected vs actual behavior
- Check existing issues before filing a new one

### Feature Requests

- Open a GitHub Issue with the `feature-request` label
- Describe the use case, not just the solution
- Indicate if you're willing to implement it

### Pull Requests

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/my-feature`
3. Make your changes
4. Ensure all checks pass:
   ```bash
   cargo fmt --check
   cargo clippy --features "db-sqlite plugin-all search-tantivy" -- -D warnings
   cargo test --features "db-sqlite plugin-all search-tantivy" -- --test-threads=1
   ```
5. Submit a pull request

## Development Setup

### Prerequisites

- Rust 1.85+ (edition 2024)
- SQLite 3.x

### Build

```bash
# Backend only
just build

# Backend + embed Admin UI
just build-full
```

### Test

```bash
# Unit + integration tests
cargo test --features "db-sqlite plugin-all search-tantivy" -- --test-threads=1

# Lint
cargo clippy --features "db-sqlite plugin-all search-tantivy" -- -D warnings

# Format check
cargo fmt --check
```

## Code Style

### Rust

- `cargo fmt` and `cargo clippy` are authoritative
- Public items require `///` doc comments
- Handler → Service → Repository layering enforced; handlers must not contain business logic
- No `unsafe` code (`#![deny(unsafe_code)]` at crate root)
- No `unwrap()` / `expect()` in non-test code; use `?` or explicit error handling
- Error handling: `thiserror` for `AppError` enum at handler boundaries; `anyhow` for internal service errors
- SQL queries use `sqlx::query()` / `sqlx::query_as()` (not `!` macros) + `dialect::translate()`
- All database-specific code uses `crate::db::dialect::*` helpers for portability

### Commits

- Use clear, descriptive commit messages
- Keep PRs focused on a single concern
- Rebase on main before submitting

## Architecture

Before contributing, please read:

- `docs/guide.md` — Product and architecture specification
- `AGENTS.md` — Technical constraints and coding conventions
- `docs/serverless.md` — Serverless deployment design
- `docs/product-analysis.md` — Product positioning and roadmap

## License

By contributing, you agree that your contributions will be licensed under the same license as the module you're modifying:

- Licensed: apache 2.0
