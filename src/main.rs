use std::env;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    match args[1].as_str() {
        #[cfg(feature = "server")]
        "server" => {
            let mut bind: SocketAddr = "0.0.0.0:4000".parse().unwrap();
            let mut fps = 50u64;
            let mut max_kbps = 6u64;
            let mut stats = false;
            let mut shell = None;

            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--bind" | "-b" => {
                        if i + 1 < args.len() {
                            bind = args[i + 1].parse()?;
                            i += 1;
                        }
                    }
                    "--fps" => {
                        if i + 1 < args.len() {
                            fps = args[i + 1].parse()?;
                            i += 1;
                        }
                    }
                    "--max-kbps" => {
                        if i + 1 < args.len() {
                            max_kbps = args[i + 1].parse()?;
                            i += 1;
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
                    _ => {}
                }
                i += 1;
            }
            mosh_tcp::server::run_server(bind, fps, max_kbps, stats, shell).await?;
        }
        #[cfg(feature = "client")]
        "client" => {
            let mut connect: SocketAddr = "127.0.0.1:4000".parse().unwrap();
            let mut predict = false;

            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--connect" | "-c" => {
                        if i + 1 < args.len() {
                            connect = args[i + 1].parse()?;
                            i += 1;
                        }
                    }
                    "--predict" => {
                        predict = true;
                    }
                    _ => {}
                }
                i += 1;
            }
            mosh_tcp::client::run_client(connect, predict)?;
        }
        _ => print_usage(),
    }

    Ok(())
}

fn print_usage() {
    println!("mosh-tcp - A high-latency resilient, frame-rate limited terminal tool\n");
    println!("Usage:");
    println!("  mosh-tcp server [--bind <ADDR:PORT>] [--fps <FPS>] [--max-kbps <KB/S>] [--stats] [--shell <PATH>]");
    println!("  mosh-tcp client [--connect <ADDR:PORT>] [--predict]");
}
