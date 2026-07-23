# Architectural Specification: Dynamic Port Selection, Session Security & Auto-Reconnection

## 1. Executive Summary & Root Cause Analysis

### 1.1 Problem Statement
When launching `mosh-tcp-client-cpp user@host`, the server failed with:
```text
Error: Address already in use (os error 98)
```
This occurred because the client was instructing `mosh-tcp-server` to bind to fixed port `4000` (`--bind 127.0.0.1:4000`). If port 4000 was already bound by an existing `mosh-tcp-server` session, an orphaned background process, or another system service on `host`, startup failed immediately.

### 1.2 Architectural Solution
Following original `mosh` principles (as analyzed via `deepwiki` MCP from `mobile-shell/mosh`):
1. **Dynamic Ephemeral Port Allocation**: `mosh-tcp-server` binds to `127.0.0.1:0` by default. The operating system assigns a guaranteed available TCP port.
2. **Session Key Authentication**: `mosh-tcp-server` generates a cryptographically random 128-bit hex key (`session_key`). Only clients possessing this key via SSH stdout parsing are permitted to attach.
3. **Structured SSH Handshake Protocol**:
   `mosh-tcp-server` prints on stdout: `MOSH-TCP CONNECT <remote_port> <session_key> <pid>`
4. **Session Detachment & Persistence**: When a client disconnects (or network drops), `mosh-tcp-server` keeps the PTY session running in the background for up to 1 hour, allowing seamless client reconnection.
5. **Client Auto-Reconnection**: If network connectivity breaks, `mosh-tcp-client` automatically re-establishes the SSH tunnel, sends a handshake packet with `<session_key>`, and receives a full VT100 atomic screen refresh (`\x1b[H\x1b[2J`), restoring the terminal session without losing shell state.

---

## 2. Protocol & Binary Codec Extension

### 2.1 Packet Type Extension (`src/protocol.rs`)
Add Tag 6 (`ClientHandshake`) to `Packet` enum:

| Tag Byte | Name | Payload Structure | Description |
| :--- | :--- | :--- | :--- |
| `0x06` | `ClientHandshake` | `session_key` (string), `rows` (u16), `cols` (u16) | First packet sent by client upon TCP connect to authenticate and initialize window size. |

### 2.2 Server Handshake & Key Verification
Upon TCP connection acceptance, `mosh-tcp-server` waits up to 5 seconds for `ClientHandshake`.
- If `session_key` matches server's active key: connection is authenticated, terminal size set, and full screen frame emitted.
- If `session_key` does NOT match or times out: server immediately closes connection.

---

## 3. Server Component Details (`src/server.rs` & `src/bin/mosh_tcp_server.rs`)

### 3.1 Dynamic Port Selection
- `--bind <ADDR:PORT>` default: `127.0.0.1:0`.
- When port is `0`, `tokio::net::TcpListener::bind("127.0.0.1:0")` is called.
- Retrieve assigned port via `listener.local_addr()?.port()`.
- Generate random 128-bit key using fast random generator (`hex::encode(rand_bytes)` / 32 hex chars).
- Output startup line to stdout:
  `MOSH-TCP CONNECT <port> <session_key> <pid>`
- Flush stdout immediately (`io::stdout().flush()`).

### 3.2 Session Persistence & Disconnect Timeout
- PTY task continues running independently of TCP socket lifecycle.
- When TCP connection closes: `mosh-tcp-server` enters `Detached` state.
- If no client re-attaches within 3600 seconds (1 hour) or if child shell exits, `mosh-tcp-server` terminates.

---

## 4. Client Implementations (Rust, C, C++)

### 4.1 SSH Launch & Handshake Sequence
1. Spawns SSH subprocess:
   `ssh [user@]host "mosh-tcp-server --bind 127.0.0.1:0"`
2. Captures stdout of SSH process.
3. Parses line starting with `MOSH-TCP CONNECT <remote_port> <session_key> <pid>`.
4. Spawns SSH port forwarding tunnel:
   `ssh -N -L <local_port>:127.0.0.1:<remote_port> [user@]host`
5. Connects TCP socket to `127.0.0.1:<local_port>`.
6. Sends `ClientHandshake` (Tag 6) with `<session_key>`.

### 4.2 Auto-Reconnection Loop
If network error / EOF occurs on TCP socket:
1. Client displays banner: `[mosh-tcp client] Connection lost. Reconnecting...`
2. Retry loop attempts to re-establish SSH tunnel to `<remote_port>`.
3. If `<remote_port>` is refused (server process exited), client re-runs SSH launcher `mosh-tcp-server` (or attach mode).
4. Re-sends `ClientHandshake`.
5. Server sends atomic VT100 screen refresh frame, restoring screen state.

---

## 5. File Inventory & Implementation Roadmap

| Component | File Path | Scope of Modifications |
| :--- | :--- | :--- |
| Protocol Codec | [`src/protocol.rs`](file:///workspace/src/mosh-tcp/src/protocol.rs) | Add `Packet::ClientHandshake` (Tag 6) serialization & deserialization. |
| Server Core | [`src/server.rs`](file:///workspace/src/mosh-tcp/src/server.rs) | Update `run_server` for `127.0.0.1:0` dynamic port binding, random session key generation, stdout printing, handshake authentication, and PTY session detachment/persistence. |
| Server Binary | [`src/bin/mosh_tcp_server.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh_tcp_server.rs) | Change default `--bind` address to `127.0.0.1:0`. |
| Rust Client Core | [`src/client.rs`](file:///workspace/src/mosh-tcp/src/client.rs) | Add SSH stdout parsing for `MOSH-TCP CONNECT`, `ClientHandshake` packet generation, and auto-reconnection loop. |
| Rust Client Binary | [`src/bin/mosh_tcp_client.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh_tcp_client.rs) | Update SSH orchestration workflow. |
| POSIX C Client | [`clients/c/mosh_tcp_client.c`](file:///workspace/src/mosh-tcp/clients/c/mosh_tcp_client.c) | Add SSH pipe stdout parsing for `MOSH-TCP CONNECT`, Tag 6 handshake packet building, and reconnect retry loop. |
| C++20 Client | [`clients/cpp/mosh_tcp_client.cpp`](file:///workspace/src/mosh-tcp/clients/cpp/mosh_tcp_client.cpp) | Update stdio pipe parsing, Tag 6 `ClientHandshake` handling, and reconnect loop. |
| Test Matrix | [`tests/test_ssh_login.rs`](file:///workspace/src/mosh-tcp/tests/test_ssh_login.rs) | Add tests for dynamic port binding, key authentication failure/success, and reconnection after network drop. |
