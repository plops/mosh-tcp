# Implementation Plan: Streamlined SSH Login for mosh-tcp

## 1. Overview & Goal

Currently, using `mosh-tcp` requires users to manually execute a two-step login process:
1. Manually launching `mosh-tcp-server` on the remote server via SSH.
2. Manually creating an SSH tunnel (`ssh -N -L 4000:localhost:4000 ...`).
3. Connecting the local `mosh-tcp-client` to `127.0.0.1:4000`.

The objective of this specification and implementation plan is to **streamline the login process** directly inside the `mosh-tcp` clients (`mosh-tcp-client` in Rust, `mosh-tcp-client-c` in POSIX C, and `mosh-tcp-client-cpp` in C++20). 

When executed with a destination argument like `mosh-tcp-client user@host`, the client will automatically:
1. Allocate a local port for the SSH tunnel.
2. Spawn the system `ssh` executable to connect to `user@host`, set up local port forwarding `-L <local_port>:127.0.0.1:<remote_port>`, and execute `mosh-tcp-server --bind 127.0.0.1:<remote_port>` on the remote host.
3. Automatically wait for the tunnel and remote server to become ready, then connect to `127.0.0.1:<local_port>`.
4. Gracefully terminate the child `ssh` process (and remote server session) upon client exit.

Existing direct connection functionality via `--connect <ADDR:PORT>` / `-c <ADDR:PORT>` is fully preserved for backward compatibility and manual/custom tunneling setups.

---

## 2. Command Line Interface (CLI) Specification

### 2.1 Usage Syntax
Across all client implementations (Rust, C, C++), the command syntax is standardized as follows:

```bash
# Streamlined SSH login mode
mosh-tcp-client [options] [user@]host [-- [server_options]]
mosh-tcp-client-c [options] [user@]host
mosh-tcp-client-cpp [options] [user@]host

# Direct socket connection mode (legacy/manual)
mosh-tcp-client --connect <ADDR:PORT> [options]
```

### 2.2 Standard Option Flags

| Short Flag | Long Flag | Description | Default Value |
| :--- | :--- | :--- | :--- |
| `-c` | `--connect <ADDR:PORT>` | Direct TCP connection to existing mosh-tcp server (bypasses SSH) | `None` (if destination host provided) / `127.0.0.1:4000` (if `--connect` alone) |
| `-s` | `--ssh <COMMAND>` | SSH client command/path to execute | `ssh` |
| `-p` | `--ssh-port <PORT>` | Remote SSH server port | `22` (passed to SSH via `-p <PORT>`) |
| | `--server <COMMAND>` | Command to start `mosh-tcp-server` on remote host | `mosh-tcp-server` |
| | `--port <PORT>` | Remote port for `mosh-tcp-server` to bind to on `127.0.0.1` | `4000` |
| | `--predict` | Enable 2D cell local predictive echo (Rust client only) | `false` |
| `-h` | `--help` | Display usage information and exit | N/A |

---

## 3. SSH Tunneling Architecture & Process Lifecycle

