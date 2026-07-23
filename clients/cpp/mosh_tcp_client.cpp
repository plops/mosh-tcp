/* mosh_tcp_client.cpp - Standalone Modern C++20 mosh-tcp Client
 * Lightweight POSIX client for mosh-tcp protocol using C++20 RAII & std::span.
 */

#define _DEFAULT_SOURCE
#define _POSIX_C_SOURCE 200809L

#include <iostream>
#include <vector>
#include <string>
#include <string_view>
#include <span>
#include <variant>
#include <optional>
#include <memory>
#include <thread>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <cstdlib>
#include <cerrno>

#include <unistd.h>
#include <termios.h>
#include <signal.h>
#include <sys/socket.h>
#include <sys/ioctl.h>
#include <sys/poll.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <arpa/inet.h>

#include "../c/puff.h"

namespace mosh {

class SshTunnel {
private:
    pid_t launcher_pid_{-1};
    pid_t tunnel_pid_{-1};

public:
    SshTunnel() = default;
    SshTunnel(pid_t launcher_pid, pid_t tunnel_pid)
        : launcher_pid_(launcher_pid), tunnel_pid_(tunnel_pid) {}
    ~SshTunnel() {
        stop();
    }
    void stop() {
        if (tunnel_pid_ > 0) {
            kill(tunnel_pid_, SIGTERM);
            waitpid(tunnel_pid_, nullptr, 0);
            tunnel_pid_ = -1;
        }
        if (launcher_pid_ > 0) {
            kill(launcher_pid_, SIGTERM);
            waitpid(launcher_pid_, nullptr, 0);
            launcher_pid_ = -1;
        }
    }
    pid_t tunnel_pid() const { return tunnel_pid_; }
    pid_t launcher_pid() const { return launcher_pid_; }
    SshTunnel(const SshTunnel&) = delete;
    SshTunnel& operator=(const SshTunnel&) = delete;
    SshTunnel(SshTunnel&& other) noexcept
        : launcher_pid_(other.launcher_pid_), tunnel_pid_(other.tunnel_pid_) {
        other.launcher_pid_ = -1;
        other.tunnel_pid_ = -1;
    }
    SshTunnel& operator=(SshTunnel&& other) noexcept {
        if (this != &other) {
            stop();
            launcher_pid_ = other.launcher_pid_;
            tunnel_pid_ = other.tunnel_pid_;
            other.launcher_pid_ = -1;
            other.tunnel_pid_ = -1;
        }
        return *this;
    }
};

struct ClientInput {
    std::vector<uint8_t> data;
};

struct ClientResize {
    uint16_t rows;
    uint16_t cols;
};

struct Ping {
    uint64_t timestamp;
};

struct Pong {
    uint64_t timestamp;
};

struct ServerFrame {
    uint64_t seq;
    bool compressed;
    std::vector<uint8_t> data;
};

struct ClientHandshake {
    std::string session_key;
    uint16_t rows;
    uint16_t cols;
};

using Packet = std::variant<ClientInput, ClientResize, Ping, Pong, ServerFrame, ClientHandshake>;

class TerminalGuard {
private:
    struct termios orig_termios_{};
    bool active_{false};

public:
    TerminalGuard() {
        if (tcgetattr(STDIN_FILENO, &orig_termios_) == 0) {
            active_ = true;
            struct termios raw = orig_termios_;
            cfmakeraw(&raw);
            tcsetattr(STDIN_FILENO, TCSAFLUSH, &raw);
        }
    }

    ~TerminalGuard() {
        restore();
    }

    void restore() {
        if (active_) {
            tcsetattr(STDIN_FILENO, TCSANOW, &orig_termios_);
            active_ = false;
        }
    }

