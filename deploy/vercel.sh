#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR/../frontend/raisfast.com"

cd "$PROJECT_DIR"

echo "==> Installing dependencies..."
pnpm install --frozen-lockfile

echo "==> Building..."
pnpm build

echo "==> Deploying to Vercel (production)..."
npx vercel --prod --yes

echo "==> Done!"
