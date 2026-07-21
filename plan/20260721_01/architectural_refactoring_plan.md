# Architectural Refactoring & Client Binary Size Minimization Plan

**Date**: 2026-07-21  
**Target Project**: `mosh-tcp`  
**Primary Goal**: Minimize client binary size for slow network downloads, refactor code structure for maintainability, and eliminate test flakiness.

---

## Codebase File Index & Context References

| File Path | Description | Key Refactoring Tasks |
| :--- | :--- | :--- |
| [`Cargo.toml`](file:///workspace/src/mosh-tcp/Cargo.toml) | Package manifest and dependencies | Add `panic = "abort"`, split `bin` targets (`mosh-tcp-client`, `mosh-tcp-server`), make `portable-pty` optional under `server` feature. |
| [`src/lib.rs`](file:///workspace/src/mosh-tcp/src/lib.rs) | Library exports | Export `ansi`, `client`, `predictive`, `protocol`, `server` modules conditionally. |
| [`src/main.rs`](file:///workspace/src/mosh-tcp/src/main.rs) | CLI entry point | Split into [`src/bin/mosh-tcp-client.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh-tcp-client.rs) and [`src/bin/mosh-tcp-server.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh-tcp-server.rs). Retain manual argument parsing for client. |
| [`src/protocol.rs`](file:///workspace/src/mosh-tcp/src/protocol.rs) | Packet framing & serialization | Replace `bincode`/`serde` with lightweight manual binary tag codec for zero-dependency serialization. |
| [`src/predictive.rs`](file:///workspace/src/mosh-tcp/src/predictive.rs) | Local predictive echo engine | Extract ANSI/VT100 sequence matching to `ansi.rs`. Keep 2D cell confirmation logic clean. |
| [`src/server.rs`](file:///workspace/src/mosh-tcp/src/server.rs) | PTY handling, 20ms frame loop, rate limiter | Consolidate 5 separate `Arc<Mutex<...>>` variables into `ServerSessionState`. Extract escape sequence filtering. |
| [`src/client.rs`](file:///workspace/src/mosh-tcp/src/client.rs) | Client raw mode terminal & network loop | Switch Tokio runtime to `current_thread`. Eliminate `anyhow` dependency for simple `io::Result`. |
| [`tests/integration.rs`](file:///workspace/src/mosh-tcp/tests/integration.rs) | Heavy output & backspace integration test | Replace hardcoded port `4099` with `127.0.0.1:0` dynamic port allocation. |
| [`tests/test_browsh.rs`](file:///workspace/src/mosh-tcp/tests/test_browsh.rs) | Browsh TUI integration test | Replace hardcoded port `4093` with `127.0.0.1:0`. |
| [`tests/test_rate_limit.rs`](file:///workspace/src/mosh-tcp/tests/test_rate_limit.rs) | Bandwidth & Carbonyl rate limit tests | Replace hardcoded ports `4096`/`4097` with `127.0.0.1:0`. |
| [`tests/test_tmux.rs`](file:///workspace/src/mosh-tcp/tests/test_tmux.rs) | Tmux session integration test | Replace hardcoded port `4098` with `127.0.0.1:0`. |
| [`tests/test_vt100_resize.rs`](file:///workspace/src/mosh-tcp/tests/test_vt100_resize.rs) | Wide character resize test | Replace hardcoded port `4098` with `127.0.0.1:0`. |

---

## Phase 1: Client Binary Size Optimization (High Priority)

### Step 1.1: Release Profile Tuning in [`Cargo.toml`](file:///workspace/src/mosh-tcp/Cargo.toml)
Add `panic = "abort"` to remove exception unwinding landing pads and tables:
```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

### Step 1.2: Binary Target & Feature Flag Separation
Split binary targets so `mosh-tcp-client` does not link `portable-pty` or server code:
1. Make `portable-pty` optional in [`Cargo.toml`](file:///workspace/src/mosh-tcp/Cargo.toml):
   ```toml
   [dependencies]
   portable-pty = { version = "0.9", optional = true }

   [features]
   default = ["client", "server"]
   client = []
   server = ["dep:portable-pty"]
   ```
2. Create [`src/bin/mosh-tcp-client.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh-tcp-client.rs) containing only client initialization and manual argument parsing.
3. Create [`src/bin/mosh-tcp-server.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh-tcp-server.rs) containing server runner logic.

### Step 1.3: Tokio Single-Threaded Runtime for Client
In [`src/bin/mosh-tcp-client.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh-tcp-client.rs), use Tokio's `current_thread` flavor:
```rust
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    // Client entry point
}
```

### Step 1.4: Lightweight Binary Codec (Remove `bincode` / `serde` from Client)
In [`src/protocol.rs`](file:///workspace/src/mosh-tcp/src/protocol.rs), replace `bincode` serialization for `Packet` with a manual binary codec using 1-byte tags:
- `Tag 1`: `ClientInput` (`u8` tag + `u32` len + data bytes)
- `Tag 2`: `ClientResize` (`u8` tag + `u16` rows + `u16` cols)
- `Tag 3`: `Ping` (`u8` tag + `u64` timestamp)
- `Tag 4`: `Pong` (`u8` tag + `u64` timestamp)
- `Tag 5`: `ServerFrame` (`u8` tag + `u64` seq + `u8` compressed_flag + `u32` len + data bytes)

---

## Phase 2: Architecture & Code Cleanup

### Step 2.1: Consolidate Mutex State in [`src/server.rs`](file:///workspace/src/mosh-tcp/src/server.rs)
Replace multiple separate `Arc<Mutex<...>>` variables (`vt_parser`, `pty_buffer`, `telemetry`, `pty_writer`) with a consolidated state struct:
```rust
pub struct ServerSessionState {
    pub vt_parser: Vt100Parser,
    pub pty_buffer: Vec<u8>,
    pub telemetry: Telemetry,
}
```

### Step 2.2: Extract ANSI / VT100 Helpers
Create `src/ansi.rs` to encapsulate escape sequence detection logic currently scattered in [`src/predictive.rs`](file:///workspace/src/mosh-tcp/src/predictive.rs#L168) and [`src/server.rs`](file:///workspace/src/mosh-tcp/src/server.rs#L437).

---

## Phase 3: Integration Test Stabilization

### Step 3.1: Ephemeral Port Allocation
In all test files in [`tests/`](file:///workspace/src/mosh-tcp/tests):
Replace hardcoded ports (`4093`, `4096`, `4097`, `4098`, `4099`) with dynamic port binding:
```rust
let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
let assigned_port = listener.local_addr()?.port();
// Pass assigned_port to server spawn & client connection
```

### Step 3.2: In-Memory Duplex Tests
Add a test in [`tests/integration.rs`](file:///workspace/src/mosh-tcp/tests/integration.rs) using `tokio::io::duplex(64 * 1024)` to test client and server protocol framing in memory without network sockets.

---

## Verification & Acceptance Criteria
1. `cargo test` passes 100% cleanly without port conflict errors.
2. `cargo build --release --no-default-features --features client --bin mosh-tcp-client` produces a standalone client binary.
3. Client binary size is verified using `ls -lh target/release/mosh-tcp-client` (target: < 300 KB).
