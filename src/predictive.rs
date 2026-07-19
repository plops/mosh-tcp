use std::io::{self, Write};
use crossterm::style::{Attribute, SetAttribute};

pub struct LocalPredictor {
    enabled: bool,
    active_predictions: usize,
}

impl LocalPredictor {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            active_predictions: 0,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Process user keystrokes for local echo
    pub fn handle_keystroke(&mut self, data: &[u8]) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let mut stdout = io::stdout();

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
}