    TerminalGuard(const TerminalGuard&) = delete;
    TerminalGuard& operator=(const TerminalGuard&) = delete;
};

static std::atomic<bool> g_sigwinch_pending{false};
static std::atomic<bool> g_running{true};

static void handle_signal(int sig) {
    if (sig == SIGWINCH) {
        g_sigwinch_pending.store(true, std::memory_order_relaxed);
    } else if (sig == SIGINT || sig == SIGTERM) {
        g_running.store(false, std::memory_order_relaxed);
    }
}

static void setup_signals() {
    struct sigaction sa{};
    sa.sa_handler = handle_signal;
    sigemptyset(&sa.sa_mask);

    sigaction(SIGWINCH, &sa, nullptr);
    sigaction(SIGINT, &sa, nullptr);
    sigaction(SIGTERM, &sa, nullptr);
}

static int write_all(int fd, const void *buf, size_t count) {
    size_t written = 0;
    const char *ptr = static_cast<const char *>(buf);
    while (written < count) {
        ssize_t n = write(fd, ptr + written, count - written);
        if (n <= 0) {
            if (n < 0 && (errno == EINTR || errno == EAGAIN)) continue;
            return -1;
        }
        written += n;
    }
    return 0;
}

static std::vector<uint8_t> serialize_packet(const Packet& pkt) {
    std::vector<uint8_t> buf;
    std::visit([&buf](auto&& arg) {
        using T = std::decay_t<decltype(arg)>;
        if constexpr (std::is_same_v<T, ClientInput>) {
            buf.push_back(1);
            uint32_t len = static_cast<uint32_t>(arg.data.size());
            buf.push_back((len >> 24) & 0xFF);
            buf.push_back((len >> 16) & 0xFF);
            buf.push_back((len >> 8) & 0xFF);
            buf.push_back(len & 0xFF);
            buf.insert(buf.end(), arg.data.begin(), arg.data.end());
        } else if constexpr (std::is_same_v<T, ClientResize>) {
            buf.push_back(2);
            buf.push_back((arg.rows >> 8) & 0xFF);
            buf.push_back(arg.rows & 0xFF);
            buf.push_back((arg.cols >> 8) & 0xFF);
            buf.push_back(arg.cols & 0xFF);
        } else if constexpr (std::is_same_v<T, Ping>) {
            buf.push_back(3);
            for (int i = 7; i >= 0; --i) {
                buf.push_back((arg.timestamp >> (i * 8)) & 0xFF);
            }
        } else if constexpr (std::is_same_v<T, Pong>) {
            buf.push_back(4);
            for (int i = 7; i >= 0; --i) {
                buf.push_back((arg.timestamp >> (i * 8)) & 0xFF);
            }
        } else if constexpr (std::is_same_v<T, ServerFrame>) {
            buf.push_back(5);
            for (int i = 7; i >= 0; --i) {
                buf.push_back((arg.seq >> (i * 8)) & 0xFF);
            }
            buf.push_back(arg.compressed ? 1 : 0);
            uint32_t len = static_cast<uint32_t>(arg.data.size());
            buf.push_back((len >> 24) & 0xFF);
            buf.push_back((len >> 16) & 0xFF);
            buf.push_back((len >> 8) & 0xFF);
            buf.push_back(len & 0xFF);
            buf.insert(buf.end(), arg.data.begin(), arg.data.end());
        } else if constexpr (std::is_same_v<T, ClientHandshake>) {
            buf.push_back(6);
            uint16_t klen = static_cast<uint16_t>(arg.session_key.size());
            buf.push_back((klen >> 8) & 0xFF);
            buf.push_back(klen & 0xFF);
            buf.insert(buf.end(), arg.session_key.begin(), arg.session_key.end());
            buf.push_back((arg.rows >> 8) & 0xFF);
            buf.push_back(arg.rows & 0xFF);
            buf.push_back((arg.cols >> 8) & 0xFF);
            buf.push_back(arg.cols & 0xFF);
        }
    }, pkt);
    return buf;
}

static bool send_framed_packet(int sock, const Packet& pkt) {
    auto payload = serialize_packet(pkt);
    uint32_t len = static_cast<uint32_t>(payload.size());
    uint32_t be_len = htonl(len);

    if (write_all(sock, &be_len, 4) < 0) return false;
    if (write_all(sock, payload.data(), payload.size()) < 0) return false;
    return true;
}

static std::optional<Packet> deserialize_packet(std::span<const uint8_t> payload) {
    if (payload.empty()) return std::nullopt;

    uint8_t tag = payload[0];
    auto body = payload.subspan(1);

    switch (tag) {
        case 1: {
            if (body.size() < 4) return std::nullopt;
            uint32_t len = (body[0] << 24) | (body[1] << 16) | (body[2] << 8) | body[3];
            if (body.size() < 4 + len) return std::nullopt;
            auto data_span = body.subspan(4, len);
            return ClientInput{ .data = std::vector<uint8_t>(data_span.begin(), data_span.end()) };
        }
        case 2: {
            if (body.size() < 4) return std::nullopt;
            uint16_t rows = (body[0] << 8) | body[1];
            uint16_t cols = (body[2] << 8) | body[3];
            return ClientResize{ .rows = rows, .cols = cols };
        }
        case 3: {
            if (body.size() < 8) return std::nullopt;
            uint64_t ts = 0;
            for (size_t i = 0; i < 8; ++i) ts = (ts << 8) | body[i];
            return Ping{ .timestamp = ts };
        }
        case 4: {
            if (body.size() < 8) return std::nullopt;
            uint64_t ts = 0;
            for (size_t i = 0; i < 8; ++i) ts = (ts << 8) | body[i];
            return Pong{ .timestamp = ts };
        }
        case 5: {
            if (body.size() < 13) return std::nullopt;
            uint64_t seq = 0;
            for (size_t i = 0; i < 8; ++i) seq = (seq << 8) | body[i];
            bool compressed = (body[8] != 0);
            uint32_t len = (body[9] << 24) | (body[10] << 16) | (body[11] << 8) | body[12];
            if (body.size() < 13 + len) return std::nullopt;
            auto data_span = body.subspan(13, len);
            return ServerFrame{
                .seq = seq,
                .compressed = compressed,
                .data = std::vector<uint8_t>(data_span.begin(), data_span.end())
            };
        }
        default:
            return std::nullopt;
    }
}

static bool decompress_frame(std::span<const uint8_t> data, std::vector<uint8_t>& out) {
    if (data.size() < 10) return false;
    size_t offset = 0;
    if (data[0] == 0x1f && data[1] == 0x8b) {
        if (data[2] != 8) return false;
        uint8_t flg = data[3];
        offset = 10;
        if (flg & 4) {
            if (offset + 2 > data.size()) return false;
            uint16_t extra_len = data[offset] | (data[offset + 1] << 8);
            offset += 2 + extra_len;
        }
        if (flg & 8) {
            while (offset < data.size() && data[offset] != 0) offset++;
            offset++;
        }
        if (flg & 16) {
            while (offset < data.size() && data[offset] != 0) offset++;
            offset++;
        }
        if (flg & 2) offset += 2;
        if (offset + 8 > data.size()) return false;

        unsigned long sourcelen = data.size() - offset - 8;
        out.resize(256 * 1024);
        unsigned long destlen = out.size();
        int err = puff(out.data(), &destlen, data.data() + offset, &sourcelen);
        if (err == 0) {
            out.resize(destlen);
            return true;
        }
        return false;
    } else {
        unsigned long sourcelen = data.size();
        out.resize(256 * 1024);
        unsigned long destlen = out.size();
        int err = puff(out.data(), &destlen, data.data(), &sourcelen);
        if (err == 0) {
            out.resize(destlen);
            return true;
        }
        return false;
    }
}

static void handle_server_packet(const Packet& pkt) {
    if (auto sf = std::get_if<ServerFrame>(&pkt)) {
        if (sf->compressed) {
            std::vector<uint8_t> decomp;
            if (decompress_frame(sf->data, decomp)) {
                write_all(STDOUT_FILENO, decomp.data(), decomp.size());
            }
        } else {
            write_all(STDOUT_FILENO, sf->data.data(), sf->data.size());
        }
    }
}

} // namespace mosh

