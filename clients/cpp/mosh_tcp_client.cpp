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
#include <netinet/in.h>
#include <arpa/inet.h>

#include "../c/puff.h"

namespace mosh {

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

using Packet = std::variant<ClientInput, ClientResize, Ping, Pong, ServerFrame>;

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

static void print_usage(const char *prog) {
    std::cerr << "mosh-tcp-client-cpp - Standalone Modern C++20 mosh-tcp client\n\n"
              << "Usage:\n"
              << "  " << prog << " [--connect <ADDR:PORT>]\n";
}

int main(int argc, char **argv) {
    std::string connect_addr = "127.0.0.1";
    int port = 4000;

    for (int i = 1; i < argc; i++) {
        std::string_view arg = argv[i];
        if (arg == "--connect" || arg == "-c") {
            if (i + 1 < argc) {
                std::string target = argv[++i];
                auto colon = target.rfind(':');
                if (colon != std::string::npos) {
                    connect_addr = target.substr(0, colon);
                    port = std::stoi(target.substr(colon + 1));
                } else {
                    connect_addr = target;
                }
            }
        } else if (arg == "--help" || arg == "-h") {
            print_usage(argv[0]);
            return 0;
        }
    }

    int sock = socket(AF_INET, SOCK_STREAM, 0);
    if (sock < 0) {
        perror("socket creation failed");
        return 1;
    }

    struct sockaddr_in serv_addr{};
    serv_addr.sin_family = AF_INET;
    serv_addr.sin_port = htons(port);
    if (inet_pton(AF_INET, connect_addr.c_str(), &serv_addr.sin_addr) <= 0) {
        std::cerr << "Invalid IP address: " << connect_addr << "\n";
        close(sock);
        return 1;
    }

    if (connect(sock, reinterpret_cast<struct sockaddr*>(&serv_addr), sizeof(serv_addr)) < 0) {
        perror("Connection to server failed");
        close(sock);
        return 1;
    }

    mosh::TerminalGuard term_guard;
    mosh::setup_signals();

    /* Initial resize */
    struct winsize ws{};
    if (ioctl(STDOUT_FILENO, TIOCGWINSZ, &ws) == 0) {
        mosh::send_framed_packet(sock, mosh::ClientResize{ .rows = ws.ws_row, .cols = ws.ws_col });
    } else {
        mosh::send_framed_packet(sock, mosh::ClientResize{ .rows = 24, .cols = 80 });
    }

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

        if (fds[0].revents & POLLIN) {
            ssize_t n = read(STDIN_FILENO, in_buf.data(), in_buf.size());
            if (n > 0) {
                std::vector<uint8_t> input_data(in_buf.begin(), in_buf.begin() + n);
                if (!mosh::send_framed_packet(sock, mosh::ClientInput{ .data = input_data })) break;
            } else if (n == 0) {
                break;
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
