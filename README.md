# mosh-tcp

[![CI](https://github.com/plops/mosh-tcp/actions/workflows/ci.yml/badge.svg)](https://github.com/plops/mosh-tcp/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)

A latency-tolerant, bandwidth-throttled terminal proxy (Client & Server) designed for ultra-slow connections (such as free 6 kB/s mobile internet or severe tethering caps) and **double-CGNAT environments** over **SSH tunnels**.


---

## 🎯 Purpose & Scope

Many mobile network providers offer low-speed free internet tiers (e.g., capped at 6 kB/s) or heavily throttled tethering connections with high round-trip latency. Using standard terminal tools over these links poses major challenges:

1. **The "Wall of Text" Problem**:
   Accidentally running `dmesg`, dumping a log file with `cat`, or using modern AI CLI tools (such as `kiro-cli` or `codex`) often sends tens or hundreds of kilobytes of output in a fraction of a second. On a 6 kB/s link, this output clogs the network buffer and completely freezes the interactive session for seconds or minutes.
2. **UDP & Double CGNAT Failures**:
   Standard `mosh` relies on UDP state synchronization. When both client and server sit behind Carrier-Grade NAT (CGNAT) without direct port forwarding or VPN relays, UDP packets are dropped or blocked.
3. **No Encryption Overhead / SSH Native**:
   Encryption and authentication are **intentionally out of scope** in `mosh-tcp`. Instead, `mosh-tcp` runs over standard **SSH port-forwarding tunnels**, leveraging existing SSH credentials and encryption while keeping client binaries tiny, dependency-free, and portable.

`mosh-tcp` solves these problems by performing **server-side output filtering, frame batching, and bandwidth throttling**, ensuring that interactive terminal sessions remain fluid and responsive even on the slowest connections.

---

## ✨ Key Features

* 🛡️ **Server-Side Flood Protection & Atomic Screen Refresh**:
  If a command produces a massive output burst exceeding internal server buffers (>16 KB), `mosh-tcp-server` discards the raw scrollback text and synthesizes a single compressed **atomic VT100 2D virtual screen refresh frame** (`\x1b[H\x1b[2J` with final screen state). The wall of text is filtered *at the server*, protecting your limited 6 kB/s line from clogging.
* ⚡ **Token Bucket Bandwidth Throttling**:
  Strictly caps outbound network traffic (configurable via `--max-kbps`, defaulting to 6 kB/s). Frames are suppressed when bandwidth quotas are reached.
* ⏱️ **Frame Rate Limiting (50 FPS / 20ms)**:
  Aggregates PTY outputs within a 20 ms window into a single TCP frame, preventing thousands of tiny TCP packet headers from overwhelming high-latency links.
* 🗜️ **Payload Compression**:
  Frames exceeding 128 bytes are automatically compressed using Deflate/Gzip. Lightweight clients include embedded decompression engines for zero-dependency compilation.
* ⌨️ **Predictive Local Echo (Rust Client)**:
  Renders typed characters locally with underline styling before server roundtrip confirmation, eliminating visual input lag on high-latency links.
* 🧰 **Multi-Client Ecosystem (C, C++, Rust)**:
  Provides standalone client binaries tailored for different target environments, from embedded OpenWrt routers to modern desktops.
* 🔒 **SSH Tunneling Friendly**:
  Uses framed TCP so sessions can easily be tunneled via `ssh -L 4000:localhost:4000` or reverse SSH tunnels (`ssh -R`).
* 🧹 **ANSI Terminal Query Stripping**:
  Filters out probe queries (`\x1b[>c`, `\x1b]10;?`, `\x1b]11;?`) at the server to prevent terminal state feedback loops or prompt corruption in `tmux` or `vim`.

---

## 📦 Binary Footprint & Client Variants

`mosh-tcp` provides three client implementations to match any deployment requirement:

| Target Binary | Language / Standard | Uncompressed Size | UPX LZMA Compressed | External Dependencies | Target Environment |
| :--- | :--- | :---: | :---: | :---: | :--- |
| **`mosh-tcp-client-c`** | POSIX C99 / C11 | 26 KB | **~14 KB** | **None** (Embedded `puff.c`) | OpenWrt routers, embedded devices, micro-VMs |
| **`mosh-tcp-client-cpp`** | Modern C++20 | 40 KB | **~19 KB** | **None** (Embedded `puff.c`) | Minimal Linux containers, resource-constrained hosts |
| **`mosh-tcp-client`** | Rust 2024 Edition | 504 KB | **~219 KB** | libc (`crossterm`) | Desktops, laptops, feature-rich clients (Predictive Echo) |
| **`mosh-tcp-server`** | Rust (Tokio Async) | 682 KB | **~284 KB** | PTY / Tokio | Remote Linux Server / VPS |

---

## 🚀 Quick Start & Usage

### Step 1: Start the Server on your Remote VPS

Run `mosh-tcp-server` on your remote VPS. By default, it listens on port `4000` and caps bandwidth at 6 kB/s:

```bash
# Basic usage (defaults: 0.0.0.0:4000, 50 FPS, 6 KB/s max bandwidth)
./mosh-tcp-server

# Custom parameters:
./mosh-tcp-server --bind 127.0.0.1:4000 --max-kbps 6 --fps 50 --stats --shell /bin/zsh
```

**Server Command-Line Options:**
* `--bind <ADDR:PORT>` / `-b`: Network interface and port to bind (default: `0.0.0.0:4000`).
* `--max-kbps <KB/S>`: Maximum network bandwidth limit in KB/s (default: `6`).
* `--fps <FPS>`: Frame rate in frames per second (default: `50`, equivalent to 20ms per frame).
* `--stats`: Enable real-time bandwidth, compression ratio, and skipped-frame telemetry printing.
* `--shell <PATH>`: Custom shell executable to launch (default: `$SHELL` or `/bin/bash`).

---

### Step 2: Establish an SSH Tunnel from your Local Machine

Since `mosh-tcp` relies on standard SSH for encryption and authentication, create a TCP port-forwarding tunnel to your VPS:

```bash
ssh -N -L 4000:localhost:4000 user@your-vps.com
```

*(If your client machine is behind CGNAT and cannot initiate direct connections, you can also use reverse port forwarding via `ssh -R`).*

---

### Step 3: Connect using your Preferred Client

Once the SSH tunnel is active, connect locally to `127.0.0.1:4000`:

#### Option A: C Client (Recommended for embedded / low-memory hosts)
```bash
./mosh-tcp-client-c --connect 127.0.0.1:4000
```

#### Option B: C++ Client (Modern C++20 implementation)
```bash
./mosh-tcp-client-cpp --connect 127.0.0.1:4000
```

#### Option C: Rust Client (Recommended for desktops with local predictive echo)
```bash
./mosh-tcp-client --connect 127.0.0.1:4000 --predict
```

**Client Command-Line Options:**
* `--connect <ADDR:PORT>` / `-c`: Target server address (default: `127.0.0.1:4000`).
* `--predict`: Enable 2D cell local predictive echo (Rust client only).

---

## 🏗️ Building from Source

### Automated Release Build
To compile and compress all binaries with UPX LZMA:

```bash
cd /workspace/src/mosh-tcp
./build_release.sh
```

Built binaries will be located in:
* `target/release/mosh-tcp-server`
* `target/release/mosh-tcp-client`
* `clients/c/mosh-tcp-client-c`
* `clients/cpp/mosh-tcp-client-cpp`

### Individual Builds

* **Rust Server & Client:**
  ```bash
  cargo build --release
  ```
* **C Client:**
  ```bash
  cd clients/c && make
  ```
* **C++ Client:**
  ```bash
  cd clients/cpp && make
  ```

---

## 🛠️ Architecture & Project Structure

* Detailed Architectural Document: [`doc/architecture.md`](doc/architecture.md)

```text
mosh-tcp/
├── src/
│   ├── bin/
│   │   ├── mosh_tcp_server.rs   # Tokio async server entry point
│   │   └── mosh_tcp_client.rs   # Standalone Rust client entry point
│   ├── server.rs                # PTY management, rate limiter, frame synthesizer
│   ├── client.rs                # crossterm terminal control & event loop
│   ├── protocol.rs              # Length-prefixed binary codec & compression
│   ├── predictive.rs            # 2D Cell local predictive echo engine
│   └── ansi.rs                  # ANSI terminal query filter
├── clients/
│   ├── c/                       # Standalone C99 client + puff.c inflate engine
│   └── cpp/                     # Standalone Modern C++20 client + puff.c
├── tests/                       # Integration test matrix (Rust, C, C++ clients)
├── doc/
│   └── architecture.md          # In-depth architectural specification
└── build_release.sh             # Unified compilation & UPX packaging script
```

### Key Components

* [`src/`](src/): Server and client Rust core modules.
* [`clients/c/`](clients/c/): Standalone POSIX C99 client implementation with zero external dependencies.
* [`clients/cpp/`](clients/cpp/): Modern C++20 client implementation with zero external dependencies.
* [`doc/architecture.md`](doc/architecture.md): Deep-dive architectural specification and data flow diagrams.
* [`build_release.sh`](build_release.sh): Build script for producing UPX-compressed release binaries.

---

## 🧪 Testing

`mosh-tcp` includes a comprehensive multi-layer test matrix covering packet framing, heavy output handling, bandwidth throttling (6 KB/s cap), window resizing (`SIGWINCH`), and tmux session integration across Rust, C, and C++ clients:

```bash
cargo build
cargo test
```

---

## 📜 License

Licensed under the **GNU General Public License v3.0 (GPLv3)**. See [LICENSE](LICENSE) for details.
