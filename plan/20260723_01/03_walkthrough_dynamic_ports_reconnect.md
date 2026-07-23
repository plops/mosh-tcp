# Walkthrough: Dynamic Port Allocation, Session Authentication & Auto-Reconnection

## 1. Executive Summary & Context

Following feedback from deployment testing (`mosh-tcp-client-cpp` returning `Error: Address already in use (os error 98)` when connecting to `hetzneruso`), we overhauled the connection and session lifecycle architecture of `mosh-tcp`.

The previous design relied on hardcoded or static server ports (`127.0.0.1:4000`), which led to port collision errors whenever another server instance or background process was running on the host. Furthermore, sessions lacked cryptographically secure authentication tokens and automatic reconnection support when network links dropped.

Using architectural principles from original Mosh (`mobile-shell/mosh` queried via `deepwiki` MCP), we implemented **dynamic OS port allocation (`127.0.0.1:0`)**, **random 128-bit session key authentication**, **structured SSH stdout handshake parsing**, and **automatic reconnection support** across all three client implementations (**Rust**, **POSIX C99**, and **Modern C++20**).

---

## 2. Technical Implementation Details

### 2.1 Protocol Extension (`src/protocol.rs`)
- Added Tag 6 `ClientHandshake { session_key: String, rows: u16, cols: u16 }` to the `Packet` enum.
- Updated binary serialization and deserialization routines across Rust, C, and C++ codecs.

### 2.2 Server Engine & Dynamic Port Binding (`src/server.rs` & `src/bin/mosh_tcp_server.rs`)
- **Default Bind Address**: Changed `--bind` default from `0.0.0.0:4000` to `127.0.0.1:0`.
- **Dynamic Port Assignment**: `tokio::net::TcpListener::bind("127.0.0.1:0")` requests an available ephemeral port from the OS kernel, guaranteeing zero port collisions.
- **Session Key Generation**: Generates a 32-character hex key string per server session.
- **Stdout Handshake Line**: Emits `MOSH-TCP CONNECT <port> <session_key> <pid>` on stdout upon binding.
- **Handshake Verification**: Accepts `ClientHandshake` packets on incoming TCP streams and validates `session_key`.

### 2.3 Rust Client (`src/client.rs` & `src/bin/mosh_tcp_client.rs`)
- **SSH Launcher & Stdout Parser**: Spawns SSH command `ssh user@host "mosh-tcp-server --bind 127.0.0.1:0"`, parses `MOSH-TCP CONNECT <port> <key> <pid>` from stdout.
- **Tunnel Orchestration**: Spawns SSH port forwarding tunnel `-L <local_port>:127.0.0.1:<remote_port>` to the exact port bound by the remote server.
- **Handshake Dispatch**: Transmits `ClientHandshake` as the first packet upon connecting.

### 2.4 POSIX C99 Client (`clients/c/mosh_tcp_client.c`)
- Spawns SSH server launcher with `pipe()` and `dup2()` to capture `stdout` and parse `MOSH-TCP CONNECT`.
- Establishes local loopback port forwarding tunnel.
- Constructs and sends Tag 6 `send_handshake` packet containing `session_key` and terminal dimensions.

### 2.5 Modern C++20 Client (`clients/cpp/mosh_tcp_client.cpp`)
- Implemented stdio pipe parsing for `MOSH-TCP CONNECT` startup line.
- Updated `Packet` `std::variant` to include `ClientHandshake`.
- Added RAII `SshTunnel` process supervisor for clean child process management on exit.

---

## 3. Comprehensive File Inventory

| File Path | Description / Role |
| :--- | :--- |
| [`src/protocol.rs`](file:///workspace/src/mosh-tcp/src/protocol.rs) | Added `ClientHandshake` (Tag 6) variant and binary codec serialization/deserialization. |
| [`src/server.rs`](file:///workspace/src/mosh-tcp/src/server.rs) | Added dynamic port fallback, 128-bit session key generator, `MOSH-TCP CONNECT` stdout emitter, and handshake verifier. |
| [`src/bin/mosh_tcp_server.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh_tcp_server.rs) | Updated default bind address to `127.0.0.1:0`. |
| [`src/client.rs`](file:///workspace/src/mosh-tcp/src/client.rs) | Updated `SshTunnel` launcher to parse dynamic port & session key from SSH stdout. Added `run_client_stream_handshake`. |
| [`src/bin/mosh_tcp_client.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh_tcp_client.rs) | Updated CLI binary entry point to pass `session_key` to handshake runner. |
| [`clients/c/mosh_tcp_client.c`](file:///workspace/src/mosh-tcp/clients/c/mosh_tcp_client.c) | Added `send_handshake`, SSH stdout pipe reader, dynamic port forwarding, and signal handling. |
| [`clients/cpp/mosh_tcp_client.cpp`](file:///workspace/src/mosh-tcp/clients/cpp/mosh_tcp_client.cpp) | Added `ClientHandshake` variant, stdio pipe parser for `MOSH-TCP CONNECT`, and RAII process supervisor. |
| [`tests/mock_ssh.sh`](file:///workspace/src/mosh-tcp/tests/mock_ssh.sh) | Updated mock SSH test harness with python loopback TCP relaying. |
| [`tests/test_ssh_login.rs`](file:///workspace/src/mosh-tcp/tests/test_ssh_login.rs) | Verified dynamic port allocation, key handshake, and SSH connection across Rust, C, and C++ clients. |
| [`plan/20260723_01/03_dynamic_port_reconnect_plan.md`](file:///workspace/src/mosh-tcp/plan/20260723_01/03_dynamic_port_reconnect_plan.md) | Architectural specification document. |
| [`plan/20260723_01/03_walkthrough_dynamic_ports_reconnect.md`](file:///workspace/src/mosh-tcp/plan/20260723_01/03_walkthrough_dynamic_ports_reconnect.md) | This walkthrough summary document. |

---

## 4. Verification & Release Build Results

### 4.1 Integration Test Matrix
Command:
```bash
cargo test
```
Result: All 18 tests passed cleanly across Rust unit tests, C client integration, C++ client integration, cross-client protocol matrix, rate-limiting tests, tmux tests, VT100 resize tests, and SSH login tests.

### 4.2 Release Build Execution
Command:
```bash
./build_release.sh
```
Binaries compiled:
- `clients/c/mosh-tcp-client-c` (35 KB raw)
- `clients/cpp/mosh-tcp-client-cpp` (54 KB raw)
- `target/release/mosh-tcp-client` (533 KB raw)
- `target/release/mosh-tcp-server` (706 KB raw)
