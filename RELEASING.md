# Release Process & Instructions (`mosh-tcp`)

This document describes the automated release workflow for `mosh-tcp`.

---

## 1. Overview & Architecture

`mosh-tcp` uses GitHub Actions (`.github/workflows/release.yml`) to automatically build Linux release binaries and publish release assets whenever a git tag matching `v*` (e.g. `v0.2.0`) is pushed to GitHub.

### Release Artifacts Published
Each GitHub Release contains:
- `mosh-tcp-linux-amd64.tar.gz` — Tarball containing:
  - `mosh-tcp` — The release binary (`x86_64-unknown-linux-gnu`)
  - `README.md` — Project documentation
  - `walkthrough.md` — Architecture walkthrough & VT100/mouse engine documentation
- **Automated Collated Release Notes** — Published directly on the GitHub Release webpage.

---

## 2. Instructions for Creating a Release (Human User or AI Agent)

Follow these steps whenever creating a new release:

### Step 1: Pre-Release Verification
Before creating a release, ensure the codebase is clean, tested, and ready:

```bash
./scripts/release-check.sh
```

The script verifies:
1. Working tree is clean (no uncommitted files).
2. Repository is on the `main` branch.
3. `cargo check` completes without errors.
4. `cargo test` passes all unit and integration tests.

---

### Step 2: Execute Release Script & Provide Release Summary
Run `./scripts/release.sh` with the target semver version number and an optional release summary:

```bash
./scripts/release.sh <version> "[Optional high-level summary]"

# Example:
./scripts/release.sh 0.2.0 "Implemented 2D VT100 state sync, SGR mouse tracking, and atomic frame generator"
```

---

## 3. How Change Collation & Release Notes Work

To ensure the GitHub Release webpage contains a full collation of all changes:

1. **Git Commit History Scanning**:
   `./scripts/release.sh` automatically finds the previous release tag (`git describe --tags --abbrev=0`) and extracts all commits since that release (`git log ${LAST_TAG}..HEAD`).

2. **Annotated Tag Generation**:
   The script creates an annotated git tag (`v<version>`) populated with:
   - User/Agent provided high-level release notes summary.
   - Full collated list of conventional commits (`feat: ...`, `fix: ...`, `docs: ...`, `ci: ...`).

3. **GitHub Release Page Generation**:
   When the tag is pushed to GitHub, `.github/workflows/release.yml` triggers `softprops/action-gh-release@v2` with `generate_release_notes: true`.
   GitHub parses the tag message, pull requests, and commit logs to build a clean, formatted changelog directly on the release webpage next to the downloadable `mosh-tcp-linux-amd64.tar.gz` asset.

---

## 4. Manual Step Summary (For Reference)

If running commands manually without `./scripts/release.sh`:

```bash
# 1. Update Cargo.toml version
sed -i 's/^version = ".*"/version = "0.2.0"/' Cargo.toml
cargo check

# 2. Commit version bump
git add Cargo.toml Cargo.lock
git commit -m "chore(release): bump version to v0.2.0"

# 3. Create annotated tag with collated changelog
git tag -a v0.2.0 -m "Release v0.2.0: Mosh 2D state sync and mouse tracking"

# 4. Push commit and tag
git push origin main
git push origin v0.2.0
```
