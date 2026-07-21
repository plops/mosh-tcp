#!/usr/bin/env bash
set -e

echo "=== Building optimized release binaries for mosh-tcp ==="

echo "1. Building standalone Rust client binary (mosh-tcp-client)..."
cargo build --release --no-default-features --features client --bin mosh-tcp-client

echo "2. Building standalone Rust server binary (mosh-tcp-server)..."
cargo build --release --no-default-features --features server --bin mosh-tcp-server

echo "3. Building standalone C client binary (mosh-tcp-client-c)..."
make -C clients/c clean all

echo "4. Building standalone C++ client binary (mosh-tcp-client-cpp)..."
make -C clients/cpp clean all

CLIENT_BIN="target/release/mosh-tcp-client"
SERVER_BIN="target/release/mosh-tcp-server"
CLIENT_C_BIN="clients/c/mosh-tcp-client-c"
CLIENT_CPP_BIN="clients/cpp/mosh-tcp-client-cpp"

if command -v upx >/dev/null 2>&1; then
    echo "=== Compressing Rust, C, and C++ client & server binaries with UPX (LZMA) ==="
    upx --best --lzma "$CLIENT_BIN" "$SERVER_BIN" "$CLIENT_C_BIN" "$CLIENT_CPP_BIN" || true
else
    echo ""
    echo "Notice: 'upx' is not installed."
    echo "To compress client binaries down to minimal footprint, install UPX using your package manager:"
    echo "  - Debian / Ubuntu / Termux: sudo apt install upx"
    echo "  - Arch Linux:               sudo pacman -S upx"
    echo "  - Fedora / RHEL:            sudo dnf install upx"
    echo "  - Gentoo Linux:             sudo emerge -av upx-bin"
    echo "  - macOS (Homebrew):         brew install upx"
    echo "  - Alpine Linux:             sudo apk add upx"
fi

echo ""
echo "=== Final Binaries Info ==="
ls -lh "$CLIENT_BIN" "$SERVER_BIN" "$CLIENT_C_BIN" "$CLIENT_CPP_BIN"
