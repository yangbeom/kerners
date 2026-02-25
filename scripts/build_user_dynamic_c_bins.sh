#!/bin/bash
# Build minimal dynamic user ELF binaries for Phase 15-3 using C toolchain (clang + rust-lld).
#
# Usage:
#   ./scripts/build_user_dynamic_c_bins.sh [ARCH] [OUT_DIR]
#
# Outputs:
#   <OUT_DIR>/hello_dyn
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
LD_SRC="$TMP_DIR/ld_kerners.c"
HELLO_OBJ="$TMP_DIR/hello_dyn.o"
LD_OBJ="$TMP_DIR/ld_kerners.o"
HELLO_BIN="$OUT_DIR/hello_dyn"
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
"$CLANG" --target="$TARGET_TRIPLE" "${COMMON_FLAGS[@]}" -c "$LD_SRC" -o "$LD_OBJ"

"$RUST_LLD" -flavor gnu -m "$LLD_EMULATION" -pie \
    --dynamic-linker "/lib/$LD_SO_NAME" \
    -e _start -o "$HELLO_BIN" "$HELLO_OBJ"

"$RUST_LLD" -flavor gnu -m "$LLD_EMULATION" -pie \
    -e _start -o "$LD_BIN" "$LD_OBJ"

if command -v file >/dev/null 2>&1; then
    print_info "hello: $(file "$HELLO_BIN")"
    print_info "loader: $(file "$LD_BIN")"
fi

if [[ ! -x "$HELLO_BIN" || ! -x "$LD_BIN" ]]; then
    print_error "build failed: output file missing"
    exit 1
fi

if ! file "$HELLO_BIN" | grep -q "dynamically linked"; then
    print_warn "hello_dyn is not reported as dynamically linked"
fi

print_info "output:"
print_info "  $HELLO_BIN"
print_info "  $LD_BIN"
