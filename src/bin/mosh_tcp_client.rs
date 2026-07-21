use mosh_tcp::client;
use std::env;
use std::net::SocketAddr;

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    let mut connect: SocketAddr = "127.0.0.1:4000".parse().unwrap();
    let mut predict = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--connect" | "-c" => {
                if i + 1 < args.len() {
                    if let Ok(addr) = args[i + 1].parse() {
                        connect = addr;
                        i += 1;
                    }
                }
            }
            "--predict" => {
                predict = true;
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    client::run_client(connect, predict)?;
    Ok(())
}

fn print_usage() {
    println!("mosh-tcp-client - Lightweight latency-tolerant terminal client\n");
    println!("Usage:");
    println!("  mosh-tcp-client [--connect <ADDR:PORT>] [--predict]");
}
