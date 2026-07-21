use mosh_tcp::protocol::{Packet, PacketCodec};
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use std::net::SocketAddr;
use std::process::Stdio;
use std::time::{Duration, Instant};
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
async fn test_bandwidth_throttling_and_frame_skipping() -> anyhow::Result<()> {
    let bind_addr = get_free_address();
    let bind_str = bind_addr.to_string();

    // 1. Launch server with 6 KB/s max bandwidth limit and --stats enabled
    let mut server_proc = Command::new("./target/debug/mosh-tcp-server")
        .args(&["--bind", &bind_str, "--max-kbps", "6", "--stats"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(500)).await;

    // 2. Connect client
    let socket = TcpStream::connect(bind_addr).await?;
    let framed = Framed::new(socket, PacketCodec::new());
    let (mut writer, mut reader) = framed.split();

    writer.send(Packet::ClientResize { rows: 24, cols: 80 }).await?;

    // 3. Simulate kiro-cli / tmux heavy history dump (300,000 bytes of text output)
    println!(">>> Dumping 300,000 bytes of text to test 6 KB/s bandwidth cap...");
    let heavy_cmd = b"seq 1 50000\necho THROTTLED_STREAM_END\n";
    writer.send(Packet::ClientInput { data: heavy_cmd.to_vec() }).await?;

    let start_time = Instant::now();
    let mut total_net_bytes = 0;
    let mut frame_count = 0;

    // Read for 3 seconds
    let test_duration = Duration::from_millis(3000);
    while Instant::now() - start_time < test_duration {
        if let Ok(Some(Ok(Packet::ServerFrame { data, .. }))) =
            tokio::time::timeout(Duration::from_millis(100), reader.next()).await
        {
            total_net_bytes += data.len();
            frame_count += 1;
        }
    }

    let elapsed_secs = start_time.elapsed().as_secs_f64();
    let kb_per_sec = (total_net_bytes as f64 / 1024.0) / elapsed_secs;

    println!("=== BANDWIDTH RATE LIMITER TEST RESULTS ===");
    println!("Elapsed Time: {:.2} s", elapsed_secs);
    println!("Total Net Bytes Transferred: {} bytes across {} frames", total_net_bytes, frame_count);
    println!("Effective Net Transfer Speed: {:.2} KB/s (Max Cap: 6.0 KB/s)", kb_per_sec);
    println!("===========================================");

    // Verify bandwidth cap: net transfer speed must NOT exceed 7.5 KB/s (allowing slight burst tolerance)
    assert!(
        kb_per_sec <= 7.5,
        "Rate limiter failed! Measured speed {:.2} KB/s exceeded 6 KB/s limit!",
        kb_per_sec
    );

    let _ = server_proc.kill().await;
    Ok(())
}

#[tokio::test]
async fn test_carbonyl_ansi_heavy_rate_limit() -> anyhow::Result<()> {
    let bind_addr = get_free_address();
    let bind_str = bind_addr.to_string();

    // 1. Launch server with 6 KB/s max bandwidth limit
    let mut server_proc = Command::new("./target/debug/mosh-tcp-server")
        .args(&["--bind", &bind_str, "--max-kbps", "6", "--stats"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(500)).await;

    // 2. Connect client
    let socket = TcpStream::connect(bind_addr).await?;
    let framed = Framed::new(socket, PacketCodec::new());
    let (mut writer, mut reader) = framed.split();

    writer.send(Packet::ClientResize { rows: 24, cols: 80 }).await?;

    // 3. Simulate Carbonyl high-frequency truecolor escape sequence output
    println!(">>> Dumping heavy Carbonyl ANSI escape sequence buffer to test rate limit...");
    let heavy_carbonyl_cmd = b"python3 -c \"import sys, time; [(sys.stdout.write(f'\\x1b[38;2;{i%256};{(i*3)%256};{(i*7)%256}mX\\x1b[0m' * 500 + '\\n'), sys.stdout.flush(), time.sleep(0.005)) for i in range(100)]\"\n";
    writer.send(Packet::ClientInput { data: heavy_carbonyl_cmd.to_vec() }).await?;

    let start_time = Instant::now();
    let mut total_net_bytes = 0;
    let mut frame_count = 0;

    let test_duration = Duration::from_millis(2500);
    while Instant::now() - start_time < test_duration {
        if let Ok(Some(Ok(Packet::ServerFrame { data, .. }))) =
            tokio::time::timeout(Duration::from_millis(100), reader.next()).await
        {
            total_net_bytes += data.len();
            frame_count += 1;
        }
    }

    let elapsed_secs = start_time.elapsed().as_secs_f64();
    let kb_per_sec = (total_net_bytes as f64 / 1024.0) / elapsed_secs;

    println!("=== CARBONYL ANSI HEAVY RATE LIMITER TEST RESULTS ===");
    println!("Elapsed Time: {:.2} s", elapsed_secs);
    println!("Total Net Bytes Transferred: {} bytes across {} frames", total_net_bytes, frame_count);
    println!("Effective Net Speed: {:.2} KB/s (Max Cap: 6.0 KB/s)", kb_per_sec);
    println!("=====================================================");

    assert!(
        kb_per_sec <= 7.5,
        "Carbonyl rate limit failed! Measured speed {:.2} KB/s exceeded 6 KB/s limit!",
        kb_per_sec
    );

    let _ = server_proc.kill().await;
    Ok(())
}
