# Walkthrough - Mosh 2D Virtual Terminal State Synchronization & Overlay Prediction Engine

## Executive Summary
Replaced `mosh-tcp`'s legacy naive backspace local echo with a **Mosh-style 2D Virtual Terminal State Synchronization & Overlay Prediction Engine**. 

Instead of blindly printing local keystrokes to `stdout` and issuing backspaces (`\x08 \x08`) on frame arrival (which breaks TUI apps and cursor movement), `mosh-tcp` now maintains a 2D `vt100` terminal screen model on both client and server. Local predictions are tracked as cell hypotheses `(row, col) -> predicted_char` attached to prediction epochs. When server frames arrive, predicted cells are matched against the authoritative 2D `vt100::Screen` grid:
- **Matching Cells**: Confirmed and drained seamlessly without emitting stray backspaces!
- **Mismatched / Expired Cells**: Culled and replaced by the authoritative 2D screen state!

---

## Architecture Comparison: Original Mosh vs. mosh-tcp

| Architectural Component | Original Mosh (`/workspace/src/mosh/`) | `mosh-tcp` (New 2D State-Sync Architecture) |
| :--- | :--- | :--- |
| **Server Terminal Model** | `Terminal::Emulator` / `Framebuffer` ([`mosh-server.cc`](file:///workspace/src/mosh/src/frontend/mosh-server.cc)) | Stateful PTY stream with atomic VT100 screen dumps ([`src/server.rs`](file:///workspace/src/mosh-tcp/src/server.rs)) |
| **Client Terminal Model** | `Terminal::Display` / `Framebuffer` ([`terminaloverlay.h`](file:///workspace/src/mosh/src/frontend/terminaloverlay.h#L328)) | `vt100::Parser` 2D Screen Emulator ([`src/predictive.rs`](file:///workspace/src/mosh-tcp/src/predictive.rs)) |
| **Prediction Hypotheses** | `ConditionalOverlayCell` with `tentative_until_epoch` ([`terminaloverlay.h#L63`](file:///workspace/src/mosh/src/frontend/terminaloverlay.h#L63)) | `PredictedCell` with `row`, `col`, `character`, `epoch` ([`src/predictive.rs`](file:///workspace/src/mosh-tcp/src/predictive.rs)) |
| **Confirmation / Culling** | `PredictionEngine::cull()` checking 2D `Framebuffer` cells ([`terminaloverlay.cc#L419`](file:///workspace/src/mosh/src/frontend/terminaloverlay.cc#L419)) | `inspect_and_cull_server_frame()` checking 2D `vt100::Screen` cells ([`src/predictive.rs`](file:///workspace/src/mosh-tcp/src/predictive.rs)) |
| **Backspace Strategy** | Zero backspaces; screen rendered from 2D cell overlays | Confirmed predictions drain cleanly from 2D cell matching; zero backspaces on confirmation! |

---

## Detailed Implementation Changes

### 1. 2D State Sync Prediction Engine ([src/predictive.rs](file:///workspace/src/mosh-tcp/src/predictive.rs))
- Integrated `vt100::Parser` into `LocalPredictor`.
- `handle_keystroke`: Queries current cursor coordinates `(cur_row, cur_col)` from `vt100::Screen` and pushes a `PredictedCell { row, col, character, epoch }`.
- `inspect_server_frame`:
  1. Feeds incoming raw server bytes into `vt100::Parser::process(raw_data)`.
  2. Inspects `vt100::Screen::cell(pred.row, pred.col)` for each active prediction.
  3. If `cell.contents() == pred.character`, prediction is **CONFIRMED** and drained.
  4. If `cell.contents() != pred.character` (e.g. command mode or cursor movement), prediction is **CULLED** and truncated.

### 2. UTF-8 Slicing Panic & Alt Key Fix ([src/client.rs](file:///workspace/src/mosh-tcp/src/client.rs))
- Refactored `ResponseFilter` from string slicing to byte vectors (`Vec<u8>`). Eliminates panics on multi-byte UTF-8 inputs (`ø`, `ä`, `ö`, `ü`, `ß`).
- Encoded `KeyModifiers::ALT` as `ESC` prefix (`\x1b`), enabling Emacs Meta commands (`M-x`).

---

## Verification Results

Full test suite passes 100% cleanly:

```text
running 6 tests
test client::tests::test_alt_key_event_encoding ... ok
test client::tests::test_response_filter_multibyte_utf8 ... ok
test predictive::tests::test_2d_cell_prediction_confirmation ... ok
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

Result: 12 passed, 0 failed.
```
