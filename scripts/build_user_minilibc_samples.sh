#!/bin/bash
# Build Phase 15-4 minimal libc replacement and sample user programs.
#
# Usage:
#   ./scripts/build_user_minilibc_samples.sh [ARCH] [OUT_DIR]
#
# Outputs:
#   <OUT_DIR>/sample_hello_dyn
#   <OUT_DIR>/sample_syscall_smoke_dyn
#   <OUT_DIR>/sample_tls_smoke_dyn
#   <OUT_DIR>/sample_tls_ie_smoke_dyn
#   <OUT_DIR>/libtls_ie.so
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
TLS_SRC="$PROJECT_ROOT/userland/init/sample_tls_smoke.c"
TLS_IE_SRC="$PROJECT_ROOT/userland/init/sample_tls_ie_smoke.c"
TLS_IE_LIB_SRC="$PROJECT_ROOT/userland/init/libtls_ie.c"

for src in "$CRT_SRC" "$LIB_SRC" "$HELLO_SRC" "$SMOKE_SRC" "$TLS_SRC" "$TLS_IE_SRC" "$TLS_IE_LIB_SRC"; do
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
TLS_OBJ="$TMP_DIR/sample_tls_smoke.o"
TLS_IE_OBJ="$TMP_DIR/sample_tls_ie_smoke.o"
TLS_IE_LIB_OBJ="$TMP_DIR/libtls_ie.o"

HELLO_BIN="$OUT_DIR/sample_hello_dyn"
SMOKE_BIN="$OUT_DIR/sample_syscall_smoke_dyn"
TLS_BIN="$OUT_DIR/sample_tls_smoke_dyn"
TLS_IE_BIN="$OUT_DIR/sample_tls_ie_smoke_dyn"
TLS_IE_LIB_SO="$OUT_DIR/libtls_ie.so"
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
# 15.5 baseline은 local-exec TLS 모델을 기준으로 커널 주도 TP/TLS 블록 분리를 검증한다.
# (__tls_get_addr + DTV 기반 동적 TLS 재배치는 Phase 15.5-3 범위)
"$CLANG" --target="$TARGET_TRIPLE" "${COMMON_FLAGS[@]}" -ftls-model=local-exec -c "$TLS_SRC" -o "$TLS_OBJ"
"$CLANG" --target="$TARGET_TRIPLE" "${COMMON_FLAGS[@]}" -ftls-model=initial-exec -c "$TLS_IE_SRC" -o "$TLS_IE_OBJ"
"$CLANG" --target="$TARGET_TRIPLE" -O2 -fno-stack-protector -fno-builtin -ffreestanding -nostdlib -fPIC -Wall -Wextra \
    -I"$MINILIBC_DIR" -ftls-model=initial-exec -c "$TLS_IE_LIB_SRC" -o "$TLS_IE_LIB_OBJ"

print_info "link libtls_ie.so ($ARCH)"
"$RUST_LLD" -flavor gnu -m "$LLD_EMULATION" -shared \
    -soname libtls_ie.so \
    -o "$TLS_IE_LIB_SO" \
    "$TLS_IE_LIB_OBJ"

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

print_info "link sample_tls_smoke_dyn ($ARCH)"
"$RUST_LLD" -flavor gnu -m "$LLD_EMULATION" -pie \
    --dynamic-linker "/lib/$LD_SO_NAME" \
    -e _start \
    -o "$TLS_BIN" \
    "$CRT_OBJ" "$LIB_OBJ" "$TLS_OBJ"

print_info "link sample_tls_ie_smoke_dyn ($ARCH)"
"$RUST_LLD" -flavor gnu -m "$LLD_EMULATION" -pie \
    --dynamic-linker "/lib/$LD_SO_NAME" \
    -e _start \
    -o "$TLS_IE_BIN" \
    "$CRT_OBJ" "$LIB_OBJ" "$TLS_IE_OBJ" "$TLS_IE_LIB_SO"

if [[ ! -x "$HELLO_BIN" || ! -x "$SMOKE_BIN" || ! -x "$TLS_BIN" || ! -x "$TLS_IE_BIN" || ! -f "$TLS_IE_LIB_SO" || ! -x "$LD_BIN" ]]; then
    print_error "build failed: output file missing"
    exit 1
fi

if command -v file >/dev/null 2>&1; then
    print_info "sample_hello: $(file "$HELLO_BIN")"
    print_info "sample_smoke: $(file "$SMOKE_BIN")"
    print_info "sample_tls: $(file "$TLS_BIN")"
    print_info "sample_tls_ie: $(file "$TLS_IE_BIN")"
    print_info "libtls_ie: $(file "$TLS_IE_LIB_SO")"
    print_info "loader: $(file "$LD_BIN")"
fi

if command -v shasum >/dev/null 2>&1; then
    print_info "sha256:"
    shasum -a 256 "$HELLO_BIN" "$SMOKE_BIN" "$TLS_BIN" "$TLS_IE_BIN" "$TLS_IE_LIB_SO" "$LD_BIN"
fi

print_info "output:"
print_info "  $HELLO_BIN"
print_info "  $SMOKE_BIN"
print_info "  $TLS_BIN"
print_info "  $TLS_IE_BIN"
print_info "  $TLS_IE_LIB_SO"
print_info "  $LD_BIN"
