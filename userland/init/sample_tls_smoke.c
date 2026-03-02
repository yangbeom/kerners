#include "minilibc.h"

__thread int tls_counter = 41;
static char tls_child_stack[16384] __attribute__((aligned(16)));
static volatile ku_u64 tls_parent_tp_snapshot = 0;

#if defined(__aarch64__)
#define TLS_DATA_OFFSET 16
#elif defined(__riscv)
#define TLS_DATA_OFFSET 0
#else
#define TLS_DATA_OFFSET 0
#endif

static ku_u64 current_tp(void) {
#if defined(__aarch64__)
    ku_u64 tp = 0;
    __asm__ volatile("mrs %0, tpidr_el0" : "=r"(tp));
    return tp;
#elif defined(__riscv)
    ku_u64 tp = 0;
    __asm__ volatile("mv %0, tp" : "=r"(tp));
    return tp;
#else
    return 0;
#endif
}

static volatile int *tls_slot_ptr(void) {
    return (volatile int *)(current_tp() + TLS_DATA_OFFSET);
}

static int fail_step(const char *step, ku_s64 rc) {
    ku_write_str("TLS_SMOKE_FAIL_");
    ku_write_str(step);
    ku_write_str("_RC=");
    ku_write_s64(rc);
    ku_write_raw("\n", 1);
    return 1;
}

static int run_multithread_tls_smoke(void) {
    ku_u64 stack_top = (ku_u64)&tls_child_stack[sizeof(tls_child_stack)];
    stack_top &= ~(ku_u64)0xFUL;

    tls_parent_tp_snapshot = current_tp();
    volatile int *parent_slot = tls_slot_ptr();
    *parent_slot = 111;
    ku_write_str("TLS_SMOKE_PARENT_TP=");
    ku_write_u64(tls_parent_tp_snapshot);
    ku_write_raw("\n", 1);
    ku_s64 child = ku_clone(KU_CLONE_VM | KU_SIGCHLD, (void *)stack_top, 0, 0, 0);
    if (child < 0) {
        return fail_step("CLONE", child);
    }

    if (child == 0) {
        volatile int *child_slot = tls_slot_ptr();
        ku_write_str("TLS_SMOKE_CHILD_TP=");
        ku_write_u64(current_tp());
        ku_write_raw("\n", 1);
        if (current_tp() == tls_parent_tp_snapshot) {
            ku_exit(204);
        }
        if (*child_slot != 41) {
            ku_write_str("TLS_SMOKE_CHILD_INIT=");
            ku_write_s64(*child_slot);
            ku_write_raw("\n", 1);
            ku_exit(201);
        }
        *child_slot = 333;
        if (*child_slot != 333) {
            ku_exit(202);
        }
        if (ku_gettid() <= 0) {
            ku_exit(203);
        }
        ku_exit(0);
    }

    if (*parent_slot != 111) {
        return fail_step("PARENT_PREWAIT", *parent_slot);
    }

    int status = 0;
    ku_s64 waited = ku_wait4(child, &status, 0, 0);
    if (waited != child) {
        return fail_step("WAIT4", waited);
    }
    if ((status & 0x7f) != 0 || ((status >> 8) & 0xff) != 0) {
        return fail_step("WAIT_STATUS", status);
    }
    if (*parent_slot != 111) {
        return fail_step("PARENT_POSTWAIT", *parent_slot);
    }

    ku_write_line("TLS_SMOKE_MT_OK");
    return 0;
}

int main(int argc, char **argv, char **envp) {
    (void)argc;
    (void)argv;
    (void)envp;

    ku_write_line("TLS_SMOKE_BEGIN");

    if (tls_counter != 41) {
        return fail_step("INIT", tls_counter);
    }
    tls_counter += 1;
    if (tls_counter != 42) {
        return fail_step("INCR", tls_counter);
    }

    if (run_multithread_tls_smoke() != 0) {
        return 1;
    }

    ku_write_line("TLS_SMOKE_OK");
    return 0;
}
