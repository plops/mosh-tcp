# Walkthrough - Dynamic Local Predictor Suspension & TUI Support

## Summary of Changes
Resolved layout corruption and spurious character insertion ("komische Zeichen") when running full-screen TUI applications (such as Emacs, Vim, Tmux, and Browsh) under `mosh-tcp` with local prediction enabled (`--predict`).

### Root Cause Analysis
`mosh-tcp`'s local predictor previously printed printable input keystrokes with underline formatting directly to `stdout` at the cursor's current position and relied on backspace sequences (`\x08 \x08`) to erase predictions upon receiving server frames. When full-screen TUI programs (Emacs, Vim, Browsh, Tmux) were active, they moved the terminal cursor dynamically across the screen using ANSI escape sequences. Issuing local echo predictions or clearing them with backspaces while in an alternate screen buffer or after cursor movement caused misplaced backspaces and rendered garbage characters.

### Key Changes
1. **Dynamic Predictor Suspension ([src/predictive.rs](file:///workspace/src/mosh-tcp/src/predictive.rs))**:
   - Added `suspend_predictions: bool` state to `LocalPredictor`.
   - Added `inspect_server_frame(&mut self, raw_data: &[u8], stdout: &mut io::Stdout)` to inspect incoming server frames for Alternate Screen Buffer enable/disable sequences:
     - **Enable sequences**: `\x1b[?1049h`, `\x1b[?1047h`, `\x1b[?47h` (automatically flushes active predictions, suspends predictor).
     - **Disable sequences**: `\x1b[?1049l`, `\x1b[?1047l`, `\x1b[?47l` (re-enables predictor for standard shell sessions).
     - **Atomic redraw frames**: `\x1b[H\x1b[2J` or `\x1b[2J` (resets prediction counters without backspacing into redrawn screens).
2. **Escape Sequence & Control Character Protection**:
   - In `handle_keystroke`: If input bytes contain `0x1b` (ESC) or control characters (like Arrow keys, Alt sequences, etc.), local predictions are immediately cleared before cursor movement and no local echo is performed for the sequence.
3. **Client Integration ([src/client.rs](file:///workspace/src/mosh-tcp/src/client.rs))**:
   - Integrated `inspect_server_frame` in Task 3 (Network Receiver & Renderer) prior to outputting frames to `stdout`.
4. **Automated Testing ([tests/test_predictive.rs](file:///workspace/src/mosh-tcp/tests/test_predictive.rs) & [src/predictive.rs](file:///workspace/src/mosh-tcp/src/predictive.rs))**:
   - Added unit tests for predictor state transitions, atomic frame resets, control sequence clearing, and alternate screen detection.

---

## Verification Results

### Unit & Integration Test Suite (`cargo test`)
Ran all automated tests including existing integration suites (`integration.rs`, `test_browsh.rs`, `test_tmux.rs`, `test_rate_limit.rs`) and new predictive tests:

```text
running 3 tests
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

Result: 9 passed, 0 failed.
```
