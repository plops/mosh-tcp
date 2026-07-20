---
name: release-process
description: Use when creating a release, bumping the version, tagging a commit, or publishing release assets for mosh-tcp.
---

# Release Process (`mosh-tcp`)

## Overview
`mosh-tcp` uses GitHub Actions to automatically build Linux binaries (`mosh-tcp-linux-amd64.tar.gz`) and publish collated release notes on the GitHub webpage whenever a git tag matching `v*` is pushed.

## Automated Release Workflow

### 1. Run Pre-Release Checks
```bash
./scripts/release-check.sh
```

### 2. Run Release Automation Script
```bash
./scripts/release.sh <version> "[summary of changes]"
# Example:
./scripts/release.sh 0.2.0 "Added 2D VT100 state sync and SGR mouse tracking"
```

## How Change Collation Works
- `scripts/release.sh` gathers all commits since the last release tag (`git log`).
- It updates `Cargo.toml` and creates an annotated tag `v<version>` containing the change collation.
- The GitHub Actions workflow (`.github/workflows/release.yml`) builds the Linux binary and publishes both the `tar.gz` artifact and the collated changelog on the GitHub Release webpage.
