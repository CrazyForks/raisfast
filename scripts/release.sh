#!/bin/bash
set -euo pipefail

REMOTE="github"
BRANCH="master"
REMOTE_BRANCH="main"

usage() {
  echo "Usage: $0 <command> [options]"
  echo ""
  echo "Commands:"
  echo "  commit <message>              Commit and push"
  echo "  release <version> [message]   Bump version, commit, tag, push"
  echo ""
  echo "Examples:"
  echo "  $0 commit \"fix: some bug\""
  echo "  $0 release 0.3.0"
  echo "  $0 release 0.3.0 \"add new feature\""
  exit 1
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
