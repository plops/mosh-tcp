#!/bin/bash
# mock_ssh.sh - Helper script for mosh-tcp SSH login integration tests

IS_TUNNEL=0
LOCAL_PORT=""
REMOTE_PORT=""
REMOTE_CMD=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        -L)
            IS_TUNNEL=1
            FWD="$2"
            LOCAL_PORT="${FWD%%:*}"
            REMOTE_PORT="${FWD##*:}"
            shift 2
            ;;
        -N|-o|-p)
            shift
            ;;
        *)
            if [[ "$1" == *"mosh-tcp-server"* ]]; then
                REMOTE_CMD="$1"
            fi
            shift
            ;;
    esac
done

if [ "$IS_TUNNEL" -eq 1 ]; then
    # Relay TCP connections from LOCAL_PORT to REMOTE_PORT
    exec python3 -c "
import socket, threading, sys

l_port = int(sys.argv[1])
r_port = int(sys.argv[2])

server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(('127.0.0.1', l_port))
server.settimeout(0.5)
server.listen(5)

def handle(client_sock):
    remote_sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        remote_sock.connect(('127.0.0.1', r_port))
    except Exception:
        client_sock.close()
        return

    def forward(src, dst):
        try:
            while True:
                data = src.recv(4096)
                if not data: break
                dst.sendall(data)
        except Exception:
            pass
        finally:
            try: src.close()
            except Exception: pass
            try: dst.close()
            except Exception: pass

    t1 = threading.Thread(target=forward, args=(client_sock, remote_sock), daemon=True)
    t2 = threading.Thread(target=forward, args=(remote_sock, client_sock), daemon=True)
    t1.start()
    t2.start()

while True:
    try:
        cs, _ = server.accept()
        threading.Thread(target=handle, args=(cs,), daemon=True).start()
    except socket.timeout:
        continue
    except Exception:
        break
" "$LOCAL_PORT" "$REMOTE_PORT"
else
    # Launch server command directly and pass stdout through
    exec $REMOTE_CMD
fi
