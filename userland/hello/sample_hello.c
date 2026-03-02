#include "minilibc.h"

int main(int argc, char **argv, char **envp) {
    (void)argc;
    (void)argv;
    (void)envp;

    ku_write_line("MINILIBC_HELLO_OK");
    return 42;
}
