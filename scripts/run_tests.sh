#!/bin/bash
# 전체 테스트 오케스트레이션 스크립트
#
# Usage: ./scripts/run_tests.sh [ARCH] [TIMEOUT]
#   ARCH:    aarch64 (default) or riscv64
#   TIMEOUT: 초 단위 (default: 30)
#
# 과정:
# 1. 테스트 모듈 빌드
# 2. FAT32 디스크 이미지 준비
# 3. 커널 빌드 (--features test_runner)
# 4. QEMU 실행 → 출력 캡처 → 결과 파싱

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-aarch64}"
TIMEOUT="${2:-30}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[test]${NC} $1"; }
print_error() { echo -e "${RED}[test]${NC} $1"; }
print_pass() { echo -e "${GREEN}[test] ✓ ALL TESTS PASSED ($ARCH)${NC}"; }
print_fail() { echo -e "${RED}[test] ✗ TESTS FAILED ($ARCH)${NC}"; }

# 아키텍처별 설정
case "$ARCH" in
    aarch64)
        TARGET="aarch64-unknown-none-softfloat"
        QEMU="qemu-system-aarch64"
        QEMU_ARGS="-machine virt -cpu cortex-a57 -semihosting-config enable=on,target=native"
        ;;
    riscv64)
        TARGET="riscv64gc-unknown-none-elf"
        QEMU="qemu-system-riscv64"
        QEMU_ARGS="-machine virt -bios none"
        ;;
    *)
        print_error "Unknown architecture: $ARCH"
        exit 1
        ;;
esac

# QEMU 확인
if ! command -v "$QEMU" &>/dev/null; then
    print_error "$QEMU not found. Install QEMU first."
    exit 1
fi

cd "$PROJECT_ROOT"

# ---- Step 1: 테스트 모듈 빌드 ----
print_info "Building test modules for $ARCH..."
"$SCRIPT_DIR/build_test_modules.sh" "$ARCH"

# ---- Step 2: FAT32 디스크 이미지 준비 ----
print_info "Preparing test disk image..."
"$SCRIPT_DIR/prepare_test_disk.sh" "$ARCH"

# ---- Step 3: 커널 빌드 (test_runner feature) ----
print_info "Building kernel with test_runner for $ARCH..."
cargo build --release --target "$TARGET" --features test_runner

KERNEL_ELF="target/${TARGET}/release/kerners"

# aarch64: ELF → raw binary 변환
if [ "$ARCH" = "aarch64" ]; then
    KERNEL_BIN="target/${TARGET}/release/kerners_test.bin"
    ${LLVM_OBJCOPY:-llvm-objcopy} -O binary "$KERNEL_ELF" "$KERNEL_BIN" 2>/dev/null || \
    ${OBJCOPY:-objcopy} -O binary "$KERNEL_ELF" "$KERNEL_BIN" 2>/dev/null || {
        print_error "objcopy not found, using ELF directly"
        KERNEL="$KERNEL_ELF"
    }
    KERNEL="${KERNEL:-$KERNEL_BIN}"
else
    KERNEL="$KERNEL_ELF"
fi

# ---- Step 4: QEMU 실행 ----
DISK_IMG="$PROJECT_ROOT/disk_test.img"
VIRTIO_BLK=""
if [ -f "$DISK_IMG" ]; then
    VIRTIO_BLK="-drive file=$DISK_IMG,format=raw,if=none,id=hd0 -device virtio-blk-device,drive=hd0"
fi

print_info "Running QEMU ($ARCH, timeout=${TIMEOUT}s)..."
echo ""

# QEMU 실행 + timeout
set +e
if command -v gtimeout &>/dev/null; then
    TIMEOUT_CMD="gtimeout"
elif command -v timeout &>/dev/null; then
    TIMEOUT_CMD="timeout"
else
    # macOS fallback: 직접 timeout 구현
    TIMEOUT_CMD=""
fi

if [ -n "$TIMEOUT_CMD" ]; then
    OUTPUT=$($TIMEOUT_CMD "$TIMEOUT" $QEMU $QEMU_ARGS -m 512M -nographic $VIRTIO_BLK -kernel "$KERNEL" 2>&1)
    QEMU_EXIT=$?
else
    # timeout 명령 없으면 background + wait
    $QEMU $QEMU_ARGS -m 512M -nographic $VIRTIO_BLK -kernel "$KERNEL" > /tmp/kerners_test_output.txt 2>&1 &
    QEMU_PID=$!
    sleep "$TIMEOUT"
    if kill -0 "$QEMU_PID" 2>/dev/null; then
        kill "$QEMU_PID" 2>/dev/null
        wait "$QEMU_PID" 2>/dev/null
    fi
    OUTPUT=$(cat /tmp/kerners_test_output.txt)
    QEMU_EXIT=$?
fi
set -e

echo "$OUTPUT"
echo ""

# ---- Step 5: 결과 파싱 ----
if echo "$OUTPUT" | grep -q "TEST_STATUS: PASS"; then
    print_pass
    exit 0
elif echo "$OUTPUT" | grep -q "TEST_STATUS: FAIL"; then
    print_fail
    exit 1
else
    print_error "TEST TIMEOUT OR CRASH ($ARCH)"
    exit 2
fi
