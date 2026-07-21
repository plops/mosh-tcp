use std::io::{self, Write};
use crossterm::style::{Attribute, SetAttribute};
use vt100::Parser as Vt100Parser;

#[derive(Debug, Clone)]
pub struct PredictedCell {
    pub row: u16,
    pub col: u16,
    pub character: char,
    pub epoch: u64,
    pub timestamp: std::time::Instant,
}

pub struct LocalPredictor {
    enabled: bool,
    prediction_epoch: u64,
    confirmed_epoch: u64,
    predictions: Vec<PredictedCell>,
    vt_parser: Vt100Parser,
    suspend_predictions: bool,
}

impl LocalPredictor {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            prediction_epoch: 1,
            confirmed_epoch: 0,
            predictions: Vec::new(),
            vt_parser: Vt100Parser::new(24, 80, 1000),
            suspend_predictions: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_suspended(&self) -> bool {
        self.suspend_predictions
    }

    pub fn set_suspended(&mut self, suspended: bool) {
        self.suspend_predictions = suspended;
        if suspended {
            self.predictions.clear();
        }
    }

    pub fn set_size(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        self.vt_parser = Vt100Parser::new(rows, cols, 1000);
    }

    pub fn active_predictions(&self) -> usize {
        self.predictions.len()
    }

    /// Process user keystrokes for local echo
    pub fn handle_keystroke(&mut self, data: &[u8]) -> io::Result<()> {
        if !self.enabled || self.suspend_predictions {
            return Ok(());
        }

        let mut stdout = io::stdout();

        // If input contains ESC (27) or non-printable control characters,
        // advance prediction epoch (make predictions tentative) and clear active predictions
        if data.contains(&27) {
            self.become_tentative();
            self.clear_predictions(&mut stdout)?;
            return Ok(());
        }

        let (mut cur_row, mut cur_col) = self.vt_parser.screen().cursor_position();

        for &byte in data {
            // Echo printable ASCII characters immediately
            if (32..=126).contains(&byte) {
                let ch = byte as char;

                self.predictions.push(PredictedCell {
                    row: cur_row,
                    col: cur_col,
                    character: ch,
                    epoch: self.prediction_epoch,
                    timestamp: std::time::Instant::now(),
                });

                // Render predicted character with underline styling for visual distinction
                write!(
                    stdout,
                    "{}{}{}",
                    SetAttribute(Attribute::Underlined),
                    ch,
                    SetAttribute(Attribute::Reset)
                )?;
                cur_col += 1;
            } else if byte == 13 || byte == 10 {
                // Enter key: advance prediction epoch
                self.become_tentative();
                self.clear_predictions(&mut stdout)?;
            } else if byte == 8 || byte == 127 {
                // Backspace key
                if let Some(last_pred) = self.predictions.pop() {
                    write!(stdout, "\x08 \x08")?;
                    cur_row = last_pred.row;
                    cur_col = last_pred.col;
                }
            } else {
                self.become_tentative();
                self.clear_predictions(&mut stdout)?;
            }
        }
        stdout.flush()?;
        Ok(())
    }

    /// Erase local predictions from stdout
    pub fn clear_predictions(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        if self.enabled && !self.predictions.is_empty() {
            for _ in 0..self.predictions.len() {
                write!(stdout, "\x08 \x08")?;
            }
            self.predictions.clear();
            stdout.flush()?;
        }
        Ok(())
    }

    /// Reset internal prediction count without emitting backspaces
    pub fn reset_predictions(&mut self) {
        self.predictions.clear();
    }

    /// Reset predictor state on paste or bulk input
    pub fn reset(&mut self) {
        self.reset_predictions();
        self.become_tentative();
    }

