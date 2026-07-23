# Walkthrough: Streamlined SSH Login Implementation for mosh-tcp

## 1. Overview & Accomplishments

The login process for `mosh-tcp` has been successfully streamlined across all three client implementations (**Rust**, **POSIX C99**, and **Modern C++20**).

Users can now initiate a complete `mosh-tcp` session over an SSH tunnel with a single command:
```bash
mosh-tcp-client user@remote-vps.com --predict
mosh-tcp-client-c user@remote-vps.com
mosh-tcp-client-cpp user@remote-vps.com
```

### Key Capabilities Implemented:
1. **Automated SSH Subprocess Management**:
   - Spawns the client system's `ssh` binary using `-o ExitOnForwardFailure=yes -L <local_port>:127.0.0.1:<remote_port>`.
   - Starts `mosh-tcp-server --bind 127.0.0.1:<remote_port>` on the remote server host.
2. **Dynamic Local Port Selection**:
   - Automatically finds an open loopback port (`127.0.0.1`) on the client machine to avoid port collisions when multiple sessions or local services are active.
3. **Connection Handshake & Retry Loop**:
   - Polls `127.0.0.1:<local_port>` up to 15 seconds while checking SSH process liveness.
   - Switches client terminal to raw mode only after the TCP connection to `mosh-tcp-server` is established.
4. **Clean Termination & Signal Handling**:
   - Automatically terminates child `ssh` process (which shuts down the remote `mosh-tcp-server`) when the client session ends or when SIGINT/SIGTERM is received.
5. **Backward Compatibility**:
   - Legacy direct TCP socket connection (`--connect <ADDR:PORT>`) remains fully supported across all clients.

---

## 2. Implementation & File Changes Summary

| Target / File | Key Changes Made |
| :--- | :--- |
| [`src/client.rs`](file:///workspace/src/mosh-tcp/src/client.rs) | Added `SshTunnel` supervisor struct and `run_client_stream(TcpStream, bool)` function. |
| [`src/bin/mosh_tcp_client.rs`](file:///workspace/src/mosh-tcp/src/bin/mosh_tcp_client.rs) | Updated CLI option parser for positional `[user@]host` and SSH flags (`--ssh`, `--ssh-port`, `--server`, `--port`). |
| [`clients/c/mosh_tcp_client.c`](file:///workspace/src/mosh-tcp/clients/c/mosh_tcp_client.c) | Added `fork()` + `execvp()` SSH process manager (`g_ssh_pid`), `find_free_port()`, signal/exit cleanup handler (`atexit(cleanup_ssh)`), and SSH flag parsing. |
| [`clients/cpp/mosh_tcp_client.cpp`](file:///workspace/src/mosh-tcp/clients/cpp/mosh_tcp_client.cpp) | Implemented `SshTunnel` RAII process manager guard, `find_free_port()`, and updated option parsing. |
| [`tests/mock_ssh.sh`](file:///workspace/src/mosh-tcp/tests/mock_ssh.sh) | Created mock SSH executable for non-interactive local automated testing. |
| [`tests/test_ssh_login.rs`](file:///workspace/src/mosh-tcp/tests/test_ssh_login.rs) | Created integration test suite verifying SSH login execution across Rust, C, and C++ clients. |
| [`README.md`](file:///workspace/src/mosh-tcp/README.md) | Updated Quick Start documentation and command-line usage guides. |
| [`doc/architecture.md`](file:///workspace/src/mosh-tcp/doc/architecture.md) | Updated architectural specifications and process lifecycle documentation. |

---

## 3. Verification & Test Results

### 3.1 New SSH Integration Test Suite
Command:
```bash
cargo test --test test_ssh_login
```
Output:
```text
running 3 tests
test test_c_client_ssh_login ... ok
test test_cpp_client_ssh_login ... ok
test test_rust_client_ssh_login ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.09s
```

### 3.2 Full Test Matrix Execution
Command:
```bash
cargo test
```
Result: All 18 tests passed across Rust unit tests, C client integration, C++ client integration, cross-client protocol matrix, rate-limiting tests, tmux tests, VT100 resize tests, and SSH login tests.

### 3.3 Release Binary Build Verification
Command:
```bash
./build_release.sh
```
Built binaries:
- `clients/c/mosh-tcp-client-c` (31 KB raw)
- `clients/cpp/mosh-tcp-client-cpp` (50 KB raw)
- `target/release/mosh-tcp-client` (527 KB raw)
- `target/release/mosh-tcp-server` (701 KB raw)

---

## 4. Deviations & Lessons Learned

1. **Terminal Raw Mode Timing**:
   - Initially, raw mode was enabled before spawning SSH. However, keeping canonical terminal mode during SSH invocation allows SSH to interactively prompt for host key verification or passwords if necessary.
   - Raw mode is now enabled immediately after the local TCP stream connects to the SSH tunnel.

2. **Port Allocation Safety**:
   - Using static port 4000 for local SSH tunnel endpoints could lead to `EADDRINUSE` if multiple clients ran on the same machine or if `mosh-tcp-server` was also running locally.
   - Dynamic port lookup (`bind(0)` / `TcpListener::bind("127.0.0.1:0")`) was introduced to guarantee unique, collision-free local tunnel ports.

---

## 5. Git Commit History

The following Conventional Commits were created during implementation:
- `feat(client): implement streamlined SSH login for Rust client`
- `feat(c-client): implement streamlined SSH login for C client`
- `feat(cpp-client): implement streamlined SSH login for C++ client`
- `docs(all): update documentation and integration tests for streamlined SSH login`
