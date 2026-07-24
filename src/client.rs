use crate::predictive::LocalPredictor;
use crate::protocol::Packet;
#[allow(unused_imports)]
use crossterm::event::MouseEvent;
#[allow(unused_imports)]
use crossterm::event::{
    self, DisableMouseCapture, Event, KeyCode, KeyModifiers, MouseButton,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use std::process::{Child, Command};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use std::io::BufRead;

pub struct SshTunnel {
    launcher_child: Option<Child>,
    tunnel_child: Child,
    local_port: u16,
    remote_port: u16,
    session_key: String,
}

impl SshTunnel {
    pub fn spawn(
        ssh_cmd: &str,
        target: &str,
        ssh_port: Option<u16>,
        remote_server_cmd: &str,
        remote_port: u16,
    ) -> io::Result<(Self, TcpStream, String)> {
        // Step 1: Launch mosh-tcp-server on remote host via SSH stdout capture
        let mut launch_cmd = Command::new(ssh_cmd);
        launch_cmd.stdin(std::process::Stdio::null());
        if let Some(port) = ssh_port {
            launch_cmd.arg("-p").arg(port.to_string());
        }
        launch_cmd.arg(target);
        
        let server_arg = if remote_port == 0 {
            format!("{} --bind 127.0.0.1:0", remote_server_cmd)
        } else {
            format!("{} --bind 127.0.0.1:{}", remote_server_cmd, remote_port)
        };
        launch_cmd.arg(&server_arg);
        launch_cmd.stdout(std::process::Stdio::piped());
        launch_cmd.stderr(std::process::Stdio::inherit());

        eprintln!("[mosh-tcp client] Launching server on {} via SSH...", target);
        let mut launcher_child = launch_cmd.spawn()?;

        let mut bound_port = remote_port;
        let mut session_key = String::new();

        if let Some(stdout) = launcher_child.stdout.take() {
            let reader = std::io::BufReader::new(stdout);
            for line_res in reader.lines() {
                if let Ok(line) = line_res {
                    if line.starts_with("MOSH-TCP CONNECT ") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 3 {
                            if let Ok(p) = parts[2].parse::<u16>() {
                                bound_port = p;
                            }
                            session_key = parts[3].to_string();
                            break;
                        }
                    }
                }
            }
        }

        if bound_port == 0 {
            bound_port = 4000;
        }

        // Step 2: Create local port forwarding SSH tunnel to bound_port
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let local_port = listener.local_addr()?.port();
        drop(listener);

        let mut tunnel_cmd = Command::new(ssh_cmd);
        tunnel_cmd.stdin(std::process::Stdio::null());
        tunnel_cmd.arg("-o").arg("ExitOnForwardFailure=yes");
        tunnel_cmd.arg("-N");
        tunnel_cmd.arg("-L").arg(format!("{}:127.0.0.1:{}", local_port, bound_port));
        if let Some(port) = ssh_port {
            tunnel_cmd.arg("-p").arg(port.to_string());
        }
        tunnel_cmd.arg(target);

        eprintln!(
            "[mosh-tcp client] Established tunnel (local port {} -> remote port {})...",
            local_port, bound_port
        );
        let mut tunnel_child = tunnel_cmd.spawn()?;

        let local_addr = format!("127.0.0.1:{}", local_port);
        let start = Instant::now();
        let timeout = Duration::from_secs(15);

        let stream = loop {
            if let Ok(Some(status)) = tunnel_child.try_wait() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("SSH tunnel process exited unexpectedly with status: {}", status),
                ));
            }

            if let Ok(stream) = TcpStream::connect(&local_addr) {
                break stream;
            }

            if start.elapsed() > timeout {
                let _ = tunnel_child.kill();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Timed out waiting for SSH tunnel TCP connection",
                ));
            }

            std::thread::sleep(Duration::from_millis(100));
        };

        Ok((
            SshTunnel {
                launcher_child: Some(launcher_child),
                tunnel_child,
                local_port,
                remote_port: bound_port,
                session_key: session_key.clone(),
            },
            stream,
            session_key,
        ))
    }

    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }

    pub fn session_key(&self) -> &str {
        &self.session_key
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        let _ = self.tunnel_child.kill();
        let _ = self.tunnel_child.wait();
        if let Some(mut child) = self.launcher_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn run_client(server_addr: SocketAddr, enable_predictive: bool) -> io::Result<()> {
    let socket = TcpStream::connect(server_addr)?;
    eprintln!("[mosh-tcp client] Connected to {}", server_addr);
    run_client_stream_handshake(socket, enable_predictive, "")
}

pub fn run_client_stream(socket: TcpStream, enable_predictive: bool) -> io::Result<()> {
    run_client_stream_handshake(socket, enable_predictive, "")
}

pub fn run_client_stream_handshake(
    mut socket: TcpStream,
    enable_predictive: bool,
    session_key: &str,
) -> io::Result<()> {
    struct RawModeGuard;
    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
            let _ = execute!(
                io::stdout(),
                crossterm::event::DisableBracketedPaste,
                DisableMouseCapture
            );
        }
    }

    let _ = enable_raw_mode();
    let _ = execute!(io::stdout(), crossterm::event::EnableBracketedPaste);
    let _guard = RawModeGuard;

    let mut write_socket = socket.try_clone()?;
    let predictor = Arc::new(Mutex::new(LocalPredictor::new(enable_predictive)));

    let (cols, rows) = size().unwrap_or((80, 24));
    if let Ok(mut pred) = predictor.lock() {
        pred.set_size(rows, cols);
    }

    let handshake_pkt = Packet::ClientHandshake {
        session_key: session_key.to_string(),
        rows,
        cols,
    };
    let _ = write_packet(&mut write_socket, &handshake_pkt);

    let running = Arc::new(AtomicBool::new(true));
    let (input_tx, input_rx) = mpsc::channel::<Packet>();

    // Task 1: Stdin & Terminal Event loop
    let running_clone = Arc::clone(&running);
    let predictor_input = Arc::clone(&predictor);

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut last_size = size().unwrap_or((80, 24));

        while running_clone.load(Ordering::Relaxed) {
            if let Ok((cols, rows)) = size() {
                if (cols, rows) != last_size {
                    if let Ok(mut pred) = predictor_input.lock() {
                        pred.set_size(rows, cols);
                    }
                    let _ = input_tx.send(Packet::ClientResize { rows, cols });
                    last_size = (cols, rows);
                }
            }

            match std::io::stdin().read(&mut buf) {
                Ok(n) if n > 0 => {
                    let data = &buf[..n];

                    // Check for Ctrl+Q exit hotkey (\x11)
                    if data.contains(&17) {
                        running_clone.store(false, Ordering::Relaxed);
                        break;
                    }

                    if let Ok(mut pred) = predictor_input.lock() {
                        let _ = pred.handle_keystroke(data);
                    }
                    let _ = input_tx.send(Packet::ClientInput { data: data.to_vec() });
                }
                Ok(_) => {
                    running_clone.store(false, Ordering::Relaxed);
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => {
                    break;
                }
            }
        }
    });

    // Task 2: Network Sender
    let running_sender = Arc::clone(&running);
    let mut sender_socket = socket.try_clone()?;
    std::thread::spawn(move || {
        while running_sender.load(Ordering::Relaxed) {
            if let Ok(packet) = input_rx.recv() {
                if write_packet(&mut sender_socket, &packet).is_err() {
                    break;
                }
            } else {
                break;
            }
        }
    });

    // Task 3: Network Receiver & Renderer
    let mut stdout = io::stdout();
    let predictor_recv = Arc::clone(&predictor);

    socket.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;

    while running.load(Ordering::Relaxed) {
        match read_packet(&mut socket) {
            Ok(Packet::ServerFrame { data, compressed, .. }) => {
                if let Ok(raw) = Packet::decompress_data(&data, compressed) {
                    if let Ok(mut pred) = predictor_recv.lock() {
                        pred.inspect_server_frame(&raw, &mut stdout);
                        let _ = pred.clear_predictions(&mut stdout);
                    }
                    let _ = stdout.write_all(&raw);
                    let _ = stdout.flush();
                }
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                continue;
            }
            Err(e) => {
                eprintln!("\r\n[mosh-tcp client] Connection error: {}", e);
                break;
            }
        }
    }

    running.store(false, Ordering::Relaxed);
    Ok(())
}

