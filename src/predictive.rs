use std::io::{self, Write};
use crossterm::style::{Attribute, SetAttribute};

pub struct LocalPredictor {
    enabled: bool,
    active_predictions: usize,
    suspend_predictions: bool,
}

impl LocalPredictor {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            active_predictions: 0,
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
            self.active_predictions = 0;
        }
    }

    pub fn active_predictions(&self) -> usize {
        self.active_predictions
    }

    /// Process user keystrokes for local echo
    pub fn handle_keystroke(&mut self, data: &[u8]) -> io::Result<()> {
        if !self.enabled || self.suspend_predictions {
            return Ok(());
        }

        let mut stdout = io::stdout();

        // If input contains ESC (27 / \x1b), it is an escape sequence (e.g. arrow keys, Alt combinations).
        // Clear active predictions locally before cursor moves and suppress local echo for the sequence.
        if data.contains(&27) {
            let _ = self.clear_predictions(&mut stdout);
            return Ok(());
        }

        for &byte in data {
            // Echo printable ASCII characters immediately
            if (32..=126).contains(&byte) {
                let char_str = (byte as char).to_string();
                // Render predicted character with underline styling for visual distinction
                write!(
                    stdout,
                    "{}{}{}",
                    SetAttribute(Attribute::Underlined),
                    char_str,
                    SetAttribute(Attribute::Reset)
                )?;
                self.active_predictions += 1;
            } else if byte == 13 || byte == 10 {
                // Enter key: reset local prediction count as shell will produce output
                self.active_predictions = 0;
            } else if byte == 8 || byte == 127 {
                // Backspace key
                if self.active_predictions > 0 {
                    write!(stdout, "\x08 \x08")?;
                    self.active_predictions -= 1;
                }
            } else {
                // Other control keys (e.g. Ctrl+C, Ctrl+D, Ctrl+L, etc.):
                // Clear active predictions locally before execution
                let _ = self.clear_predictions(&mut stdout);
            }
        }
        stdout.flush()?;
        Ok(())
    }

    /// Erase local predictions before printing authoritative server frame output
    pub fn clear_predictions(&mut self, stdout: &mut io::Stdout) -> io::Result<()> {
        if self.enabled && self.active_predictions > 0 {
            for _ in 0..self.active_predictions {
                // Backspace, space, backspace to erase predicted character
                write!(stdout, "\x08 \x08")?;
            }
            self.active_predictions = 0;
            stdout.flush()?;
        }
        Ok(())
    }

    /// Reset internal prediction count without emitting backspaces
    /// (e.g. when screen is completely redrawn by atomic frame or mode change)
    pub fn reset_predictions(&mut self) {
        self.active_predictions = 0;
    }

    /// Inspect incoming server frame bytes for terminal mode changes and screen clears
    pub fn inspect_server_frame(&mut self, raw_data: &[u8], stdout: &mut io::Stdout) {
        if !self.enabled {
            return;
        }

        // Check for Alternate Screen Buffer enable/disable sequences
        // Enable: \x1b[?1049h, \x1b[?1047h, \x1b[?47h
        // Disable: \x1b[?1049l, \x1b[?1047l, \x1b[?47l
        let enables_alt = contains_subslice(raw_data, b"\x1b[?1049h")
            || contains_subslice(raw_data, b"\x1b[?1047h")
            || contains_subslice(raw_data, b"\x1b[?47h");

        let disables_alt = contains_subslice(raw_data, b"\x1b[?1049l")
            || contains_subslice(raw_data, b"\x1b[?1047l")
            || contains_subslice(raw_data, b"\x1b[?47l");

        if enables_alt {
            let _ = self.clear_predictions(stdout);
            self.suspend_predictions = true;
            self.active_predictions = 0;
        } else if disables_alt {
            self.suspend_predictions = false;
            self.active_predictions = 0;
        }

        // Check for full screen clear (e.g. atomic screen frame \x1b[H\x1b[2J)
        if contains_subslice(raw_data, b"\x1b[H\x1b[2J") || contains_subslice(raw_data, b"\x1b[2J") {
            self.reset_predictions();
        }
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

        pred.active_predictions = 5;
        pred.inspect_server_frame(b"\x1b[H\x1b[2JSome screen output", &mut stdout);
        assert_eq!(pred.active_predictions(), 0);
    }
}
