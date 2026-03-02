#include "minilibc.h"

#define KU_SYS_OPENAT 56
#define KU_SYS_CLOSE 57
#define KU_SYS_READ 63
#define KU_SYS_WRITE 64
#define KU_SYS_GETDENTS64 61
#define KU_SYS_MKDIRAT 34
#define KU_SYS_UNLINKAT 35
#define KU_SYS_GETPID 172
#define KU_SYS_GETPPID 173
#define KU_SYS_GETTID 178
#define KU_SYS_CLOCK_GETTIME 113
#define KU_SYS_CLONE 220
#define KU_SYS_WAIT4 260
#define KU_SYS_EXIT 93

#if defined(__aarch64__)
ku_s64 ku_raw_syscall6(
    ku_u64 nr,
    ku_u64 a0,
    ku_u64 a1,
    ku_u64 a2,
    ku_u64 a3,
    ku_u64 a4,
    ku_u64 a5
) {
    register ku_u64 x0 __asm__("x0") = a0;
    register ku_u64 x1 __asm__("x1") = a1;
    register ku_u64 x2 __asm__("x2") = a2;
    register ku_u64 x3 __asm__("x3") = a3;
    register ku_u64 x4 __asm__("x4") = a4;
    register ku_u64 x5 __asm__("x5") = a5;
    register ku_u64 x8 __asm__("x8") = nr;
    __asm__ volatile("svc #0"
                     : "+r"(x0)
                     : "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5), "r"(x8)
                     : "memory");
    return (ku_s64)x0;
}
#elif defined(__riscv)
ku_s64 ku_raw_syscall6(
    ku_u64 nr,
    ku_u64 a0,
    ku_u64 a1,
    ku_u64 a2,
    ku_u64 a3,
    ku_u64 a4,
    ku_u64 a5
) {
    register ku_u64 x10 __asm__("a0") = a0;
    register ku_u64 x11 __asm__("a1") = a1;
    register ku_u64 x12 __asm__("a2") = a2;
    register ku_u64 x13 __asm__("a3") = a3;
    register ku_u64 x14 __asm__("a4") = a4;
    register ku_u64 x15 __asm__("a5") = a5;
    register ku_u64 x17 __asm__("a7") = nr;
    __asm__ volatile("ecall"
                     : "+r"(x10)
                     : "r"(x11), "r"(x12), "r"(x13), "r"(x14), "r"(x15), "r"(x17)
                     : "memory");
    return (ku_s64)x10;
}
#else
#error unsupported arch
#endif

ku_s64 ku_openat(int dirfd, const char *path, ku_u64 flags, ku_u64 mode) {
    return ku_raw_syscall6(KU_SYS_OPENAT, (ku_u64)(ku_s64)dirfd, (ku_u64)path, flags, mode, 0, 0);
}

ku_s64 ku_close(int fd) {
    return ku_raw_syscall6(KU_SYS_CLOSE, (ku_u64)(ku_s64)fd, 0, 0, 0, 0, 0);
}

ku_s64 ku_read(int fd, void *buf, ku_u64 len) {
    return ku_raw_syscall6(KU_SYS_READ, (ku_u64)(ku_s64)fd, (ku_u64)buf, len, 0, 0, 0);
}

ku_s64 ku_write(int fd, const void *buf, ku_u64 len) {
    return ku_raw_syscall6(KU_SYS_WRITE, (ku_u64)(ku_s64)fd, (ku_u64)buf, len, 0, 0, 0);
}

ku_s64 ku_getdents64(int fd, void *buf, ku_u64 len) {
    return ku_raw_syscall6(KU_SYS_GETDENTS64, (ku_u64)(ku_s64)fd, (ku_u64)buf, len, 0, 0, 0);
}

ku_s64 ku_mkdirat(int dirfd, const char *path, ku_u64 mode) {
    return ku_raw_syscall6(KU_SYS_MKDIRAT, (ku_u64)(ku_s64)dirfd, (ku_u64)path, mode, 0, 0, 0);
}

ku_s64 ku_unlinkat(int dirfd, const char *path, ku_u64 flags) {
    return ku_raw_syscall6(KU_SYS_UNLINKAT, (ku_u64)(ku_s64)dirfd, (ku_u64)path, flags, 0, 0, 0);
}

ku_s64 ku_getpid(void) {
    return ku_raw_syscall6(KU_SYS_GETPID, 0, 0, 0, 0, 0, 0);
}

ku_s64 ku_getppid(void) {
    return ku_raw_syscall6(KU_SYS_GETPPID, 0, 0, 0, 0, 0, 0);
}

ku_s64 ku_gettid(void) {
    return ku_raw_syscall6(KU_SYS_GETTID, 0, 0, 0, 0, 0, 0);
}

ku_s64 ku_clone(
    ku_u64 flags,
    void *child_stack,
    void *parent_tid_ptr,
    ku_u64 tls,
    void *child_tid_ptr
) {
    return ku_raw_syscall6(
        KU_SYS_CLONE,
        flags,
        (ku_u64)child_stack,
        (ku_u64)parent_tid_ptr,
        tls,
        (ku_u64)child_tid_ptr,
        0
    );
}

ku_s64 ku_wait4(ku_s64 pid, int *status, int options, void *rusage) {
    return ku_raw_syscall6(
        KU_SYS_WAIT4,
        (ku_u64)pid,
        (ku_u64)status,
        (ku_u64)(ku_s64)options,
        (ku_u64)rusage,
        0,
        0
    );
}

ku_s64 ku_clock_gettime(int clock_id, struct ku_timespec *ts) {
    return ku_raw_syscall6(KU_SYS_CLOCK_GETTIME, (ku_u64)(ku_s64)clock_id, (ku_u64)ts, 0, 0, 0, 0);
}

void ku_exit(int code) {
    (void)ku_raw_syscall6(KU_SYS_EXIT, (ku_u64)(ku_s64)code, 0, 0, 0, 0, 0);
    for (;;) {
    }
}

ku_u64 ku_strlen(const char *s) {
    ku_u64 n = 0;
    while (s[n] != '\0') {
        n += 1;
    }
    return n;
}

int ku_memcmp(const void *a, const void *b, ku_u64 n) {
    const unsigned char *pa = (const unsigned char *)a;
    const unsigned char *pb = (const unsigned char *)b;
    ku_u64 i = 0;

    while (i < n) {
        if (pa[i] != pb[i]) {
            return 1;
        }
        i += 1;
    }

    return 0;
}

void ku_write_raw(const char *buf, ku_u64 len) {
    (void)ku_write(1, buf, len);
}

void ku_write_str(const char *s) {
    ku_write_raw(s, ku_strlen(s));
}

void ku_write_line(const char *s) {
    ku_write_str(s);
    ku_write_raw("\n", 1);
}

void ku_write_u64(ku_u64 value) {
    char tmp[32];
    ku_u64 i = 0;

    if (value == 0) {
        ku_write_raw("0", 1);
        return;
    }

    while (value != 0 && i < (ku_u64)sizeof(tmp)) {
        tmp[i] = (char)('0' + (value % 10));
        value /= 10;
        i += 1;
    }

    while (i > 0) {
        i -= 1;
        ku_write_raw(&tmp[i], 1);
    }
}

void ku_write_s64(ku_s64 value) {
    if (value < 0) {
        ku_u64 magnitude = (ku_u64)(-(value + 1)) + 1;
        ku_write_raw("-", 1);
        ku_write_u64(magnitude);
        return;
    }

    ku_write_u64((ku_u64)value);
}
