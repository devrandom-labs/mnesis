/* Test-only syscall interposition. A marker arms persistent EIO failures in
 * this child process; no production code or device fault is being simulated.
 * Large uncompressed frames force write failures inside journal write_batch.
 * fsync/fdatasync failures exercise the real journal persistence error path. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int armed(const char *operation) {
    const char *marker = getenv("MNESIS_FAULT_MARKER");
    const char *selected = getenv("MNESIS_FAULT_OPERATION");
    return marker && selected && strcmp(selected, operation) == 0
        && access(marker, F_OK) == 0;
}

static ssize_t fault_write(int fd, const void *data, size_t len) {
    if (fd > STDERR_FILENO && armed("write")) {
        errno = EIO;
        return -1;
    }
#ifdef __APPLE__
    /* dyld does not apply an image's interposition to its own references. */
    return write(fd, data, len);
#else
    ssize_t (*original)(int, const void *, size_t) = dlsym(RTLD_NEXT, "write");
    return original(fd, data, len);
#endif
}

static int fault_fsync(int fd) {
    if (armed("sync")) {
        errno = EIO;
        return -1;
    }
#ifdef __APPLE__
    return fsync(fd);
#else
    int (*original)(int) = dlsym(RTLD_NEXT, "fsync");
    return original(fd);
#endif
}

#ifndef __APPLE__
static int fault_fdatasync(int fd) {
    if (armed("sync")) {
        errno = EIO;
        return -1;
    }
    int (*original)(int) = dlsym(RTLD_NEXT, "fdatasync");
    return original(fd);
}
#endif

#ifdef __APPLE__
/* Rust uses F_FULLFSYNC on Darwin before its fsync fallback. Forward only
 * command shapes used by this fixture; reject unknown varargs shapes rather
 * than guessing their type. This library is never a production preload. */
static int fault_fcntl(int fd, int command, ...) {
    switch (command) {
        case F_FULLFSYNC:
#ifdef F_BARRIERFSYNC
        case F_BARRIERFSYNC:
#endif
            if (armed("sync")) { errno = EIO; return -1; }
            return fcntl(fd, command);
        case F_GETFD: case F_GETFL: case F_GETOWN:
            return fcntl(fd, command);
        case F_DUPFD: case F_DUPFD_CLOEXEC:
        case F_SETFD: case F_SETFL: case F_SETOWN:
        case F_NOCACHE: case F_RDAHEAD: case F_SETNOSIGPIPE: {
            va_list args; va_start(args, command);
            int value = va_arg(args, int);
            va_end(args);
            return fcntl(fd, command, value);
        }
        case F_GETPATH: case F_PREALLOCATE: case F_GETLK:
        case F_SETLK: case F_SETLKW: case F_RDADVISE: case F_LOG2PHYS: {
            va_list args; va_start(args, command);
            void *value = va_arg(args, void *);
            va_end(args);
            return fcntl(fd, command, value);
        }
        default: errno = EINVAL; return -1;
    }
}
#define INTERPOSE(replacement, original) \
    __attribute__((used, section("__DATA,__interpose"))) \
    static const struct { const void *replace; const void *replacee; } \
        pair_##original = { (const void *)&replacement, (const void *)&original }
INTERPOSE(fault_write, write);
INTERPOSE(fault_fsync, fsync);
INTERPOSE(fault_fcntl, fcntl);
#else
ssize_t write(int fd, const void *data, size_t len) { return fault_write(fd, data, len); }
int fsync(int fd) { return fault_fsync(fd); }
int fdatasync(int fd) { return fault_fdatasync(fd); }
#endif
