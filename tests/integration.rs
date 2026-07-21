use mosh_tcp::protocol::{Packet, PacketCodec};
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::sleep;
use tokio_util::codec::Framed;

pub fn get_free_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

#[tokio::test]
async fn test_in_memory_duplex_framing() -> anyhow::Result<()> {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);

    let mut client_framed = Framed::new(client_io, PacketCodec::new());
    let mut server_framed = Framed::new(server_io, PacketCodec::new());

    // Client sends Packet::ClientResize
    client_framed.send(Packet::ClientResize { rows: 30, cols: 100 }).await?;

    // Server receives Packet::ClientResize
    let received = server_framed.next().await.unwrap()?;
    assert_eq!(received, Packet::ClientResize { rows: 30, cols: 100 });

    // Server sends compressed Packet::ServerFrame
    let payload = b"In-Memory Duplex Test Payload".to_vec();
    let (compressed_data, is_compressed) = Packet::compress_data(&payload);
    server_framed
        .send(Packet::ServerFrame {
            seq: 1,
            data: compressed_data,
            compressed: is_compressed,
        })
        .await?;

    // Client receives Packet::ServerFrame and decompresses
    let client_received = client_framed.next().await.unwrap()?;
    if let Packet::ServerFrame { data, compressed, .. } = client_received {
        let decompressed = Packet::decompress_data(&data, compressed)?;
        assert_eq!(decompressed, payload);
    } else {
        panic!("Expected ServerFrame packet!");
    }

    Ok(())
}

#[tokio::test]
async fn test_server_editing_and_heavy_output() -> anyhow::Result<()> {
    let bind_addr = get_free_address();
    let bind_str = bind_addr.to_string();

    // Launch server binary in background with 1000 KB/s cap for fast throughput test
    let mut server_proc = Command::new("./target/debug/mosh-tcp-server")
        .args(&["--bind", &bind_str, "--fps", "50", "--max-kbps", "1000"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(500)).await;

    let socket = TcpStream::connect(bind_addr).await?;
    let framed = Framed::new(socket, PacketCodec::new());
    let (mut writer, mut reader) = framed.split();

    writer.send(Packet::ClientResize { rows: 24, cols: 80 }).await?;

    // --- TEST 1: Command Line Editing & Backspaces ---
    let mut input_bytes = Vec::new();
    input_bytes.extend_from_slice(b"echo WRONG_TEXT");
    input_bytes.extend(std::iter::repeat(127).take(10));
    input_bytes.extend_from_slice(b"CORRECT_TEXT\n");

    writer.send(Packet::ClientInput { data: input_bytes }).await?;

    let mut received_output = Vec::new();
    let mut found_correct = false;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(Ok(Packet::ServerFrame { data, compressed, .. }))) =
            tokio::time::timeout(Duration::from_millis(100), reader.next()).await
        {
            let raw = Packet::decompress_data(&data, compressed)?;
            received_output.extend_from_slice(&raw);
            let text = String::from_utf8_lossy(&received_output);
            if text.contains("CORRECT_TEXT") && !text.contains("echo WRONG_TEXT\r\nWRONG_TEXT") {
                found_correct = true;
                break;
            }
        }
    }

    assert!(found_correct, "Line editing with backspaces failed!");
    println!("✓ Interactive Command Line Editing & Backspace Test Passed!");

    // --- TEST 2: Massive Text Stream ---
    let heavy_cmd = b"seq 1 50000\necho MASSIVE_STREAM_COMPLETED\n";
    writer.send(Packet::ClientInput { data: heavy_cmd.to_vec() }).await?;

    let mut heavy_output_len = 0;
    let mut compressed_frames_count = 0;
    let mut found_done = false;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(Ok(Packet::ServerFrame { data, compressed, .. }))) =
            tokio::time::timeout(Duration::from_millis(100), reader.next()).await
        {
            if compressed {
                compressed_frames_count += 1;
            }
            let raw = Packet::decompress_data(&data, compressed)?;
            heavy_output_len += raw.len();
            let text = String::from_utf8_lossy(&raw);
            if text.contains("MASSIVE_STREAM_COMPLETED") {
                found_done = true;
                break;
            }
        }
    }

    assert!(found_done, "Server failed on 50,000 lines heavy stream!");
    assert!(heavy_output_len > 0, "Stream output length too small!");

    println!(
        "✓ Massive Text Stream Test Passed! Processed {} bytes of decompressed stream data across 20ms frames ({} frames compressed).",
        heavy_output_len, compressed_frames_count
    );

    let _ = server_proc.kill().await;
    Ok(())
}
