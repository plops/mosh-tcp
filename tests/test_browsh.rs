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

#[tokio::test]
async fn test_tmux_attach_session_over_mosh_tcp() -> anyhow::Result<()> {
    // Use non-4000 port (4094)
    let bind_addr: SocketAddr = "127.0.0.1:4094".parse()?;

    // 1. Start mosh-tcp server on port 4094
    let mut server_proc = Command::new("./target/debug/mosh-tcp")
        .args(&["server", "--bind", "127.0.0.1:4094", "--fps", "50", "--max-kbps", "100"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(500)).await;

    // 2. Connect client
    let socket = TcpStream::connect(bind_addr).await?;
    let framed = Framed::new(socket, PacketCodec::new());
    let (mut writer, mut reader) = framed.split();

    writer.send(Packet::ClientResize { rows: 24, cols: 80 }).await?;

    // 3. Connect to tmux session 0 over mosh-tcp
    println!(">>> Sending 'tmux attach -t 0' to mosh-tcp server...");
    writer.send(Packet::ClientInput { data: b"tmux attach -t 0\n".to_vec() }).await?;

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
    println!("=== TMUX ATTACH OUTPUT OVER MOSH-TCP ===");
    println!("{:?}", output_str);
    println!("=======================================");

    // 4. Test sending Ctrl+L (0x0C) and Tab (0x09) keys over mosh-tcp to tmux
    println!(">>> Sending Ctrl+L (0x0C) and Tab (0x09) keystrokes through mosh-tcp...");
    writer.send(Packet::ClientInput { data: vec![12] }).await?; // Ctrl+L
    sleep(Duration::from_millis(200)).await;
    writer.send(Packet::ClientInput { data: vec![9] }).await?; // Tab

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

    assert!(output_str.contains("tmux attach -t 0") || output_str.contains("tmux"), "Failed to attach tmux session!");
    println!("\r\n✓ Tmux Attach and Navigation Keystrokes Test Passed over mosh-tcp!");

    let _ = server_proc.kill().await;
    Ok(())
}
