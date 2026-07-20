#!/usr/bin/env bash
set -e

echo "=== Building optimized release binary for mosh-tcp ==="
cargo build --release

BINARY="target/release/mosh-tcp"

if command -v upx >/dev/null 2>&1; then
    echo "=== Compressing binary with UPX (LZMA) ==="
    upx --best --lzma "$BINARY"
else
    echo "Notice: 'upx' is not installed. Install upx to compress the binary down to ~300 kB."
fi

echo ""
echo "=== Final Binary Info ==="
ls -lh "$BINARY"
