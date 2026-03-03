#!/bin/bash
# Phase 15-5 TLS smoke verification.
#
# Usage:
#   ./scripts/verify_phase15_5_tls_smoke.sh [ARCH] [BUSYBOX_STATIC_PATH] [TIMEOUT_SEC]
#
# Examples:
#   ./scripts/verify_phase15_5_tls_smoke.sh aarch64
#   ./scripts/verify_phase15_5_tls_smoke.sh riscv64
#   ./scripts/verify_phase15_5_tls_smoke.sh all

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-all}"
TIMEOUT_SEC="${3:-60}"
STAMP="$(date +%Y%m%d-%H%M%S)"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[phase15-5-tls]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[phase15-5-tls]${NC} $1"; }
print_error() { echo -e "${RED}[phase15-5-tls]${NC} $1"; }

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

    print_info "build minilibc samples ($arch)"
    "$SCRIPT_DIR/build_user_minilibc_samples.sh" "$arch"

    local out_dir="$PROJECT_ROOT/target/user/$arch"
    local tls_bin="$out_dir/sample_tls_smoke_dyn"
    local tls_ie_bin="$out_dir/sample_tls_ie_smoke_dyn"
    local tls_desc_bin="$out_dir/sample_tls_desc_smoke_dyn"
    local tls_ie_lib="$out_dir/libtls_ie.so"
    local ld_bin="$out_dir/$ld_name"
    local disk_img="$PROJECT_ROOT/logs/phase15-5-tls-${arch}-${STAMP}.img"
    local run_log="$PROJECT_ROOT/logs/phase15-5-tls-${arch}-${STAMP}.log"
    local rcs_file="/tmp/phase15-5-tls-rcS-${arch}-${STAMP}"

    if [[ ! -x "$tls_bin" || ! -x "$tls_ie_bin" || ! -x "$tls_desc_bin" || ! -f "$tls_ie_lib" || ! -x "$ld_bin" ]]; then
        print_error "tls sample artifacts missing: $tls_bin / $tls_ie_bin / $tls_desc_bin / $tls_ie_lib / $ld_bin"
        return 1
    fi

    mkdir -p "$PROJECT_ROOT/logs"
    rm -f "$disk_img" "$run_log"

    print_info "prepare disk image ($arch): $disk_img"
    "$SCRIPT_DIR/prepare_user_disk.sh" "$arch" "$busybox_static" "$disk_img" >/dev/null

    mmd -i "$disk_img" ::/etc/init.d >/dev/null 2>&1 || true
    mmd -i "$disk_img" ::/lib >/dev/null 2>&1 || true

cat >"$rcs_file" <<'EOF'
#!/bin/sh
echo PH15_5_TLS_BEGIN
/bin/sample_tls_smoke_dyn
echo PH15_5_TLS_LE_RC=$?
/bin/sample_tls_ie_smoke_dyn
echo PH15_5_TLS_IE_RC=$?
/bin/sample_tls_desc_smoke_dyn
echo PH15_5_TLS_DESC_RC=$?
echo PH15_5_TLS_END
EOF

    mcopy -o -i "$disk_img" "$rcs_file" ::/etc/init.d/rcS >/dev/null
    mcopy -o -i "$disk_img" "$tls_bin" ::/bin/sample_tls_smoke_dyn >/dev/null
    mcopy -o -i "$disk_img" "$tls_ie_bin" ::/bin/sample_tls_ie_smoke_dyn >/dev/null
    mcopy -o -i "$disk_img" "$tls_desc_bin" ::/bin/sample_tls_desc_smoke_dyn >/dev/null
    mcopy -o -i "$disk_img" "$tls_ie_lib" ::/lib/libtls_ie.so >/dev/null
    mcopy -o -i "$disk_img" "$ld_bin" "::/lib/$ld_name" >/dev/null
    rm -f "$rcs_file"

    print_info "boot and run TLS smoke ($arch)"
    KERNERS_DISK_IMG="$disk_img" \
    KERNERS_BOOTARGS="kerners.root=fat32" \
    "$PROJECT_ROOT/run.sh" "$arch" 512 1 >"$run_log" 2>&1 &
    local run_pid=$!

    local elapsed=0
    local stop_reason="timeout"
    while kill -0 "$run_pid" 2>/dev/null; do
        if rg -q "PH15_5_TLS_END" "$run_log" 2>/dev/null; then
            stop_reason="done"
            break
        fi
        if rg -q "TLS_SMOKE_FAIL_|TLS_IE_SMOKE_FAIL_|TLS_DESC_SMOKE_FAIL_|failed to start '/bin/sample_tls_smoke_dyn'|failed to start '/bin/sample_tls_ie_smoke_dyn'|failed to start '/bin/sample_tls_desc_smoke_dyn'|Kernel panic|Kernels panic|panic|Unknown syscall|terminating by SIGSEGV" "$run_log" 2>/dev/null; then
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

    local phase_section_log
    phase_section_log="$(mktemp "${TMPDIR:-/tmp}/kerners-phase15-5-section.XXXXXX")"
    awk '
        /PH15_5_TLS_BEGIN/ { capture = 1 }
        capture { print }
        /PH15_5_TLS_END/ && capture { exit }
    ' "$run_log" >"$phase_section_log"

    local failed=0
    local required_markers=(
        "PH15_5_TLS_BEGIN"
        "TLS_SMOKE_BEGIN"
        "TLS_SMOKE_MT_OK"
        "TLS_SMOKE_OK"
        "PH15_5_TLS_LE_RC=0"
        "TLS_IE_SMOKE_BEGIN"
        "TLS_IE_SMOKE_MT_OK"
        "TLS_IE_SMOKE_OK"
        "PH15_5_TLS_IE_RC=0"
        "TLS_DESC_SMOKE_BEGIN"
        "TLS_DESC_SMOKE_MT_OK"
        "TLS_DESC_SMOKE_OK"
        "PH15_5_TLS_DESC_RC=0"
        "PH15_5_TLS_END"
    )

    local marker=""
    for marker in "${required_markers[@]}"; do
        if ! rg -q "$marker" "$phase_section_log"; then
            print_error "marker missing: $marker ($arch)"
            failed=1
        fi
    done

    if rg -q "TLS_SMOKE_FAIL_|TLS_IE_SMOKE_FAIL_|TLS_DESC_SMOKE_FAIL_|failed to start '/bin/sample_tls_smoke_dyn'|failed to start '/bin/sample_tls_ie_smoke_dyn'|failed to start '/bin/sample_tls_desc_smoke_dyn'|Kernel panic|Kernels panic|Unknown syscall|terminating by SIGSEGV" "$phase_section_log"; then
        print_error "fatal marker detected in log ($arch)"
        failed=1
    fi

    rm -f "$phase_section_log"

    if [[ "$failed" -eq 0 ]]; then
        print_info "PASS ($arch): TLS smoke executed"
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
