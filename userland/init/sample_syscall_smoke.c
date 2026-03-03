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
    int pipefd[2];
    ku_s64 proc_fd;
    ku_s64 n;

    (void)argc;
    (void)argv;
    (void)envp;

    ku_write_line("MINILIBC_SMOKE_BEGIN");

    if (ku_getpid() <= 0 || ku_getppid() < 0) {
        return fail_step("PID", -1);
    }

    n = ku_pipe2(pipefd, 0);
    if (n != 0) {
        return fail_step("PIPE2", n);
    }

    n = ku_write(pipefd[1], payload, (ku_u64)(sizeof(payload) - 1));
    if (n != (ku_s64)(sizeof(payload) - 1)) {
        return fail_step("WRITE", n);
    }

    n = ku_read(pipefd[0], io_buf, (ku_u64)sizeof(io_buf));
    if (n != (ku_s64)(sizeof(payload) - 1)) {
        return fail_step("READ", n);
    }

    if (ku_memcmp(io_buf, payload, (ku_u64)(sizeof(payload) - 1)) != 0) {
        return fail_step("READ_CMP", -1);
    }

    proc_fd = ku_openat(KU_AT_FDCWD, "/proc", KU_O_RDONLY, 0);
    if (proc_fd < 0) {
        return fail_step("OPEN_PROC", proc_fd);
    }

    n = ku_getdents64((int)proc_fd, dent_buf, (ku_u64)sizeof(dent_buf));
    (void)ku_close((int)proc_fd);
    if (n <= 0) {
        return fail_step("GETDENTS", n);
    }

    if (ku_clock_gettime(KU_CLOCK_MONOTONIC, &ts) < 0) {
        return fail_step("CLOCK", -1);
    }

    if (ts.tv_nsec < 0 || ts.tv_nsec >= 1000000000LL) {
        return fail_step("CLOCK_RANGE", ts.tv_nsec);
    }

    if (ku_close(pipefd[0]) < 0 || ku_close(pipefd[1]) < 0) {
        return fail_step("CLOSE_PIPE", -1);
    }

    ku_write_line("MINILIBC_SMOKE_OK");
    return 0;
}
