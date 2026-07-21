use crate::ansi::{find_safe_split_point, strip_terminal_queries_stateful};
use crate::protocol::{Packet, PacketCodec};
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::interval;
use tokio_util::codec::Framed;
use vt100::Parser as Vt100Parser;

#[derive(Default)]
pub struct Telemetry {
    pub pty_bytes_in: AtomicU64,
    pub net_bytes_out: AtomicU64,
    pub bytes_dropped: AtomicU64,
    pub frames_sent: AtomicU64,
    pub frames_skipped: AtomicU64,
    pub rtt_ms: AtomicU64,
}

pub struct ServerSessionState {
    pub vt_parser: Vt100Parser,
    pub pty_buffer: Vec<u8>,
}

impl ServerSessionState {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            vt_parser: Vt100Parser::new(rows, cols, 1000),
            pty_buffer: Vec::new(),
        }
    }
}


pub async fn run_server(
    bind_addr: SocketAddr,
    fps: u64,
    max_kbps: u64,
    enable_stats: bool,
    shell_cmd: Option<String>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(bind_addr).await?;
    println!("[mosh-tcp server] Listening on TCP {}", bind_addr);
    println!(
        "[mosh-tcp server] Frame rate: {} FPS | Max Bandwidth: {} KB/s",
        fps, max_kbps
    );

    loop {
        let (socket, client_addr) = listener.accept().await?;
        println!("[mosh-tcp server] Accepted connection from {}", client_addr);
        let frame_ms = 1000 / fps;
        let shell = shell_cmd.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, frame_ms, max_kbps, enable_stats, shell).await {
                eprintln!("[mosh-tcp server] Client session error: {}", e);
            }
            println!("[mosh-tcp server] Client disconnected: {}", client_addr);
        });
    }
}

fn format_rate(bytes_per_sec: f64) -> String {
    let kb_per_sec = bytes_per_sec / 1024.0;
    if kb_per_sec < 0.1 {
        format!("{:5.1} B/s ", bytes_per_sec)
    } else if kb_per_sec < 1000.0 {
        format!("{:5.1} KB/s", kb_per_sec)
    } else {
        let mb_per_sec = kb_per_sec / 1024.0;
        format!("{:5.1} MB/s", mb_per_sec)
    }
}

