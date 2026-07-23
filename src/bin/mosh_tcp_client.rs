use mosh_tcp::client::{self, SshTunnel};
use std::env;
use std::net::SocketAddr;

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    let mut connect_addr: Option<SocketAddr> = None;
    let mut target_host: Option<String> = None;
    let mut predict = false;
    let mut ssh_cmd = "ssh".to_string();
    let mut ssh_port: Option<u16> = None;
    let mut remote_server_cmd = "mosh-tcp-server".to_string();
    let mut remote_port: u16 = 4000;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--connect" | "-c" => {
                if i + 1 < args.len() {
                    if let Ok(addr) = args[i + 1].parse() {
                        connect_addr = Some(addr);
                        i += 1;
                    }
                }
            }
            "--ssh" | "-s" => {
                if i + 1 < args.len() {
                    ssh_cmd = args[i + 1].clone();
                    i += 1;
                }
            }
            "--ssh-port" | "-p" => {
                if i + 1 < args.len() {
                    if let Ok(port) = args[i + 1].parse() {
                        ssh_port = Some(port);
                        i += 1;
                    }
                }
            }
            "--server" => {
                if i + 1 < args.len() {
                    remote_server_cmd = args[i + 1].clone();
                    i += 1;
                }
            }
            "--port" => {
                if i + 1 < args.len() {
                    if let Ok(port) = args[i + 1].parse() {
                        remote_port = port;
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
            arg => {
                if !arg.starts_with('-') && target_host.is_none() {
                    target_host = Some(arg.to_string());
                }
            }
        }
        i += 1;
    }

    if let Some(addr) = connect_addr {
        client::run_client(addr, predict)?;
    } else if let Some(target) = target_host {
        let (_tunnel, stream) = SshTunnel::spawn(&ssh_cmd, &target, ssh_port, &remote_server_cmd, remote_port)?;
        client::run_client_stream(stream, predict)?;
    } else {
        let default_addr: SocketAddr = "127.0.0.1:4000".parse().unwrap();
        client::run_client(default_addr, predict)?;
    }

    Ok(())
}

fn print_usage() {
    println!("mosh-tcp-client - Lightweight latency-tolerant terminal client\n");
    println!("Usage:");
    println!("  mosh-tcp-client [options] [user@]host");
    println!("  mosh-tcp-client --connect <ADDR:PORT> [options]\n");
    println!("Options:");
    println!("  -c, --connect <ADDR:PORT>  Connect directly to mosh-tcp-server without SSH");
    println!("  -s, --ssh <COMMAND>        SSH command to run (default: ssh)");
    println!("  -p, --ssh-port <PORT>      Remote SSH port (default: 22)");
    println!("      --server <COMMAND>     Remote mosh-tcp-server binary (default: mosh-tcp-server)");
    println!("      --port <PORT>          Remote server listening port (default: 4000)");
    println!("      --predict              Enable local predictive echo");
    println!("  -h, --help                 Display this help message");
}

