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

    let vt_parser = Arc::new(Mutex::new(Vt100Parser::new(24, 80, 1000)));
    let pty_buffer = Arc::new(Mutex::new(Vec::<u8>::new()));

    let vt_parser_pty = Arc::clone(&vt_parser);
    let pty_buffer_clone = Arc::clone(&pty_buffer);

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

                    // 1. Process bytes in VT100 virtual screen emulator
                    if let Ok(mut vt) = vt_parser_pty.lock() {
                        vt.process(chunk);
                    }

                    // 2. Accumulate raw bytes for low-latency direct pass-through
                    if let Ok(mut guard) = pty_buffer_clone.lock() {
                        guard.extend_from_slice(chunk);
                    }
                }
                Err(_) => break,
            }
        }
    });

    let framed = Framed::new(socket, PacketCodec::new());
    let (mut writer, mut reader) = framed.split();

    let pty_buffer_task = Arc::clone(&pty_buffer);
    let vt_parser_task = Arc::clone(&vt_parser);
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

        const MAX_PTY_BUFFER_CAP: usize = 16384;

        loop {
            frame_timer.tick().await;
            let now = Instant::now();
            let elapsed_secs = now.duration_since(last_tick).as_secs_f64();
            last_tick = now;

            tokens = (tokens + max_bytes_per_sec * elapsed_secs).min(burst_capacity);

            let raw_data_opt = {
                if let Ok(mut guard) = pty_buffer_task.lock() {
                    if guard.is_empty() {
                        None
                    } else if guard.len() > MAX_PTY_BUFFER_CAP {
                        // Buffer overflow detected (e.g. Browsh heavy page render or large text dump)!
                        // Discard raw buffer and generate a 100% ATOMIC, UNCORRUPTED VT100 2D screen frame!
                        let dropped_len = guard.len();
                        guard.clear();
                        telemetry_send.bytes_dropped.fetch_add(dropped_len as u64, Ordering::Relaxed);
                        telemetry_send.frames_skipped.fetch_add(1, Ordering::Relaxed);

                        if let Ok(vt) = vt_parser_task.lock() {
                            let atomic_frame = generate_atomic_screen_frame(&vt);
                            Some(atomic_frame)
                        } else {
                            None
                        }
                    } else {
                        // Normal throughput: apply query stripping and send clean raw chunk
                        let (cleaned, remaining) = strip_terminal_queries_stateful(&guard);
                        *guard = remaining;

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
                                    new_guard.extend_from_slice(&guard);
                                    *guard = new_guard;
                                }
                                Some(chunk)
                            } else {
                                let mut new_guard = cleaned;
                                new_guard.extend_from_slice(&guard);
                                *guard = new_guard;
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
    let vt_parser_resize = Arc::clone(&vt_parser);
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
                if let Ok(m) = master_resize.lock() {
                    let _ = m.resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
                if let Ok(mut vt) = vt_parser_resize.lock() {
                    vt.set_size(rows, cols);
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

fn find_safe_split_point(buf: &[u8], target: usize) -> usize {
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

fn strip_terminal_queries_stateful(data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut result = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if i + 5 <= data.len() && (&data[i..i + 5] == b"\x1b]10;" || &data[i..i + 5] == b"\x1b]11;") {
            let mut j = i + 5;
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
                i = j;
                continue;
            } else {
                return (result, data[i..].to_vec());
            }
        }

        if data[i] == 0x1b {
            if i + 1 == data.len() {
                return (result, data[i..].to_vec());
            }
            let next = data[i + 1];
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
                    if sub == b"\x1b[c" || sub == b"\x1b[0c" || sub == b"\x1b[>c" || sub == b"\x1b[>0c" || sub == b"\x1b[>q" {
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
