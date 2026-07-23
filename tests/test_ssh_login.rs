use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, sleep};
use std::time::{Duration, Instant};

fn build_all() -> anyhow::Result<()> {
    let cargo_status = Command::new("cargo")
        .args(&["build", "--bin", "mosh-tcp-server", "--bin", "mosh-tcp-client"])
        .status()?;
    assert!(cargo_status.success(), "Failed to build Rust binaries");

    let make_c = Command::new("make")
        .args(&["-C", "clients/c"])
        .status()?;
    assert!(make_c.success(), "Failed to build C client");

    let make_cpp = Command::new("make")
        .args(&["-C", "clients/cpp"])
        .status()?;
    assert!(make_cpp.success(), "Failed to build C++ client");

    Ok(())
}

fn run_client_ssh_test(client_cmd: &str, client_args: &[&str], expected_output: &str) -> anyhow::Result<()> {
    build_all()?;

    let mock_ssh_path = std::fs::canonicalize("tests/mock_ssh.sh")?;
    let mock_ssh_str = mock_ssh_path.to_str().unwrap();

    let server_path = std::fs::canonicalize("target/debug/mosh-tcp-server")?;
    let server_str = server_path.to_str().unwrap();

    let mut full_args = vec![
        "--ssh", mock_ssh_str,
        "--server", server_str,
    ];
    full_args.extend_from_slice(client_args);
    full_args.push("testuser@localhost");

    let mut client_proc = Command::new(client_cmd)
        .args(&full_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(500));

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let mut client_stdout = client_proc.stdout.take().expect("Failed to open stdout");
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = client_stdout.read(&mut buf) {
            if n == 0 { break; }
            if tx.send(buf[..n].to_vec()).is_err() { break; }
        }
    });

    let client_stdin = client_proc.stdin.as_mut().expect("Failed to open stdin");
    let test_cmd = format!("echo {}\n", expected_output);
    client_stdin.write_all(test_cmd.as_bytes())?;
    client_stdin.flush()?;

    let mut accumulated = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut found = false;

    while Instant::now() < deadline {
        while let Ok(chunk) = rx.try_recv() {
            accumulated.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&accumulated);
        if text.contains(expected_output) {
            found = true;
            break;
        }
        sleep(Duration::from_millis(50));
    }

    assert!(found, "Client {} failed SSH streamlined login test! Output received: {}", client_cmd, String::from_utf8_lossy(&accumulated));

    let pid_str = client_proc.id().to_string();
    let _ = Command::new("kill").args(&["-SIGINT", &pid_str]).status();
    let status = client_proc.wait()?;
    assert!(status.success() || status.code().is_none() || status.code() == Some(0) || status.code() == Some(130));

    Ok(())
}

#[test]
fn test_rust_client_ssh_login() -> anyhow::Result<()> {
    run_client_ssh_test("./target/debug/mosh-tcp-client", &[], "RUST_SSH_LOGIN_SUCCESS")
}

#[test]
fn test_c_client_ssh_login() -> anyhow::Result<()> {
    run_client_ssh_test("./clients/c/mosh-tcp-client-c", &[], "C_SSH_LOGIN_SUCCESS")
}

#[test]
fn test_cpp_client_ssh_login() -> anyhow::Result<()> {
    run_client_ssh_test("./clients/cpp/mosh-tcp-client-cpp", &[], "CPP_SSH_LOGIN_SUCCESS")
}

#[test]
fn test_rust_client_keystroke_reliability() -> anyhow::Result<()> {
    build_all()?;
    let mock_ssh_path = std::fs::canonicalize("tests/mock_ssh.sh")?;
    let mock_ssh_str = mock_ssh_path.to_str().unwrap();
    let server_path = std::fs::canonicalize("target/debug/mosh-tcp-server")?;
    let server_str = server_path.to_str().unwrap();

    let mut client_proc = Command::new("./target/debug/mosh-tcp-client")
        .args(&["--ssh", mock_ssh_str, "--server", server_str, "testuser@localhost"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(500));

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let mut stdout = client_proc.stdout.take().unwrap();
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 { break; }
            if tx.send(buf[..n].to_vec()).is_err() { break; }
        }
    });

    let stdin = client_proc.stdin.as_mut().unwrap();
    let keys = ['a', 'b', 'c', '1', '2', '3', 'X', 'Y', 'Z'];
    for k in keys {
        stdin.write_all(&[k as u8])?;
        stdin.flush()?;
        sleep(Duration::from_millis(30));
    }
    stdin.write_all(b"\n")?;
    stdin.flush()?;

    let mut accumulated = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(4);
    let expected = "abc123XYZ";
    let mut found = false;

    while Instant::now() < deadline {
        while let Ok(chunk) = rx.try_recv() {
            accumulated.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&accumulated);
        if text.contains(expected) {
            found = true;
            break;
        }
        sleep(Duration::from_millis(50));
    }

    assert!(found, "Keystroke reliability failed for Rust client. Received: {}", String::from_utf8_lossy(&accumulated));

    let _ = Command::new("kill").args(&["-SIGINT", &client_proc.id().to_string()]).status();
    let _ = client_proc.wait();
    Ok(())
}

