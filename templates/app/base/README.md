# {{ name }}

{{ description }}

## Quick Start

```bash
cp .env.example .env
raisfast
```

That's it — running `raisfast` (or `raisfast server start`) starts the
server and creates the schema automatically on first run (SQLite file
included). No migrate step needed.

`raisfast db migrate` is only for upgrading an existing database after
installing a newer binary. Other useful commands:

```bash
raisfast db rollback   # roll back the last migration batch
raisfast db backup     # backup the database
```

## API Endpoints

- Public API: http://localhost:9898/api/v1/cms/
- Admin API: http://localhost:9898/api/v1/admin/cms/
- API Docs: http://localhost:9898/api/docs
- Admin UI: http://localhost:9898/admin

## Project Structure

```
extensions/
  content_types/    — Content Type TOML definitions
  plugins/          — Plugin JS/Lua/WASM files
storage/            — Runtime data (auto-managed, gitignored)
  db/               — SQLite database
  uploads/          — Uploaded media files
  logs/             — Application logs
  search_index/     — Full-text search index
  vfs/              — Plugin virtual filesystem
  backups/          — Database backups
```

## Configuration

All configuration is via environment variables (`.env`). See `.env.example`
for common options — database URL, JWT secret, host/port, etc.
