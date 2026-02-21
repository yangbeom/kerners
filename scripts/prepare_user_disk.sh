#!/bin/bash
# BusyBox 기반 유저 디스크 이미지 생성 스크립트
#
# Usage: ./scripts/prepare_user_disk.sh [ARCH] [BUSYBOX_PATH] [DISK_IMG]
#   ARCH: aarch64 (default) or riscv64 (정보 출력용)
#   BUSYBOX_PATH: prebuilt static busybox ELF 경로 (필수)
#   DISK_IMG: 출력 디스크 이미지 경로
#             (default: $KERNERS_DISK_IMG or ./disk.img)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-aarch64}"
BUSYBOX_PATH="${2:-}"
DEFAULT_DISK_IMG="${KERNERS_DISK_IMG:-$PROJECT_ROOT/disk.img}"
DISK_IMG="${3:-$DEFAULT_DISK_IMG}"
DISK_SIZE_MB="${DISK_SIZE_MB:-64}"
BUSYBOX_COPY_COUNT=7
DISK_OVERHEAD_MB=32

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
print_error() { echo -e "${RED}[ERROR]${NC} $1"; }

if [[ -z "$BUSYBOX_PATH" ]]; then
    print_error "Missing BUSYBOX_PATH"
    echo "Usage: $0 [ARCH] [BUSYBOX_PATH] [DISK_IMG]"
    exit 1
fi

if [[ ! -f "$BUSYBOX_PATH" ]]; then
    print_error "BusyBox not found: $BUSYBOX_PATH"
    exit 1
fi

if ! command -v mcopy >/dev/null 2>&1 || ! command -v mmd >/dev/null 2>&1; then
    print_error "mtools (mcopy/mmd) is required"
    echo "  macOS: brew install mtools"
    echo "  Linux: apt install mtools"
    exit 1
fi

if command -v file >/dev/null 2>&1; then
    FILE_DESC="$(file "$BUSYBOX_PATH")"
    print_info "BusyBox file: $FILE_DESC"
    if [[ "$FILE_DESC" != *"ELF"* ]]; then
        print_warn "The file does not look like an ELF binary"
    fi
    if [[ "$FILE_DESC" != *"statically linked"* ]]; then
        print_warn "BusyBox is not reported as statically linked (dynamic ELF is not supported yet)"
    fi
fi

busybox_size_bytes=0
if command -v stat >/dev/null 2>&1; then
    if busybox_size_bytes="$(stat -f %z "$BUSYBOX_PATH" 2>/dev/null)"; then
        :
    elif busybox_size_bytes="$(stat -c %s "$BUSYBOX_PATH" 2>/dev/null)"; then
        :
    else
        busybox_size_bytes=0
    fi
fi

if [[ "$busybox_size_bytes" =~ ^[0-9]+$ ]] && [[ "$busybox_size_bytes" -gt 0 ]]; then
    required_bytes=$((busybox_size_bytes * BUSYBOX_COPY_COUNT))
    required_bytes=$((required_bytes + DISK_OVERHEAD_MB * 1024 * 1024))
    required_mb=$(((required_bytes + 1024 * 1024 - 1) / (1024 * 1024)))
    if [[ "$required_mb" -gt "$DISK_SIZE_MB" ]]; then
        print_info "Auto-adjusting disk size: ${DISK_SIZE_MB}MB -> ${required_mb}MB"
        DISK_SIZE_MB="$required_mb"
    fi
fi

print_info "Creating FAT32 user disk image: $DISK_IMG (${DISK_SIZE_MB}MB, arch=$ARCH)"
dd if=/dev/zero of="$DISK_IMG" bs=1M count="$DISK_SIZE_MB" 2>/dev/null

formatted=0
if command -v mkfs.vfat >/dev/null 2>&1; then
    if mkfs.vfat -F 32 "$DISK_IMG" >/dev/null 2>&1; then
        formatted=1
    fi
fi

# macOS에서는 mformat이 raw image에 대해 더 안정적이다.
if [[ "$formatted" -eq 0 ]] && command -v mformat >/dev/null 2>&1; then
    if mformat -i "$DISK_IMG" -F :: >/dev/null 2>&1; then
        formatted=1
    fi
fi

if [[ "$formatted" -eq 0 ]] && command -v newfs_msdos >/dev/null 2>&1; then
    if newfs_msdos -F 32 "$DISK_IMG" >/dev/null 2>&1; then
        formatted=1
    fi
fi

if [[ "$formatted" -eq 0 ]]; then
    print_error "Cannot format disk image (no mkfs.vfat/newfs_msdos/mformat)"
    exit 1
fi

# 디렉토리 구성
mmd -i "$DISK_IMG" ::/bin >/dev/null 2>&1 || true
mmd -i "$DISK_IMG" ::/sbin >/dev/null 2>&1 || true
mmd -i "$DISK_IMG" ::/etc >/dev/null 2>&1 || true
mmd -i "$DISK_IMG" ::/usr >/dev/null 2>&1 || true
mmd -i "$DISK_IMG" ::/usr/bin >/dev/null 2>&1 || true
mmd -i "$DISK_IMG" ::/usr/sbin >/dev/null 2>&1 || true

# BusyBox 엔트리 복사 (symlink 미지원 환경을 고려해 복제)
mcopy -o -i "$DISK_IMG" "$BUSYBOX_PATH" ::/bin/busybox
mcopy -o -i "$DISK_IMG" "$BUSYBOX_PATH" ::/init
mcopy -o -i "$DISK_IMG" "$BUSYBOX_PATH" ::/sbin/init
mcopy -o -i "$DISK_IMG" "$BUSYBOX_PATH" ::/bin/init
mcopy -o -i "$DISK_IMG" "$BUSYBOX_PATH" ::/bin/sh
mcopy -o -i "$DISK_IMG" "$BUSYBOX_PATH" ::/usr/bin/busybox
mcopy -o -i "$DISK_IMG" "$BUSYBOX_PATH" ::/usr/bin/sh

print_info "Installed BusyBox entries:"
print_info "  /bin/busybox"
print_info "  /init"
print_info "  /sbin/init"
print_info "  /bin/init"
print_info "  /bin/sh"
print_info "  /usr/bin/busybox"
print_info "  /usr/bin/sh"

print_info "Disk image contents (/):"
mdir -i "$DISK_IMG" :: 2>/dev/null || true
print_info "Disk image contents (/bin):"
mdir -i "$DISK_IMG" ::/bin 2>/dev/null || true
print_info "Disk image contents (/sbin):"
mdir -i "$DISK_IMG" ::/sbin 2>/dev/null || true
print_info "Disk image contents (/usr/bin):"
mdir -i "$DISK_IMG" ::/usr/bin 2>/dev/null || true

print_info "User disk ready: $DISK_IMG"
