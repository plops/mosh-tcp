# Walkthrough - Dynamic Local Predictor Suspension, Mosh Source Code Analysis & UTF-8 Panic Fix

## 1. Summary of Changes & Fixes

### A. UTF-8 Slicing Panic & Alt Key Fix (`src/client.rs`)
- **Root Cause**: `ResponseFilter::filter` previously stored data in a `String` and sliced it byte-by-byte (`&self.buffer[idx..]`). When multi-byte UTF-8 characters (e.g. `ø` `[0xC3, 0xB8]`, `ä`, `ö`, `ü`, `ß`) or Meta/Alt key sequences were received, slicing at a non-character boundary caused a fatal Rust panic: `start byte index 1 is not a char boundary; it is inside 'ø' (bytes 0..2)`.
- **Fix**:
  1. Refactored `ResponseFilter` to operate directly on `Vec<u8>` rather than `String`, eliminating all string slicing panic risks.
  2. Pushed byte-accurate clean input directly into `clean_data` without lossy UTF-8 conversions.
  3. Added `KeyModifiers::ALT` support in `key_event_to_bytes`, encoding Meta combinations like Emacs `M-x` as `\x1bx` (`[27, 120]`).

### B. Dynamic Predictor Suspension (`src/predictive.rs` & `src/client.rs`)
- **Root Cause**: `mosh-tcp`'s local predictor previously printed printable input keystrokes with underline formatting directly to `stdout` and relied on backspace sequences (`\x08 \x08`) to erase predictions upon receiving server frames. When full-screen TUI programs (Emacs, Vim, Browsh, Tmux) were active, they moved the terminal cursor dynamically across the screen using ANSI escape sequences. Issuing local echo predictions or clearing them with backspaces while in an alternate screen buffer or after cursor movement caused misplaced backspaces and rendered garbage characters.
- **Fix**:
  1. Added `suspend_predictions: bool` state to `LocalPredictor`.
  2. Added `inspect_server_frame(&mut self, raw_data: &[u8], stdout: &mut io::Stdout)` to inspect incoming server frames for Alternate Screen Buffer enable/disable sequences:
     - **Enable sequences**: `\x1b[?1049h`, `\x1b[?1047h`, `\x1b[?47h` (flushes active predictions, suspends predictor).
     - **Disable sequences**: `\x1b[?1049l`, `\x1b[?1047l`, `\x1b[?47l` (re-enables predictor for standard shell sessions).
     - **Atomic redraw frames**: `\x1b[H\x1b[2J` or `\x1b[2J` (resets prediction counters without backspacing into redrawn screens).
  3. Cleared local predictions whenever escape sequences (`0x1b`) or control characters were typed before the cursor moved.

---

## 2. Reviewer Evaluation & Mosh Source Code Analysis

### Was the reviewer right?
**Yes, the reviewer's explanation of original Mosh's architecture and prediction mechanisms is 100% accurate.**

### Deep Dive into Mosh C++ Codebase (`/workspace/src/mosh/`)

1. **State Synchronization Engine (2D Grid)**:
   - *Reviewer claim*: Mosh runs a full terminal emulator on both server and client, syncing 2D screen states rather than streaming raw bytes.
   - *Mosh Source Code Reference*:
     - Server: [`mosh-server.cc`](file:///workspace/src/mosh/src/frontend/mosh-server.cc) and [`terminalframebuffer.h`](file:///workspace/src/mosh/src/terminal/terminalframebuffer.h#L100-L200) (`class Framebuffer`). The server parses PTY output into an authoritative 2D grid of styled `Cell` objects.
     - Transport: [`transportsender.h`](file:///workspace/src/mosh/src/network/transportsender.h) and [`src/statesync/`](file:///workspace/src/mosh/src/statesync/) send diffs of `Terminal::State` over UDP.
     - Client: [`stmclient.cc`](file:///workspace/src/mosh/src/frontend/stmclient.cc) and [`terminaloverlay.h`](file:///workspace/src/mosh/src/frontend/terminaloverlay.h#L328-L346) (`class OverlayManager`).

2. **Prediction Engine, Hypotheses & Epochs**:
   - *Reviewer claim*: Mosh creates local predictions as "hypotheses" assigned to epochs, comparing them with server state updates to confirm or overwrite predictions.
   - *Mosh Source Code Reference*:
     - [`terminaloverlay.h`](file:///workspace/src/mosh/src/frontend/terminaloverlay.h#L63-L75): `ConditionalOverlayCell` stores `tentative_until_epoch` (epoch boundary) and predicted `replacement` `Cell`.
     - [`terminaloverlay.h`](file:///workspace/src/mosh/src/frontend/terminaloverlay.h#L242-L243): `prediction_epoch` and `confirmed_epoch` track prediction age.
     - [`terminaloverlay.cc`](file:///workspace/src/mosh/src/frontend/terminaloverlay.cc#L865-L875): `PredictionEngine::become_tentative()` advances the `prediction_epoch`.
     - [`terminaloverlay.cc`](file:///workspace/src/mosh/src/frontend/terminaloverlay.cc#L419-L565): `PredictionEngine::cull()` validates predicted cells against the authoritative server `Framebuffer`. If cells match (`Correct`), `confirmed_epoch` advances. If mismatched or expired (`IncorrectOrExpired`), `kill_epoch()` or `reset()` clears the invalid predictions from the local overlay grid.

3. **Heuristic Prediction Suppression**:
   - *Reviewer claim*: Mosh turns off predictions when complex escape sequences or control actions occur.
   - *Mosh Source Code Reference*:
     - [`terminaloverlay.cc`](file:///workspace/src/mosh/src/frontend/terminaloverlay.cc#L797-L828): In `Parser::Execute`, `Parser::Esc_Dispatch`, or unhandled `Parser::CSI_Dispatch`, Mosh calls `become_tentative()`, immediately marking predictions as tentative or hiding them.

4. **Relevance to `mosh-tcp`**:
   - Because `mosh-tcp` streams raw byte chunks over TCP rather than maintaining a dual 2D `Framebuffer` state-sync engine like original Mosh, sending raw backspaces (`\x08 \x08`) while TUI apps move the cursor corrupts the screen.
   - The reviewer's proposed fix—detecting Alternate Screen Buffer toggles (`\x1b[?1049h`) to dynamically suspend prediction—is the correct design for `mosh-tcp`'s architecture, providing robust TUI support (Emacs, Vim, Browsh, Tmux) while retaining fast local predictive echo in standard shell sessions.

---

## 3. Verification & Test Results

All 11 unit and integration test cases run cleanly with `cargo test`:

```text
running 5 tests
test client::tests::test_alt_key_event_encoding ... ok
test client::tests::test_response_filter_multibyte_utf8 ... ok
test predictive::tests::test_atomic_frame_resets_predictions ... ok
test predictive::tests::test_predictor_disabled_by_default ... ok
test predictive::tests::test_predictor_suspension_toggle ... ok

running 1 test
test test_server_editing_and_heavy_output ... ok

running 1 test
test test_tmux_attach_session_over_mosh_tcp ... ok

running 2 tests
test test_alternate_screen_detection_and_suspension ... ok
test test_control_character_clears_predictions ... ok

running 1 test
test test_bandwidth_throttling_and_frame_skipping ... ok

running 1 test
test test_tmux_session ... ok

Result: 11 passed, 0 failed.
```
