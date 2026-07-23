use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
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
