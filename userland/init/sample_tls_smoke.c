#include "minilibc.h"

__thread int tls_counter = 41;

static int fail_step(const char *step, ku_s64 rc) {
    ku_write_str("TLS_SMOKE_FAIL_");
    ku_write_str(step);
    ku_write_str("_RC=");
    ku_write_s64(rc);
    ku_write_raw("\n", 1);
    return 1;
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

    ku_write_line("TLS_SMOKE_OK");
    return 0;
}
