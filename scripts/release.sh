#!/bin/bash
set -euo pipefail

REMOTE="github"
BRANCH="master"
REMOTE_BRANCH="main"

CI_FEATURES="db-sqlite plugin-all search-tantivy"

usage() {
  echo "Usage: $0 <command> [options]"
  echo ""
  echo "Commands:"
  echo "  ci                            Run format, clippy, test (same as CI)"
  echo "  commit <message>              Commit and push"
  echo "  release <version> [message]   Bump version, commit, tag, push"
  echo ""
  echo "Examples:"
  echo "  $0 ci"
  echo "  $0 commit \"fix: some bug\""
  echo "  $0 release 0.3.0"
  echo "  $0 release 0.3.0 \"add new feature\""
  exit 1
}

cmd_ci() {
  echo "=== Step 1/3: cargo fmt --check ==="
  cargo fmt --all -- --check
  echo "  OK"

  echo "=== Step 2/3: cargo clippy ==="
  SQLX_OFFLINE=true cargo clippy --tests --no-default-features --features "$CI_FEATURES" -- -D warnings
  echo "  OK"

  echo "=== Step 3/3: cargo test ==="
  SQLX_OFFLINE=true cargo test --no-default-features --features "$CI_FEATURES"
  echo "  OK"

  echo "=== All CI checks passed ==="
}

current_version() {
  grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'
}

bump_version() {
  local new_ver="$1"
  sed -i '' "s/^version = \".*\"/version = \"$new_ver\"/" Cargo.toml
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
