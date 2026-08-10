#!/bin/bash
set -euo pipefail

REMOTE="github"
BRANCH="master"
REMOTE_BRANCH="main"

CI_DB="sqlite"
CI_FEATURES="db-sqlite plugin-js plugin-rhai search-tantivy payment-all tunnel mcp cron-system"

usage() {
  echo "Usage: $0 <command> [options]"
  echo ""
  echo "Commands:"
  echo "  ci                            Run format, clippy, test (same as CI)"
  echo "  commit <message>              Commit and push"
  echo "  release <version> [message]   Bump version, commit, tag, push"
  echo ""
  echo "Options:"
  echo "  --db <sqlite|postgres|mysql>  Database backend (default: sqlite)"
  echo ""
  echo "Examples:"
  echo "  $0 ci"
  echo "  $0 ci --db mysql"
  echo "  $0 commit \"fix: some bug\""
  echo "  $0 release 0.3.0"
  echo "  $0 release 0.3.0 \"add new feature\""
  exit 1
}

# Parse global --db flag from any position
parse_db_flag() {
  local args=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --db)
        CI_DB="$2"
        shift 2
        ;;
      *)
        args+=("$1")
        shift
        ;;
    esac
  done
  set -- "${args[@]}" 2>/dev/null || true
  # Return remaining args via ARGV
  ARGV=("${args[@]}")
}

# Derive CI_FEATURES from CI_DB
update_features() {
  CI_FEATURES="db-$CI_DB plugin-js plugin-rhai search-tantivy payment-all tunnel mcp cron-system"
}

# Link .sqlx to the cache matching CI_DB, generate if missing
link_sqlx_cache() {
  local cache=".sqlx-$CI_DB"
  if [ ! -d "$cache" ]; then
    echo ">> sqlx cache '$cache' not found, generating..."
    case "$CI_DB" in
      sqlite)   RAISFAST_DB=sqlite   just db-prepare ;;
      postgres) RAISFAST_DB=postgres just db-prepare ;;
      mysql)    RAISFAST_DB=mysql    just db-prepare ;;
      *) echo "Unknown db: $CI_DB"; exit 1 ;;
    esac
  fi
  ln -sfn "$cache" .sqlx
  echo ">> .sqlx -> $cache"
}

cmd_ci() {
  update_features
  link_sqlx_cache

  echo "=== Step 1/3: cargo fmt --check ==="
  cargo fmt --all -- --check
  echo "  OK"

  echo "=== Step 2/3: cargo clippy ($CI_DB) ==="
  SQLX_OFFLINE=true cargo clippy --tests --no-default-features --features "$CI_FEATURES" -- -D warnings
  echo "  OK"

  echo "=== Step 3/3: cargo test ($CI_DB) ==="
  SQLX_OFFLINE=true cargo test --no-default-features --features "$CI_FEATURES"
  echo "  OK"

  echo "=== All CI checks passed ($CI_DB) ==="
}

CORE_TOML="crates/core/Cargo.toml"

current_version() {
  grep '^version' "$CORE_TOML" | head -1 | sed 's/version = "\(.*\)"/\1/'
}

bump_version() {
  local new_ver="$1"
  sed -i '' "s/^version = \".*\"/version = \"$new_ver\"/" "$CORE_TOML"
  echo "Version bumped: $(current_version) -> $new_ver"
}

cmd_commit() {
  local msg="$1"
  git add -A
  if git diff --cached --quiet; then
    echo "Nothing to commit."
    return
  fi
  git commit -m "$msg"
  git push "$REMOTE" "$BRANCH:$REMOTE_BRANCH"
  echo "Pushed to $REMOTE:$REMOTE_BRANCH"
}

cmd_release() {
  local ver="$1"
  local msg="${2:-release v$ver}"
  local tag="v$ver"

  cmd_ci

  if git tag -l "$tag" | grep -q .; then
    echo "Tag $tag already exists!"
    exit 1
  fi

  git add -A
  if ! git diff --cached --quiet; then
    echo "Staged changes detected, committing first..."
    git commit -m "$msg"
  fi

  bump_version "$ver"

  if command -v git-cliff &>/dev/null; then
    git-cliff --tag "$tag" -o CHANGELOG.md
    echo "Changelog generated"
  fi

  git add -A
  git commit -m "release: v$ver"
  git tag "$tag"
  git push "$REMOTE" "$BRANCH:$REMOTE_BRANCH"
  git push "$REMOTE" "$tag"
  echo "Released $tag and pushed to $REMOTE"
}

if [ $# -lt 1 ]; then
  usage
fi

# Extract --db flag before dispatching
parse_db_flag "$@"
set -- "${ARGV[@]}" 2>/dev/null || true

if [ $# -lt 1 ]; then
  usage
fi

case "$1" in
  ci)
    cmd_ci
    ;;
  commit)
    [ $# -lt 2 ] && usage
    cmd_commit "$2"
    ;;
  release)
    [ $# -lt 2 ] && usage
    cmd_release "$2" "${3:-}"
    ;;
  *)
    usage
    ;;
esac