static int find_free_port() {
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0) return 4000;
    struct sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    addr.sin_port = 0;
    if (bind(s, reinterpret_cast<struct sockaddr*>(&addr), sizeof(addr)) < 0) {
        close(s);
        return 4000;
    }
    socklen_t len = sizeof(addr);
    if (getsockname(s, reinterpret_cast<struct sockaddr*>(&addr), &len) < 0) {
        close(s);
        return 4000;
    }
    int p = ntohs(addr.sin_port);
    close(s);
    return p;
}

static void print_usage(const char *prog) {
    std::cerr << "mosh-tcp-client-cpp - Standalone Modern C++20 mosh-tcp client\n\n"
              << "Usage:\n"
              << "  " << prog << " [options] [user@]host\n"
              << "  " << prog << " --connect <ADDR:PORT> [options]\n\n"
              << "Options:\n"
              << "  -c, --connect <ADDR:PORT>  Connect directly to mosh-tcp-server\n"
              << "  -s, --ssh <COMMAND>        SSH command to run (default: ssh)\n"
              << "  -p, --ssh-port <PORT>      Remote SSH port (default: 22)\n"
              << "      --server <COMMAND>     Remote mosh-tcp-server binary (default: mosh-tcp-server)\n"
              << "      --port <PORT>          Remote server listening port (default: 4000)\n"
              << "  -h, --help                 Display this help message\n";
}

