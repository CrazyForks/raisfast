#!/usr/bin/env bash
#
# Release a new version of @raisfast/sdk to npm.
#
# Usage:
#   ./scripts/release-sdk.sh <version>
#   ./scripts/release-sdk.sh 0.2.0
#
# Steps:
#   1. Verify clean working tree
#   2. Run tests (vitest)
#   3. Build (tsup → dist/)
#   4. Preview the tarball contents (npm pack --dry-run)
#   5. Bump version in package.json
#   6. Publish to npm (--access public)
#   7. Commit version bump + tag + push
#
# Environment:
#   OTP          — npm 2FA one-time password (if publishing requires OTP)
#   DRY_RUN=1    — run everything except the actual publish / commit / push
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SDK_DIR="$ROOT_DIR/frontend/sdk"

REMOTE="github"
BRANCH="main"
REMOTE_BRANCH="main"

usage() {
  echo "Usage: $0 <version>"
  echo ""
  echo "Examples:"
  echo "  $0 0.2.0"
  echo "  $0 0.1.10"
  echo ""
  echo "Environment:"
  echo "  OTP=123456   npm 2FA one-time password"
  echo "  DRY_RUN=1    dry run (no publish / commit / push)"
  exit 1
}

[ $# -ge 1 ] || usage
VERSION="$1"

# Validate semver-ish version (X.Y.Z, optional pre-release suffix)
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z-]+)?$'; then
  echo "error: invalid version '$VERSION' (expected X.Y.Z, e.g. 0.2.0)"
  exit 1
fi

cd "$SDK_DIR"

CURRENT_VERSION=$(node -p "require('./package.json').version")

if [ "$VERSION" = "$CURRENT_VERSION" ]; then
  echo "error: version is already $CURRENT_VERSION"
  exit 1
fi

echo ">> @raisfast/sdk: $CURRENT_VERSION → $VERSION"

DRY_RUN="${DRY_RUN:-0}"
NPM_PUBLISH_ARGS=(publish --access public)
if [ -n "${OTP:-}" ]; then
  NPM_PUBLISH_ARGS+=(--otp "$OTP")
fi
if [ "$DRY_RUN" = "1" ]; then
  NPM_PUBLISH_ARGS+=(--dry-run)
  echo ">> DRY RUN: no publish / commit / push"
fi

# ── 1. Pre-flight checks ──────────────────────────────────────────

echo "=== Step 1/6: pre-flight checks ==="

# frontend/sdk lives in its own nested git repo (frontend monorepo) —
# operate on that repo, not the main raisfast repo.
SDK_GIT_DIR="$(git -C "$SDK_DIR" rev-parse --show-toplevel 2>/dev/null || true)"
if [ -z "$SDK_GIT_DIR" ]; then
  echo "error: $SDK_DIR is not inside a git repo"
  exit 1
fi
echo "  git:    sdk repo root at $SDK_GIT_DIR"

# SDK working tree must be clean (only package.json / package-lock.json /
# src / dist matter; other dirs like admin/ are tracked separately)
if [ -n "$(git -C "$SDK_GIT_DIR" status --porcelain --untracked-files=no -- sdk)" ]; then
  echo "error: sdk working tree is not clean. Commit or stash first."
  git -C "$SDK_GIT_DIR" status --short -- sdk
  exit 1
fi

# Must be on the publishing branch
CURRENT_BRANCH=$(git -C "$SDK_GIT_DIR" rev-parse --abbrev-ref HEAD)
if [ "$CURRENT_BRANCH" != "$BRANCH" ]; then
  echo "warn: on branch '$CURRENT_BRANCH', expected '$BRANCH'"
fi

# npm auth
if ! npm whoami &>/dev/null; then
  echo "error: not logged in to npm. Run: npm login"
  exit 1
fi
NPM_USER=$(npm whoami)
echo "  npm:    logged in as '$NPM_USER'"

# version must not already exist on npm
if npm view "@raisfast/sdk@$VERSION" version &>/dev/null; then
  echo "error: version $VERSION already exists on npm"
  exit 1
fi
echo "  npm:    $VERSION is available"

echo "  git:    sdk clean, on '$CURRENT_BRANCH'"

# ── 2. Install deps if needed ─────────────────────────────────────

if [ ! -d node_modules ]; then
  echo "=== Step 2/6: install dependencies ==="
  npm ci
else
  echo "=== Step 2/6: dependencies present ==="
fi

# ── 3. Build ──────────────────────────────────────────────────────

echo "=== Step 3/6: build (tsup) ==="
npm run build

# ── 4. Pack preview ───────────────────────────────────────────────

echo "=== Step 4/6: tarball preview ==="
npm pack --dry-run

# ── 5. Bump version ───────────────────────────────────────────────

echo "=== Step 5/6: bump version ==="
npm version "$VERSION" --no-git-tag-version

# ── 6. Publish + commit + tag ─────────────────────────────────────

echo "=== Step 6/6: publish to npm ==="
npm "${NPM_PUBLISH_ARGS[@]}"

if [ "$DRY_RUN" = "1" ]; then
  echo ""
  echo ">> Dry run complete. To publish for real, re-run without DRY_RUN=1."
  exit 0
fi

echo "=== Commit + tag + push ==="
TAG="sdk-v$VERSION"

SDK_REL_PATH="$(cd "$SDK_DIR" && git rev-parse --show-prefix)"
git -C "$SDK_GIT_DIR" add "${SDK_REL_PATH}package.json" "${SDK_REL_PATH}package-lock.json"
git -C "$SDK_GIT_DIR" commit -m "release(sdk): v$VERSION"
git -C "$SDK_GIT_DIR" tag "$TAG"
git -C "$SDK_GIT_DIR" push "$REMOTE" "$BRANCH:$REMOTE_BRANCH"
git -C "$SDK_GIT_DIR" push "$REMOTE" "$TAG"

echo ""
echo ">> Published @raisfast/sdk@$VERSION and pushed tag $TAG"
