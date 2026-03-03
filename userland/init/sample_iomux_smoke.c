#include "minilibc.h"

static int fail_step(const char *step, ku_s64 rc) {
    ku_write_str("MINILIBC_IOMUX_FAIL_");
    ku_write_str(step);
    ku_write_str("_RC=");
    ku_write_s64(rc);
    ku_write_raw("\n", 1);
    return 1;
}

static void fdset_zero(unsigned char *set, ku_u64 len) {
    ku_u64 i = 0;
    while (i < len) {
        set[i] = 0;
        i += 1;
    }
}

static void fdset_set(unsigned char *set, int fd) {
    ku_u64 idx = (ku_u64)((unsigned int)fd >> 3);
    unsigned int bit = (unsigned int)fd & 7u;
    set[idx] = (unsigned char)(set[idx] | (unsigned char)(1u << bit));
}

static int fdset_isset(const unsigned char *set, int fd) {
    ku_u64 idx = (ku_u64)((unsigned int)fd >> 3);
    unsigned int bit = (unsigned int)fd & 7u;
    return (set[idx] & (unsigned char)(1u << bit)) != 0;
}

int main(int argc, char **argv, char **envp) {
    int pipefd[2];
    int epfd;
    char byte = 'X';
    struct ku_timespec zero = {0, 0};
    struct ku_pollfd pfd;
    unsigned char readfds[128];
    struct ku_epoll_event watch;
    struct ku_epoll_event events[2];
    ku_s64 rc;

    (void)argc;
    (void)argv;
    (void)envp;

    ku_write_line("MINILIBC_IOMUX_BEGIN");

    rc = ku_pipe2(pipefd, 0);
    if (rc != 0) {
        return fail_step("PIPE2", rc);
    }

    pfd.fd = pipefd[0];
    pfd.events = KU_POLLIN;
    pfd.revents = 0;

    rc = ku_ppoll(&pfd, 1, &zero, (const ku_u64 *)0, 0);
    if (rc != 0) {
        return fail_step("PPOLL_EMPTY", rc);
    }

    rc = ku_write(pipefd[1], &byte, 1);
    if (rc != 1) {
        return fail_step("PPOLL_WRITE", rc);
    }

    pfd.revents = 0;
    rc = ku_ppoll(&pfd, 1, &zero, (const ku_u64 *)0, 0);
    if (rc != 1 || (pfd.revents & KU_POLLIN) == 0) {
        return fail_step("PPOLL_READY", rc);
    }

    rc = ku_read(pipefd[0], &byte, 1);
    if (rc != 1) {
        return fail_step("PPOLL_READ", rc);
    }

    fdset_zero(readfds, (ku_u64)sizeof(readfds));
    fdset_set(readfds, pipefd[0]);

    rc = ku_write(pipefd[1], &byte, 1);
    if (rc != 1) {
        return fail_step("PSELECT_WRITE", rc);
    }

    rc = ku_pselect6(pipefd[0] + 1, readfds, 0, 0, &zero, (const struct ku_pselect_sigmask_arg *)0);
    if (rc != 1 || !fdset_isset(readfds, pipefd[0])) {
        return fail_step("PSELECT_READY", rc);
    }

    rc = ku_read(pipefd[0], &byte, 1);
    if (rc != 1) {
        return fail_step("PSELECT_READ", rc);
    }

    epfd = (int)ku_epoll_create1(0);
    if (epfd < 0) {
        return fail_step("EPOLL_CREATE", epfd);
    }

    watch.events = KU_EPOLLIN | KU_EPOLLET;
    watch.data = 0x1111222233334444UL;
    rc = ku_epoll_ctl(epfd, KU_EPOLL_CTL_ADD, pipefd[0], &watch);
    if (rc != 0) {
        return fail_step("EPOLL_ADD", rc);
    }

    rc = ku_epoll_pwait(epfd, events, 2, 0, (const ku_u64 *)0, 0);
    if (rc != 0) {
        return fail_step("EPOLL_ET_EMPTY", rc);
    }

    rc = ku_write(pipefd[1], &byte, 1);
    if (rc != 1) {
        return fail_step("EPOLL_ET_WRITE1", rc);
    }

    rc = ku_epoll_pwait(epfd, events, 2, 0, (const ku_u64 *)0, 0);
    if (rc != 1 || (events[0].events & KU_EPOLLIN) == 0 || events[0].data != 0x1111222233334444UL) {
        return fail_step("EPOLL_ET_READY1", rc);
    }

    rc = ku_epoll_pwait(epfd, events, 2, 0, (const ku_u64 *)0, 0);
    if (rc != 0) {
        return fail_step("EPOLL_ET_STABLE", rc);
    }

    rc = ku_read(pipefd[0], &byte, 1);
    if (rc != 1) {
        return fail_step("EPOLL_ET_READ1", rc);
    }

    rc = ku_write(pipefd[1], &byte, 1);
    if (rc != 1) {
        return fail_step("EPOLL_ET_WRITE2", rc);
    }

    rc = ku_epoll_pwait(epfd, events, 2, 0, (const ku_u64 *)0, 0);
    if (rc != 1 || (events[0].events & KU_EPOLLIN) == 0) {
        return fail_step("EPOLL_ET_READY2", rc);
    }

    watch.events = KU_EPOLLIN | KU_EPOLLONESHOT;
    watch.data = 0xAAAABBBBCCCCDDDDUL;
    rc = ku_epoll_ctl(epfd, KU_EPOLL_CTL_MOD, pipefd[0], &watch);
    if (rc != 0) {
        return fail_step("EPOLL_ONESHOT_MOD1", rc);
    }

    rc = ku_epoll_pwait(epfd, events, 2, 0, (const ku_u64 *)0, 0);
    if (rc != 1 || (events[0].events & KU_EPOLLIN) == 0 || events[0].data != 0xAAAABBBBCCCCDDDDUL) {
        return fail_step("EPOLL_ONESHOT_FIRST", rc);
    }

    rc = ku_epoll_pwait(epfd, events, 2, 0, (const ku_u64 *)0, 0);
    if (rc != 0) {
        return fail_step("EPOLL_ONESHOT_MASKED", rc);
    }

    watch.events = KU_EPOLLIN | KU_EPOLLONESHOT;
    watch.data = 0xDEADBEEF10203040UL;
    rc = ku_epoll_ctl(epfd, KU_EPOLL_CTL_MOD, pipefd[0], &watch);
    if (rc != 0) {
        return fail_step("EPOLL_ONESHOT_MOD2", rc);
    }

    rc = ku_epoll_pwait(epfd, events, 2, 0, (const ku_u64 *)0, 0);
    if (rc != 1 || (events[0].events & KU_EPOLLIN) == 0 || events[0].data != 0xDEADBEEF10203040UL) {
        return fail_step("EPOLL_ONESHOT_REARM", rc);
    }

    (void)ku_close(epfd);
    (void)ku_close(pipefd[0]);
    (void)ku_close(pipefd[1]);

    ku_write_line("MINILIBC_IOMUX_OK");
    return 0;
}
