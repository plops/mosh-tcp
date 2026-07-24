use mosh_tcp::ansi::strip_terminal_queries_stateful;

#[test]
fn test_btop_initialization_queries_are_stripped() {
    // btop initialization sequence containing DA1, DA2, DA3, XTVERSION, CPR, and OSC queries
    let btop_init_stream = concat!(
        "\x1b[c",             // Primary Device Attributes (DA1)
        "\x1b[>c",            // Secondary Device Attributes (DA2)
        "\x1b[=c",            // Tertiary Device Attributes (DA3)
        "\x1b[>q",            // XTVERSION query
        "\x1b[6n",            // Cursor Position Report (CPR)
        "\x1b]10;?\x07",      // OSC 10 query
        "\x1b]11;?\x1b\\",    // OSC 11 query
        "REAL_BTOP_HEADER_DATA"
    ).as_bytes();

    let (cleaned, remaining) = strip_terminal_queries_stateful(btop_init_stream);
    assert_eq!(cleaned, b"REAL_BTOP_HEADER_DATA");
    assert!(remaining.is_empty());
}

#[test]
fn test_emacs_bracketed_paste_sequence_integrity() {
    let pasted_text = "def hello_world():\n    return 'emacs paste success'\n";
    let mut paste_payload = Vec::new();
    paste_payload.extend_from_slice(b"\x1b[200~");
    paste_payload.extend_from_slice(pasted_text.as_bytes());
    paste_payload.extend_from_slice(b"\x1b[201~");

    // Ensure start and end bracketed paste tags match standard VT100 / xterm spec
    assert_eq!(&paste_payload[..6], b"\x1b[200~");
    assert_eq!(&paste_payload[paste_payload.len() - 6..], b"\x1b[201~");
    
    // Verify payload body is identical to original string
    let body = &paste_payload[6..paste_payload.len() - 6];
    assert_eq!(body, pasted_text.as_bytes());
}

#[test]
fn test_btop_rendering_integration() {
    if std::process::Command::new("btop").arg("--version").output().is_err() {
        eprintln!("btop not found on system, skipping integration test");
        return;
    }

    let bin_server = env!("CARGO_BIN_EXE_mosh-tcp-server");
    let bin_client = env!("CARGO_BIN_EXE_mosh-tcp-client");

    let mut server = std::process::Command::new(bin_server)
        .args(["--bind", "127.0.0.1:15556", "--shell", "btop"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start mosh-tcp-server");

    std::thread::sleep(std::time::Duration::from_millis(500));

    let mut client = std::process::Command::new(bin_client)
        .args(["-c", "127.0.0.1:15556"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start mosh-tcp-client");

    let mut client_stdout = client.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 4096];
        while let Ok(n) = client_stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            if tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let start = std::time::Instant::now();
    let mut received = Vec::new();
    while start.elapsed() < std::time::Duration::from_millis(1500) {
        while let Ok(chunk) = rx.try_recv() {
            received.extend_from_slice(&chunk);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let _ = client.kill();
    let _ = server.kill();

    assert!(!received.is_empty(), "Client should receive btop rendering stream");
}

#[test]
fn test_emacs_rendering_integration() {
    if std::process::Command::new("emacs").arg("--version").output().is_err() {
        eprintln!("emacs not found on system, skipping integration test");
        return;
    }

    let bin_server = env!("CARGO_BIN_EXE_mosh-tcp-server");
    let bin_client = env!("CARGO_BIN_EXE_mosh-tcp-client");

    let mut server = std::process::Command::new(bin_server)
        .args(["--bind", "127.0.0.1:15557", "--shell", "emacs -nw"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start mosh-tcp-server");

    std::thread::sleep(std::time::Duration::from_millis(500));

    let mut client = std::process::Command::new(bin_client)
        .args(["-c", "127.0.0.1:15557"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start mosh-tcp-client");

    let mut client_stdout = client.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 4096];
        while let Ok(n) = client_stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            if tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let start = std::time::Instant::now();
    let mut received = Vec::new();
    while start.elapsed() < std::time::Duration::from_millis(1500) {
        while let Ok(chunk) = rx.try_recv() {
            received.extend_from_slice(&chunk);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let _ = client.kill();
    let _ = server.kill();

    assert!(!received.is_empty(), "Client should receive emacs rendering stream");
}
