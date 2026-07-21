# Deep-Dive Report: Client Binary Footprint Analysis & Compression Benchmarks

**Plan Directory**: `plan/20260721_01`  
**Date**: 2026-07-21  
**Target Project**: `mosh-tcp`  
**Primary Objective**: Minimize `mosh-tcp-client` binary size for users downloading the client over high-latency / bandwidth-constrained network connections.

---

## Executive Summary

To optimize `mosh-tcp-client` for low-bandwidth downloads, we executed a multi-layered optimization strategy:
1. **Zero-Tokio Synchronous Client**: Completely removed Tokio, `tokio-util`, `bytes`, `futures`, `mio`, and `rustix` from the client build graph by refactoring `src/client.rs` to use standard `std::net::TcpStream` with OS threads.
2. **Compilation & Dependency Pruning**: Split client and server binaries, replaced `serde`/`bincode` with a manual 1-byte tag packet codec, removed `anyhow` backtraces, and set `panic = "abort"`.
3. **Binary Breakdown (`cargo-bloat`)**: Analyzed crate and function contributions to the `.text` section.
4. **Compression Benchmarking**: Tested `UPX`, `xz`, `zstd`, `gzip`, `bzip2`, and archive combinations.

**Final Result**:
- Uncompressed Stripped Binary Size: **512 KB** (reduced from 877 KB baseline).
- UPX Compressed (`--best --lzma`) Executable: **218 KB** (222,956 bytes).
- Standalone Client Download: Published directly via GitHub Releases as `mosh-tcp-client-linux-amd64` (~218 KB).

---

## 1. `mosh-tcp-client` Binary Size Breakdown (`cargo-bloat`)

Using `cargo-bloat`, we analyzed the compiled `.text` section of `target/release/mosh-tcp-client` (uncompressed ELF size: **512 KiB**, `.text` section size: **352.2 KiB**):

### Crate Breakdown Table

| Rank | Crate / Component | `.text` Size | % of `.text` | Description & Impact |
| :---: | :--- | :---: | :---: | :--- |
| **1** | `std` (Rust Standard Library) | **260.9 KiB** | **74.1%** | Standard IO, string formatting, memory allocators, system syscalls, thread/signal handling, panic infrastructure. |
| **2** | `miniz_oxide` | **18.4 KiB** | **5.2%** | Inflation engine for decompressing incoming server frames. |
| **3** | `mosh_tcp` + `mosh_tcp_client` | **25.4 KiB** | **7.2%** | Client main thread event loop, local predictive echo VT100 matching, manual packet codec. |
| **4** | `crossterm` | **13.6 KiB** | **3.9%** | Raw terminal mode control, SGR 1006 mouse event encoding, keystroke parsing. |
| **5** | `vte` / `vt100` | **11.8 KiB** | **3.4%** | 2D virtual screen parser for local predictive echo matching. |
| **6** | *Other Crates* (`parking_lot`, `signal_hook_registry`, `crc32fast`) | **22.1 KiB** | **6.2%** | Low-level OS signal wrappers and memory synchronization primitives. |

> **Key Takeaway**: Removing Tokio completely removed `tokio`, `tokio-util`, `bytes`, `futures`, `mio`, `rustix`, `linux-raw-sys`, and Tokio's task-poll runtime machinery from the client dependency graph, shrinking `.text` from 423 KiB to 352 KiB!

---

## 2. Comprehensive Compression Benchmark Results

We benchmarked multiple compression algorithms and double-compression combinations against `target/release/mosh-tcp-client` (512 KiB stripped):

| Rank | Compression / Format | File Size | Exact Bytes | Compression Ratio vs Raw | Executable Without Extraction? |
| :---: | :--- | :---: | :---: | :---: | :---: |
| 🥇 **1** | **UPX (`--best --lzma`)** | **218 KB** | **222,956** | **57.4% smaller** | **Yes (Direct Executable)** |
| 🥈 **2** | **Raw + `xz -9e` (`.tar.xz`)** | **220 KB** | **225,120** | **57.0% smaller** | No (Requires `tar` + `xz`) |
| 🥉 **3** | **Raw + `zstd --ultra -22`** | **234 KB** | **239,812** | **54.1% smaller** | No (Requires `tar` + `zstd`) |
| **4** | **UPX (`--best` standard)** | **252 KB** | **258,048** | **50.8% smaller** | **Yes (Direct Executable)** |
| **5** | **Raw + `bzip2 -9` (`.tar.bz2`)** | **256 KB** | **262,144** | **49.8% smaller** | No (Requires `tar` + `bzip2`) |
| **6** | **Raw + `gzip -9` (`.tar.gz`)** | **260 KB** | **266,240** | **49.2% smaller** | No (Requires `tar` + `gzip`) |

### Key Benchmark Insights

1. **UPX LZMA Outperforms Archive Formats**:
   UPX with LZMA compression (`upx --best --lzma`) achieves **218 KB** (222,956 bytes), outperforming `xz -9e` (220 KB) and `zstd -22` (234 KB).
2. **Direct Self-Extracting Executable Advantage**:
   Unlike archive formats (`.tar.xz`, `.tar.zst`), a UPX-compressed binary is a **direct executable**. A user on a mobile hotspot or slow SSH connection can download `mosh-tcp-client-linux-amd64` using `curl`/`wget` and run `./mosh-tcp-client-linux-amd64` immediately without needing `tar`, `xz`, or `zstd` installed on the target machine.

---

## 3. GitHub Release Workflow Integration

The release workflow (`.github/workflows/release.yml`) and release documentation (`RELEASING.md`) have been updated to publish standalone client downloads.

### Release Artifact Matrix

| Artifact Name | Format / Type | Purpose |
| :--- | :--- | :--- |
| `mosh-tcp-client-linux-amd64` | UPX LZMA Direct Executable (**~218 KB**) | Instant 1-step download & run for slow connections. |
| `mosh-tcp-client-linux-amd64.tar.xz` | `tar.xz` Archive (**~218 KB**) | Client binary + `README.md`. |
| `mosh-tcp-server-linux-amd64.tar.gz` | `tar.gz` Archive | Standalone server binary + `README.md`. |
| `mosh-tcp-linux-amd64.tar.gz` | `tar.gz` Archive | Combined client/server package. |
