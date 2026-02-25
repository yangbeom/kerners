#!/bin/bash
# Phase 15-3 dynamic busybox(init) boot + command execution verification.
#
# Usage:
#   ./scripts/verify_phase15_3_busybox_dyn.sh [ARCH] [BUSYBOX_STATIC_PATH] [TIMEOUT_SEC]
#
# Examples:
#   ./scripts/verify_phase15_3_busybox_dyn.sh aarch64
#   ./scripts/verify_phase15_3_busybox_dyn.sh riscv64
#   ./scripts/verify_phase15_3_busybox_dyn.sh all

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-all}"
TIMEOUT_SEC="${3:-45}"
STAMP="$(date +%Y%m%d-%H%M%S)"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[phase15-3-bbdyn]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[phase15-3-bbdyn]${NC} $1"; }
print_error() { echo -e "${RED}[phase15-3-bbdyn]${NC} $1"; }

run_one_arch() {
    local arch="$1"
    local busybox_static="$2"
    local ld_name=""

    case "$arch" in
        aarch64) ld_name="ld-kerners-aarch64.so" ;;
        riscv64) ld_name="ld-kerners-riscv64.so" ;;
        *) print_error "unsupported arch: $arch"; return 1 ;;
    esac

    if [[ ! -f "$busybox_static" ]]; then
        print_error "static busybox not found: $busybox_static"
        return 1
    fi

    print_info "build dynamic C bins ($arch)"
    "$SCRIPT_DIR/build_user_dynamic_c_bins.sh" "$arch"

    local out_dir="$PROJECT_ROOT/target/user/$arch"
    local busybox_dyn="$out_dir/busybox_dyn"
    local ld_bin="$out_dir/$ld_name"
    local disk_img="$PROJECT_ROOT/logs/phase15-3-bbdyn-${arch}-${STAMP}.img"
    local run_log="$PROJECT_ROOT/logs/phase15-3-bbdyn-${arch}-${STAMP}.log"

    if [[ ! -x "$busybox_dyn" || ! -x "$ld_bin" ]]; then
        print_error "dynamic busybox artifacts missing: $busybox_dyn / $ld_bin"
        return 1
    fi

    mkdir -p "$PROJECT_ROOT/logs"
    rm -f "$disk_img" "$run_log"

    print_info "prepare disk image ($arch): $disk_img"
    "$SCRIPT_DIR/prepare_user_disk.sh" "$arch" "$busybox_static" "$disk_img" >/dev/null

    mmd -i "$disk_img" ::/lib >/dev/null 2>&1 || true
    mcopy -o -i "$disk_img" "$busybox_dyn" ::/bin/busybox >/dev/null
    mcopy -o -i "$disk_img" "$busybox_dyn" ::/sbin/init >/dev/null
    mcopy -o -i "$disk_img" "$busybox_dyn" ::/bin/init >/dev/null
    mcopy -o -i "$disk_img" "$busybox_dyn" ::/bin/sh >/dev/null
    mcopy -o -i "$disk_img" "$ld_bin" "::/lib/$ld_name" >/dev/null

    print_info "boot and execute dynamic busybox init scenario ($arch)"
    KERNERS_DISK_IMG="$disk_img" \
    KERNERS_BOOTARGS="kerners.root=fat32" \
    "$PROJECT_ROOT/run.sh" "$arch" 512 1 >"$run_log" 2>&1 &
    local run_pid=$!

    local elapsed=0
    local stop_reason="timeout"
    while kill -0 "$run_pid" 2>/dev/null; do
        if rg -q "BBDYN_BOOT_END" "$run_log" 2>/dev/null; then
            stop_reason="done"
            break
        fi
        if rg -q "failed to start '/sbin/init'|failed to start '/bin/busybox'|Kernel panic|Kernels panic|panic|Unknown syscall|terminating by SIGSEGV" "$run_log" 2>/dev/null; then
            stop_reason="error"
            break
        fi
        if [[ "$elapsed" -ge "$TIMEOUT_SEC" ]]; then
            stop_reason="timeout"
            break
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    kill "$run_pid" >/dev/null 2>&1 || true
    wait "$run_pid" >/dev/null 2>&1 || true

    local failed=0
    local required_markers=(
        "BBDYN_BOOT_BEGIN"
        "BBDYN_CMD_ECHO_OK"
        "BBDYN_CMD_CAT_OK"
        "BBDYN_CMD_MKDIR_OK"
        "BBDYN_CMD_HEAD_OK"
        "BBDYN_CMD_PS_OK"
        "BBDYN_CMD_RM_OK"
        "BBDYN_CMD_RMDIR_OK"
        "BBDYN_BOOT_END"
    )

    local marker=""
    for marker in "${required_markers[@]}"; do
        if ! rg -q "$marker" "$run_log"; then
            print_error "marker missing: $marker ($arch)"
            failed=1
        fi
    done

    if rg -q "failed to start '/sbin/init'|failed to start '/bin/busybox'|Kernel panic|Kernels panic|Unknown syscall|terminating by SIGSEGV" "$run_log"; then
        print_error "fatal marker detected in log ($arch)"
        failed=1
    fi

    if [[ "$failed" -eq 0 ]]; then
        print_info "PASS ($arch): dynamic busybox init + commands executed"
        print_info "log: $run_log"
        print_info "disk: $disk_img"
        return 0
    fi

    print_warn "FAIL ($arch): stop_reason=$stop_reason"
    print_warn "log: $run_log"
    tail -n 150 "$run_log" || true
    return 1
}

main() {
    local failed=0

    case "$ARCH" in
        all)
            local aarch64_busybox="${2:-$PROJECT_ROOT/target/user/aarch64/busybox}"
            local riscv64_busybox="${2:-$PROJECT_ROOT/target/user/riscv64/busybox}"
            run_one_arch aarch64 "$aarch64_busybox" || failed=1
            run_one_arch riscv64 "$riscv64_busybox" || failed=1
            ;;
        aarch64|riscv64)
            local busybox_default="$PROJECT_ROOT/target/user/$ARCH/busybox"
            local busybox_path="${2:-$busybox_default}"
            run_one_arch "$ARCH" "$busybox_path" || failed=1
            ;;
        *)
            print_error "unsupported arch: $ARCH (expected: aarch64, riscv64, all)"
            exit 1
            ;;
    esac

    if [[ "$failed" -ne 0 ]]; then
        exit 1
    fi
}

main "$@"
