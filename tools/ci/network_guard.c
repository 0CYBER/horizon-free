#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <errno.h>
#include <netdb.h>
#include <stdlib.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>
#include <string.h>

/* Per-invocation append-only channel, inherited by child processes. A refusal is
 * recorded even if the test handles the error. Never permit a lost diagnostic. */
static void record(const char *operation) {
    const char *path = getenv("HORIZON_NETWORK_GUARD_LOG");
    if (!path) _exit(125);
    int fd = open(path, O_WRONLY | O_APPEND | O_CLOEXEC);
    if (fd < 0) _exit(125);
    size_t n = strlen(operation);
    if (write(fd, operation, n) != (ssize_t)n || close(fd)) _exit(125);
}
__attribute__((constructor)) static void loaded(void) {
    char marker[64];
    snprintf(marker, sizeof marker, "loaded %ld\n", (long)getpid());
    record(marker);
}
static int inet(int family) { return family == AF_INET || family == AF_INET6; }
int socket(int family, int type, int protocol) {
    if (inet(family)) { record("socket\n"); errno = EACCES; return -1; }
    int (*real)(int, int, int) = dlsym(RTLD_NEXT, "socket");
    return real(family, type, protocol);
}
int bind(int fd, const struct sockaddr *address, socklen_t size) {
    if (address && inet(address->sa_family)) { record("bind\n"); errno = EACCES; return -1; }
    int (*real)(int, const struct sockaddr *, socklen_t) = dlsym(RTLD_NEXT, "bind");
    return real(fd, address, size);
}
int connect(int fd, const struct sockaddr *address, socklen_t size) {
    if (address && inet(address->sa_family)) { record("connect\n"); errno = EACCES; return -1; }
    int (*real)(int, const struct sockaddr *, socklen_t) = dlsym(RTLD_NEXT, "connect");
    return real(fd, address, size);
}
int getaddrinfo(const char *name, const char *service, const struct addrinfo *hints,
                struct addrinfo **result) {
    (void)name; (void)service; (void)hints; (void)result;
    record("getaddrinfo\n");
    return EAI_FAIL;
}

struct hostent *gethostbyname(const char *name) {
    (void)name;
    record("gethostbyname\n"); h_errno = NO_RECOVERY; return NULL;
}
struct hostent *gethostbyname2(const char *name, int family) {
    (void)name; (void)family;
    record("gethostbyname2\n"); h_errno = NO_RECOVERY; return NULL;
}
int gethostbyname_r(const char *name, struct hostent *host, char *buffer, size_t size,
                    struct hostent **result, int *error) {
    (void)name; (void)host; (void)buffer; (void)size;
    record("gethostbyname_r\n"); *result = NULL; *error = NO_RECOVERY; return EACCES;
}
int gethostbyname2_r(const char *name, int family, struct hostent *host, char *buffer,
                     size_t size, struct hostent **result, int *error) {
    (void)name; (void)family; (void)host; (void)buffer; (void)size;
    record("gethostbyname2_r\n"); *result = NULL; *error = NO_RECOVERY; return EACCES;
}
int getnameinfo(const struct sockaddr *address, socklen_t size, char *host, socklen_t host_size,
                char *service, socklen_t service_size, int flags) {
    (void)address; (void)size; (void)host; (void)host_size;
    (void)service; (void)service_size; (void)flags;
    record("getnameinfo\n"); return EAI_FAIL;
}
struct hostent *gethostbyaddr(const void *address, socklen_t size, int family) {
    (void)address; (void)size; (void)family;
    record("gethostbyaddr\n"); h_errno = NO_RECOVERY; return NULL;
}
int gethostbyaddr_r(const void *address, socklen_t address_size, int family, struct hostent *host,
                    char *buffer, size_t size, struct hostent **result, int *error) {
    (void)address; (void)address_size; (void)family; (void)host; (void)buffer; (void)size;
    record("gethostbyaddr_r\n"); *result = NULL; *error = NO_RECOVERY; return EACCES;
}
