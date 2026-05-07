# {{ name }}

{{ description }}

## Quick Start

```bash
cp .env.example .env
raisfast db migrate
raisfast db seed
raisfast server start
```

## API Endpoints

- Public API: http://localhost:9898/api/v1/cms/
- Admin API: http://localhost:9898/api/v1/admin/cms/
- Swagger UI: http://localhost:9898/swagger-ui/

## Project Structure

```
extensions/
  content_types/    — Content Type TOML definitions
  plugins/          — Plugin JS/Lua/WASM files
migrations/         — SQL migration files
data/               — SQLite database
logs/               — Application logs
public/uploads/     — Uploaded files
```
