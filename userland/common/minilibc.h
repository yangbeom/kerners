#ifndef KERNERS_MINILIBC_H
#define KERNERS_MINILIBC_H

typedef unsigned long ku_u64;
typedef long ku_s64;

struct ku_timespec {
    ku_s64 tv_sec;
    ku_s64 tv_nsec;
};

enum {
    KU_AT_FDCWD = -100,
    KU_AT_REMOVEDIR = 0x200,
};

enum {
    KU_O_RDONLY = 0,
    KU_O_WRONLY = 1,
    KU_O_RDWR = 2,
    KU_O_CREAT = 0x40,
    KU_O_TRUNC = 0x200,
};

enum {
    KU_CLOCK_MONOTONIC = 1,
};

enum {
    KU_SIGCHLD = 17,
    KU_CLONE_VM = 0x00000100,
    KU_CLONE_FS = 0x00000200,
    KU_CLONE_FILES = 0x00000400,
    KU_CLONE_SIGHAND = 0x00000800,
    KU_CLONE_SETTLS = 0x00080000,
};

ku_s64 ku_raw_syscall6(
    ku_u64 nr,
    ku_u64 a0,
    ku_u64 a1,
    ku_u64 a2,
    ku_u64 a3,
    ku_u64 a4,
    ku_u64 a5
);

ku_s64 ku_openat(int dirfd, const char *path, ku_u64 flags, ku_u64 mode);
ku_s64 ku_close(int fd);
ku_s64 ku_read(int fd, void *buf, ku_u64 len);
ku_s64 ku_write(int fd, const void *buf, ku_u64 len);
ku_s64 ku_getdents64(int fd, void *buf, ku_u64 len);
ku_s64 ku_mkdirat(int dirfd, const char *path, ku_u64 mode);
ku_s64 ku_unlinkat(int dirfd, const char *path, ku_u64 flags);
ku_s64 ku_getpid(void);
ku_s64 ku_getppid(void);
ku_s64 ku_gettid(void);
ku_s64 ku_clone(
    ku_u64 flags,
    void *child_stack,
    void *parent_tid_ptr,
    ku_u64 tls,
    void *child_tid_ptr
);
ku_s64 ku_wait4(ku_s64 pid, int *status, int options, void *rusage);
ku_s64 ku_clock_gettime(int clock_id, struct ku_timespec *ts);
void ku_exit(int code) __attribute__((noreturn));

ku_u64 ku_strlen(const char *s);
int ku_memcmp(const void *a, const void *b, ku_u64 n);

void ku_write_raw(const char *buf, ku_u64 len);
void ku_write_str(const char *s);
void ku_write_line(const char *s);
void ku_write_u64(ku_u64 value);
void ku_write_s64(ku_s64 value);

#endif
