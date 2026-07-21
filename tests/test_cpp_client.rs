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

#[test]
fn test_cpp_client_integration() -> anyhow::Result<()> {
    // 1. Compile C++ client
    let make_status = Command::new("make")
        .args(&["-C", "clients/cpp"])
        .status()?;
    assert!(make_status.success(), "Failed to build mosh-tcp-client-cpp");

    // 2. Ensure mosh-tcp-server is built
    let build_status = Command::new("cargo")
        .args(&["build", "--bin", "mosh-tcp-server"])
        .status()?;
    assert!(build_status.success(), "Failed to build mosh-tcp-server");

    let bind_addr = get_free_address();
    let bind_str = bind_addr.to_string();

    // 3. Spawn mosh-tcp-server
    let mut server_proc = Command::new("./target/debug/mosh-tcp-server")
        .args(&["--bind", &bind_str, "--fps", "50"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(500));

    // 4. Spawn C++ client connecting to server
    let mut client_proc = Command::new("./clients/cpp/mosh-tcp-client-cpp")
        .args(&["--connect", &bind_str])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(300));

    // Background thread to read client stdout non-blockingly
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let mut client_stdout = client_proc.stdout.take().expect("Failed to open stdout");
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = client_stdout.read(&mut buf) {
            if n == 0 { break; }
            if tx.send(buf[..n].to_vec()).is_err() { break; }
        }
    });

    // 5. Test input flow through C++ client
    let client_stdin = client_proc.stdin.as_mut().expect("Failed to open stdin");
    let test_cmd = b"echo CPP_CLIENT_INTEGRATION_TEST_PASSED\n";
    client_stdin.write_all(test_cmd)?;
    client_stdin.flush()?;

    let mut accumulated = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut found = false;

    while Instant::now() < deadline {
        while let Ok(chunk) = rx.try_recv() {
            accumulated.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&accumulated);
        if text.contains("CPP_CLIENT_INTEGRATION_TEST_PASSED") {
            found = true;
            break;
        }
        sleep(Duration::from_millis(50));
    }

    assert!(found, "C++ client failed to transmit or render server output!");

    // 6. Test SIGWINCH handling
    let pid_str = client_proc.id().to_string();
    let _ = Command::new("kill").args(&["-SIGWINCH", &pid_str]).status();
    sleep(Duration::from_millis(100));

    // 7. Clean disconnect with SIGINT
    let _ = Command::new("kill").args(&["-SIGINT", &pid_str]).status();

    let status = client_proc.wait()?;
    assert!(status.success() || status.code().is_none() || status.code() == Some(0) || status.code() == Some(130), "C++ client did not exit cleanly on SIGINT");

    let _ = server_proc.kill();
    Ok(())
}
