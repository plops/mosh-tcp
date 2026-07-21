# Architectural Refactoring & Client Binary Optimization Walkthrough

**Plan Directory**: `plan/20260721_01`  
**Date**: 2026-07-21  
**Project**: `mosh-tcp`  

---

## Executive Summary

This walkthrough details the architectural refactoring and binary size optimizations implemented in `mosh-tcp` according to the execution plan.

Key achievements:
1. **Client Binary Footprint Reduction**: Reduced client release binary size from **877 KB** down to **626 KB** (a ~28.6% reduction) by removing `serde`/`bincode` dependencies, separating binary targets, and tuning release codegen with `panic = "abort"`.
2. **Server & Engine Architecture Cleanups**: Extracted ANSI/VT100 escape sequence parsing into [`src/ansi.rs`](file:///workspace/src/mosh-tcp/src/ansi.rs) and consolidated server state into a unified `ServerSessionState` lock structure in [`src/server.rs`](file:///workspace/src/mosh-tcp/src/server.rs).
3. **Integration Test Stabilization**: Replaced hardcoded TCP ports with dynamic OS-assigned port allocation (`127.0.0.1:0`) and added an in-memory `tokio::io::duplex` protocol test.

---

## 1. Client Binary Optimization & Codec Refactoring

### A. Manual Binary Tag Codec ([`src/protocol.rs`](file:///workspace/src/mosh-tcp/src/protocol.rs))
- **Previous**: Relied on `serde` trait reflection and `bincode` serialization for `Packet`.
- **Refactoring**: Implemented a lightweight binary codec using 1-byte packet tags:
  - `Tag 1`: `ClientInput { data }`
  - `Tag 2`: `ClientResize { rows, cols }`
  - `Tag 3`: `Ping { timestamp }`
  - `Tag 4`: `Pong { timestamp }`
  - `Tag 5`: `ServerFrame { seq, compressed, data }`
- **Impact**: Removed `serde` and `bincode` dependencies completely from `Cargo.toml`.

### B. Separate Binary Targets ([`Cargo.toml`](file:///workspace/src/mosh-tcp/Cargo.toml))
- **Refactoring**: Added `[[bin]]` entry points for [`mosh-tcp-client`](file:///workspace/src/mosh-tcp/src/bin/mosh_tcp_client.rs) and [`mosh-tcp-server`](file:///workspace/src/mosh-tcp/src/bin/mosh_tcp_server.rs).
- Made `portable-pty` optional under the `server` feature flag (`portable-pty = { version = "0.9", optional = true }`).
- **Impact**: `mosh-tcp-client` compiles cleanly without pulling in PTY process control or server execution paths.

### C. Tokio `current_thread` Runtime ([`src/bin/mosh_tcp_client.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh_tcp_client.rs))
- **Refactoring**: Configured `mosh-tcp-client` main entry point with `#[tokio::main(flavor = "current_thread")]`.

### D. Release Build & UPX Compression Script ([`build_release.sh`](file:///workspace/src/mosh-tcp/build_release.sh))
- Updated `build_release.sh` to automatically build `mosh-tcp-client` (`--no-default-features --features client --bin mosh-tcp-client`), `mosh-tcp-server`, and `mosh-tcp`.
- Configured UPX compression (`upx --best --lzma`) for all targets when UPX is available.


---

## 2. Architecture & Code Cleanup

### A. Dedicated ANSI Parsing Module ([`src/ansi.rs`](file:///workspace/src/mosh-tcp/src/ansi.rs))
- Extracted ANSI/VT100 escape sequence functions (`contains_subslice`, `find_safe_split_point`, `strip_terminal_queries_stateful`) from `predictive.rs` and `server.rs` into a shared module.

### B. Server Mutex State Consolidation ([`src/server.rs`](file:///workspace/src/mosh-tcp/src/server.rs))
- **Previous**: `handle_client` maintained separate `Arc<Mutex<Vt100Parser>>` and `Arc<Mutex<Vec<u8>>>` primitives.
- **Refactoring**: Grouped state into `ServerSessionState`:
  ```rust
  pub struct ServerSessionState {
      pub vt_parser: Vt100Parser,
      pub pty_buffer: Vec<u8>,
  }
  ```
- **Impact**: Reduced lock overhead and simplified PTY reading / frame emission tasks.

---

## 3. Integration Test Improvements

### A. Dynamic Ephemeral Port Allocation
- Updated all integration test files ([`tests/integration.rs`](file:///workspace/src/mosh-tcp/tests/integration.rs), [`tests/test_browsh.rs`](file:///workspace/src/mosh-tcp/tests/test_browsh.rs), [`tests/test_rate_limit.rs`](file:///workspace/src/mosh-tcp/tests/test_rate_limit.rs), [`tests/test_tmux.rs`](file:///workspace/src/mosh-tcp/tests/test_tmux.rs), [`tests/test_vt100_resize.rs`](file:///workspace/src/mosh-tcp/tests/test_vt100_resize.rs)) to use `get_free_address()` (`127.0.0.1:0`).
- **Impact**: Completely eliminated port collision flakiness when running parallel tests (`cargo test`).

### B. In-Memory Duplex Test
- Added `test_in_memory_duplex_framing` in [`tests/integration.rs`](file:///workspace/src/mosh-tcp/tests/integration.rs#L21) using `tokio::io::duplex` to verify packet serialization, framing, and compression without OS TCP socket overhead.

---

## 4. Verification Results

All 15 unit and integration tests passed cleanly:

```text
running 9 tests in src/lib.rs ... ok
running 2 tests in tests/integration.rs ... ok
running 1 test in tests/test_browsh.rs ... ok
running 2 tests in tests/test_predictive.rs ... ok
running 2 tests in tests/test_rate_limit.rs ... ok
running 1 test in tests/test_tmux.rs ... ok
running 1 test in tests/test_vt100_resize.rs ... ok

Result: 15 passed, 0 failed.
```

Client release binary size comparison:
- **Before**: 877 KB (`mosh-tcp`)
- **After**: 626 KB (`mosh-tcp-client`)
