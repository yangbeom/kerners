#!/bin/bash
# Build minimal dynamic user ELF binaries for Phase 15-3 using C toolchain (clang + rust-lld).
#
# Usage:
#   ./scripts/build_user_dynamic_c_bins.sh [ARCH] [OUT_DIR]
#
# Outputs:
#   <OUT_DIR>/hello_dyn
#   <OUT_DIR>/busybox_dyn
#   <OUT_DIR>/ld-kerners-<arch>.so

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-aarch64}"
OUT_DIR="${2:-$PROJECT_ROOT/target/user/$ARCH}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[phase15-3-cdyn]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[phase15-3-cdyn]${NC} $1"; }
print_error() { echo -e "${RED}[phase15-3-cdyn]${NC} $1"; }

find_rust_lld() {
    if command -v rust-lld >/dev/null 2>&1; then
        command -v rust-lld
        return 0
    fi

    if ! command -v rustc >/dev/null 2>&1; then
        return 1
    fi

    local sysroot
    sysroot="$(rustc --print sysroot 2>/dev/null || true)"
    if [[ -z "$sysroot" ]]; then
        return 1
    fi

    local candidate
    for candidate in "$sysroot"/lib/rustlib/*/bin/rust-lld; do
        if [[ -x "$candidate" ]]; then
            echo "$candidate"
            return 0
        fi
    done

    return 1
}

if [[ -x "/opt/homebrew/opt/llvm/bin/clang" ]]; then
    CLANG="/opt/homebrew/opt/llvm/bin/clang"
elif command -v clang >/dev/null 2>&1; then
    CLANG="$(command -v clang)"
else
    print_error "clang not found"
    exit 1
fi

if ! RUST_LLD="$(find_rust_lld)"; then
    print_error "rust-lld not found"
    exit 1
fi

case "$ARCH" in
    aarch64)
        TARGET_TRIPLE="aarch64-linux-gnu"
        LLD_EMULATION="aarch64linux"
        LD_SO_NAME="ld-kerners-aarch64.so"
        ;;
    riscv64)
        TARGET_TRIPLE="riscv64-linux-gnu"
        LLD_EMULATION="elf64lriscv"
        LD_SO_NAME="ld-kerners-riscv64.so"
        ;;
    *)
        print_error "unsupported arch: $ARCH (expected: aarch64 or riscv64)"
        exit 1
        ;;
esac

mkdir -p "$OUT_DIR"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/kerners-cdyn.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

HELLO_SRC="$TMP_DIR/hello_dyn.c"
BB_SRC="$TMP_DIR/busybox_dyn.c"
LD_SRC="$TMP_DIR/ld_kerners.c"
HELLO_OBJ="$TMP_DIR/hello_dyn.o"
BB_OBJ="$TMP_DIR/busybox_dyn.o"
LD_OBJ="$TMP_DIR/ld_kerners.o"
HELLO_BIN="$OUT_DIR/hello_dyn"
BB_BIN="$OUT_DIR/busybox_dyn"
LD_BIN="$OUT_DIR/$LD_SO_NAME"

cat >"$HELLO_SRC" <<'EOF'
typedef unsigned long u64;
typedef long s64;

#if defined(__aarch64__)
static s64 sys_write(int fd, const char *buf, u64 len) {
    register s64 x0 __asm__("x0") = (s64)fd;
    register const char *x1 __asm__("x1") = buf;
    register u64 x2 __asm__("x2") = len;
    register s64 x8 __asm__("x8") = 64;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x1), "r"(x2), "r"(x8) : "memory");
    return x0;
}

__attribute__((noreturn))
static void sys_exit(int code) {
    register s64 x0 __asm__("x0") = (s64)code;
    register s64 x8 __asm__("x8") = 93;
    __asm__ volatile("svc #0" : : "r"(x0), "r"(x8) : "memory");
    for (;;) {}
}
#elif defined(__riscv)
static s64 sys_write(int fd, const char *buf, u64 len) {
    register s64 a0 __asm__("a0") = (s64)fd;
    register const char *a1 __asm__("a1") = buf;
    register u64 a2 __asm__("a2") = len;
    register s64 a7 __asm__("a7") = 64;
    __asm__ volatile("ecall" : "+r"(a0) : "r"(a1), "r"(a2), "r"(a7) : "memory");
    return a0;
}

__attribute__((noreturn))
static void sys_exit(int code) {
    register s64 a0 __asm__("a0") = (s64)code;
    register s64 a7 __asm__("a7") = 93;
    __asm__ volatile("ecall" : : "r"(a0), "r"(a7) : "memory");
    for (;;) {}
}
#else
#error unsupported arch
#endif

__attribute__((noreturn))
void _start(void) {
    static const char msg[] = "CDYN_HELLO_OK\n";
    (void)sys_write(1, msg, sizeof(msg) - 1);
    sys_exit(42);
}
EOF

cat >"$BB_SRC" <<'EOF'
typedef unsigned long u64;
typedef long s64;
typedef unsigned int u32;

#define AT_FDCWD (-100)
#define AT_REMOVEDIR 0x200
#define O_RDONLY 0
#define O_WRONLY 1
#define O_CREAT 0x40
#define O_TRUNC 0x200

#define SYS_READ 63
#define SYS_WRITE 64
#define SYS_OPENAT 56
#define SYS_CLOSE 57
#define SYS_GETDENTS64 61
#define SYS_MKDIRAT 34
#define SYS_UNLINKAT 35
#define SYS_GETPID 172
#define SYS_GETPPID 173
#define SYS_EXIT 93

static u64 c_strlen(const char *s) {
    u64 n = 0;
    while (s[n] != '\0') {
        n += 1;
    }
    return n;
}

static int c_streq(const char *a, const char *b) {
    u64 i = 0;
    while (a[i] != '\0' || b[i] != '\0') {
        if (a[i] != b[i]) {
            return 0;
        }
        i += 1;
    }
    return 1;
}

static const char *base_name(const char *path) {
    const char *base = path;
    u64 i = 0;
    while (path[i] != '\0') {
        if (path[i] == '/') {
            base = &path[i + 1];
        }
        i += 1;
    }
    return base;
}

static void write_raw(const char *buf, u64 len);

static void write_str(const char *s) {
    write_raw(s, c_strlen(s));
}

static void write_line(const char *s) {
    write_str(s);
    write_raw("\n", 1);
}

static void write_u64(u64 value) {
    char tmp[32];
    u64 i = 0;
    if (value == 0) {
        write_raw("0", 1);
        return;
    }
    while (value != 0 && i < (u64)sizeof(tmp)) {
        tmp[i] = (char)('0' + (value % 10));
        value /= 10;
        i += 1;
    }
    while (i > 0) {
        i -= 1;
        write_raw(&tmp[i], 1);
    }
}

#if defined(__aarch64__)
static s64 raw_syscall6(u64 nr, u64 a0, u64 a1, u64 a2, u64 a3, u64 a4, u64 a5) {
    register u64 x0 __asm__("x0") = a0;
    register u64 x1 __asm__("x1") = a1;
    register u64 x2 __asm__("x2") = a2;
    register u64 x3 __asm__("x3") = a3;
    register u64 x4 __asm__("x4") = a4;
    register u64 x5 __asm__("x5") = a5;
    register u64 x8 __asm__("x8") = nr;
    __asm__ volatile("svc #0"
                     : "+r"(x0)
                     : "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5), "r"(x8)
                     : "memory");
    return (s64)x0;
}

__attribute__((noreturn, naked))
void _start(void) {
    __asm__ volatile(
        "mov x0, sp\n"
        "b bb_start\n"
    );
}
#elif defined(__riscv)
static s64 raw_syscall6(u64 nr, u64 a0, u64 a1, u64 a2, u64 a3, u64 a4, u64 a5) {
    register u64 x10 __asm__("a0") = a0;
    register u64 x11 __asm__("a1") = a1;
    register u64 x12 __asm__("a2") = a2;
    register u64 x13 __asm__("a3") = a3;
    register u64 x14 __asm__("a4") = a4;
    register u64 x15 __asm__("a5") = a5;
    register u64 x17 __asm__("a7") = nr;
    __asm__ volatile("ecall"
                     : "+r"(x10)
                     : "r"(x11), "r"(x12), "r"(x13), "r"(x14), "r"(x15), "r"(x17)
                     : "memory");
    return (s64)x10;
}

__attribute__((noreturn, naked))
void _start(void) {
    __asm__ volatile(
        "mv a0, sp\n"
        "j bb_start\n"
    );
}
#else
#error unsupported arch
#endif

static s64 sys_read(int fd, void *buf, u64 len) {
    return raw_syscall6(SYS_READ, (u64)fd, (u64)buf, len, 0, 0, 0);
}

static s64 sys_write(int fd, const void *buf, u64 len) {
    return raw_syscall6(SYS_WRITE, (u64)fd, (u64)buf, len, 0, 0, 0);
}

static s64 sys_openat(int dirfd, const char *path, u64 flags, u64 mode) {
    return raw_syscall6(SYS_OPENAT, (u64)dirfd, (u64)path, flags, mode, 0, 0);
}

static s64 sys_close(int fd) {
    return raw_syscall6(SYS_CLOSE, (u64)fd, 0, 0, 0, 0, 0);
}

static s64 sys_mkdirat(int dirfd, const char *path, u64 mode) {
    return raw_syscall6(SYS_MKDIRAT, (u64)dirfd, (u64)path, mode, 0, 0, 0);
}

static s64 sys_unlinkat(int dirfd, const char *path, u64 flags) {
    return raw_syscall6(SYS_UNLINKAT, (u64)dirfd, (u64)path, flags, 0, 0, 0);
}

static s64 sys_getdents64(int fd, void *buf, u64 len) {
    return raw_syscall6(SYS_GETDENTS64, (u64)fd, (u64)buf, len, 0, 0, 0);
}

static s64 sys_getpid(void) {
    return raw_syscall6(SYS_GETPID, 0, 0, 0, 0, 0, 0);
}

static s64 sys_getppid(void) {
    return raw_syscall6(SYS_GETPPID, 0, 0, 0, 0, 0, 0);
}

__attribute__((noreturn))
static void sys_exit(int code) {
    (void)raw_syscall6(SYS_EXIT, (u64)code, 0, 0, 0, 0, 0);
    for (;;) {}
}

static void write_raw(const char *buf, u64 len) {
    (void)sys_write(1, buf, len);
}

static int app_echo(int argc, char **argv) {
    int i;
    for (i = 1; i < argc; i++) {
        if (i > 1) {
            write_raw(" ", 1);
        }
        write_str(argv[i]);
    }
    write_raw("\n", 1);
    return 0;
}

static int app_cat(const char *path) {
    char buf[256];
    s64 fd = sys_openat(AT_FDCWD, path, O_RDONLY, 0);
    if (fd < 0) {
        return 1;
    }
    while (1) {
        s64 n = sys_read((int)fd, buf, sizeof(buf));
        if (n <= 0) {
            break;
        }
        if (sys_write(1, buf, (u64)n) < 0) {
            (void)sys_close((int)fd);
            return 1;
        }
    }
    (void)sys_close((int)fd);
    return 0;
}

static int app_head(const char *path) {
    char buf[256];
    s64 fd = sys_openat(AT_FDCWD, path, O_RDONLY, 0);
    u64 i;
    if (fd < 0) {
        return 1;
    }
    s64 n = sys_read((int)fd, buf, sizeof(buf));
    (void)sys_close((int)fd);
    if (n <= 0) {
        return 1;
    }
    for (i = 0; i < (u64)n; i++) {
        if (sys_write(1, &buf[i], 1) < 0) {
            return 1;
        }
        if (buf[i] == '\n') {
            break;
        }
    }
    if (i == (u64)n) {
        write_raw("\n", 1);
    }
    return 0;
}

static int app_ps(void) {
    char dent_buf[1024];
    char status_buf[256];
    s64 fd = sys_openat(AT_FDCWD, "/proc", O_RDONLY, 0);
    if (fd >= 0) {
        (void)sys_getdents64((int)fd, dent_buf, sizeof(dent_buf));
        (void)sys_close((int)fd);
    }

    write_str("PID=");
    write_u64((u64)sys_getpid());
    write_str(" PPID=");
    write_u64((u64)sys_getppid());
    write_raw("\n", 1);

    fd = sys_openat(AT_FDCWD, "/proc/self/status", O_RDONLY, 0);
    if (fd < 0) {
        return 1;
    }
    s64 n = sys_read((int)fd, status_buf, sizeof(status_buf));
    (void)sys_close((int)fd);
    if (n > 0) {
        if (sys_write(1, status_buf, (u64)n) < 0) {
            return 1;
        }
    }
    return 0;
}

static int run_applet(const char *applet, int argc, char **argv) {
    if (c_streq(applet, "echo")) {
        return app_echo(argc, argv);
    }
    if (c_streq(applet, "cat")) {
        if (argc < 2) {
            return 1;
        }
        return app_cat(argv[1]);
    }
    if (c_streq(applet, "mkdir")) {
        if (argc < 2) {
            return 1;
        }
        return sys_mkdirat(AT_FDCWD, argv[1], 0755) < 0 ? 1 : 0;
    }
    if (c_streq(applet, "rm")) {
        if (argc < 2) {
            return 1;
        }
        return sys_unlinkat(AT_FDCWD, argv[1], 0) < 0 ? 1 : 0;
    }
    if (c_streq(applet, "rmdir")) {
        if (argc < 2) {
            return 1;
        }
        return sys_unlinkat(AT_FDCWD, argv[1], AT_REMOVEDIR) < 0 ? 1 : 0;
    }
    if (c_streq(applet, "head")) {
        if (argc < 2) {
            return 1;
        }
        return app_head(argv[1]);
    }
    if (c_streq(applet, "ps")) {
        return app_ps();
    }
    if (c_streq(applet, "sh")) {
        write_line("BBDYN_SH_BYPASS");
        return 0;
    }
    return 127;
}

static int prepare_sample_file(void) {
    static const char content[] = "bbdyn_line1\nbbdyn_line2\n";
    s64 fd = sys_openat(AT_FDCWD, "/bbdyn.txt", O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) {
        return 1;
    }
    if (sys_write((int)fd, content, sizeof(content) - 1) < 0) {
        (void)sys_close((int)fd);
        return 1;
    }
    (void)sys_close((int)fd);
    return 0;
}

static int app_init(void) {
    char *echo_argv[] = {"echo", "BBDYN_ECHO", "OK", 0};
    char *cat_argv[] = {"cat", "/bbdyn.txt", 0};
    char *mkdir_argv[] = {"mkdir", "/bbdyn_dir", 0};
    char *head_argv[] = {"head", "/bbdyn.txt", 0};
    char *ps_argv[] = {"ps", 0};
    char *rm_argv[] = {"rm", "/bbdyn.txt", 0};
    char *rmdir_argv[] = {"rmdir", "/bbdyn_dir", 0};

    write_line("BBDYN_BOOT_BEGIN");

    if (prepare_sample_file() != 0) {
        write_line("BBDYN_PREP_FAIL");
        return 1;
    }

    if (run_applet("echo", 3, echo_argv) != 0) {
        write_line("BBDYN_CMD_ECHO_FAIL");
        return 1;
    }
    write_line("BBDYN_CMD_ECHO_OK");

    if (run_applet("cat", 2, cat_argv) != 0) {
        write_line("BBDYN_CMD_CAT_FAIL");
        return 1;
    }
    write_line("BBDYN_CMD_CAT_OK");

    if (run_applet("mkdir", 2, mkdir_argv) != 0) {
        write_line("BBDYN_CMD_MKDIR_FAIL");
        return 1;
    }
    write_line("BBDYN_CMD_MKDIR_OK");

    if (run_applet("head", 2, head_argv) != 0) {
        write_line("BBDYN_CMD_HEAD_FAIL");
        return 1;
    }
    write_line("BBDYN_CMD_HEAD_OK");

    if (run_applet("ps", 1, ps_argv) != 0) {
        write_line("BBDYN_CMD_PS_FAIL");
        return 1;
    }
    write_line("BBDYN_CMD_PS_OK");

    if (run_applet("rm", 2, rm_argv) != 0) {
        write_line("BBDYN_CMD_RM_FAIL");
        return 1;
    }
    write_line("BBDYN_CMD_RM_OK");

    if (run_applet("rmdir", 2, rmdir_argv) != 0) {
        write_line("BBDYN_CMD_RMDIR_FAIL");
        return 1;
    }
    write_line("BBDYN_CMD_RMDIR_OK");

    write_line("BBDYN_BOOT_END");
    return 0;
}

static int app_main(int argc, char **argv) {
    const char *argv0 = "busybox";
    const char *applet;

    if (argc > 0 && argv != 0 && argv[0] != 0) {
        argv0 = argv[0];
    }

    applet = base_name(argv0);
    if (c_streq(applet, "busybox")) {
        if (argc < 2 || argv[1] == 0) {
            write_line("busybox_dyn");
            return 0;
        }
        applet = base_name(argv[1]);
        argc -= 1;
        argv += 1;
    }

    if (c_streq(applet, "init")) {
        return app_init();
    }

    int rc = run_applet(applet, argc, argv);
    if (rc == 127) {
        write_str("BBDYN_UNKNOWN_APPLET=");
        write_line(applet);
    }
    return rc;
}

__attribute__((noreturn))
void bb_start(u64 *sp) {
    int argc = (int)sp[0];
    char **argv = (char **)(sp + 1);
    int rc = app_main(argc, argv);
    sys_exit(rc);
}
EOF

cat >"$LD_SRC" <<'EOF'
typedef unsigned long u64;
typedef long s64;

#define AT_NULL 0
#define AT_ENTRY 9

#if defined(__aarch64__)
static s64 sys_write(int fd, const char *buf, u64 len) {
    register s64 x0 __asm__("x0") = (s64)fd;
    register const char *x1 __asm__("x1") = buf;
    register u64 x2 __asm__("x2") = len;
    register s64 x8 __asm__("x8") = 64;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x1), "r"(x2), "r"(x8) : "memory");
    return x0;
}

__attribute__((noreturn))
static void sys_exit(int code) {
    register s64 x0 __asm__("x0") = (s64)code;
    register s64 x8 __asm__("x8") = 93;
    __asm__ volatile("svc #0" : : "r"(x0), "r"(x8) : "memory");
    for (;;) {}
}

__attribute__((noreturn))
static void jump_to_entry(u64 *sp, u64 entry) {
    __asm__ volatile(
        "mov sp, %0\n"
        "br %1\n"
        :
        : "r"(sp), "r"(entry)
        : "memory"
    );
    __builtin_unreachable();
}

__attribute__((noreturn, naked))
void _start(void) {
    __asm__ volatile(
        "mov x0, sp\n"
        "b ld_start\n"
    );
}
#elif defined(__riscv)
static s64 sys_write(int fd, const char *buf, u64 len) {
    register s64 a0 __asm__("a0") = (s64)fd;
    register const char *a1 __asm__("a1") = buf;
    register u64 a2 __asm__("a2") = len;
    register s64 a7 __asm__("a7") = 64;
    __asm__ volatile("ecall" : "+r"(a0) : "r"(a1), "r"(a2), "r"(a7) : "memory");
    return a0;
}

__attribute__((noreturn))
static void sys_exit(int code) {
    register s64 a0 __asm__("a0") = (s64)code;
    register s64 a7 __asm__("a7") = 93;
    __asm__ volatile("ecall" : : "r"(a0), "r"(a7) : "memory");
    for (;;) {}
}

__attribute__((noreturn))
static void jump_to_entry(u64 *sp, u64 entry) {
    __asm__ volatile(
        "mv sp, %0\n"
        "jr %1\n"
        :
        : "r"(sp), "r"(entry)
        : "memory"
    );
    __builtin_unreachable();
}

__attribute__((noreturn, naked))
void _start(void) {
    __asm__ volatile(
        "mv a0, sp\n"
        "j ld_start\n"
    );
}
#else
#error unsupported arch
#endif

__attribute__((noreturn))
void ld_start(u64 *sp) {
    u64 *orig_sp = sp;
    u64 argc = *sp;
    sp += 1;
    sp += argc;
    sp += 1;

    while (*sp != 0) {
        sp += 1;
    }
    sp += 1;

    u64 entry = 0;
    while (1) {
        u64 tag = sp[0];
        u64 value = sp[1];
        if (tag == AT_NULL) {
            break;
        }
        if (tag == AT_ENTRY) {
            entry = value;
        }
        sp += 2;
    }

    if (entry == 0) {
        static const char msg[] = "CDYN_LD_NO_ENTRY\n";
        (void)sys_write(2, msg, sizeof(msg) - 1);
        sys_exit(127);
    }

    jump_to_entry(orig_sp, entry);
}
EOF

COMMON_FLAGS=(
    -O2
    -ffreestanding
    -fno-stack-protector
    -fno-builtin
    -fpie
    -nostdlib
)

print_info "building C dynamic hello for $ARCH"
"$CLANG" --target="$TARGET_TRIPLE" "${COMMON_FLAGS[@]}" -c "$HELLO_SRC" -o "$HELLO_OBJ"
"$CLANG" --target="$TARGET_TRIPLE" "${COMMON_FLAGS[@]}" -c "$BB_SRC" -o "$BB_OBJ"
"$CLANG" --target="$TARGET_TRIPLE" "${COMMON_FLAGS[@]}" -c "$LD_SRC" -o "$LD_OBJ"

"$RUST_LLD" -flavor gnu -m "$LLD_EMULATION" -pie \
    --dynamic-linker "/lib/$LD_SO_NAME" \
    -e _start -o "$HELLO_BIN" "$HELLO_OBJ"

"$RUST_LLD" -flavor gnu -m "$LLD_EMULATION" -pie \
    --dynamic-linker "/lib/$LD_SO_NAME" \
    -e _start -o "$BB_BIN" "$BB_OBJ"

"$RUST_LLD" -flavor gnu -m "$LLD_EMULATION" -pie \
    -e _start -o "$LD_BIN" "$LD_OBJ"

if command -v file >/dev/null 2>&1; then
    print_info "hello: $(file "$HELLO_BIN")"
    print_info "busybox: $(file "$BB_BIN")"
    print_info "loader: $(file "$LD_BIN")"
fi

if [[ ! -x "$HELLO_BIN" || ! -x "$BB_BIN" || ! -x "$LD_BIN" ]]; then
    print_error "build failed: output file missing"
    exit 1
fi

if ! file "$HELLO_BIN" | grep -q "dynamically linked"; then
    print_warn "hello_dyn is not reported as dynamically linked"
fi

if ! file "$BB_BIN" | grep -q "dynamically linked"; then
    print_warn "busybox_dyn is not reported as dynamically linked"
fi

print_info "output:"
print_info "  $HELLO_BIN"
print_info "  $BB_BIN"
print_info "  $LD_BIN"