```
┌────────────────────────────────────────────────────────────────────────┐
│ Client Machine                                                         │
│                                                                        │
│ ┌──────────────────┐  spawn   ┌─────────────────────────────────────┐  │
│ │ mosh-tcp-client  ├─────────►│ ssh -L <local_port>:127.0.0.1:<port>│  │
│ └────────┬─────────┘          │       [user@]host ...               │  │
│          │ connect            └──────────────────┬──────────────────┘  │
│          ▼                                       │ SSH Tunnel Encrypted│
│   127.0.0.1:<local_port>                         │ (Port 22 / TCP)     │
└──────────┼───────────────────────────────────────┼─────────────────────┘
           │                                       │
           └───────────────────┐                   │
                               ▼                   ▼
┌────────────────────────────────────────────────────────────────────────┐
│ Remote VPS / Server Host                                               │
│                                                                        │
│                               ┌─────────────────────────────────────┐  │
│                               │ mosh-tcp-server                     │  │
│                               │ --bind 127.0.0.1:<remote_port>      │  │
│                               └─────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Step-by-Step Lifecycle

1. **Argument Parsing & Mode Selection**:
   - If positional argument `[user@]host` is present (and no `--connect` is explicitly provided), client enters **SSH Streamlined Mode**.
   - If `--connect` is supplied, client enters **Direct Connection Mode**.

2. **Local Port & Remote Command Assembly**:
   - Client picks a local port (e.g. dynamic port or `4000`, retrying if bound).
   - SSH Command construction:
     ```bash
     ssh -o ExitOnForwardFailure=yes -L <local_port>:127.0.0.1:<remote_port> [-p <ssh_port>] [user@]host "mosh-tcp-server --bind 127.0.0.1:<remote_port>"
     ```

3. **Subprocess Execution & Tunnel Wait**:
   - Client spawns `ssh` as a child process.
   - Client polls `127.0.0.1:<local_port>` with a retry loop (e.g. 50ms interval, up to 10 seconds timeout) to establish the TCP connection once SSH port forwarding is active.

4. **Interactive Terminal Session**:
   - Once connected, client switches terminal to raw mode (`termios` / `crossterm`) and starts state loop.

5. **Cleanup & Termination**:
   - When client exits (user disconnects or SIGINT/SIGTERM received), terminal is restored to canonical mode.
   - Client sends SIGTERM / kills child SSH process, causing remote SSH session and remote `mosh-tcp-server` to terminate cleanly.

---

## 4. Comprehensive File Inventory

The following files are relevant, touched, or created during this implementation:

| File Path | Description / Role in Task |
| :--- | :--- |
| [`src/bin/mosh_tcp_client.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh_tcp_client.rs) | **Modified**: Primary CLI entry point for Rust client. Updated to parse positional `[user@]host` and SSH flags (`--ssh`, `--ssh-port`, `--server`, `--port`), spawn SSH child process, and handle cleanup. |
| [`src/client.rs`](file:///workspace/src/mosh-tcp/src/client.rs) | **Modified**: Rust client engine. Updated helper functions for connecting via socket or managing SSH process lifecycle. |
| [`clients/c/mosh_tcp_client.c`](file:///workspace/src/mosh-tcp/clients/c/mosh_tcp_client.c) | **Modified**: POSIX C99 client. Added `fork()` + `execvp()` process management for `ssh`, SSH option parsing, local tunnel polling, and `SIGCHLD`/exit cleanup. |
| [`clients/c/Makefile`](file:///workspace/src/mosh-tcp/clients/c/Makefile) | **Relevant**: Build rule for C client (`mosh-tcp-client-c`). |
| [`clients/cpp/mosh_tcp_client.cpp`](file:///workspace/src/mosh-tcp/clients/cpp/mosh_tcp_client.cpp) | **Modified**: Modern C++20 client. Added process management for `ssh` using POSIX process APIs / `fork`+`execvp`, SSH flag parsing, and cleanup. |
| [`clients/cpp/Makefile`](file:///workspace/src/mosh-tcp/clients/cpp/Makefile) | **Relevant**: Build rule for C++ client (`mosh-tcp-client-cpp`). |
| [`tests/test_ssh_login.rs`](file:///workspace/src/mosh-tcp/tests/test_ssh_login.rs) | **New**: Comprehensive integration test suite testing SSH login argument parsing, tunnel connection helper logic, and child process cleanup across Rust, C, and C++ clients. |
| [`tests/integration_matrix.rs`](file:///workspace/src/mosh-tcp/tests/integration_matrix.rs) | **Modified**: Updated cross-client integration test matrix to verify new CLI options across Rust, C, and C++ clients. |
| [`README.md`](file:///workspace/src/mosh-tcp/README.md) | **Modified**: Updated Quick Start, Usage, and CLI flag documentation to reflect streamlined `mosh-tcp-client [user@]host` workflow. |
| [`doc/architecture.md`](file:///workspace/src/mosh-tcp/doc/architecture.md) | **Modified**: Updated architectural data flow diagrams and network connection lifecycle documentation. |
| [`plan/20260723_01/01_implementation_plan_ssh_streamlined_login.md`](file:///workspace/src/mosh-tcp/plan/20260723_01/01_implementation_plan_ssh_streamlined_login.md) | **New**: This implementation plan document. |
| [`plan/20260723_01/02_walkthrough.md`](file:///workspace/src/mosh-tcp/plan/20260723_01/02_walkthrough.md) | **New**: Final walkthrough and verification summary document created upon completion. |

---

## 5. Implementation Roadmap & Milestones

### Phase 1: Rust Client SSH Login Implementation (Step 1)
- Add command-line argument parsing for `[user@]host`, `--ssh`, `--ssh-port`, `--server`, `--port` in `src/bin/mosh_tcp_client.rs`.
- Implement `SshTunnel` supervisor struct in `src/client.rs` to spawn SSH sub-process, wait for local port readiness, and kill SSH process on drop.
- Add tests in `tests/test_ssh_login.rs` to verify Rust client argument parsing and SSH connection creation.
- **Commit**: `feat(client): implement streamlined SSH login for Rust client`

### Phase 2: POSIX C Client SSH Login Implementation (Step 2)
- Add argument parsing for `[user@]host`, `-s`/`--ssh`, `-p`/`--ssh-port`, `--server`, `--port` in `clients/c/mosh_tcp_client.c`.
- Implement `fork()` + `execvp()` SSH subprocess execution, TCP port retry loop, and `SIGINT`/`SIGTERM`/`exit` signal cleanup.
- Add test coverage for C client SSH CLI behavior in `tests/test_ssh_login.rs`.
- **Commit**: `feat(c-client): implement streamlined SSH login for C client`

### Phase 3: C++20 Client SSH Login Implementation (Step 3)
- Add argument parsing and SSH tunnel process manager to `clients/cpp/mosh_tcp_client.cpp`.
- Ensure clean child process termination and resource cleanup in C++ RAII guard.
- Add test coverage for C++ client SSH CLI behavior in `tests/test_ssh_login.rs`.
- **Commit**: `feat(cpp-client): implement streamlined SSH login for C++ client`

### Phase 4: Integration Matrix, Documentation & Release Verification (Step 4)
- Update `tests/integration_matrix.rs` to validate all three clients with new SSH login syntax.
- Update `README.md` and `doc/architecture.md` with streamlined SSH usage examples and diagrams.
- Run release build script `build_release.sh` and execute full test matrix `cargo test`.
- **Commit**: `docs(all): update documentation and integration matrix for SSH streamlined login`

### Phase 5: Walkthrough & Verification Summary (Step 5)
- Generate `/workspace/src/mosh-tcp/plan/20260723_01/02_walkthrough.md` detailing implementation details, test results, and any deviations.
- Mark goal complete.
