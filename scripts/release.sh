#!/usr/bin/env bash
# Release automation script for mosh-tcp
# Usage: ./scripts/release.sh <version> [release-notes]
# Example: ./scripts/release.sh 0.2.0 "Added SGR mouse tracking and VT100 atomic screen frames"

set -euo pipefail

VERSION="${1:-}"
CUSTOM_NOTES="${2:-}"

if [ -z "$VERSION" ]; then
    echo "Usage: $0 <version> [release-notes]"
    echo "Example: $0 0.2.0 \"Add mouse support & VT100 state sync\""
    echo ""
    echo "Current version: $(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')"
    exit 1
fi

# Validate version format (semver X.Y.Z)
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "Error: Version must be in semver format (e.g. 0.2.0)"
    exit 1
fi

TAG="v${VERSION}"

# Check for uncommitted changes
if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "Error: You have uncommitted changes. Please commit or stash them first."
    exit 1
fi

# Check branch is main
BRANCH=$(git branch --show-current)
if [ "$BRANCH" != "main" ]; then
    echo "Warning: You are on branch '$BRANCH', not 'main'."
    read -rp "Continue anyway? [y/N] " confirm
    if [ "$confirm" != "y" ] && [ "$confirm" != "Y" ]; then
        exit 1
    fi
fi

# Check tag does not already exist
if git tag -l "$TAG" | grep -q "$TAG"; then
    echo "Error: Tag $TAG already exists."
    exit 1
fi

# Collate changes since last tag
LAST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
echo "==> Collating changes since last tag '${LAST_TAG:-initial}'..."

CHANGES_FILE=$(mktemp)
echo "## Release ${TAG}" > "$CHANGES_FILE"
echo "" >> "$CHANGES_FILE"

if [ -n "$CUSTOM_NOTES" ]; then
    echo "$CUSTOM_NOTES" >> "$CHANGES_FILE"
    echo "" >> "$CHANGES_FILE"
fi

echo "### Collated Changes:" >> "$CHANGES_FILE"
if [ -n "$LAST_TAG" ]; then
    git log "${LAST_TAG}..HEAD" --pretty=format:"* %s (%h)" >> "$CHANGES_FILE"
else
    git log --pretty=format:"* %s (%h)" >> "$CHANGES_FILE"
fi
echo "" >> "$CHANGES_FILE"

echo ""
cat "$CHANGES_FILE"
echo ""

# Update Cargo.toml version
echo "==> Updating Cargo.toml version to $VERSION"
sed -i "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml

# Update Cargo.lock
echo "==> Updating Cargo.lock..."
cargo check --quiet

# Commit version bump
echo "==> Committing version bump for $TAG"
git add Cargo.toml Cargo.lock
git commit -m "chore(release): bump version to $TAG"

# Create annotated tag with collated release notes
echo "==> Creating annotated tag $TAG with collated changes..."
git tag -a "$TAG" -F "$CHANGES_FILE"

rm -f "$CHANGES_FILE"

echo "==> Pushing commit and tag to origin..."
git push origin main || echo "Note: Push main failed or no remote configured"
git push origin "$TAG" || echo "Note: Push tag failed or no remote configured"

echo ""
echo "✓ Release $TAG successfully created and pushed!"
echo "GitHub Actions will build the release binary and publish the release notes to the GitHub webpage."