async fn handle_client(
    socket: TcpStream,
    frame_ms: u64,
    max_kbps: u64,
    enable_stats: bool,
    shell_cmd: Option<String>,
) -> anyhow::Result<()> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let shell = shell_cmd.unwrap_or_else(|| {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    });

    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let mut _child = pair.slave.spawn_command(cmd)?;

    let mut pty_reader = pair.master.try_clone_reader()?;
    let pty_writer = Arc::new(Mutex::new(pair.master.take_writer()?));
    let master_pair = Arc::new(Mutex::new(pair.master));

    let session_state = Arc::new(Mutex::new(ServerSessionState::new(24, 80)));

    let state_pty = Arc::clone(&session_state);
    let telemetry = Arc::new(Telemetry::default());
    let telemetry_pty = Arc::clone(&telemetry);

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    telemetry_pty.pty_bytes_in.fetch_add(n as u64, Ordering::Relaxed);

                    if let Ok(mut state) = state_pty.lock() {
                        // 1. Process bytes in VT100 virtual screen emulator safely
                        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            state.vt_parser.process(chunk);
                        }));
                        if res.is_err() {
                            let (r, c) = state.vt_parser.screen().size();
                            state.vt_parser = Vt100Parser::new(r, c, 1000);
                        }

                        // 2. Accumulate raw bytes for low-latency direct pass-through
                        state.pty_buffer.extend_from_slice(chunk);
                    }
                }
                Err(_) => break,
            }
        }
    });

    let framed = Framed::new(socket, PacketCodec::new());
    let (mut writer, mut reader) = framed.split();

    let state_send = Arc::clone(&session_state);
    let telemetry_send = Arc::clone(&telemetry);
    let telemetry_stats = Arc::clone(&telemetry);

    let stats_handle = if enable_stats {
        Some(tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(1));
            let mut last_pty_in = 0u64;
            let mut last_net_out = 0u64;

            loop {
                interval.tick().await;
                let cur_pty_in = telemetry_stats.pty_bytes_in.load(Ordering::Relaxed);
                let cur_net_out = telemetry_stats.net_bytes_out.load(Ordering::Relaxed);
                let dropped = telemetry_stats.bytes_dropped.load(Ordering::Relaxed);
                let sent_frames = telemetry_stats.frames_sent.load(Ordering::Relaxed);
                let skipped_frames = telemetry_stats.frames_skipped.load(Ordering::Relaxed);
                let rtt = telemetry_stats.rtt_ms.load(Ordering::Relaxed);

                let delta_pty_in = cur_pty_in.saturating_sub(last_pty_in);
                let delta_net_out = cur_net_out.saturating_sub(last_net_out);

                last_pty_in = cur_pty_in;
                last_net_out = cur_net_out;

                let pty_rate_str = format_rate(delta_pty_in as f64);
                let net_rate_str = format_rate(delta_net_out as f64);

                let inst_comp = if delta_pty_in > 0 {
                    ((1.0 - (delta_net_out as f64 / delta_pty_in as f64)) * 100.0).clamp(0.0, 99.9)
                } else {
                    0.0
                };

                let total_comp = if cur_pty_in > 0 {
                    ((1.0 - (cur_net_out as f64 / cur_pty_in as f64)) * 100.0).clamp(0.0, 99.9)
                } else {
                    0.0
                };

                println!(
                    "[mosh-tcp stats] PTY In: {} | Net Out: {} (Max: {:5.1} KB/s) | Comp: {:4.1}% (cur) / {:4.1}% (tot) | Skipped: {:5.1} KB | RTT: {:3} ms | Frames: {:5} sent / {:5} skipped",
                    pty_rate_str, net_rate_str, max_kbps as f64, inst_comp, total_comp, dropped as f64 / 1024.0, rtt, sent_frames, skipped_frames
                );
            }
        }))
    } else {
        None
    };

    let send_task = tokio::spawn(async move {
        let max_bytes_per_sec = (max_kbps * 1024) as f64;
        let tick_duration = Duration::from_millis(frame_ms);
        let mut frame_timer = interval(tick_duration);

        let burst_capacity = (max_bytes_per_sec * 0.5).max(2048.0);
        let mut tokens = burst_capacity;

        let mut seq: u64 = 0;
        let mut last_tick = Instant::now();
        let mut pending_atomic_refresh = false;

        const MAX_PTY_BUFFER_CAP: usize = 16384;

        loop {
            frame_timer.tick().await;
            let now = Instant::now();
            let elapsed_secs = now.duration_since(last_tick).as_secs_f64();
            last_tick = now;

            tokens = (tokens + max_bytes_per_sec * elapsed_secs).min(burst_capacity);

            let raw_data_opt = {
                if let Ok(mut state) = state_send.lock() {
                    if state.pty_buffer.len() > MAX_PTY_BUFFER_CAP {
                        // Buffer overflow detected (e.g. Carbonyl/Browsh heavy page render or large text dump)!
                        // Discard raw buffer, record dropped bytes, and queue an atomic VT100 2D screen refresh.
                        let dropped_len = state.pty_buffer.len();
                        state.pty_buffer.clear();
                        telemetry_send.bytes_dropped.fetch_add(dropped_len as u64, Ordering::Relaxed);
                        pending_atomic_refresh = true;
                    }

                    if tokens <= 0.0 {
                        // Bandwidth quota exhausted: strictly suppress sending network frames.
                        telemetry_send.frames_skipped.fetch_add(1, Ordering::Relaxed);
                        None
                    } else if pending_atomic_refresh {
                        state.pty_buffer.clear();
                        pending_atomic_refresh = false;
                        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            generate_atomic_screen_frame(&state.vt_parser)
                        }));
                        match res {
                            Ok(atomic_frame) => Some(atomic_frame),
                            Err(_) => {
                                telemetry_send.frames_skipped.fetch_add(1, Ordering::Relaxed);
                                None
                            }
                        }
                    } else if state.pty_buffer.is_empty() {
                        None
                    } else {
                        // Normal throughput: apply query stripping and send clean raw chunk
                        let (cleaned, remaining) = strip_terminal_queries_stateful(&state.pty_buffer);
                        state.pty_buffer = remaining;

                        if cleaned.is_empty() {
                            None
                        } else {
                            let available = cleaned.len();
                            let split_point = find_safe_split_point(&cleaned, tokens as usize);

                            if split_point > 0 {
                                let chunk = cleaned[..split_point].to_vec();
                                if split_point < available {
                                    let rest = &cleaned[split_point..];
                                    let mut new_guard = rest.to_vec();
                                    new_guard.extend_from_slice(&state.pty_buffer);
                                    state.pty_buffer = new_guard;
                                }
                                Some(chunk)
                            } else {
                                let mut new_guard = cleaned;
                                new_guard.extend_from_slice(&state.pty_buffer);
                                state.pty_buffer = new_guard;
                                telemetry_send.frames_skipped.fetch_add(1, Ordering::Relaxed);
                                None
                            }
                        }
                    }
                } else {
                    None
                }
            };

            if let Some(raw_data) = raw_data_opt {
                seq += 1;
                let (payload, compressed) = Packet::compress_data(&raw_data);
                let payload_len = payload.len();

                let packet = Packet::ServerFrame {
                    seq,
                    data: payload,
                    compressed,
                };

                if writer.send(packet).await.is_ok() {
                    tokens -= payload_len as f64;
                    telemetry_send.net_bytes_out.fetch_add(payload_len as u64, Ordering::Relaxed);
                    telemetry_send.frames_sent.fetch_add(1, Ordering::Relaxed);
                } else {
                    break;
                }
            }
        }
    });

    let master_resize = Arc::clone(&master_pair);
    let state_resize = Arc::clone(&session_state);
    let pty_writer_input = Arc::clone(&pty_writer);
    let telemetry_ping = Arc::clone(&telemetry);

    while let Some(packet_result) = reader.next().await {
        match packet_result {
            Ok(Packet::ClientInput { data }) => {
                if let Ok(mut w) = pty_writer_input.lock() {
                    let _ = w.write_all(&data);
                    let _ = w.flush();
                }
            }
            Ok(Packet::ClientResize { rows, cols }) => {
                let rows = rows.max(1);
                let cols = cols.max(1);
                if let Ok(m) = master_resize.lock() {
                    let _ = m.resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
                if let Ok(mut state) = state_resize.lock() {
                    state.vt_parser = Vt100Parser::new(rows, cols, 1000);
                }
            }
            Ok(Packet::Ping { timestamp }) => {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let rtt = now_ms.saturating_sub(timestamp);
                telemetry_ping.rtt_ms.store(rtt, Ordering::Relaxed);
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[mosh-tcp server] Packet decode error: {}", e);
                break;
            }
        }
    }

    send_task.abort();
    if let Some(h) = stats_handle {
        h.abort();
    }
    Ok(())
}

fn generate_atomic_screen_frame(parser: &Vt100Parser) -> Vec<u8> {
    let screen = parser.screen();
    let mut frame = Vec::new();

    // 1. Clear screen & home cursor (xterm / vt100 reset sequence)
    frame.extend_from_slice(b"\x1b[H\x1b[2J");

    // 2. Render exact 2D formatted screen grid contents
    let contents = screen.contents_formatted();
    frame.extend_from_slice(&contents);

    // 3. Set exact cursor position & visibility
    let (row, col) = screen.cursor_position();
    frame.extend_from_slice(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
    if screen.hide_cursor() {
        frame.extend_from_slice(b"\x1b[?25l");
    } else {
        frame.extend_from_slice(b"\x1b[?25h");
    }

    frame
}

