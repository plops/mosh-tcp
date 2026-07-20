#!/usr/bin/env bash
# Pre-release check script for mosh-tcp
# Usage: ./scripts/release-check.sh

set -euo pipefail

echo "==> Pre-release checks for mosh-tcp"
echo ""

CURRENT_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
echo "Current Cargo.toml version: $CURRENT_VERSION"
echo ""

# 1. Clean working tree check
echo -n "Clean working tree: "
if git diff --quiet && git diff --cached --quiet; then
    echo "✓"
else
    echo "✗ (uncommitted changes detected)"
fi

# 2. Branch check
echo -n "On main branch: "
BRANCH=$(git branch --show-current)
if [ "$BRANCH" = "main" ]; then
    echo "✓"
else
    echo "✗ (currently on '$BRANCH')"
fi

# 3. Cargo check
echo -n "Cargo check: "
if cargo check --quiet 2>/dev/null; then
    echo "✓"
else
    echo "✗ (cargo check failed)"
fi

# 4. Unit and Integration tests
echo -n "Cargo test: "
if cargo test --quiet 2>/dev/null; then
    echo "✓"
else
    echo "✗ (cargo test failed)"
fi

echo ""
echo "If all checks pass, run:"
echo "  ./scripts/release.sh <new-version>"