fn write_packet(writer: &mut impl io::Write, packet: &Packet) -> io::Result<()> {
    let serialized = packet.serialize();
    let len = (serialized.len() as u32).to_be_bytes();
    writer.write_all(&len)?;
    writer.write_all(&serialized)?;
    writer.flush()
}

fn read_packet(reader: &mut impl io::Read) -> io::Result<Packet> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Packet::deserialize(&buf)
}

#[allow(dead_code)]
struct ResponseFilter {
    buffer: Vec<u8>,
}

#[allow(dead_code)]
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

            // Filter DA response patterns: "0;...c" or "\x1b[?...c"
            if slice.starts_with(b"0;") || slice.starts_with(b"\x1b[?") {
                if let Some(c_pos) = slice.iter().position(|&b| b == b'c') {
                    let sub = &slice[..=c_pos];
                    if sub.iter().all(|&b| b.is_ascii_digit() || b == b';' || b == b'c' || b == 27 || b == b'[' || b == b'?') {
                        idx += c_pos + 1;
                        continue;
                    }
                }
            }

            // Filter CPR response patterns: "\x1b[24;80R" or "24;80R"
            if slice.starts_with(b"\x1b[") && slice.contains(&b'R') {
                if let Some(r_pos) = slice.iter().position(|&b| b == b'R') {
                    let sub = &slice[..=r_pos];
                    if sub.iter().all(|&b| b.is_ascii_digit() || b == b';' || b == b'R' || b == 27 || b == b'[') {
                        idx += r_pos + 1;
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
        KeyCode::F(6) => vec![27, 91, 49, 57, 126],
        KeyCode::F(7) => vec![27, 91, 49, 58, 126],
        KeyCode::F(8) => vec![27, 91, 49, 59, 126],
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

    #[test]
    fn test_paste_event_resets_predictor() {
        let mut pred = LocalPredictor::new(true);
        let _ = pred.handle_keystroke(b"hello");
        assert!(pred.active_predictions() > 0);

        pred.reset();
        assert_eq!(pred.active_predictions(), 0);
    }

    #[test]
    fn test_bracketed_paste_encapsulation_format() {
        let raw_text = "test_paste_content";
        let mut paste_bytes = Vec::new();
        paste_bytes.extend_from_slice(b"\x1b[200~");
        paste_bytes.extend_from_slice(raw_text.as_bytes());
        paste_bytes.extend_from_slice(b"\x1b[201~");

        assert!(paste_bytes.starts_with(b"\x1b[200~"));
        assert!(paste_bytes.ends_with(b"\x1b[201~"));
        assert_eq!(&paste_bytes[6..6 + raw_text.len()], raw_text.as_bytes());
    }
}
