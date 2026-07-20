use crate::predictive::LocalPredictor;
use crate::protocol::{Packet, PacketCodec};
#[allow(unused_imports)]
use crossterm::event::MouseEvent;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseButton,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::codec::Framed;

pub async fn run_client(server_addr: SocketAddr, enable_predictive: bool) -> anyhow::Result<()> {
    let socket = TcpStream::connect(server_addr).await?;
    println!("[mosh-tcp client] Connected to {}", server_addr);

    struct RawModeGuard;
    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), DisableMouseCapture);
        }
    }

    enable_raw_mode()?;
    let _ = execute!(io::stdout(), EnableMouseCapture);
    let _guard = RawModeGuard;

    let framed = Framed::new(socket, PacketCodec::new());
    let (mut writer, mut reader) = framed.split();

    let predictor = Arc::new(Mutex::new(LocalPredictor::new(enable_predictive)));

    if let Ok((cols, rows)) = size() {
        if let Ok(mut pred) = predictor.lock() {
            pred.set_size(rows, cols);
        }
        let _ = writer.send(Packet::ClientResize { rows, cols }).await;
    }

    let running = Arc::new(AtomicBool::new(true));
    let (input_tx, mut input_rx) = mpsc::channel::<Packet>(100);

    // Task 1: Stdin & Terminal Event loop
    let running_clone = Arc::clone(&running);
    let predictor_input = Arc::clone(&predictor);

    std::thread::spawn(move || {
        let mut filter = ResponseFilter::new();

        while running_clone.load(Ordering::Relaxed) {
            if event::poll(std::time::Duration::from_millis(20)).unwrap_or(false) {
                match event::read() {
                    Ok(Event::Key(key_event)) => {
                        if key_event.code == KeyCode::Char('q')
                            && key_event.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            running_clone.store(false, Ordering::Relaxed);
                            break;
                        }

                        let raw_data = key_event_to_bytes(key_event);
                        if !raw_data.is_empty() {
                            let clean_data = filter.filter(&raw_data);

                            if !clean_data.is_empty() {
                                if let Ok(mut pred) = predictor_input.lock() {
                                    let _ = pred.handle_keystroke(&clean_data);
                                }
                                let _ = input_tx.blocking_send(Packet::ClientInput { data: clean_data });
                            }
                        }
                    }
                    Ok(Event::Mouse(mouse_event)) => {
                        let mouse_data = mouse_event_to_bytes(mouse_event);
                        if !mouse_data.is_empty() {
                            let _ = input_tx.blocking_send(Packet::ClientInput { data: mouse_data });
                        }
                    }
                    Ok(Event::Resize(cols, rows)) => {
                        if let Ok(mut pred) = predictor_input.lock() {
                            pred.set_size(rows, cols);
                        }
                        let _ = input_tx.blocking_send(Packet::ClientResize { rows, cols });
                    }
                    _ => {}
                }
            }
        }
    });

    // Task 2: Network Sender
    let send_handle = tokio::spawn(async move {
        while let Some(packet) = input_rx.recv().await {
            if writer.send(packet).await.is_err() {
                break;
            }
        }
    });

    // Task 3: Network Receiver & Renderer
    let mut stdout = io::stdout();
    let predictor_recv = Arc::clone(&predictor);

    while running.load(Ordering::Relaxed) {
        tokio::select! {
            packet_opt = reader.next() => {
                match packet_opt {
                    Some(Ok(Packet::ServerFrame { data, compressed, .. })) => {
                        if let Ok(raw) = Packet::decompress_data(&data, compressed) {
                            if let Ok(mut pred) = predictor_recv.lock() {
                                pred.inspect_server_frame(&raw, &mut stdout);
                                let _ = pred.clear_predictions(&mut stdout);
                            }
                            let _ = stdout.write_all(&raw);
                            let _ = stdout.flush();
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        eprintln!("\r\n[mosh-tcp client] Connection error: {}", e);
                        break;
                    }
                    None => {
                        eprintln!("\r\n[mosh-tcp client] Server closed connection.");
                        break;
                    }
                }
            }
        }
    }

    running.store(false, Ordering::Relaxed);
    send_handle.abort();
    Ok(())
}

struct ResponseFilter {
    buffer: Vec<u8>,
}

impl ResponseFilter {
    fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    fn filter(&mut self, data: &[u8]) -> Vec<u8> {
        self.buffer.extend_from_slice(data);

        let mut clean = Vec::new();
        let mut idx = 0;

        while idx < self.buffer.len() {
            let slice = &self.buffer[idx..];

            // Filter OSC 10 / OSC 11 response patterns: "]10;rgb:..." or "]11;rgb:..."
            if slice.starts_with(b"]10;rgb:") || slice.starts_with(b"]11;rgb:") {
                if let Some(end) = slice.iter().position(|&b| b == b'\\' || b == 27 || b == b'\r' || b == b'\n') {
                    idx += end + 1;
                    continue;
                } else {
                    break;
                }
            }

            // Filter DA response patterns: "0;...c"
            if slice.starts_with(b"0;") {
                if let Some(c_pos) = slice.iter().position(|&b| b == b'c') {
                    let sub = &slice[..=c_pos];
                    if sub.iter().all(|&b| b.is_ascii_digit() || b == b';' || b == b'c') {
                        idx += c_pos + 1;
                        continue;
                    }
                }
            }

            clean.push(self.buffer[idx]);
            idx += 1;
        }

        self.buffer = self.buffer[idx..].to_vec();
        clean
    }
}

