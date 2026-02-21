#!/bin/bash
# Phase 15-1 verification runner (external static ELF + BusyBox shell path)
#
# Usage:
#   ./scripts/verify_phase15_1.sh [ARCH] [BUSYBOX_PATH] [HELLO_BIN] [PROBE_BIN] [TIMEOUT_SEC]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_DIR="$PROJECT_ROOT/logs"

ARCH="${1:-aarch64}"
BUSYBOX_PATH="${2:-$PROJECT_ROOT/target/user/$ARCH/busybox}"
HELLO_BIN="${3:-$PROJECT_ROOT/target/user/$ARCH/hello}"
PROBE_BIN="${4:-$PROJECT_ROOT/target/user/$ARCH/execve_bounds}"
TIMEOUT_SEC="${5:-35}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[phase15-1]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[phase15-1]${NC} $1"; }
print_error() { echo -e "${RED}[phase15-1]${NC} $1"; }

case "$ARCH" in
    aarch64)
        TARGET="aarch64-unknown-none-softfloat"
        QEMU_BIN="qemu-system-aarch64"
        QEMU_ARGS=(
            -machine virt
            -cpu cortex-a57
            -smp 1
            -m 512M
            -nographic
        )
        ;;
    riscv64)
        TARGET="riscv64gc-unknown-none-elf"
        QEMU_BIN="qemu-system-riscv64"
        QEMU_ARGS=(
            -machine virt
            -smp 1
            -m 512M
            -nographic
            -bios none
        )
        ;;
    *)
        print_error "unsupported arch: $ARCH"
        exit 1
        ;;
esac

if ! command -v mcopy >/dev/null 2>&1 || ! command -v mmd >/dev/null 2>&1; then
    print_error "mtools (mcopy/mmd) is required"
    exit 1
fi

mkdir -p "$LOG_DIR"
STAMP="$(date +%Y%m%d-%H%M%S)"
DISK_IMG="$LOG_DIR/phase15-1-${ARCH}-${STAMP}.img"
RUN_LOG="$LOG_DIR/phase15-1-${ARCH}-${STAMP}.log"
BUILD_LOG="$LOG_DIR/phase15-1-${ARCH}-${STAMP}.build.log"

if [[ ! -f "$HELLO_BIN" || ! -f "$PROBE_BIN" ]]; then
    print_info "user bins missing, building via scripts/build_user_static_bins.sh"
    "$SCRIPT_DIR/build_user_static_bins.sh" "$ARCH"
fi

for path in "$BUSYBOX_PATH" "$HELLO_BIN" "$PROBE_BIN"; do
    if [[ ! -f "$path" ]]; then
        print_error "missing file: $path"
        exit 1
    fi
done

if command -v file >/dev/null 2>&1; then
    print_info "binary info"
    file "$BUSYBOX_PATH" "$HELLO_BIN" "$PROBE_BIN"
fi

print_info "preparing disk image: $DISK_IMG"
"$SCRIPT_DIR/prepare_user_disk.sh" "$ARCH" "$BUSYBOX_PATH" "$DISK_IMG"

# external ELF 배치
mcopy -o -i "$DISK_IMG" "$HELLO_BIN" ::/bin/hello
mcopy -o -i "$DISK_IMG" "$HELLO_BIN" ::/usr/bin/hello
mcopy -o -i "$DISK_IMG" "$PROBE_BIN" ::/bin/execve_bounds
mcopy -o -i "$DISK_IMG" "$PROBE_BIN" ::/usr/bin/execve_bounds

# PID1 우선 후보(/mnt/init)에 probe 바이너리를 배치한다.
mcopy -o -i "$DISK_IMG" "$PROBE_BIN" ::/init
mcopy -o -i "$DISK_IMG" "$PROBE_BIN" ::/sbin/init

KERNEL_ELF="$PROJECT_ROOT/target/$TARGET/release/kerners"
KERNEL_BIN="$PROJECT_ROOT/target/$TARGET/release/kerners.bin"

print_info "building kernel (target=$TARGET)"
(
    cd "$PROJECT_ROOT"
    cargo build --release --target "$TARGET"
) >"$BUILD_LOG" 2>&1

if [[ "$ARCH" == "aarch64" ]]; then
    if command -v llvm-objcopy >/dev/null 2>&1; then
        llvm-objcopy -O binary "$KERNEL_ELF" "$KERNEL_BIN"
    elif command -v objcopy >/dev/null 2>&1; then
        objcopy -O binary "$KERNEL_ELF" "$KERNEL_BIN"
    else
        print_error "objcopy/llvm-objcopy is required for aarch64"
        exit 1
    fi
    KERNEL_IMAGE="$KERNEL_BIN"
else
    KERNEL_IMAGE="$KERNEL_ELF"
fi

cleanup_qemu() {
    local pattern="${QEMU_BIN}.*file=${DISK_IMG}"
    pkill -TERM -f "$pattern" >/dev/null 2>&1 || true
    sleep 1
    pkill -KILL -f "$pattern" >/dev/null 2>&1 || true
}

print_info "running QEMU (timeout=${TIMEOUT_SEC}s)"
(
    "$QEMU_BIN" \
        "${QEMU_ARGS[@]}" \
        -drive "file=${DISK_IMG},format=raw,if=none,id=hd0" \
        -device virtio-blk-device,drive=hd0 \
        -kernel "$KERNEL_IMAGE" >"$RUN_LOG" 2>&1 &
    QEMU_PID=$!

    ELAPSED=0
    while kill -0 "$QEMU_PID" >/dev/null 2>&1; do
        if [[ "$ELAPSED" -ge "$TIMEOUT_SEC" ]]; then
            kill "$QEMU_PID" >/dev/null 2>&1 || true
            sleep 1
            kill -9 "$QEMU_PID" >/dev/null 2>&1 || true
            break
        fi
        sleep 1
        ELAPSED=$((ELAPSED + 1))
    done

    wait "$QEMU_PID" >/dev/null 2>&1 || true
)
cleanup_qemu

print_info "build log: $BUILD_LOG"
print_info "run log: $RUN_LOG"

check_fail=0

if ! grep -Eq "launched PID1 candidate '/mnt/(init|sbin/init)'" "$RUN_LOG"; then
    print_error "PID1 launch marker missing"
    check_fail=1
fi
if ! grep -q "PHASE15_1_BUSYBOX_SHELL_OK" "$RUN_LOG"; then
    print_error "busybox shell marker missing"
    check_fail=1
fi
if ! grep -q "PHASE15_1_BUSYBOX_EXEC_OK" "$RUN_LOG"; then
    print_error "busybox direct exec marker missing"
    check_fail=1
fi
if ! grep -q "PHASE15_1_HELLO_OK" "$RUN_LOG"; then
    print_error "hello direct exec marker missing"
    check_fail=1
fi
if ! grep -q "PHASE15_1_EXECVE_BOUNDS: PASS" "$RUN_LOG"; then
    print_error "execve boundary PASS marker missing"
    check_fail=1
fi
if grep -Eq "Kernel panic|Kernels panic" "$RUN_LOG"; then
    print_error "kernel panic detected"
    check_fail=1
fi
if grep -q "no executable init found, falling back to kernel shell" "$RUN_LOG"; then
    print_error "init fallback detected"
    check_fail=1
fi

if [[ "$check_fail" -ne 0 ]]; then
    print_error "Phase 15-1 verification FAILED"
    tail -n 120 "$RUN_LOG" || true
    exit 1
fi

print_info "Phase 15-1 verification PASSED ($ARCH)"
