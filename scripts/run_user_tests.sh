#!/bin/bash
# User-space test orchestration script.
#
# Usage:
#   ./scripts/run_user_tests.sh [ARCH] [TIMEOUT_SEC]
#
# ARCH:
#   - aarch64
#   - riscv64
#   - all (default)
#
# Current suite:
#   - Phase 15-3 dynamic C hello smoke (`verify_phase15_3_cdyn.sh`)
#   - Phase 15-3 dynamic busybox(init) smoke (`verify_phase15_3_busybox_dyn.sh`)
#   - Phase 15-4 internal minimal init smoke (`verify_phase15_4_internal_min.sh`)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-all}"
TIMEOUT_SEC="${2:-45}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[user-test]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[user-test]${NC} $1"; }
print_error() { echo -e "${RED}[user-test]${NC} $1"; }

run_one() {
    local arch="$1"
    print_info "running user-space suite for $arch"
    "$SCRIPT_DIR/verify_phase15_3_cdyn.sh" "$arch" "" "$TIMEOUT_SEC"
    "$SCRIPT_DIR/verify_phase15_3_busybox_dyn.sh" "$arch" "" "$TIMEOUT_SEC"
    "$SCRIPT_DIR/verify_phase15_4_internal_min.sh" "$arch" "" "$TIMEOUT_SEC"
}

case "$ARCH" in
    aarch64|riscv64)
        run_one "$ARCH"
        ;;
    all)
        run_one aarch64
        run_one riscv64
        ;;
    *)
        print_error "unsupported arch: $ARCH (expected: aarch64, riscv64, all)"
        exit 1
        ;;
esac

print_info "all user-space tests passed (arch=$ARCH)"