#[test]
fn test_rust_client_clean_exit_on_exit_cmd() -> anyhow::Result<()> {
    build_all()?;
    let mock_ssh_path = std::fs::canonicalize("tests/mock_ssh.sh")?;
    let mock_ssh_str = mock_ssh_path.to_str().unwrap();
    let server_path = std::fs::canonicalize("target/debug/mosh-tcp-server")?;
    let server_str = server_path.to_str().unwrap();

    let mut client_proc = Command::new("./target/debug/mosh-tcp-client")
        .args(&["--ssh", mock_ssh_str, "--server", server_str, "testuser@localhost"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout = client_proc.stdout.take().unwrap();
    let mut stderr = client_proc.stderr.take().unwrap();
    let stderr_acc = Arc::new(Mutex::new(Vec::new()));
    let stderr_acc_clone = Arc::clone(&stderr_acc);
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 { break; }
        }
    });
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 { break; }
            stderr_acc_clone.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });

    sleep(Duration::from_millis(500));

    let mut stdin = client_proc.stdin.take().unwrap();
    stdin.write_all(b"exit\n")?;
    stdin.flush()?;
    drop(stdin);

    let start = Instant::now();
    let mut exited_cleanly = false;

    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Some(status)) = client_proc.try_wait() {
            assert!(
                status.success() || status.code() == Some(0),
                "Rust client exited with error status: {:?}",
                status
            );
            exited_cleanly = true;
            break;
        }
        sleep(Duration::from_millis(100));
    }

    if !exited_cleanly {
        let stderr_bytes = stderr_acc.lock().unwrap().clone();
        let _ = client_proc.kill();
        panic!(
            "Rust client failed to exit cleanly upon 'exit' command within 5s! Stderr output:\n{}",
            String::from_utf8_lossy(&stderr_bytes)
        );
    }

    Ok(())
}

#[test]
fn test_multiple_concurrent_sessions_same_server() -> anyhow::Result<()> {
    build_all()?;
    let mock_ssh_path = std::fs::canonicalize("tests/mock_ssh.sh")?;
    let mock_ssh_str = mock_ssh_path.to_str().unwrap();
    let server_path = std::fs::canonicalize("target/debug/mosh-tcp-server")?;
    let server_str = server_path.to_str().unwrap();

    // Spawn Client 1 (Rust client)
    let mut c1 = Command::new("./target/debug/mosh-tcp-client")
        .args(&["--ssh", mock_ssh_str, "--server", server_str, "user1@localhost"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Spawn Client 2 (Rust client)
    let mut c2 = Command::new("./target/debug/mosh-tcp-client")
        .args(&["--ssh", mock_ssh_str, "--server", server_str, "user2@localhost"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(600));

    let (tx1, rx1) = mpsc::channel::<Vec<u8>>();
    let mut out1 = c1.stdout.take().unwrap();
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = out1.read(&mut buf) {
            if n == 0 { break; }
            let _ = tx1.send(buf[..n].to_vec());
        }
    });

    let (tx2, rx2) = mpsc::channel::<Vec<u8>>();
    let mut out2 = c2.stdout.take().unwrap();
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = out2.read(&mut buf) {
            if n == 0 { break; }
            let _ = tx2.send(buf[..n].to_vec());
        }
    });

    // Write distinct commands to each client
    c1.stdin.as_mut().unwrap().write_all(b"echo SESSION_ONE_SECRET\n")?;
    c1.stdin.as_mut().unwrap().flush()?;

    c2.stdin.as_mut().unwrap().write_all(b"echo SESSION_TWO_SECRET\n")?;
    c2.stdin.as_mut().unwrap().flush()?;

    let mut acc1 = Vec::new();
    let mut acc2 = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut f1 = false;
    let mut f2 = false;

    while Instant::now() < deadline {
        while let Ok(ch) = rx1.try_recv() { acc1.extend_from_slice(&ch); }
        while let Ok(ch) = rx2.try_recv() { acc2.extend_from_slice(&ch); }

        let t1 = String::from_utf8_lossy(&acc1);
        let t2 = String::from_utf8_lossy(&acc2);

        if t1.contains("SESSION_ONE_SECRET") { f1 = true; }
        if t2.contains("SESSION_TWO_SECRET") { f2 = true; }

        if f1 && f2 { break; }
        sleep(Duration::from_millis(50));
    }

    assert!(f1, "Session 1 did not receive its output!");
    assert!(f2, "Session 2 did not receive its output!");

    // Clean exit both
    c1.stdin.as_mut().unwrap().write_all(b"exit\n")?;
    c1.stdin.as_mut().unwrap().flush()?;
    c2.stdin.as_mut().unwrap().write_all(b"exit\n")?;
    c2.stdin.as_mut().unwrap().flush()?;

    let _ = c1.wait();
    let _ = c2.wait();

    Ok(())
}