int main(int argc, char **argv) {
    std::optional<std::string> connect_arg;
    std::optional<std::string> target_host;
    std::string ssh_cmd = "ssh";
    std::optional<std::string> ssh_port_str;
    std::string server_cmd = "mosh-tcp-server";
    int remote_port = 0;
    std::string session_key = "";

    for (int i = 1; i < argc; i++) {
        std::string_view arg = argv[i];
        if (arg == "--connect" || arg == "-c") {
            if (i + 1 < argc) connect_arg = argv[++i];
        } else if (arg == "--ssh" || arg == "-s") {
            if (i + 1 < argc) ssh_cmd = argv[++i];
        } else if (arg == "--ssh-port" || arg == "-p") {
            if (i + 1 < argc) ssh_port_str = argv[++i];
        } else if (arg == "--server") {
            if (i + 1 < argc) server_cmd = argv[++i];
        } else if (arg == "--port") {
            if (i + 1 < argc) remote_port = std::stoi(argv[++i]);
        } else if (arg == "--help" || arg == "-h") {
            print_usage(argv[0]);
            return 0;
        } else if (!arg.starts_with('-') && !target_host) {
            target_host = std::string(arg);
        }
    }

    std::string connect_ip = "127.0.0.1";
    int connect_port = 4000;
    mosh::SshTunnel ssh_tunnel;

    if (connect_arg) {
        std::string target = *connect_arg;
        auto colon = target.rfind(':');
        if (colon != std::string::npos) {
            connect_ip = target.substr(0, colon);
            connect_port = std::stoi(target.substr(colon + 1));
        } else {
            connect_ip = target;
        }
    } else if (target_host) {
        /* Step 1: Launch mosh-tcp-server on remote host and capture stdout */
        int pipefds[2];
        if (pipe(pipefds) < 0) {
            perror("pipe failed");
            return 1;
        }

        char remote_cmd_buf[256];
        if (remote_port == 0) {
            snprintf(remote_cmd_buf, sizeof(remote_cmd_buf), "%s --bind 127.0.0.1:0", server_cmd.c_str());
        } else {
            snprintf(remote_cmd_buf, sizeof(remote_cmd_buf), "%s --bind 127.0.0.1:%d", server_cmd.c_str(), remote_port);
        }

        std::vector<char*> launch_argv;
        launch_argv.push_back(const_cast<char*>(ssh_cmd.c_str()));
        if (ssh_port_str) {
            launch_argv.push_back(const_cast<char*>("-p"));
            launch_argv.push_back(const_cast<char*>(ssh_port_str->c_str()));
        }
        launch_argv.push_back(const_cast<char*>(target_host->c_str()));
        launch_argv.push_back(remote_cmd_buf);
        launch_argv.push_back(nullptr);

        std::cerr << "[mosh-tcp client-cpp] Launching server on " << *target_host << " via SSH...\n";
        pid_t launch_pid = fork();
        if (launch_pid == 0) {
            int devnull = open("/dev/null", O_RDWR);
            if (devnull >= 0) {
                dup2(devnull, STDIN_FILENO);
                close(devnull);
            }
            close(pipefds[0]);
            dup2(pipefds[1], STDOUT_FILENO);
            close(pipefds[1]);
            execvp(ssh_cmd.c_str(), launch_argv.data());
            perror("execvp ssh launcher failed");
            _exit(1);
        }
        close(pipefds[1]);

        int bound_port = remote_port;
        FILE *fp = fdopen(pipefds[0], "r");
        if (fp) {
            char line[256];
            while (fgets(line, sizeof(line), fp)) {
                std::string_view l(line);
                if (l.starts_with("MOSH-TCP CONNECT ")) {
                    int p = 0;
                    char key_buf[128] = "";
                    if (sscanf(line, "MOSH-TCP CONNECT %d %127s", &p, key_buf) >= 2) {
                        bound_port = p;
                        session_key = key_buf;
                        break;
                    }
                }
            }
            fclose(fp);
        }
        if (bound_port == 0) bound_port = 4000;

        /* Step 2: Establish local SSH tunnel */
        int local_port = find_free_port();
        connect_ip = "127.0.0.1";
        connect_port = local_port;

        char fwd_buf[128];
        snprintf(fwd_buf, sizeof(fwd_buf), "%d:127.0.0.1:%d", local_port, bound_port);

        std::vector<char*> ssh_argv;
        ssh_argv.push_back(const_cast<char*>(ssh_cmd.c_str()));
        ssh_argv.push_back(const_cast<char*>("-o"));
        ssh_argv.push_back(const_cast<char*>("ExitOnForwardFailure=yes"));
        ssh_argv.push_back(const_cast<char*>("-N"));
        ssh_argv.push_back(const_cast<char*>("-L"));
        ssh_argv.push_back(fwd_buf);
        if (ssh_port_str) {
            ssh_argv.push_back(const_cast<char*>("-p"));
            ssh_argv.push_back(const_cast<char*>(ssh_port_str->c_str()));
        }
        ssh_argv.push_back(const_cast<char*>(target_host->c_str()));
        ssh_argv.push_back(nullptr);

        std::cerr << "[mosh-tcp client-cpp] Established tunnel (local port " << local_port << " -> remote port " << bound_port << ")...\n";
        pid_t pid = fork();
        if (pid == 0) {
            int devnull = open("/dev/null", O_RDWR);
            if (devnull >= 0) {
                dup2(devnull, STDIN_FILENO);
                close(devnull);
            }
            execvp(ssh_cmd.c_str(), ssh_argv.data());
            perror("execvp ssh tunnel failed");
            _exit(1);
        } else if (pid > 0) {
            ssh_tunnel = mosh::SshTunnel(launch_pid, pid);
        } else {
            perror("fork failed");
            return 1;
        }
    }

    int sock = -1;
    int attempts = 0;
    while (attempts < 150) {
        if (ssh_tunnel.tunnel_pid() > 0) {
            int status;
            if (waitpid(ssh_tunnel.tunnel_pid(), &status, WNOHANG) > 0) {
                std::cerr << "SSH subprocess exited unexpectedly\n";
                return 1;
            }
        }

        sock = socket(AF_INET, SOCK_STREAM, 0);
        if (sock >= 0) {
            struct sockaddr_in serv_addr{};
            serv_addr.sin_family = AF_INET;
            serv_addr.sin_port = htons(connect_port);
            if (inet_pton(AF_INET, connect_ip.c_str(), &serv_addr.sin_addr) > 0) {
                if (connect(sock, reinterpret_cast<struct sockaddr*>(&serv_addr), sizeof(serv_addr)) == 0) {
                    break;
                }
            }
            close(sock);
            sock = -1;
        }
        usleep(100000);
        attempts++;
    }

    if (sock < 0) {
        std::cerr << "Failed to connect to mosh-tcp server at " << connect_ip << ":" << connect_port << "\n";
        return 1;
    }

    mosh::TerminalGuard term_guard;
    mosh::setup_signals();

    /* Send initial handshake with session key and window size */
    struct winsize ws{};
    uint16_t rows = 24, cols = 80;
    if (ioctl(STDOUT_FILENO, TIOCGWINSZ, &ws) == 0) {
        rows = ws.ws_row;
        cols = ws.ws_col;
    }
    mosh::send_framed_packet(sock, mosh::ClientHandshake{ .session_key = session_key, .rows = rows, .cols = cols });

    std::vector<uint8_t> net_buf(65536);
    std::vector<uint8_t> in_buf(4096);
    size_t net_buf_len = 0;

    struct pollfd fds[2]{};
    fds[0].fd = STDIN_FILENO;
    fds[0].events = POLLIN;
    fds[1].fd = sock;
    fds[1].events = POLLIN;

    while (mosh::g_running.load(std::memory_order_relaxed)) {
        if (mosh::g_sigwinch_pending.exchange(false, std::memory_order_relaxed)) {
            if (ioctl(STDOUT_FILENO, TIOCGWINSZ, &ws) == 0) {
                mosh::send_framed_packet(sock, mosh::ClientResize{ .rows = ws.ws_row, .cols = ws.ws_col });
            }
        }

        int ret = poll(fds, 2, 50);
        if (ret < 0) {
            if (errno == EINTR) continue;
            break;
        }

        if ((fds[0].fd >= 0) && (fds[0].revents & POLLIN)) {
            ssize_t n = read(STDIN_FILENO, in_buf.data(), in_buf.size());
            if (n > 0) {
                std::vector<uint8_t> input_data(in_buf.begin(), in_buf.begin() + n);
                if (!mosh::send_framed_packet(sock, mosh::ClientInput{ .data = input_data })) break;
            } else if (n == 0) {
                mosh::send_framed_packet(sock, mosh::ClientInput{ .data = { 0x04 } });
                fds[0].fd = -1;
            }
        }

        if (fds[1].revents & POLLIN) {
            ssize_t n = read(sock, net_buf.data() + net_buf_len, net_buf.size() - net_buf_len);
            if (n > 0) {
                net_buf_len += n;
                size_t offset = 0;
                while (net_buf_len - offset >= 4) {
                    uint32_t pkt_len = (net_buf[offset] << 24) |
                                       (net_buf[offset + 1] << 16) |
                                       (net_buf[offset + 2] << 8) |
                                       (net_buf[offset + 3]);
                    if (net_buf_len - offset < 4 + pkt_len) break;

                    std::span<const uint8_t> payload(&net_buf[offset + 4], pkt_len);
                    auto opt_pkt = mosh::deserialize_packet(payload);
                    if (opt_pkt) {
                        mosh::handle_server_packet(*opt_pkt);
                    }
                    offset += 4 + pkt_len;
                }
                if (offset > 0) {
                    std::memmove(net_buf.data(), net_buf.data() + offset, net_buf_len - offset);
                    net_buf_len -= offset;
                }
            } else if (n == 0) {
                break;
            }
        }

        if (fds[1].revents & (POLLERR | POLLHUP | POLLNVAL)) break;
    }

    close(sock);
    term_guard.restore();
    return 0;
}
