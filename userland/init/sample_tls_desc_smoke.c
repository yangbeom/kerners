#include "minilibc.h"

extern __thread int ext_tls;

static char tls_desc_child_stack[16384] __attribute__((aligned(16)));
static volatile ku_u64 tls_desc_parent_tp = 0;

__attribute__((noinline))
static int tls_desc_read(void) {
    return ext_tls;
}

__attribute__((noinline))
static void tls_desc_write(int value) {
    ext_tls = value;
}

#if defined(__aarch64__)
static ku_u64 current_tp(void) {
    ku_u64 tp = 0;
    __asm__ volatile("mrs %0, tpidr_el0" : "=r"(tp));
    return tp;
}
#elif defined(__riscv)
static ku_u64 current_tp(void) {
    ku_u64 tp = 0;
    __asm__ volatile("mv %0, tp" : "=r"(tp));
    return tp;
}
#else
static ku_u64 current_tp(void) {
    return 0;
}
#endif

static int fail_step(const char *step, ku_s64 rc) {
    ku_write_str("TLS_DESC_SMOKE_FAIL_");
    ku_write_str(step);
    ku_write_str("_RC=");
    ku_write_s64(rc);
    ku_write_raw("\n", 1);
    return 1;
}

static int run_tls_desc_mt_smoke(void) {
    ku_u64 stack_top = (ku_u64)&tls_desc_child_stack[sizeof(tls_desc_child_stack)];
    stack_top &= ~(ku_u64)0xFUL;

    tls_desc_parent_tp = current_tp();
    ku_write_str("TLS_DESC_PARENT_TP=");
    ku_write_u64(tls_desc_parent_tp);
    ku_write_raw("\n", 1);

    ku_s64 child = ku_clone(KU_CLONE_VM | KU_SIGCHLD, (void *)stack_top, 0, 0, 0);
    if (child < 0) {
        return fail_step("CLONE", child);
    }

    if (child == 0) {
        ku_u64 child_tp = current_tp();
        ku_write_str("TLS_DESC_CHILD_TP=");
        ku_write_u64(child_tp);
        ku_write_raw("\n", 1);

        if (child_tp == tls_desc_parent_tp) {
            ku_exit(201);
        }

        int child_init = tls_desc_read();
        if (child_init != 11) {
            ku_write_str("TLS_DESC_CHILD_INIT=");
            ku_write_s64(child_init);
            ku_write_raw("\n", 1);
            ku_exit(202);
        }

        tls_desc_write(77);
        if (tls_desc_read() != 77) {
            ku_exit(203);
        }
        ku_exit(0);
    }

    int status = 0;
    ku_s64 waited = ku_wait4(child, &status, 0, 0);
    if (waited != child) {
        return fail_step("WAIT4", waited);
    }
    if ((status & 0x7f) != 0 || ((status >> 8) & 0xff) != 0) {
        return fail_step("WAIT_STATUS", status);
    }
    if (tls_desc_read() != 35) {
        return fail_step("PARENT_STABLE", tls_desc_read());
    }

    ku_write_line("TLS_DESC_SMOKE_MT_OK");
    return 0;
}

int main(int argc, char **argv, char **envp) {
    (void)argc;
    (void)argv;
    (void)envp;

    ku_write_line("TLS_DESC_SMOKE_BEGIN");

    if (tls_desc_read() != 11) {
        return fail_step("INIT", tls_desc_read());
    }
    tls_desc_write(21);
    if (tls_desc_read() != 21) {
        return fail_step("WRITE1", tls_desc_read());
    }
    tls_desc_write(35);
    if (tls_desc_read() != 35) {
        return fail_step("WRITE2", tls_desc_read());
    }

    if (run_tls_desc_mt_smoke() != 0) {
        return 1;
    }

    ku_write_line("TLS_DESC_SMOKE_OK");
    return 0;
}
