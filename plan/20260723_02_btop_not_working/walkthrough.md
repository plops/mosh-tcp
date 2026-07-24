# Walkthrough - Fix `btop` Rendering & Emacs Bracketed Paste in `mosh-tcp`

## Executive Summary
This document summarizes the investigation, architectural fixes, unit and integration test implementations, and learnings for resolving two critical user-reported issues in `mosh-tcp`:
1. **Garbled `btop` UI rendering in Alacritty** (unrecognizable structure, scrambled process/tree/filter menus).
2. **Emacs Paste failure** (`Wrong type argument: char-or-string-p, 134217826`).

Both issues have been completely fixed and verified using unit tests, protocol integration tests, and matrix testing.

---

## 1. Deep Technical Root Cause Analysis

### Issue A: `btop` Distortion & Garbled Structure in Alacritty
* **Symptom**: When launching `btop` over `mosh-tcp` inside Alacritty, the layout broke, displaying active filters (`proc`, `filter`, `tree`, `cpu lazy`, `0/531`) and scrambled table columns (`btop-weird.txt`).
* **Root Cause**:
  1. Upon startup, `btop` emits terminal capability query sequences: Primary Device Attributes (`\x1b[c` / `\x1b[0c`), Secondary Device Attributes (`\x1b[>c`), Tertiary Device Attributes (`\x1b[=c`), XTVERSION (`\x1b[>q`), Cursor Position Reports (`\x1b[6n`), and OSC color/palette queries (`\x1b]10;?`, `\x1b]11;?`, `\x1b]4;...`).
  2. Previously, `mosh-tcp` server's `strip_terminal_queries_stateful` only filtered basic `\x1b]10;` and `\x1b]11;` sequences. As a result, `btop`'s query escape codes were forwarded over the PTY network stream to the client terminal (Alacritty).
  3. Alacritty responded to these queries by writing response escape sequences (`\x1b[?63;1;2c`, `\x1b[24;80R`, `\x1b]10;rgb:...`) to standard input (`stdin`).
  4. On the client side, Crossterm captured these raw stdin response bytes and translated them into `crossterm::event::Event::Key(...)` structures (misinterpreting `\x1b` as `Esc` or setting `ALT` modifier flags on subsequent digits and letters).
  5. The client re-encoded these key events and sent them back to the server as user keystrokes! In `btop`, characters like `p`, `f`, `t`, `c`, `l`, `z`, `0` triggered internal keybindings (toggling tree view, activating process filters, changing CPU view), destroying `btop`'s state and rendering layout.

### Issue B: Emacs Paste Error (`Wrong type argument: char-or-string-p, 134217826`)
* **Symptom**: Pasting text into Emacs running in `mosh-tcp` failed with `Wrong type argument: char-or-string-p, 134217826`.
* **Root Cause**:
  1. `mosh-tcp` client enables raw mode and bracketed paste via Crossterm.
  2. When pasting text in Alacritty, Crossterm intercepts `\x1b[200~...` ... `\x1b[201~` and produces `Event::Paste(text)`.
  3. `client.rs` stripped the bracketed paste markers (`\x1b[200~` and `\x1b[201~`) and sent raw unencapsulated `text` bytes to the PTY.
  4. However, Emacs enables bracketed paste mode (`\x1b[?2004h`) in its PTY buffer and expects pasted content to be explicitly bounded by `\x1b[200~` and `\x1b[201~`.
  5. Receiving raw bytes while in bracketed paste mode caused Crossterm or Emacs to parse sequence prefixes as `Meta` key events. In Emacs Lisp key representation, `134217826` corresponds to `0x8000062` (`Meta` + `'b'`). Emacs attempted to process integer key code `134217826` in a string/char context, throwing a Lisp type crash.

---

## 2. Implemented Changes & Architecture Fixes

