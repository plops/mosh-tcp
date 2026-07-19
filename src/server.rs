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
    let mut _child = pair.slave.spawn_command(cmd)?;

    let mut pty_reader = pair.master.try_clone_reader()?;
    let pty_writer = Arc::new(Mutex::new(pair.master.take_writer()?));
    let master_pair = Arc::new(Mutex::new(pair.master));

    // Shared buffer for accumulating PTY output between 20ms frames
    let pty_buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
    let pty_buffer_clone = Arc::clone(&pty_buffer);

    // Dedicated OS thread to read PTY stdout into buffer without blocking Tokio runtime
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut guard) = pty_buffer_clone.lock() {
                        guard.extend_from_slice(&buf[..n]);
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

    // Task 1: Frame accumulator & sender task (runs every frame_ms, e.g. 20ms)
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

    // Task 2: Receive inputs / resize requests from client
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
                // Ignore or handle ping if needed
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
