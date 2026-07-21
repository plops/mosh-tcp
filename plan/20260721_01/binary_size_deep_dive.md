# Deep-Dive Report: Client Binary Footprint Analysis & Compression Benchmarks

**Plan Directory**: `plan/20260721_01`  
**Date**: 2026-07-21  
**Target Project**: `mosh-tcp`  
**Primary Objective**: Minimize `mosh-tcp-client` binary size for users downloading the client over high-latency / bandwidth-constrained network connections.

---

## Executive Summary

To optimize `mosh-tcp-client` for low-bandwidth downloads, we executed a multi-layered optimization strategy:
1. **Compilation & Dependency Pruning**: Split client and server binaries, switched client to Tokio `current_thread`, replaced `serde`/`bincode` with a manual byte tag codec, removed `anyhow` backtraces, and set `panic = "abort"`.
2. **Binary Breakdown (`cargo-bloat`)**: Identified function and crate contributions to the binary `.text` section.
3. **Compression Benchmarking**: Tested `UPX`, `xz`, `zstd`, `gzip`, `bzip2`, and archive combinations.

**Final Result**:
- Uncompressed Stripped Binary Size: **613 KB** (reduced from 877 KB baseline).
- UPX Compressed (`--best --lzma`) Executable: **258 KB** (263,920 bytes).
- Standalone Client Download: Published directly via GitHub Releases as `mosh-tcp-client-linux-amd64` (~258 KB).

---

## 1. `mosh-tcp-client` Binary Size Breakdown (`cargo-bloat`)

Using `cargo-bloat`, we analyzed the compiled `.text` section of `target/release/mosh-tcp-client` (uncompressed ELF size: **613 KiB**, `.text` section size: **415.3 KiB**):

### Crate Breakdown Table

| Rank | Crate / Component | `.text` Size | % of `.text` | Description & Impact |
| :---: | :--- | :---: | :---: | :--- |
| **1** | `std` (Rust Standard Library) | **266.6 KiB** | **64.2%** | Standard IO, string formatting, memory allocators, system syscalls, signal handling, panic infrastructure. |
| **2** | `tokio` | **41.4 KiB** | **10.0%** | Single-threaded `current_thread` reactor, async socket polling, time/timer task queue. |
| **3** | `mosh_tcp_client` + `mosh_tcp` | **37.7 KiB** | **9.1%** | Client main event loop, local predictive echo VT100 matching, manual packet codec. |
| **4** | `miniz_oxide` | **18.3 KiB** | **4.4%** | Inflation engine for decompressing incoming server frames. |
| **5** | `crossterm` | **13.3 KiB** | **3.2%** | Raw terminal mode control, SGR 1006 mouse event encoding, keystroke parsing. |
| **6** | `vte` / `vt100` | **11.8 KiB** | **2.8%** | 2D virtual screen parser for local predictive echo matching. |
| **7** | *Other Crates* (`bytes`, `signal_hook`, `parking_lot`, `mio`, `rustix`) | **26.2 KiB** | **6.3%** | Low-level OS primitive wrappers and memory buffers. |

### Major Functions in `.text`

1. `mosh_tcp_client::main::{closure#0}` (14.2 KiB) — Stdin/terminal event loop and packet channel forwarding.
2. `<tokio::runtime::builder::Builder>::build` (12.9 KiB) — Single-threaded runtime initialization.
3. `miniz_oxide::inflate::core::decompress` (13.2 KiB total) — Gzip/Zlib decompression loop.
4. `vte::<Parser>::perform_action` (5.8 KiB) — VT100 state machine transition table.
5. `crossterm::event::sys::unix::parse::parse_event` (5.6 KiB) — Terminal ANSI input parser.

---

## 2. Comprehensive Compression Benchmark Results

We benchmarked multiple compression algorithms and double-compression combinations against `target/release/mosh-tcp-client` (613 KiB stripped):

| Rank | Compression / Format | File Size | Exact Bytes | Compression Ratio vs Raw | Executable Without Extraction? |
| :---: | :--- | :---: | :---: | :---: | :---: |
| 🥇 **1** | **UPX (`--best --lzma`)** | **258 KB** | **263,920** | **57.9% smaller** | **Yes (Direct Executable)** |
| 🥈 **2** | **Raw + `xz -9e` (`.tar.xz`)** | **263 KB** | **268,700** | **57.1% smaller** | No (Requires `tar` + `xz`) |
| 🥉 **3** | **Raw + `zstd --ultra -22`** | **280 KB** | **286,318** | **54.3% smaller** | No (Requires `tar` + `zstd`) |
| **4** | **UPX (`--best` standard)** | **300 KB** | **306,336** | **51.1% smaller** | **Yes (Direct Executable)** |
| **5** | **Raw + `bzip2 -9` (`.tar.bz2`)** | **305 KB** | **311,734** | **50.2% smaller** | No (Requires `tar` + `bzip2`) |
| **6** | **Raw + `gzip -9` (`.tar.gz`)** | **310 KB** | **316,926** | **49.4% smaller** | No (Requires `tar` + `gzip`) |
| **7** | **UPX + `xz -9e` (`.tar.xz`)** | **258 KB** | **263,404** | **58.0% smaller** | No (Double compressed archive) |

### Key Benchmark Insights

1. **UPX LZMA Outperforms Archive Formats**:
   UPX with LZMA compression (`upx --best --lzma`) achieves **258 KB** (263,920 bytes), outperforming `xz -9e` (263 KB) and `zstd -22` (280 KB).
2. **Direct Self-Extracting Executable Advantage**:
   Unlike archive formats (`.tar.xz`, `.tar.zst`), a UPX-compressed binary is a **direct executable**. A user on a mobile hotspot or slow SSH connection can download `mosh-tcp-client-linux-amd64` using `curl`/`wget` and run `./mosh-tcp-client-linux-amd64` immediately without needing `tar`, `xz`, or `zstd` installed on the target machine.

---

## 3. GitHub Release Workflow Integration

The release workflow (`.github/workflows/release.yml`) and release documentation (`RELEASING.md`) have been updated to publish standalone client downloads.

### Release Artifact Matrix

| Artifact Name | Format / Type | Purpose |
| :--- | :--- | :--- |
| `mosh-tcp-client-linux-amd64` | UPX LZMA Direct Executable (**~258 KB**) | Instant 1-step download & run for slow connections. |
| `mosh-tcp-client-linux-amd64.tar.xz` | `tar.xz` Archive (**~258 KB**) | Client binary + `README.md`. |
| `mosh-tcp-server-linux-amd64.tar.gz` | `tar.gz` Archive | Standalone server binary + `README.md`. |
| `mosh-tcp-linux-amd64.tar.gz` | `tar.gz` Archive | Combined client/server package. |
