/* mosh_tcp_client.c - Standalone C99/C11 mosh-tcp Client
 * Lightweight POSIX client for mosh-tcp protocol.
 */

#define _DEFAULT_SOURCE
#define _POSIX_C_SOURCE 200809L

#include <stdio.h>
#include <stdlib.h>

#include <string.h>
#include <unistd.h>
#include <signal.h>
#include <termios.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/socket.h>
#include <sys/ioctl.h>
#include <sys/poll.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include "puff.h"

#define BUFFER_SIZE 65536
#define DECOMPRESS_BUFFER_SIZE (256 * 1024)

#include <sys/wait.h>

static struct termios orig_termios;
static int termios_saved = 0;
static volatile sig_atomic_t g_sigwinch_pending = 0;
static volatile sig_atomic_t g_running = 1;
static volatile pid_t g_ssh_pid = -1;

static void cleanup_ssh(void) {
    if (g_ssh_pid > 0) {
        kill(g_ssh_pid, SIGTERM);
        waitpid(g_ssh_pid, NULL, 0);
        g_ssh_pid = -1;
    }
}

static void restore_terminal(void) {
    cleanup_ssh();
    if (termios_saved) {
        tcsetattr(STDIN_FILENO, TCSANOW, &orig_termios);
        termios_saved = 0;
    }
}

static void handle_signal(int sig) {
    if (sig == SIGWINCH) {
        g_sigwinch_pending = 1;
    } else if (sig == SIGINT || sig == SIGTERM) {
        g_running = 0;
    }
}

static void setup_signals_and_terminal(void) {
    if (tcgetattr(STDIN_FILENO, &orig_termios) == 0) {
        termios_saved = 1;
        atexit(restore_terminal);

        struct termios raw = orig_termios;
        cfmakeraw(&raw);
        tcsetattr(STDIN_FILENO, TCSAFLUSH, &raw);
    }

    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = handle_signal;
    sigemptyset(&sa.sa_mask);

    sigaction(SIGWINCH, &sa, NULL);
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
}

