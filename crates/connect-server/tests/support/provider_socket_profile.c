/* Test-process-only TCP options, before the designated loopback SYN. */
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/tcp.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <unistd.h>

static unsigned fixture_port(const char *path) {
    char value[16] = {0};
    int file = open(path, O_RDONLY | O_NOFOLLOW);
    if (file < 0) return 0;
    ssize_t count = read(file, value, sizeof(value) - 1);
    close(file);
    if (count <= 0) return 0;
    char *end = NULL;
    long port = strtol(value, &end, 10);
    if ((*end != '\0' && *end != '\n') || port <= 0 || port > 65535) return 0;
    return (unsigned)port;
}

static int fixture_connect(int fd, const struct sockaddr *address, socklen_t length) {
    const char *path = getenv("CONNECT_PROVIDER_SOCKET_PROFILE");
    if (path != NULL && address != NULL && length >= sizeof(struct sockaddr_in)
        && address->sa_family == AF_INET) {
        const struct sockaddr_in *peer = (const struct sockaddr_in *)address;
        unsigned port = fixture_port(path);
        int type = 0;
        socklen_t size = sizeof(type);
        if (port != 0 && ntohs(peer->sin_port) == port
            && peer->sin_addr.s_addr == htonl(INADDR_LOOPBACK)
            && getsockopt(fd, SOL_SOCKET, SO_TYPE, &type, &size) == 0
            && type == SOCK_STREAM) {
            int enabled = 1;
            if (setsockopt(fd, IPPROTO_TCP, TCP_NOOPT, &enabled, sizeof(enabled)) != 0)
                return -1;
            int actual = 0;
            size = sizeof(actual);
            if (getsockopt(fd, IPPROTO_TCP, TCP_NOOPT, &actual, &size) != 0 || actual == 0) {
                errno = EINVAL;
                return -1;
            }
            char marker[4096];
            int count = snprintf(marker, sizeof(marker), "%s.applied", path);
            if (count <= 0 || (size_t)count >= sizeof(marker)) { errno = EINVAL; return -1; }
            FILE *file = fopen(marker, "w");
            if (file == NULL) return -1;
            fprintf(file, "%u %d\n", port, actual);
            fclose(file);
        }
    }
    /* dyld preserves references from this image to the original function. */
    return connect(fd, address, length);
}

__attribute__((used)) static const struct {
    const void *replacement;
    const void *original;
} fixture_interpose __attribute__((section("__DATA,__interpose"))) = {
    (const void *)(uintptr_t)&fixture_connect,
    (const void *)(uintptr_t)&connect
};
