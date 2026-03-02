#!/bin/bash
# Build Phase 15-4 minimal libc replacement and sample user programs.
#
# Usage:
#   ./scripts/build_user_minilibc_samples.sh [ARCH] [OUT_DIR]
#
# Outputs:
#   <OUT_DIR>/sample_hello_dyn
#   <OUT_DIR>/sample_syscall_smoke_dyn
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

print_info() { echo -e "${GREEN}[phase15-4-mini]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[phase15-4-mini]${NC} $1"; }
print_error() { echo -e "${RED}[phase15-4-mini]${NC} $1"; }

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
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/kerners-mini.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

MINILIBC_DIR="$PROJECT_ROOT/userland/common"
CRT_SRC="$PROJECT_ROOT/userland/common/crt0.c"
LIB_SRC="$PROJECT_ROOT/userland/common/minilibc.c"
HELLO_SRC="$PROJECT_ROOT/userland/hello/sample_hello.c"
SMOKE_SRC="$PROJECT_ROOT/userland/init/sample_syscall_smoke.c"

for src in "$CRT_SRC" "$LIB_SRC" "$HELLO_SRC" "$SMOKE_SRC"; do
    if [[ ! -f "$src" ]]; then
        print_error "missing source file: $src"
        exit 1
    fi
done

print_info "prepare dynamic loader ($ARCH)"
"$SCRIPT_DIR/build_user_dynamic_c_bins.sh" "$ARCH" "$OUT_DIR" >/dev/null

CRT_OBJ="$TMP_DIR/crt0.o"
LIB_OBJ="$TMP_DIR/minilibc.o"
HELLO_OBJ="$TMP_DIR/sample_hello.o"
SMOKE_OBJ="$TMP_DIR/sample_syscall_smoke.o"

HELLO_BIN="$OUT_DIR/sample_hello_dyn"
SMOKE_BIN="$OUT_DIR/sample_syscall_smoke_dyn"
LD_BIN="$OUT_DIR/$LD_SO_NAME"

COMMON_FLAGS=(
    -O2
    -fno-stack-protector
    -fno-builtin
    -ffreestanding
    -nostdlib
    -fPIE
    -Wall
    -Wextra
    -I"$MINILIBC_DIR"
)

print_info "build minilibc objects ($ARCH)"
"$CLANG" --target="$TARGET_TRIPLE" "${COMMON_FLAGS[@]}" -c "$CRT_SRC" -o "$CRT_OBJ"
"$CLANG" --target="$TARGET_TRIPLE" "${COMMON_FLAGS[@]}" -c "$LIB_SRC" -o "$LIB_OBJ"
"$CLANG" --target="$TARGET_TRIPLE" "${COMMON_FLAGS[@]}" -c "$HELLO_SRC" -o "$HELLO_OBJ"
"$CLANG" --target="$TARGET_TRIPLE" "${COMMON_FLAGS[@]}" -c "$SMOKE_SRC" -o "$SMOKE_OBJ"

print_info "link sample_hello_dyn ($ARCH)"
"$RUST_LLD" -flavor gnu -m "$LLD_EMULATION" -pie \
    --dynamic-linker "/lib/$LD_SO_NAME" \
    -e _start \
    -o "$HELLO_BIN" \
    "$CRT_OBJ" "$LIB_OBJ" "$HELLO_OBJ"

print_info "link sample_syscall_smoke_dyn ($ARCH)"
"$RUST_LLD" -flavor gnu -m "$LLD_EMULATION" -pie \
    --dynamic-linker "/lib/$LD_SO_NAME" \
    -e _start \
    -o "$SMOKE_BIN" \
    "$CRT_OBJ" "$LIB_OBJ" "$SMOKE_OBJ"

if [[ ! -x "$HELLO_BIN" || ! -x "$SMOKE_BIN" || ! -x "$LD_BIN" ]]; then
    print_error "build failed: output file missing"
    exit 1
fi

if command -v file >/dev/null 2>&1; then
    print_info "sample_hello: $(file "$HELLO_BIN")"
    print_info "sample_smoke: $(file "$SMOKE_BIN")"
    print_info "loader: $(file "$LD_BIN")"
fi

if command -v shasum >/dev/null 2>&1; then
    print_info "sha256:"
    shasum -a 256 "$HELLO_BIN" "$SMOKE_BIN" "$LD_BIN"
fi

print_info "output:"
print_info "  $HELLO_BIN"
print_info "  $SMOKE_BIN"
print_info "  $LD_BIN"
