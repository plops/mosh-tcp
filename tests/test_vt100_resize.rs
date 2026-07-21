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
async fn test_vt100_resize_with_wide_characters_does_not_panic() -> anyhow::Result<()> {
    let bind_addr: SocketAddr = "127.0.0.1:4098".parse()?;

    // 1. Launch server
    let mut server_proc = Command::new("./target/debug/mosh-tcp")
        .args(&["server", "--bind", "127.0.0.1:4098", "--stats"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(500)).await;

    // 2. Connect client
    let socket = TcpStream::connect(bind_addr).await?;
    let framed = Framed::new(socket, PacketCodec::new());
    let (mut writer, mut reader) = framed.split();

    writer.send(Packet::ClientResize { rows: 24, cols: 80 }).await?;

    // 3. Output wide characters (CJK characters and emojis) near boundary
    println!(">>> Sending wide CJK character stream to PTY...");
    let wide_char_cmd = "python3 -c \"import sys; sys.stdout.write(' ' * 78 + '日本語テスト\\n'); sys.stdout.flush()\"\n".as_bytes();
    writer.send(Packet::ClientInput { data: wide_char_cmd.to_vec() }).await?;

    sleep(Duration::from_millis(300)).await;

    // 4. Perform rapid resize events across various dimensions (including 79 cols where wide char was truncated, and 0 cols)
    println!(">>> Sending rapid resize packets to test wide-character boundary handling...");
    let resize_dimensions = [
        (24, 79),
        (24, 80),
        (24, 78),
        (0, 0),
        (30, 100),
        (10, 40),
        (24, 80),
    ];

    for &(rows, cols) in &resize_dimensions {
        writer.send(Packet::ClientResize { rows, cols }).await?;
        sleep(Duration::from_millis(50)).await;

        // Send more text after each resize to trigger vt100 process()
        let test_input = b"echo RESIZE_CHECK\n";
        writer.send(Packet::ClientInput { data: test_input.to_vec() }).await?;
        sleep(Duration::from_millis(50)).await;
    }

    // 5. Verify server is still alive and responding
    let mut got_response = false;
    let timeout = Duration::from_millis(2000);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if let Ok(Some(Ok(Packet::ServerFrame { .. }))) =
            tokio::time::timeout(Duration::from_millis(100), reader.next()).await
        {
            got_response = true;
            break;
        }
    }

    assert!(got_response, "Server did not respond after rapid resize operations!");

    // Verify process didn't crash
    assert!(
        server_proc.try_wait()?.is_none(),
        "Server process panicked or aborted unexpectedly during resize test!"
    );

    let _ = server_proc.kill().await;
    Ok(())
}
