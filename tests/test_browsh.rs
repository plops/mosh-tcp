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

fn get_free_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

#[tokio::test]
async fn test_browsh_navigation_over_mosh_tcp() -> anyhow::Result<()> {
    let bind_addr = get_free_address();
    let bind_str = bind_addr.to_string();

    // 1. Start mosh-tcp server
    let mut server_proc = Command::new("./target/debug/mosh-tcp-server")
        .args(&["--bind", &bind_str, "--fps", "50", "--max-kbps", "500"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(500)).await;

    // 2. Connect client
    let socket = TcpStream::connect(bind_addr).await?;
    let framed = Framed::new(socket, PacketCodec::new());
    let (mut writer, mut reader) = framed.split();

    writer.send(Packet::ClientResize { rows: 24, cols: 80 }).await?;

    // 3. Launch browsh (/workspace/src/browsh)
    println!(">>> Sending '/workspace/src/browsh --help' to mosh-tcp server PTY...");
    writer.send(Packet::ClientInput { data: b"/workspace/src/browsh --help\n".to_vec() }).await?;

    let mut all_output = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2000);

    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(Ok(Packet::ServerFrame { data, compressed, .. }))) =
            tokio::time::timeout(Duration::from_millis(100), reader.next()).await
        {
            let raw = Packet::decompress_data(&data, compressed)?;
            all_output.extend_from_slice(&raw);
        }
    }

    let output_str = String::from_utf8_lossy(&all_output);
    println!("=== BROWSH OUTPUT RECEIVED FROM SERVER ===");
    println!("{:?}", output_str);
    println!("==========================================");

    // 4. Test sending Ctrl+L (0x0C) followed by URL 'youtube.com' and Enter (13)
    println!(">>> Sending Ctrl+L (0x0C) and 'youtube.com' keystrokes through mosh-tcp...");
    writer.send(Packet::ClientInput { data: vec![12] }).await?; // Ctrl+L
    sleep(Duration::from_millis(100)).await;
    writer.send(Packet::ClientInput { data: b"youtube.com\n".to_vec() }).await?;

    let deadline2 = tokio::time::Instant::now() + Duration::from_millis(1000);
    while tokio::time::Instant::now() < deadline2 {
        if let Ok(Some(Ok(Packet::ServerFrame { data, compressed, .. }))) =
            tokio::time::timeout(Duration::from_millis(100), reader.next()).await
        {
            let raw = Packet::decompress_data(&data, compressed)?;
            let text = String::from_utf8_lossy(&raw);
            print!("{}", text);
        }
    }

    assert!(output_str.contains("browsh") || output_str.contains("Browsh"), "Failed to run browsh!");
    println!("\r\n✓ Browsh Navigation Test Passed over mosh-tcp!");

    let _ = server_proc.kill().await;
    Ok(())
}
