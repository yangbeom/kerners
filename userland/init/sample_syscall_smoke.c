#include "minilibc.h"

static int fail_step(const char *step, ku_s64 rc) {
    ku_write_str("MINILIBC_SMOKE_FAIL_");
    ku_write_str(step);
    ku_write_str("_RC=");
    ku_write_s64(rc);
    ku_write_raw("\n", 1);
    return 1;
}

int main(int argc, char **argv, char **envp) {
    static const char payload[] = "minilibc-smoke\n";
    char io_buf[128];
    char dent_buf[512];
    struct ku_timespec ts;
    ku_s64 fd;
    ku_s64 n;

    (void)argc;
    (void)argv;
    (void)envp;

    ku_write_line("MINILIBC_SMOKE_BEGIN");

    if (ku_getpid() <= 0 || ku_getppid() < 0) {
        return fail_step("PID", -1);
    }

    fd = ku_openat(KU_AT_FDCWD, "/sample_minilibc.txt", KU_O_WRONLY | KU_O_CREAT | KU_O_TRUNC, 0644);
    if (fd < 0) {
        return fail_step("OPEN_WRITE", fd);
    }

    n = ku_write((int)fd, payload, (ku_u64)(sizeof(payload) - 1));
    (void)ku_close((int)fd);
    if (n != (ku_s64)(sizeof(payload) - 1)) {
        return fail_step("WRITE", n);
    }

    fd = ku_openat(KU_AT_FDCWD, "/sample_minilibc.txt", KU_O_RDONLY, 0);
    if (fd < 0) {
        return fail_step("OPEN_READ", fd);
    }

    n = ku_read((int)fd, io_buf, (ku_u64)sizeof(io_buf));
    (void)ku_close((int)fd);
    if (n != (ku_s64)(sizeof(payload) - 1)) {
        return fail_step("READ", n);
    }

    if (ku_memcmp(io_buf, payload, (ku_u64)(sizeof(payload) - 1)) != 0) {
        return fail_step("READ_CMP", -1);
    }

    if (ku_mkdirat(KU_AT_FDCWD, "/sample_minilibc_dir", 0755) < 0) {
        return fail_step("MKDIR", -1);
    }

    fd = ku_openat(KU_AT_FDCWD, "/proc", KU_O_RDONLY, 0);
    if (fd < 0) {
        return fail_step("OPEN_PROC", fd);
    }

    n = ku_getdents64((int)fd, dent_buf, (ku_u64)sizeof(dent_buf));
    (void)ku_close((int)fd);
    if (n <= 0) {
        return fail_step("GETDENTS", n);
    }

    if (ku_clock_gettime(KU_CLOCK_MONOTONIC, &ts) < 0) {
        return fail_step("CLOCK", -1);
    }

    if (ts.tv_nsec < 0 || ts.tv_nsec >= 1000000000LL) {
        return fail_step("CLOCK_RANGE", ts.tv_nsec);
    }

    if (ku_unlinkat(KU_AT_FDCWD, "/sample_minilibc.txt", 0) < 0) {
        return fail_step("UNLINK_FILE", -1);
    }

    if (ku_unlinkat(KU_AT_FDCWD, "/sample_minilibc_dir", KU_AT_REMOVEDIR) < 0) {
        return fail_step("UNLINK_DIR", -1);
    }

    ku_write_line("MINILIBC_SMOKE_OK");
    return 0;
}
