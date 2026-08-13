#!/usr/bin/env bash
#
# Backup all content type tables (including M2M junction tables) as portable
# SQL INSERT statements. Supports PostgreSQL, MySQL, and SQLite.
#
# Usage:
#   scripts/backup-content-types.sh [output_file]
#
# Environment:
#   DATABASE_URL       Connection string (auto-loaded from .env)
#   CONTENT_TYPE_DIR   Path to content type TOMLs (default: extensions/content_types)
#
# Examples:
#   scripts/backup-content-types.sh
#   scripts/backup-content-types.sh /tmp/my_backup.sql
#   DATABASE_URL=postgres://user:pass@host/db scripts/backup-content-types.sh
#
# Restore (to same DB type):
#   psql   "$DATABASE_URL" -f backup.sql   # PostgreSQL
#   mysql  "$DB_NAME"      <  backup.sql   # MySQL
#   sqlite3 "$DB_PATH"     <  backup.sql   # SQLite

set -uo pipefail

# Project root = current working directory (where the script is invoked).
PROJECT_ROOT="$PWD"
CT_DIR="${CONTENT_TYPE_DIR:-$PROJECT_ROOT/extensions/content_types}"

# ── Load .env ────────────────────────────────────────────────────────────

if [ -z "${DATABASE_URL:-}" ] && [ -f "$PROJECT_ROOT/.env" ]; then
    # shellcheck disable=SC1090
    set -a; . "$PROJECT_ROOT/.env"; set +a
fi

if [ -z "${DATABASE_URL:-}" ]; then
    echo "✗ DATABASE_URL not set (check .env or export DATABASE_URL=...)"
    exit 1
fi

# ── Detect database type & parse connection ─────────────────────────────

DB_TYPE=""
SQLITE_PATH=""
MYSQL_HOST="" MYSQL_PORT="" MYSQL_USER="" MYSQL_PASS="" MYSQL_DB=""

detect_db() {
    case "$DATABASE_URL" in
        postgres://*|postgresql://*)
            DB_TYPE="postgres"
            need_cmd pg_dump "pg_dump"
            need_cmd psql     "psql"
            ;;
        mysql://*|mariadb://*)
            DB_TYPE="mysql"
            need_cmd mysqldump "mysqldump"
            need_cmd mysql     "mysql"
            parse_mysql_url "$DATABASE_URL"
            export MYSQL_PWD="$MYSQL_PASS"
            ;;
        sqlite:*)
            DB_TYPE="sqlite"
            need_cmd sqlite3 "sqlite3"
            SQLITE_PATH="${DATABASE_URL#sqlite:}"
            SQLITE_PATH="${SQLITE_PATH%%\?*}"
            # Resolve relative paths against project root
            [[ "$SQLITE_PATH" != /* ]] && SQLITE_PATH="$PROJECT_ROOT/$SQLITE_PATH"
            if [ ! -f "$SQLITE_PATH" ]; then
                echo "✗ SQLite database not found: $SQLITE_PATH"
                exit 1
            fi
            ;;
        *)
            echo "✗ Unsupported DATABASE_URL scheme: $DATABASE_URL"
            exit 1
            ;;
    esac
}

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "✗ $2 not found — install it first"
        exit 1
    fi
}

parse_mysql_url() {
    local stripped="${1#mysql://}"
    stripped="${stripped#mariadb://}"
    local user_pass="${stripped%%@*}"
    local rest="${stripped#*@}"
    MYSQL_USER="${user_pass%%:*}"
    MYSQL_PASS="${user_pass#*:}"
    [ "$MYSQL_PASS" = "$user_pass" ] && MYSQL_PASS=""
    rest="${rest%%\?*}"    # strip query params
    rest="${rest%/}"       # strip trailing slash
    MYSQL_DB="${rest##*/}"
    local host_port="${rest%/*}"
    MYSQL_HOST="${host_port%%:*}"
    MYSQL_PORT="${host_port##*:}"
    [ "$MYSQL_PORT" = "$host_port" ] && MYSQL_PORT="3306"
}