/* Helper to write exact n bytes to socket */
static int write_all(int fd, const void *buf, size_t count) {
    size_t written = 0;
    const char *ptr = (const char *)buf;
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

/* Send a length-prefixed packet over socket */
static int send_packet(int sock, const unsigned char *payload, uint32_t len) {
    uint32_t be_len = htonl(len);
    if (write_all(sock, &be_len, 4) < 0) return -1;
    if (write_all(sock, payload, len) < 0) return -1;
    return 0;
}

static int send_resize(int sock, uint16_t rows, uint16_t cols) {
    unsigned char payload[5];
    payload[0] = 2; /* Tag 2: ClientResize */
    uint16_t be_rows = htons(rows);
    uint16_t be_cols = htons(cols);
    memcpy(&payload[1], &be_rows, 2);
    memcpy(&payload[3], &be_cols, 2);
    return send_packet(sock, payload, 5);
}

static int send_input(int sock, const unsigned char *data, uint32_t len) {
    unsigned char *payload = malloc(1 + 4 + len);
    if (!payload) return -1;
    payload[0] = 1; /* Tag 1: ClientInput */
    uint32_t be_len = htonl(len);
    memcpy(&payload[1], &be_len, 4);
    memcpy(&payload[5], data, len);
    int res = send_packet(sock, payload, 5 + len);
    free(payload);
    return res;
}

/* Decompress gzip / raw deflate buffer using puff */
static int decompress_frame(const unsigned char *data, size_t len, unsigned char *out, unsigned long *outlen) {
    if (len < 10) return -1;

    size_t offset = 0;
    /* Check for gzip magic header 0x1f 0x8b */
    if (data[0] == 0x1f && data[1] == 0x8b) {
        uint8_t cm = data[2];
        uint8_t flg = data[3];
        if (cm != 8) return -1; /* Only Deflate is supported */

        offset = 10;
        if (flg & 4) { /* FEXTRA */
            if (offset + 2 > len) return -1;
            uint16_t extra_len = data[offset] | (data[offset + 1] << 8);
            offset += 2 + extra_len;
        }
        if (flg & 8) { /* FNAME */
            while (offset < len && data[offset] != 0) offset++;
            offset++;
        }
        if (flg & 16) { /* FCOMMENT */
            while (offset < len && data[offset] != 0) offset++;
            offset++;
        }
        if (flg & 2) { /* FHCRC */
            offset += 2;
        }
        if (offset + 8 > len) return -1;

        unsigned long sourcelen = len - offset - 8;
        int err = puff(out, outlen, data + offset, &sourcelen);
        return err == 0 ? 0 : -1;
    } else {
        /* Raw deflate fallback */
        unsigned long sourcelen = len;
        int err = puff(out, outlen, data, &sourcelen);
        return err == 0 ? 0 : -1;
    }
}

static void process_packet(const unsigned char *payload, uint32_t len) {
    if (len == 0) return;
    uint8_t tag = payload[0];

    if (tag == 5) { /* ServerFrame */
        if (len < 1 + 8 + 1 + 4) return;
        /* uint64_t seq; */
        uint8_t compressed = payload[9];
        uint32_t frame_len;
        memcpy(&frame_len, &payload[10], 4);
        frame_len = ntohl(frame_len);

        if (1 + 8 + 1 + 4 + frame_len > len) return;

        const unsigned char *frame_data = &payload[14];

        if (compressed) {
            unsigned char *decomp_buf = malloc(DECOMPRESS_BUFFER_SIZE);
            if (decomp_buf) {
                unsigned long decomp_len = DECOMPRESS_BUFFER_SIZE;
                if (decompress_frame(frame_data, frame_len, decomp_buf, &decomp_len) == 0) {
                    write_all(STDOUT_FILENO, decomp_buf, decomp_len);
                }
                free(decomp_buf);
            }
        } else {
            write_all(STDOUT_FILENO, frame_data, frame_len);
        }
    }
}

static int find_free_port(void) {
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0) return 4000;
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    addr.sin_port = 0;
    if (bind(s, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(s);
        return 4000;
    }
    socklen_t len = sizeof(addr);
    if (getsockname(s, (struct sockaddr *)&addr, &len) < 0) {
        close(s);
        return 4000;
    }
    int p = ntohs(addr.sin_port);
    close(s);
    return p;
}

static void print_usage(const char *prog) {
    fprintf(stderr, "mosh-tcp-client-c - Lightweight C99/C11 mosh-tcp client\n\n");
    fprintf(stderr, "Usage:\n");
    fprintf(stderr, "  %s [options] [user@]host\n", prog);
    fprintf(stderr, "  %s --connect <ADDR:PORT> [options]\n\n", prog);
    fprintf(stderr, "Options:\n");
    fprintf(stderr, "  -c, --connect <ADDR:PORT>  Connect directly to mosh-tcp-server\n");
    fprintf(stderr, "  -s, --ssh <COMMAND>        SSH command to run (default: ssh)\n");
    fprintf(stderr, "  -p, --ssh-port <PORT>      Remote SSH port (default: 22)\n");
    fprintf(stderr, "      --server <COMMAND>     Remote mosh-tcp-server binary (default: mosh-tcp-server)\n");
    fprintf(stderr, "      --port <PORT>          Remote server listening port (default: 4000)\n");
    fprintf(stderr, "  -h, --help                 Display this help message\n");
}

int main(int argc, char **argv) {
    const char *connect_arg = NULL;
    const char *target_host = NULL;
    const char *ssh_cmd = "ssh";
    const char *ssh_port_str = NULL;
    const char *server_cmd = "mosh-tcp-server";
    int remote_port = 4000;

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--connect") == 0 || strcmp(argv[i], "-c") == 0) {
            if (i + 1 < argc) connect_arg = argv[++i];
        } else if (strcmp(argv[i], "--ssh") == 0 || strcmp(argv[i], "-s") == 0) {
            if (i + 1 < argc) ssh_cmd = argv[++i];
        } else if (strcmp(argv[i], "--ssh-port") == 0 || strcmp(argv[i], "-p") == 0) {
            if (i + 1 < argc) ssh_port_str = argv[++i];
        } else if (strcmp(argv[i], "--server") == 0) {
            if (i + 1 < argc) server_cmd = argv[++i];
        } else if (strcmp(argv[i], "--port") == 0) {
            if (i + 1 < argc) remote_port = atoi(argv[++i]);
        } else if (strcmp(argv[i], "--help") == 0 || strcmp(argv[i], "-h") == 0) {
            print_usage(argv[0]);
            return 0;
        } else if (argv[i][0] != '-' && !target_host) {
            target_host = argv[i];
        }
    }

    const char *connect_ip = "127.0.0.1";
    int connect_port = 4000;

    if (connect_arg) {
        char *target = strdup(connect_arg);
        char *colon = strrchr(target, ':');
        if (colon) {
            *colon = '\0';
            connect_ip = target;
            connect_port = atoi(colon + 1);
        } else {
            connect_ip = target;
        }
    } else if (target_host) {
        int local_port = find_free_port();
        connect_ip = "127.0.0.1";
        connect_port = local_port;

        char fwd_buf[128];
        snprintf(fwd_buf, sizeof(fwd_buf), "%d:127.0.0.1:%d", local_port, remote_port);

        char remote_cmd_buf[256];
        snprintf(remote_cmd_buf, sizeof(remote_cmd_buf), "%s --bind 127.0.0.1:%d", server_cmd, remote_port);

        char *ssh_argv[32];
        int idx = 0;
        ssh_argv[idx++] = (char *)ssh_cmd;
        ssh_argv[idx++] = "-o";
        ssh_argv[idx++] = "ExitOnForwardFailure=yes";
        ssh_argv[idx++] = "-L";
        ssh_argv[idx++] = fwd_buf;
        if (ssh_port_str) {
            ssh_argv[idx++] = "-p";
            ssh_argv[idx++] = (char *)ssh_port_str;
        }
        ssh_argv[idx++] = (char *)target_host;
        ssh_argv[idx++] = remote_cmd_buf;
        ssh_argv[idx] = NULL;

        fprintf(stderr, "[mosh-tcp client-c] Connecting to %s via SSH tunnel (local port %d)...\n", target_host, local_port);
        pid_t pid = fork();
        if (pid == 0) {
            execvp(ssh_cmd, ssh_argv);
            perror("execvp ssh failed");
            _exit(1);
        } else if (pid > 0) {
            g_ssh_pid = pid;
            atexit(cleanup_ssh);
        } else {
            perror("fork failed");
            return 1;
        }
    }

    int sock = -1;
    int attempts = 0;
    while (attempts < 150) {
        if (g_ssh_pid > 0) {
            int status;
            if (waitpid(g_ssh_pid, &status, WNOHANG) > 0) {
                fprintf(stderr, "SSH subprocess exited unexpectedly\n");
                return 1;
            }
        }

        sock = socket(AF_INET, SOCK_STREAM, 0);
        if (sock >= 0) {
            struct sockaddr_in serv_addr;
            memset(&serv_addr, 0, sizeof(serv_addr));
            serv_addr.sin_family = AF_INET;
            serv_addr.sin_port = htons(connect_port);
            if (inet_pton(AF_INET, connect_ip, &serv_addr.sin_addr) > 0) {
                if (connect(sock, (struct sockaddr *)&serv_addr, sizeof(serv_addr)) == 0) {
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
        fprintf(stderr, "Failed to connect to mosh-tcp server at %s:%d\n", connect_ip, connect_port);
        cleanup_ssh();
        return 1;
    }

    setup_signals_and_terminal();

    /* Send initial terminal window size */
    struct winsize ws;
    if (ioctl(STDOUT_FILENO, TIOCGWINSZ, &ws) == 0) {
        send_resize(sock, ws.ws_row, ws.ws_col);
    } else {
        send_resize(sock, 24, 80);
    }

    unsigned char *net_buf = malloc(BUFFER_SIZE);
    unsigned char *in_buf = malloc(4096);
    size_t net_buf_len = 0;

    struct pollfd fds[2];
    fds[0].fd = STDIN_FILENO;
    fds[0].events = POLLIN;
    fds[1].fd = sock;
    fds[1].events = POLLIN;

    while (g_running) {
        if (g_sigwinch_pending) {
            g_sigwinch_pending = 0;
            if (ioctl(STDOUT_FILENO, TIOCGWINSZ, &ws) == 0) {
                send_resize(sock, ws.ws_row, ws.ws_col);
            }
        }

        int ret = poll(fds, 2, 50);
        if (ret < 0) {
            if (errno == EINTR) continue;
            break;
        }

        /* Handle STDIN input */
        if (fds[0].revents & POLLIN) {
            ssize_t n = read(STDIN_FILENO, in_buf, 4096);
            if (n > 0) {
                if (send_input(sock, in_buf, (uint32_t)n) < 0) break;
            } else if (n == 0) {
                break; /* EOF on stdin */
            }
        }

        /* Handle TCP Socket input */
        if (fds[1].revents & POLLIN) {
            ssize_t n = read(sock, net_buf + net_buf_len, BUFFER_SIZE - net_buf_len);
            if (n > 0) {
                net_buf_len += n;
                size_t offset = 0;
                while (net_buf_len - offset >= 4) {
                    uint32_t pkt_len = (net_buf[offset] << 24) |
                                       (net_buf[offset + 1] << 16) |
                                       (net_buf[offset + 2] << 8) |
                                       (net_buf[offset + 3]);
                    if (net_buf_len - offset < 4 + pkt_len) break;

                    process_packet(net_buf + offset + 4, pkt_len);
                    offset += 4 + pkt_len;
                }
                if (offset > 0) {
                    memmove(net_buf, net_buf + offset, net_buf_len - offset);
                    net_buf_len -= offset;
                }
            } else if (n == 0) {
                break; /* Server closed connection */
            }
        }

        if (fds[1].revents & (POLLERR | POLLHUP | POLLNVAL)) break;
    }

    free(net_buf);
    free(in_buf);
    close(sock);
    restore_terminal();
    return 0;
}
