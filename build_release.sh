#!/usr/bin/env bash
set -e

echo "=== Building optimized release binaries for mosh-tcp ==="

echo "1. Building standalone client binary (mosh-tcp-client)..."
cargo build --release --no-default-features --features client --bin mosh-tcp-client

echo "2. Building standalone server binary (mosh-tcp-server)..."
cargo build --release --no-default-features --features server --bin mosh-tcp-server

echo "3. Building unified binary (mosh-tcp)..."
cargo build --release --bin mosh-tcp

CLIENT_BIN="target/release/mosh-tcp-client"
SERVER_BIN="target/release/mosh-tcp-server"
UNIFIED_BIN="target/release/mosh-tcp"

if command -v upx >/dev/null 2>&1; then
    echo "=== Compressing client and server binaries with UPX (LZMA) ==="
    upx --best --lzma "$CLIENT_BIN" "$SERVER_BIN" "$UNIFIED_BIN" || true
else
    echo ""
    echo "Notice: 'upx' is not installed."
    echo "To compress the client binary down to ~150-200 kB, install UPX using your package manager:"
    echo "  - Debian / Ubuntu / Termux: sudo apt install upx"
    echo "  - Arch Linux:               sudo pacman -S upx"
    echo "  - Fedora / RHEL:            sudo dnf install upx"
    echo "  - Gentoo Linux:             sudo emerge -av upx-bin"
    echo "  - macOS (Homebrew):         brew install upx"
    echo "  - Alpine Linux:             sudo apk add upx"
fi

echo ""
echo "=== Final Binaries Info ==="
ls -lh "$CLIENT_BIN" "$SERVER_BIN" "$UNIFIED_BIN"
