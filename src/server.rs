use crate::protocol::{Packet, PacketCodec};
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::interval;
use tokio_util::codec::Framed;

pub async fn run_server(bind_addr: SocketAddr, fps: u64, shell_cmd: Option<String>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(bind_addr).await?;
    println!("[mosh-tcp server] Listening on TCP {}", bind_addr);
    println!("[mosh-tcp server] Frame rate limit: {} ms/frame ({} FPS)", 1000 / fps, fps);

    loop {
        let (socket, client_addr) = listener.accept().await?;
        println!("[mosh-tcp server] Accepted connection from {}", client_addr);
        let frame_ms = 1000 / fps;
        let shell = shell_cmd.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, frame_ms, shell).await {
                eprintln!("[mosh-tcp server] Client session error: {}", e);
            }
            println!("[mosh-tcp server] Client disconnected: {}", client_addr);
        });
    }
}

async fn handle_client(socket: TcpStream, frame_ms: u64, shell_cmd: Option<String>) -> anyhow::Result<()> {
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

    let pty_buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
    let pty_buffer_clone = Arc::clone(&pty_buffer);

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let cleaned = strip_terminal_queries(&buf[..n]);
                    if !cleaned.is_empty() {
                        if let Ok(mut guard) = pty_buffer_clone.lock() {
                            guard.extend_from_slice(&cleaned);
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    let framed = Framed::new(socket, PacketCodec::new());
    let (mut writer, mut reader) = framed.split();

    let pty_buffer_task = Arc::clone(&pty_buffer);
    let mut frame_timer = interval(Duration::from_millis(frame_ms));
    let mut seq: u64 = 0;

    let send_task = tokio::spawn(async move {
        loop {
            frame_timer.tick().await;

            let data_to_send = {
                if let Ok(mut guard) = pty_buffer_task.lock() {
                    if guard.is_empty() {
                        None
                    } else {
                        let data = guard.clone();
                        guard.clear();
                        Some(data)
                    }
                } else {
                    None
                }
            };

            if let Some(raw_data) = data_to_send {
                seq += 1;
                let (payload, compressed) = Packet::compress_data(&raw_data);
                let packet = Packet::ServerFrame {
                    seq,
                    data: payload,
                    compressed,
                };
                if writer.send(packet).await.is_err() {
                    break;
                }
            }
        }
    });

    let master_resize = Arc::clone(&master_pair);
    let pty_writer_input = Arc::clone(&pty_writer);

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
            }
            Ok(Packet::Ping { timestamp }) => {
                let _ = timestamp;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[mosh-tcp server] Packet decode error: {}", e);
                break;
            }
        }
    }

    send_task.abort();
    Ok(())
}

fn strip_terminal_queries(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        // Strip OSC 10/11 queries: \x1b]10;?\x1b\ or \x1b]10;?\x07 or \x1b]11;?\x1b\ or \x1b]11;?\x07
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
            }
        }

        // Strip DA / DA2 / XTVERSION queries: \x1b[c, \x1b[0c, \x1b[>c, \x1b[>0c, \x1b[>q
        if i + 3 <= data.len() && &data[i..i + 3] == b"\x1b[c" {
            i += 3;
            continue;
        }
        if i + 4 <= data.len() && (&data[i..i + 4] == b"\x1b[0c" || &data[i..i + 4] == b"\x1b[>c" || &data[i..i + 4] == b"\x1b[>q") {
            i += 4;
            continue;
        }
        if i + 5 <= data.len() && &data[i..i + 5] == b"\x1b[>0c" {
            i += 5;
            continue;
        }

        result.push(data[i]);
        i += 1;
    }
    result
}