# ── Collect table names from TOML files ─────────────────────────────────
#
# Output: main tables first (sorted), then M2M junction tables (sorted).
# This ordering satisfies FK constraints during restore.
#
collect_tables() {
    local main=()
    local junctions=()

    for toml in "$CT_DIR"/*.toml; do
        [ -f "$toml" ] || continue

        # Extract main table name from [content_type] section
        local table
        table=$(awk -F'"' '/^table[[:space:]]*=/{print $2; exit}' "$toml")
        [ -z "$table" ] && continue
        main+=("$table")

        # Extract junction tables from many_to_many / many_way relations.
        # A junction table is either explicitly named via `through = "..."`
        # or auto-generated as `{table}_{target}`.
        while IFS= read -r jt; do
            [ -n "$jt" ] && junctions+=("$jt")
        done < <(awk -v t="$table" '
            BEGIN { m2m=0; thru=""; tgt="" }
            # New section — flush previous field if it was M2M
            /^\[/ {
                if (m2m) {
                    if (thru != "") print thru
                    else if (tgt != "") printf "%s_%s\n", t, tgt
                }
                m2m=0; thru=""; tgt=""
            }
            /^relation_type[[:space:]]*=[[:space:]]*"(many_to_many|many_way)"/ { m2m=1 }
            /^through[[:space:]]*=/ { if (match($0, /"[^"]*"/)) thru=substr($0, RSTART+1, RLENGTH-2) }
            /^target[[:space:]]*=/  { if (match($0, /"[^"]*"/)) tgt=substr($0, RSTART+1, RLENGTH-2) }
            END {
                if (m2m) {
                    if (thru != "") print thru
                    else if (tgt != "") printf "%s_%s\n", t, tgt
                }
            }
        ' "$toml")
    done

    # Print main tables first (sorted, deduped)
    if [ "${#main[@]}" -gt 0 ]; then
        printf '%s\n' "${main[@]}" | sort -u
    fi

    # Print junction tables last (sorted, deduped, excluding tables that are also main)
    if [ "${#junctions[@]}" -gt 0 ]; then
        for jt in "${junctions[@]}"; do
            local is_main=false
            for m in "${main[@]}"; do
                [ "$jt" = "$m" ] && is_main=true && break
            done
            $is_main || echo "$jt"
        done | sort -u
    fi
}

# ── Check if table exists in the database ────────────────────────────────

table_exists() {
    local table="$1"
    case "$DB_TYPE" in
        postgres)
            psql "$DATABASE_URL" -t -A -c \
                "SELECT 1 FROM pg_tables WHERE schemaname='public' AND tablename='$table' LIMIT 1;" \
                2>/dev/null | grep -q 1
            ;;
        mysql)
            mysql -h"$MYSQL_HOST" -P"$MYSQL_PORT" -u"$MYSQL_USER" "$MYSQL_DB" -N -e \
                "SELECT 1 FROM information_schema.tables WHERE table_schema=DATABASE() AND table_name='$table' LIMIT 1;" \
                2>/dev/null | grep -q 1
            ;;
        sqlite)
            sqlite3 "$SQLITE_PATH" \
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='$table' LIMIT 1;" \
                2>/dev/null | grep -q 1
            ;;
    esac
}

# ── Dump a single table's data as INSERT statements ──────────────────────
#
# Outputs INSERT lines to stdout. Returns 0 on success even if table is empty.

dump_table() {
    local table="$1"
    case "$DB_TYPE" in
        postgres)
            pg_dump "$DATABASE_URL" \
                --data-only --column-inserts \
                --table="$table" \
                --no-owner --no-privileges --no-comments \
                2>/dev/null \
                | sed 's/INSERT INTO public\./INSERT INTO /' \
                | grep '^INSERT INTO' || true
            ;;
        mysql)
            mysqldump -h"$MYSQL_HOST" -P"$MYSQL_PORT" -u"$MYSQL_USER" \
                --no-create-info --complete-insert --skip-extended-insert \
                --skip-triggers --no-tablespaces --compact \
                "$MYSQL_DB" "$table" 2>/dev/null \
                | grep '^INSERT INTO' || true
            ;;
        sqlite)
            local cols
            cols=$(sqlite3 "$SQLITE_PATH" \
                "SELECT group_concat(name, ', ') FROM pragma_table_info('$table');")
            sqlite3 "$SQLITE_PATH" ".mode insert $table" "SELECT * FROM $table;" 2>/dev/null \
                | sed "s/INSERT INTO ${table} VALUES/INSERT INTO \"${table}\" (${cols}) VALUES/" \
                || true
            ;;
    esac
}

# ── Count rows in a table ─────────────────────────────────────────────────

count_rows() {
    local table="$1"
    case "$DB_TYPE" in
        postgres)
            psql "$DATABASE_URL" -t -A -c "SELECT count(*) FROM \"$table\";" 2>/dev/null
            ;;
        mysql)
            mysql -h"$MYSQL_HOST" -P"$MYSQL_PORT" -u"$MYSQL_USER" "$MYSQL_DB" -N -e \
                "SELECT count(*) FROM \`$table\`;" 2>/dev/null
            ;;
        sqlite)
            sqlite3 "$SQLITE_PATH" "SELECT count(*) FROM \"$table\";" 2>/dev/null
            ;;
    esac
}

# ── Main ──────────────────────────────────────────────────────────────────

detect_db

OUTPUT="${1:-$PROJECT_ROOT/storage/backups/content_types_$(date +%Y%m%d_%H%M%S).sql}"
mkdir -p "$(dirname "$OUTPUT")"

echo "Database:  $DB_TYPE"
echo "TOML dir:  $CT_DIR"
echo "Output:    $OUTPUT"
echo

# Collect all table names, filter to those existing in DB
ALL_TABLES=()
SKIPPED=()

while IFS= read -r t; do
    [ -z "$t" ] && continue
    if table_exists "$t"; then
        ALL_TABLES+=("$t")
    else
        SKIPPED+=("$t")
    fi
done < <(collect_tables)

if [ "${#ALL_TABLES[@]}" -eq 0 ]; then
    echo "✗ No content type tables found in database"
    exit 1
fi

echo "Found ${#ALL_TABLES[@]} table(s) to backup:"
for t in "${ALL_TABLES[@]}"; do
    echo "  ✓ $t"
done
for t in "${SKIPPED[@]:-}"; do
    [ -n "$t" ] && echo "  ⚠ skipped (not in DB): $t"
done
echo

# Write backup file
{
    echo "-- =============================================================="
    echo "-- raisfast content type data backup"
    echo "-- Generated: $(date '+%Y-%m-%d %H:%M:%S')"
    echo "-- Database:  $DB_TYPE"
    echo "-- Tables:    ${#ALL_TABLES[@]}"
    echo "--"
    echo "-- Restore:"
    echo "--   psql    \"\$DATABASE_URL\" -f $(basename "$OUTPUT")"
    echo "--   mysql   \"\$DB\"          <  $(basename "$OUTPUT")"
    echo "--   sqlite3 \"\$DB_PATH\"      <  $(basename "$OUTPUT")"
    echo "-- =============================================================="
    echo ""

    # Disable FK checks during restore for tables that may reference each other
    case "$DB_TYPE" in
        mysql)  echo "SET FOREIGN_KEY_CHECKS=0;"; echo "" ;;
    esac

    echo "BEGIN;"
    echo ""

    total_rows=0
    for table in "${ALL_TABLES[@]}"; do
        rows=$(count_rows "$table")
        total_rows=$((total_rows + rows))
        echo "-- ------------------------------------------------------------"
        echo "-- Table: $table ($rows rows)"
        echo "-- ------------------------------------------------------------"
        if [ "${rows:-0}" -gt 0 ]; then
            dump_table "$table"
        else
            echo "-- (empty table)"
        fi
        echo ""
    done

    echo "COMMIT;"

    case "$DB_TYPE" in
        mysql)  echo ""; echo "SET FOREIGN_KEY_CHECKS=1;"; ;;
    esac

    echo ""
    echo "-- Done: $total_rows row(s) across ${#ALL_TABLES[@]} table(s)"
} > "$OUTPUT"

SIZE=$(wc -c < "$OUTPUT" | tr -d ' ')
echo "✓ Backup complete: $OUTPUT"
echo "  Size: ${SIZE} bytes"
