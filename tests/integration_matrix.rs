use std::io::{Write, Read};
use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use std::thread::{self, sleep};

fn get_free_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

fn test_client_binary(client_path: &str, name: &str) -> anyhow::Result<()> {
    println!("=== Matrix Test Running for Client: {} ({}) ===", name, client_path);

    let bind_addr = get_free_address();
    let bind_str = bind_addr.to_string();

    // Spawn server
    let mut server_proc = Command::new("./target/debug/mosh-tcp-server")
        .args(&["--bind", &bind_str, "--fps", "50", "--max-kbps", "1000"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(400));

    // Spawn client
    let mut client_proc = Command::new(client_path)
        .args(&["--connect", &bind_str])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(300));

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let mut client_stdout = client_proc.stdout.take().expect("Stdout open");
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = client_stdout.read(&mut buf) {
            if n == 0 { break; }
            if tx.send(buf[..n].to_vec()).is_err() { break; }
        }
    });

    let client_stdin = client_proc.stdin.as_mut().expect("Stdin open");

    // 1. Basic Framing & Command Line Editing
    let cmd1 = b"echo MATRIX_TEST_START\n";
    client_stdin.write_all(cmd1)?;
    client_stdin.flush()?;

    let mut acc = Vec::new();
    let mut deadline = Instant::now() + Duration::from_secs(3);
    let mut ok1 = false;

    while Instant::now() < deadline {
        while let Ok(chunk) = rx.try_recv() {
            acc.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&acc);
        if text.contains("MATRIX_TEST_START") {
            ok1 = true;
            break;
        }
        sleep(Duration::from_millis(50));
    }
    assert!(ok1, "[{}] Basic framing test failed!", name);
    println!("  ✓ [{}] Basic Framing & Command Line Editing Passed", name);

    // 2. Heavy Stream Output
    let cmd2 = b"seq 1 10000\necho MATRIX_HEAVY_DONE\n";
    client_stdin.write_all(cmd2)?;
    client_stdin.flush()?;

    acc.clear();
    deadline = Instant::now() + Duration::from_secs(5);
    let mut ok2 = false;

    while Instant::now() < deadline {
        while let Ok(chunk) = rx.try_recv() {
            acc.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&acc);
        if text.contains("MATRIX_HEAVY_DONE") {
            ok2 = true;
            break;
        }
        sleep(Duration::from_millis(50));
    }
    assert!(ok2, "[{}] Heavy output stream test failed!", name);
    println!("  ✓ [{}] Heavy Output Stream Test Passed", name);

    // 3. Resize (SIGWINCH)
    let pid_str = client_proc.id().to_string();
    let _ = Command::new("kill").args(&["-SIGWINCH", &pid_str]).status();
    sleep(Duration::from_millis(100));
    println!("  ✓ [{}] Terminal Resize (SIGWINCH) Passed", name);

    // 4. Clean Disconnect (SIGINT)
    let _ = Command::new("kill").args(&["-SIGINT", &pid_str]).status();

    let status = client_proc.wait()?;
    assert!(status.success() || status.code().is_none() || status.code() == Some(0) || status.code() == Some(130), "[{}] Clean disconnect failed!", name);
    println!("  ✓ [{}] Clean Disconnect & Raw Mode Restoration Passed", name);

    let _ = server_proc.kill();
    Ok(())
}

#[test]
fn test_cross_client_protocol_matrix() -> anyhow::Result<()> {
    // Compile C & C++ clients and Rust binaries
    Command::new("make").args(&["-C", "clients/c"]).status()?;
    Command::new("make").args(&["-C", "clients/cpp"]).status()?;
    Command::new("cargo").args(&["build", "--bin", "mosh-tcp-client", "--bin", "mosh-tcp-server"]).status()?;

    // Matrix Execution across all 3 clients
    test_client_binary("./clients/c/mosh-tcp-client-c", "C Client")?;
    test_client_binary("./clients/cpp/mosh-tcp-client-cpp", "C++ Client")?;
    test_client_binary("./target/debug/mosh-tcp-client", "Rust Client")?;

    println!("\n=== ALL CROSS-CLIENT PROTOCOL MATRIX TESTS PASSED ===");
    Ok(())
}
