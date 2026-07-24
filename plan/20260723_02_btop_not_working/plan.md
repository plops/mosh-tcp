# Implementation Plan - Fix `btop` Rendering and Emacs Bracketed Paste Issues

## Overview & Problem Statement
Users running `mosh-tcp` experience two distinct terminal corruption issues:
1. **Garbled `btop` UI in Alacritty**: `btop` relies on querying terminal capabilities, cursor positions, and color themes (DA1, DA2, DA3, XTVERSION, CPR `\x1b[6n`, OSC 4/10/11 color queries). When `mosh-tcp` server passes these query sequences unhandled, the client terminal (Alacritty) emits escape response sequences on standard input. Crossterm on the client misinterprets these response bytes as user key events (such as `proc`, `filter`, `tree` keys), mutating `btop` state and ruining its layout structure.
2. **Emacs Paste Failure (`Wrong type argument: char-or-string-p, 134217826`)**: When users paste text into Emacs running over `mosh-tcp`, Crossterm captures local paste events (`Event::Paste(text)`). `mosh-tcp` sends raw unencapsulated bytes to the remote PTY. However, Emacs enables bracketed paste mode (`\x1b[?2004h`) in its PTY and expects pastes wrapped in `\x1b[200~` and `\x1b[201~`. Stripping the bracket markers causes Emacs to treat incoming bytes as control/meta character key combinations (`134217826` = `0x8000062` = Meta + 'b'), triggering a Lisp type error.

---

## Architectural Context & Key Files
An AI agent working on this codebase should examine the following essential files:

1. **`src/ansi.rs`**:
   - *Role*: Central module for ANSI/VT100 escape sequence parsing, safe split point calculation, and stateful terminal query stripping.
   - *Task*: Expand `strip_terminal_queries_stateful` to filter all CSI queries (DA1, DA2, DA3, XTVERSION, CPR `\x1b[6n`, `\x1b[?6n`), OSC palette/color queries (`\x1b]10;?`, `\x1b]11;?`, `\x1b]12;?`, `\x1b]4;...;?`), and DCS status queries (`\x1bP$q...\x1b\`).

2. **`src/client.rs`**:
   - *Role*: Handles client network stream, raw mode configuration, Crossterm event processing (`Key`, `Paste`, `Mouse`, `Resize`), and input response filtering.
   - *Task*: Update `Event::Paste(text)` handling to encapsulate pasted text within standard bracketed paste boundaries (`\x1b[200~` ... `\x1b[201~`). Enhance `ResponseFilter` to drop any residual terminal escape responses on stdin.

3. **`src/server.rs`**:
   - *Role*: PTY management, rate limiting, atomic frame generation, and network transmission.
   - *Task*: Ensure stateful query stripping is applied seamlessly on the PTY output stream before compressing and sending frames to the client.

4. **`tests/test_btop_emacs.rs`** (New File):
   - *Role*: Dedicated unit and integration tests for `btop` ANSI query stripping and Emacs bracketed paste transmission.
   - *Task*: Implement tests verifying stateful stripping of DA/CPR/OSC queries and bracketed paste wrapping.

5. **`plan/20260723_02_btop_not_working/walkthrough.md`**:
   - *Role*: Post-implementation walkthrough document.
   - *Task*: Summarize implementation details, test coverage, learnings, and future improvements.

---

## Additional Requirements & Enhancements
In addition to fixing `btop` and Emacs paste:
- **Comprehensive Terminal Query Stripping**: Cover not only OSC 10/11 but also CPR (`\x1b[6n`), DA1/DA2/DA3, XTVERSION, OSC 4 palette queries, and DCS queries.
- **Robust Response Filter**: Ensure client stdin does not loop back terminal escape responses even if non-TTY or piping is used.
- **Backward Compatibility**: Ensure bracketed paste wrapping works seamlessly with standard PTY shells (bash, zsh, emacs, vim, tmux).

---

## Commit Guidelines (Conventional Commits)
All commits must follow the **Conventional Commits** format with detailed, descriptive body text explaining the rationale and technical changes.

Format:
```
<type>(<scope>): <short description>

<detailed description of what was changed and why>
```

Example Types:
- `fix(ansi)`: Comprehensive ANSI query sequence filtering in server stream
- `fix(client)`: Proper bracketed paste encapsulation for Emacs and shell pastes
- `test(ansi)`: Unit and integration tests for query stripping and paste payload
- `docs(plan)`: Walkthrough document summarizing fixes and learnings

---

## Execution Steps
1. Update `src/ansi.rs` with stateful ANSI/OSC/CSI query stripping logic and unit tests.
2. Update `src/client.rs` with bracketed paste encapsulation (`\x1b[200~` / `\x1b[201~`) and enhanced `ResponseFilter`.
3. Create `tests/test_btop_emacs.rs` containing comprehensive unit and integration tests.
4. Run `cargo test` to verify all tests pass.
5. Create Conventional Commits for code changes.
6. Write `plan/20260723_02_btop_not_working/walkthrough.md`.