    /// Inspect incoming server frame bytes, process into 2D virtual terminal, and cull predictions
    pub fn inspect_server_frame(&mut self, raw_data: &[u8], stdout: &mut io::Stdout) {
        if !self.enabled {
            return;
        }

        // 1. Process raw frame into 2D VT100 virtual terminal screen state safely
        let parser = &mut self.vt_parser;
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parser.process(raw_data);
        }));
        if res.is_err() {
            let (r, c) = self.vt_parser.screen().size();
            self.vt_parser = Vt100Parser::new(r, c, 1000);
        }

        // 2. Alternate screen buffer check
        let enables_alt = contains_subslice(raw_data, b"\x1b[?1049h")
            || contains_subslice(raw_data, b"\x1b[?1047h")
            || contains_subslice(raw_data, b"\x1b[?47h");

        let disables_alt = contains_subslice(raw_data, b"\x1b[?1049l")
            || contains_subslice(raw_data, b"\x1b[?1047l")
            || contains_subslice(raw_data, b"\x1b[?47l");

        if enables_alt {
            let _ = self.clear_predictions(stdout);
            self.suspend_predictions = true;
            self.predictions.clear();
            return;
        } else if disables_alt {
            self.suspend_predictions = false;
            self.predictions.clear();
        }

        // 3. Full screen clear reset
        if contains_subslice(raw_data, b"\x1b[H\x1b[2J") || contains_subslice(raw_data, b"\x1b[2J") {
            self.reset_predictions();
            return;
        }

        // 4. Cull predictions against authoritative 2D VT100 screen cells
        let mut confirmed_count = 0;
        let mut idx = 0;

        let screen = self.vt_parser.screen();

        while idx < self.predictions.len() {
            let pred = &self.predictions[idx];
            let cell_opt = screen.cell(pred.row, pred.col);

            let is_matched = if let Some(cell) = cell_opt {
                cell.contents() == pred.character.to_string()
            } else {
                false
            };

            if is_matched {
                if pred.epoch > self.confirmed_epoch {
                    self.confirmed_epoch = pred.epoch;
                }
                confirmed_count += 1;
                idx += 1;
            } else {
                // Server frame at (row, col) does not match predicted character.
                // Truncate remaining unconfirmed predictions from this epoch.
                self.predictions.truncate(idx);
                break;
            }
        }

        // Drain confirmed predictions (since the server frame authoritatively renders them on screen)
        if confirmed_count > 0 {
            self.predictions.drain(0..confirmed_count);
        }
    }

    fn become_tentative(&mut self) {
        self.prediction_epoch += 1;
    }
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_predictor_disabled_by_default() {
        let pred = LocalPredictor::new(false);
        assert!(!pred.is_enabled());
        assert!(!pred.is_suspended());
        assert_eq!(pred.active_predictions(), 0);
    }

    #[test]
    fn test_predictor_suspension_toggle() {
        let mut pred = LocalPredictor::new(true);
        let mut stdout = io::stdout();

        assert!(!pred.is_suspended());

        // Enable alternate screen
        pred.inspect_server_frame(b"some data \x1b[?1049h more data", &mut stdout);
        assert!(pred.is_suspended());
        assert_eq!(pred.active_predictions(), 0);

        // Keystrokes should be ignored when suspended
        pred.handle_keystroke(b"hello").unwrap();
        assert_eq!(pred.active_predictions(), 0);

        // Disable alternate screen
        pred.inspect_server_frame(b"exiting \x1b[?1049l app", &mut stdout);
        assert!(!pred.is_suspended());
    }

    #[test]
    fn test_atomic_frame_resets_predictions() {
        let mut pred = LocalPredictor::new(true);
        let mut stdout = io::stdout();

        pred.inspect_server_frame(b"\x1b[H\x1b[2JSome screen output", &mut stdout);
        assert_eq!(pred.active_predictions(), 0);
    }

    #[test]
    fn test_2d_cell_prediction_confirmation() {
        let mut pred = LocalPredictor::new(true);
        let mut stdout = io::stdout();

        // Keystrokes predict characters
        pred.handle_keystroke(b"echo").unwrap();
        assert_eq!(pred.active_predictions(), 4);

        // Server frame returns authoritative 2D screen state matching "echo"
        pred.inspect_server_frame(b"echo", &mut stdout);
        // Predictions matching 2D screen cells are confirmed and drained!
        assert_eq!(pred.active_predictions(), 0);
    }
}