fn mouse_event_to_bytes(mouse: event::MouseEvent) -> Vec<u8> {
    let col = mouse.column + 1;
    let row = mouse.row + 1;

    let (base_button, is_release) = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => (0, false),
        MouseEventKind::Down(MouseButton::Middle) => (1, false),
        MouseEventKind::Down(MouseButton::Right) => (2, false),
        MouseEventKind::Up(MouseButton::Left) => (0, true),
        MouseEventKind::Up(MouseButton::Middle) => (1, true),
        MouseEventKind::Up(MouseButton::Right) => (2, true),
        MouseEventKind::Drag(MouseButton::Left) => (32, false),
        MouseEventKind::Drag(MouseButton::Middle) => (33, false),
        MouseEventKind::Drag(MouseButton::Right) => (34, false),
        MouseEventKind::Moved => (35, false),
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::ScrollLeft => (66, false),
        MouseEventKind::ScrollRight => (67, false),
    };

    let mut modifier_bits = 0;
    if mouse.modifiers.contains(KeyModifiers::SHIFT) {
        modifier_bits += 4;
    }
    if mouse.modifiers.contains(KeyModifiers::ALT) {
        modifier_bits += 8;
    }
    if mouse.modifiers.contains(KeyModifiers::CONTROL) {
        modifier_bits += 16;
    }

    let button_code = base_button + modifier_bits;

    // SGR 1006 extended mouse encoding format: \x1b[<b;c;rM (or m for release)
    let final_char = if is_release { 'm' } else { 'M' };
    format!("\x1b[<{};{};{}{}", button_code, col, row, final_char).into_bytes()
}

fn key_event_to_bytes(key: event::KeyEvent) -> Vec<u8> {
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                let ascii = (c as u8).to_ascii_lowercase();
                if (b'a'..=b'z').contains(&ascii) {
                    vec![ascii - b'a' + 1]
                } else {
                    vec![]
                }
            } else if key.modifiers.contains(KeyModifiers::ALT) {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf).as_bytes();
                let mut res = vec![27];
                res.extend_from_slice(encoded);
                res
            } else {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }
        KeyCode::Enter => vec![13],
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![9],
        KeyCode::Esc => vec![27],
        KeyCode::Up => vec![27, 91, 65],
        KeyCode::Down => vec![27, 91, 66],
        KeyCode::Right => vec![27, 91, 67],
        KeyCode::Left => vec![27, 91, 68],
        KeyCode::Home => vec![27, 91, 72],
        KeyCode::End => vec![27, 91, 70],
        KeyCode::PageUp => vec![27, 91, 53, 126],    // \x1b[5~
        KeyCode::PageDown => vec![27, 91, 54, 126],  // \x1b[6~
        KeyCode::Insert => vec![27, 91, 50, 126],    // \x1b[2~
        KeyCode::Delete => vec![27, 91, 51, 126],    // \x1b[3~
        KeyCode::F(1) => vec![27, 79, 80],
        KeyCode::F(2) => vec![27, 79, 81],
        KeyCode::F(3) => vec![27, 79, 82],
        KeyCode::F(4) => vec![27, 79, 83],
        KeyCode::F(5) => vec![27, 91, 49, 53, 126],
        KeyCode::F(6) => vec![27, 91, 49, 55, 126],
        KeyCode::F(7) => vec![27, 91, 49, 56, 126],
        KeyCode::F(8) => vec![27, 91, 49, 57, 126],
        KeyCode::F(9) => vec![27, 91, 50, 48, 126],
        KeyCode::F(10) => vec![27, 91, 50, 49, 126],
        KeyCode::F(11) => vec![27, 91, 50, 51, 126],
        KeyCode::F(12) => vec![27, 91, 50, 52, 126],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_filter_multibyte_utf8() {
        let mut filter = ResponseFilter::new();
        let input = "ø".as_bytes();
        let cleaned = filter.filter(input);
        assert_eq!(cleaned, input);
    }

    #[test]
    fn test_alt_key_event_encoding() {
        let key = event::KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        let bytes = key_event_to_bytes(key);
        assert_eq!(bytes, vec![27, b'x']);
    }

    #[test]
    fn test_sgr_mouse_event_encoding() {
        let mouse_down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(mouse_event_to_bytes(mouse_down), b"\x1b[<0;11;6M");

        let mouse_up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(mouse_event_to_bytes(mouse_up), b"\x1b[<0;11;6m");

        let mouse_move = MouseEvent {
            kind: MouseEventKind::Moved,
            column: 15,
            row: 8,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(mouse_event_to_bytes(mouse_move), b"\x1b[<35;16;9M");
    }
}
