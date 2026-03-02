#include "minilibc.h"

extern int main(int argc, char **argv, char **envp);

__attribute__((noreturn))
void ku_crt_start(ku_u64 *sp) {
    int argc = (int)sp[0];
    char **argv = (char **)&sp[1];
    char **envp = argv + argc + 1;
    int rc = main(argc, argv, envp);
    ku_exit(rc);
}

#if defined(__aarch64__)
__attribute__((noreturn, naked))
void _start(void) {
    __asm__ volatile(
        "mov x0, sp\n"
        "b ku_crt_start\n"
    );
}
#elif defined(__riscv)
__attribute__((noreturn, naked))
void _start(void) {
    __asm__ volatile(
        "mv a0, sp\n"
        "j ku_crt_start\n"
    );
}
#else
#error unsupported arch
#endif
