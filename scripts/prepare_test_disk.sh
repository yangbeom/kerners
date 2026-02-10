#!/bin/bash
# FAT32 테스트 디스크 이미지 생성 + .ko 파일 복사
#
# Usage: ./scripts/prepare_test_disk.sh [ARCH]
#   ARCH: aarch64 (default) or riscv64
#
# 의존성: mtools (mcopy)
#   macOS: brew install mtools
#   Linux: apt install mtools

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-aarch64}"
DISK_IMG="$PROJECT_ROOT/disk_test.img"
DISK_SIZE=32  # MB
MODULE_DIR="$PROJECT_ROOT/target/modules/$ARCH"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
print_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# mtools 확인
if ! command -v mcopy &>/dev/null; then
    print_error "mtools not found. Install with:"
    echo "  macOS: brew install mtools"
    echo "  Linux: apt install mtools"
    exit 1
fi

# .ko 파일 확인
KO_FILES=("$MODULE_DIR"/test_*.ko)
if [ ! -f "${KO_FILES[0]}" ]; then
    print_error "No test modules found in $MODULE_DIR"
    print_error "Run ./scripts/build_test_modules.sh $ARCH first"
    exit 1
fi

# 디스크 이미지 생성 (항상 새로 생성)
print_info "Creating FAT32 disk image ($DISK_IMG, ${DISK_SIZE}MB)..."
dd if=/dev/zero of="$DISK_IMG" bs=1M count=$DISK_SIZE 2>/dev/null

# FAT32 포맷
if command -v mkfs.vfat &>/dev/null; then
    mkfs.vfat -F 32 "$DISK_IMG" >/dev/null 2>&1
elif command -v mformat &>/dev/null; then
    # macOS: mtools의 mformat 사용 (newfs_msdos는 raw 파일 미지원)
    mformat -i "$DISK_IMG" -F ::
else
    print_error "Cannot format disk image (no mkfs.vfat or mformat)"
    exit 1
fi

# .ko 파일을 FAT32 이미지에 복사
print_info "Copying test modules to disk image..."
for ko in "${KO_FILES[@]}"; do
    fname=$(basename "$ko")
    mcopy -i "$DISK_IMG" "$ko" "::$fname"
    print_info "  → $fname"
done

# 확인
print_info "Disk image contents:"
mdir -i "$DISK_IMG" :: 2>/dev/null || true

print_info "Test disk ready: $DISK_IMG"