### 1. Stateful Server-Side ANSI Query Stripping ([src/ansi.rs](file:///workspace/src/mosh-tcp/src/ansi.rs))
* Updated `strip_terminal_queries_stateful(data: &[u8])` to perform stateful, multi-protocol query stripping on the PTY output stream before frame generation:
  - **CSI Queries**: Strips `\x1b[c`, `\x1b[0c`, `\x1b[?c` (DA1), `\x1b[>c`, `\x1b[>0c`, `\x1b[>1c` (DA2), `\x1b[=c`, `\x1b[=0c` (DA3), `\x1b[>q`, `\x1b[q` (XTVERSION), `\x1b[6n`, `\x1b[?6n` (CPR), and parameterised DA/n queries (`\x1b[?...c`, `\x1b[?...n`).
  - **OSC Queries**: Strips `\x1b]10;`, `\x1b]11;`, `\x1b]12;`, `\x1b]4;` color/palette queries as well as any OSC string containing query character `?`.
  - **DCS Queries**: Strips DECRQSS status queries (`\x1bP$q...\x1b\`).

### 2. Client-Side Bracketed Paste Encapsulation ([src/client.rs](file:///workspace/src/mosh-tcp/src/client.rs))
* Updated `Event::Paste(text)` handling in `run_client_stream_handshake`:
  - When Crossterm emits `Event::Paste(text)`, `client.rs` now properly wraps the text payload in standard VT100 / xterm bracketed paste markers: `\x1b[200~` + `text.as_bytes()` + `\x1b[201~`.
  - Remote PTY applications (Emacs, Vim, Bash, Zsh, Tmux) receive complete bracketed paste chunks, preventing character key mangling and eliminating the `134217826` Emacs error.

### 3. Client Stdin Response Filtering ([src/client.rs](file:///workspace/src/mosh-tcp/src/client.rs))
* Enhanced `ResponseFilter::filter`:
  - Added pattern matching to drop residual DA (`0;...c`, `\x1b[?...c`) and CPR (`\x1b[...;...R`) escape responses on stdin, ensuring terminal response sequences never corrupt the input stream.

---

## 3. Verification & Test Suite

### Unit & Integration Tests Added
1. **`src/ansi.rs` Unit Tests**:
   - `test_strip_terminal_queries_stateful_da_and_cpr`: Verifies that DA1, DA2, DA3, and CPR queries are completely stripped while preserving normal PTY data.
   - `test_strip_terminal_queries_osc_color`: Verifies stateful stripping of OSC 10/11 color queries terminated by BEL (`0x07`) or ST (`\x1b\`).
   - `test_strip_terminal_queries_partial_sequence`: Verifies stateful buffering when sequence fragments are split across packet boundaries.
2. **`src/client.rs` Unit Tests**:
   - `test_bracketed_paste_encapsulation_format`: Verifies bracketed paste payload framing (`\x1b[200~` and `\x1b[201~`).
3. **`tests/test_btop_emacs.rs` Integration Test Suite**:
   - `test_btop_initialization_queries_are_stripped`: Simulates full `btop` startup sequence and verifies 100% query stripping.
   - `test_emacs_bracketed_paste_sequence_integrity`: Tests paste payload framing and byte-exact preservation.

### Test Execution Results
All test targets executed cleanly and passed:
- `tests/test_btop_emacs.rs`: **2 passed**
- `tests/integration.rs`: **2 passed**
- `tests/integration_matrix.rs`: **1 passed**
- `tests/test_browsh.rs`: **1 passed**
- `tests/test_c_client.rs`: **1 passed**
- `tests/test_cpp_client.rs`: **1 passed**
- `tests/test_predictive.rs`: **2 passed**
- `tests/test_rate_limit.rs`: **2 passed**
- `tests/test_ssh_login.rs`: **6 passed**
- `tests/test_tmux.rs`: **1 passed**
- `tests/test_vt100_resize.rs`: **1 passed**

---

## 4. Learnings & Future Enhancements

### Learnings
1. **Interactive TUI Feedback Loops**: Terminal applications querying capabilities over PTY stdout must have their query requests filtered server-side. Otherwise, local client terminals write escape responses back to stdin, which Crossterm converts into spurious keystrokes.
2. **PTY Bracketed Paste Expectations**: Terminal applications (like Emacs) expect bracketed paste markers (`\x1b[200~` / `\x1b[201~`) passed directly to their PTY when bracketed paste mode is active (`\x1b[?2004h`). Stripping these markers leads to modifier byte misinterpretation (`Meta+b` / `134217826`).

### Potential Future Enhancements
- **Dynamic Terminal Query Proxying**: Synthesize local terminal answers (such as current palette or term version) directly on the server without sending queries down to the client.
- **Extended Paste Chunking**: Support large multi-megabyte pastes with adaptive throttling to maintain PTY responsiveness during massive clipboard pastes.
