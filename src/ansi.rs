/// Helper utilities for ANSI/VT100 escape sequence parsing and manipulation.

/// Checks if a byte slice contains a specific subslice (window matching).
pub fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// Finds a safe split point in a UTF-8/ANSI byte buffer without truncating
/// multi-byte UTF-8 sequences or ANSI escape sequence parameters.
pub fn find_safe_split_point(buf: &[u8], target: usize) -> usize {
    if target >= buf.len() {
        return buf.len();
    }
    if target == 0 {
        return 0;
    }

    let mut safe_points = Vec::new();
    safe_points.push(0);

    let mut i = 0;
    while i < buf.len() {
        if buf[i] == 0x1b {
            i += 1;
            if i < buf.len() {
                if buf[i] == b'[' {
                    i += 1;
                    while i < buf.len() {
                        let b = buf[i];
                        i += 1;
                        if (0x40..=0x7E).contains(&b) {
                            break;
                        }
                    }
                } else if buf[i] == b']' {
                    i += 1;
                    while i < buf.len() {
                        if buf[i] == 0x07 {
                            i += 1;
                            break;
                        } else if buf[i] == 0x1b && i + 1 < buf.len() && buf[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            safe_points.push(i);
        } else {
            let b = buf[i];
            if b < 0x80 || b >= 0xC0 {
                safe_points.push(i);
            }
            i += 1;
        }
    }
    safe_points.push(buf.len());

    let mut best = 0;
    for &pt in &safe_points {
        if pt <= target {
            best = pt;
        } else {
            break;
        }
    }
    best
}

/// Strips terminal color/query escape sequences (OSC 4/10/11/12, DA1/DA2/DA3, CPR 6n, DCS) statefully.
/// Returns (cleaned_slice, unparsed_remaining_buffer).
pub fn strip_terminal_queries_stateful(data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut result = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0x1b {
            if i + 1 == data.len() {
                return (result, data[i..].to_vec());
            }
            let next = data[i + 1];

            // 1. OSC sequences (\x1b])
            if next == b']' {
                let mut j = i + 2;
                let mut found_st = false;
                while j < data.len() {
                    if data[j] == 0x07 {
                        j += 1;
                        found_st = true;
                        break;
                    } else if data[j] == 0x1b && j + 1 < data.len() && data[j + 1] == b'\\' {
                        j += 2;
                        found_st = true;
                        break;
                    }
                    j += 1;
                }
                if found_st {
                    let sub = &data[i..j];
                    // Strip color/palette queries (OSC 4, OSC 10, OSC 11, OSC 12) or any query containing '?'
                    if sub.starts_with(b"\x1b]10;")
                        || sub.starts_with(b"\x1b]11;")
                        || sub.starts_with(b"\x1b]12;")
                        || sub.starts_with(b"\x1b]4;")
                        || sub.contains(&b'?')
                    {
                        i = j;
                        continue;
                    }
                } else {
                    return (result, data[i..].to_vec());
                }
            }

            // 2. DCS sequences (\x1bP)
            if next == b'P' {
                let mut j = i + 2;
                let mut found_st = false;
                while j < data.len() {
                    if data[j] == 0x07 {
                        j += 1;
                        found_st = true;
                        break;
                    } else if data[j] == 0x1b && j + 1 < data.len() && data[j + 1] == b'\\' {
                        j += 2;
                        found_st = true;
                        break;
                    }
                    j += 1;
                }
                if found_st {
                    let sub = &data[i..j];
                    if sub.contains_str("$q") || sub.contains(&b'$') {
                        i = j;
                        continue;
                    }
                } else {
                    return (result, data[i..].to_vec());
                }
            }

            // 3. CSI sequences (\x1b[)
            if next == b'[' {
                let mut j = i + 2;
                let mut found_end = false;
                while j < data.len() {
                    let b = data[j];
                    if (0x40..=0x7E).contains(&b) {
                        found_end = true;
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                if found_end {
                    let sub = &data[i..j];
                    // Check known query patterns: DA1, DA2, DA3, XTVERSION, CPR (6n)
                    if sub == b"\x1b[c"
                        || sub == b"\x1b[0c"
                        || sub == b"\x1b[?c"
                        || sub == b"\x1b[>c"
                        || sub == b"\x1b[>0c"
                        || sub == b"\x1b[>1c"
                        || sub == b"\x1b[=c"
                        || sub == b"\x1b[=0c"
                        || sub == b"\x1b[>q"
                        || sub == b"\x1b[q"
                        || sub == b"\x1b[6n"
                        || sub == b"\x1b[?6n"
                        || (sub.starts_with(b"\x1b[?") && sub.ends_with(b"c"))
                        || (sub.starts_with(b"\x1b[?") && sub.ends_with(b"n"))
                    {
                        i = j;
                        continue;
                    }
                } else {
                    return (result, data[i..].to_vec());
                }
            }
        }

        result.push(data[i]);
        i += 1;
    }
    (result, Vec::new())
}

trait ContainsStr {
    fn contains_str(&self, needle: &str) -> bool;
}

impl ContainsStr for [u8] {
    fn contains_str(&self, needle: &str) -> bool {
        contains_subslice(self, needle.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_terminal_queries_stateful_da_and_cpr() {
        let input = b"Hello\x1b[cWorld\x1b[6n!\x1b[>0cTest";
        let (cleaned, remaining) = strip_terminal_queries_stateful(input);
        assert_eq!(cleaned, b"HelloWorld!Test");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_strip_terminal_queries_osc_color() {
        let input = b"Prefix\x1b]10;?\x07Middle\x1b]11;?\x1b\\Suffix";
        let (cleaned, remaining) = strip_terminal_queries_stateful(input);
        assert_eq!(cleaned, b"PrefixMiddleSuffix");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_strip_terminal_queries_partial_sequence() {
        let input = b"Text\x1b[6";
        let (cleaned, remaining) = strip_terminal_queries_stateful(input);
        assert_eq!(cleaned, b"Text");
        assert_eq!(remaining, b"\x1b[6");
    }
}

