#include "minilibc.h"

extern __thread int ext_tls;
int lib_tls_read(void);
void lib_tls_write(int value);
ku_u64 lib_tls_tp(void);

static char tls_ie_child_stack[16384] __attribute__((aligned(16)));
static volatile ku_u64 tls_ie_parent_tp = 0;

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
    ku_write_str("TLS_IE_SMOKE_FAIL_");
    ku_write_str(step);
    ku_write_str("_RC=");
    ku_write_s64(rc);
    ku_write_raw("\n", 1);
    return 1;
}

static int run_tls_ie_mt_smoke(void) {
    ku_u64 stack_top = (ku_u64)&tls_ie_child_stack[sizeof(tls_ie_child_stack)];
    stack_top &= ~(ku_u64)0xFUL;

    tls_ie_parent_tp = current_tp();
    ku_write_str("TLS_IE_PARENT_TP=");
    ku_write_u64(tls_ie_parent_tp);
    ku_write_raw("\n", 1);
    ku_s64 child = ku_clone(KU_CLONE_VM | KU_SIGCHLD, (void *)stack_top, 0, 0, 0);
    if (child < 0) {
        return fail_step("CLONE", child);
    }

    if (child == 0) {
        ku_u64 child_tp = current_tp();
        ku_write_str("TLS_IE_CHILD_TP=");
        ku_write_u64(child_tp);
        ku_write_raw("\n", 1);
        ku_write_str("TLS_IE_CHILD_LIBTP=");
        ku_write_u64(lib_tls_tp());
        ku_write_raw("\n", 1);
        if (child_tp == tls_ie_parent_tp) {
            ku_exit(201);
        }
        int child_init = lib_tls_read();
        if (child_init != 11) {
            ku_write_str("TLS_IE_CHILD_INIT=");
            ku_write_s64(child_init);
            ku_write_raw("\n", 1);
            ku_exit(202);
        }
        lib_tls_write(77);
        if (lib_tls_read() != 77) {
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
    if (ext_tls != 35) {
        return fail_step("PARENT_STABLE", ext_tls);
    }

    ku_write_line("TLS_IE_SMOKE_MT_OK");
    return 0;
}

int main(int argc, char **argv, char **envp) {
    (void)argc;
    (void)argv;
    (void)envp;

    ku_write_line("TLS_IE_SMOKE_BEGIN");
    ku_write_str("TLS_IE_MAIN_TP=");
    ku_write_u64(current_tp());
    ku_write_raw("\n", 1);
    ku_write_str("TLS_IE_MAIN_LIBTP=");
    ku_write_u64(lib_tls_tp());
    ku_write_raw("\n", 1);

    if (ext_tls != 11) {
        return fail_step("INIT", ext_tls);
    }
    ext_tls = 21;
    if (lib_tls_read() != 21) {
        return fail_step("SHARED_VIEW", lib_tls_read());
    }
    lib_tls_write(35);
    if (ext_tls != 35) {
        return fail_step("WRITE_BACK", ext_tls);
    }

    if (run_tls_ie_mt_smoke() != 0) {
        return 1;
    }

    ku_write_line("TLS_IE_SMOKE_OK");
    return 0;
}
