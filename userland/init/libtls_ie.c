#include "minilibc.h"

__thread int ext_tls = 11;

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

int lib_tls_read(void) {
    return ext_tls;
}

void lib_tls_write(int value) {
    ext_tls = value;
}

ku_u64 lib_tls_tp(void) {
    return current_tp();
}
