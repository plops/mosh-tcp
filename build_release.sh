#!/usr/bin/env bash
set -e

echo "=== Building optimized release binary for mosh-tcp ==="
cargo build --release

BINARY="target/release/mosh-tcp"

if command -v upx >/dev/null 2>&1; then
    echo "=== Compressing binary with UPX (LZMA) ==="
    upx --best --lzma "$BINARY"
else
    echo "Notice: 'upx' is not installed."
    echo "To compress the binary down to ~300 kB, install UPX using your package manager:"
    echo "  - Debian / Ubuntu / Termux: sudo apt install upx"
    echo "  - Arch Linux:               sudo pacman -S upx"
    echo "  - Fedora / RHEL:            sudo dnf install upx"
    echo "  - Gentoo Linux:             sudo emerge -av upx-bin"
    echo "  - macOS (Homebrew):         brew install upx"
    echo "  - Alpine Linux:             sudo apk add upx"
fi

echo ""
echo "=== Final Binary Info ==="
ls -lh "$BINARY"
