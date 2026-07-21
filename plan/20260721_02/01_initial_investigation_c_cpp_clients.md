# Initial Investigation: C & Modern C++ Standalone Client Implementation

**Plan Directory**: `plan/20260721_02`  
**Document Sequence**: `01`  
**Date**: 2026-07-21  
**Target Project**: `mosh-tcp`  
**Target Clients**: `mosh-tcp-client-c` (C99/C11) and `mosh-tcp-client-cpp` (Modern C++20)

---

## Executive Summary

Following the zero-Tokio synchronous Rust client refactoring (`mosh-tcp-client`), the client binary size reached **512 KB** (raw stripped) and **218 KB** (UPX LZMA compressed). `cargo-bloat` analysis revealed that **~74% of the binary (.text section size ~260 KiB)** is occupied by the Rust `std` library core (formatting, panicking infrastructure, memory allocation wrappers, and stack unwinding metadata).

This investigation evaluates replacing or supplementing the Rust client with standalone C and modern C++ implementations (`mosh-tcp-client-c` and `mosh-tcp-client-cpp`) to achieve an ultra-lightweight client binary footprint of **~10–20 KB UPX LZMA** for low-bandwidth, memory-constrained environments (e.g. OpenWrt routers, embedded Linux, micro-VMs).

---

## 1. Binary Size & Memory Overhead Breakdown

| Implementation | Runtime / Dependencies | Raw Stripped Size | UPX LZMA Size | RAM Footprint | Target Use Case |
| :--- | :--- | :---: | :---: | :---: | :--- |
| **Current Rust Client** | Rust 2024 (`std`, Zero-Tokio) | **512 KB** | **218 KB** | ~6.5 MB | Desktop, Laptop, Standard Linux |
| **Proposed C++20 Client** | C++20 (`libstdc++`, `zlib`) | **~35 – 50 KB** | **~15 – 22 KB** | ~1.5 MB | Modern Linux, Edge Nodes |
| **Proposed C99 Client** | Pure C99 (`libc`, `zlib` / `puff.c`) | **~20 – 30 KB** | **~10 – 15 KB** | ~0.8 MB | OpenWrt Routers, Embedded, IoT |

### Root Cause of Binary Size Difference
1. **Rust `std` Core Overhead**: ~260 KiB of formatting, backtracing, and panic landing pads.
2. **C / C++ Dynamic Linking**: `libc` (`glibc` or `musl`) and `zlib` are already present on target systems, reducing the client binary to purely application logic (~12–18 KB).

---

## 2. Technical Comparison Matrix (C vs Modern C++ vs Rust)

| Dimension | Pure C (C99/C11) | Modern C++ (C++20) | Current Rust Client |
| :--- | :--- | :--- | :--- |
| **Executable Footprint (UPX)** | **~10 – 15 KB** (Ultra-tiny) | **~15 – 22 KB** (Very small) | **218 KB** (Baseline) |
| **Terminal Raw Mode Safety** | Manual `atexit()` & `signal()` handlers | **RAII Destructors (`std::scope_exit`)** | **`Drop` trait implementation** |
| **Memory & Packet Safety** | Manual raw pointer bounds checks | `std::span`, bounds checking | **Strict compile-time borrow checker** |
| **Protocol Matching** | C `struct` & manual tag byte parsing | `std::variant`, `std::expected` | Rust `enum` pattern matching |
| **Compiler Compatibility** | Any C compiler (`gcc`, `clang`, `tcc`) | GCC 10+, Clang 11+ | Rust toolchain (`cargo`) |
| **Code Base Maintenance** | High C boilerplate | Clean modern abstractions | Single-language codebase |

---

## 3. Key Technical Challenges & Design Considerations

1. **Terminal Raw Mode Restoration**:
   - When entering terminal raw mode (`termios`), if the network connection drops or a signal (`SIGINT`, `SIGTERM`) arrives, standard terminal settings must be restored cleanly.
   - **C++**: Solved cleanly via RAII destructors (`TerminalGuard`).
   - **C**: Solved via signal handlers (`signal(SIGINT, cleanup)`) and `atexit(restore_terminal)`.

2. **Binary Protocol Alignment**:
   - Both clients must strictly adhere to the 4-byte big-endian length prefix and 1-byte packet tags defined in [`src/protocol.rs`](file:///workspace/src/mosh-tcp/src/protocol.rs):
     - `Tag 1`: `ClientInput`
     - `Tag 2`: `ClientResize`
     - `Tag 3`: `Ping`
     - `Tag 4`: `Pong`
     - `Tag 5`: `ServerFrame`

3. **Deflate Frame Decompression**:
   - Frames with `compressed == 1` are compressed using standard Deflate.
   - **C / C++**: Can link system `zlib` (`-lz`) or include single-header decompressor (`puff.c` / `miniz.h`).
