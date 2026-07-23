use mosh_tcp::server;
use std::env;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();

    let mut bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let mut fps = 50u64;
    let mut max_kbps = 6u64;
    let mut stats = false;
    let mut shell = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--bind" | "-b" => {
                if i + 1 < args.len() {
                    if let Ok(addr) = args[i + 1].parse() {
                        bind = addr;
                        i += 1;
                    }
                }
            }
            "--fps" => {
                if i + 1 < args.len() {
                    if let Ok(val) = args[i + 1].parse() {
                        fps = val;
                        i += 1;
                    }
                }
            }
            "--max-kbps" => {
                if i + 1 < args.len() {
                    if let Ok(val) = args[i + 1].parse() {
                        max_kbps = val;
                        i += 1;
                    }
                }
            }
            "--stats" => {
                stats = true;
            }
            "--shell" => {
                if i + 1 < args.len() {
                    shell = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    server::run_server(bind, fps, max_kbps, stats, shell).await?;
    Ok(())
}

fn print_usage() {
    println!("mosh-tcp-server - Latency-tolerant terminal server\n");
    println!("Usage:");
    println!("  mosh-tcp-server [--bind <ADDR:PORT>] [--fps <FPS>] [--max-kbps <KB/S>] [--stats] [--shell <PATH>]");
}
