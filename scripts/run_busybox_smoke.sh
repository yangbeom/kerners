#!/bin/bash
# BusyBox init 부팅 스모크 테스트
#
# Usage:
#   ./scripts/run_busybox_smoke.sh [ARCH] [BUSYBOX_PATH] [RUNS] [TIMEOUT_SEC]
#
# Examples:
#   ./scripts/run_busybox_smoke.sh aarch64 /abs/path/to/busybox 3 30
#   KERNERS_BUSYBOX=/abs/path/to/busybox ./scripts/run_busybox_smoke.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-aarch64}"
BUSYBOX_PATH="${2:-${KERNERS_BUSYBOX:-}}"
RUNS="${3:-3}"
TIMEOUT_SEC="${4:-30}"
LOG_DIR="$PROJECT_ROOT/logs"
REQUIRE_COW="${BUSYBOX_SMOKE_REQUIRE_COW:-0}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[busybox-smoke]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[busybox-smoke]${NC} $1"; }
print_error() { echo -e "${RED}[busybox-smoke]${NC} $1"; }

if [[ -z "$BUSYBOX_PATH" ]]; then
    print_error "BusyBox path is required"
    echo "Usage: $0 [ARCH] [BUSYBOX_PATH] [RUNS] [TIMEOUT_SEC]"
    exit 1
fi

if [[ ! -f "$BUSYBOX_PATH" ]]; then
    print_error "BusyBox not found: $BUSYBOX_PATH"
    exit 1
fi

if [[ "$RUNS" -lt 1 ]]; then
    print_error "RUNS must be >= 1"
    exit 1
fi

mkdir -p "$LOG_DIR"
STAMP="$(date +%Y%m%d-%H%M%S)"
SUMMARY_LOG="$LOG_DIR/busybox-init-${ARCH}-${STAMP}.summary.log"

if command -v gtimeout >/dev/null 2>&1; then
    TIMEOUT_CMD="gtimeout"
elif command -v timeout >/dev/null 2>&1; then
    TIMEOUT_CMD="timeout"
else
    TIMEOUT_CMD=""
fi

PASS_COUNT=0
FAIL_COUNT=0

cleanup_qemu_for_disk() {
    local disk_path="$1"
    local pattern="qemu-system-${ARCH}.*file=${disk_path}"
    pkill -TERM -f "$pattern" >/dev/null 2>&1 || true
    sleep 1
    pkill -KILL -f "$pattern" >/dev/null 2>&1 || true
}

{
    echo "# BusyBox init smoke summary"
    echo "# date: $(date '+%Y-%m-%d %H:%M:%S')"
    echo "# arch: $ARCH"
    echo "# busybox: $BUSYBOX_PATH"
    echo "# runs: $RUNS"
    echo "# timeout_sec: $TIMEOUT_SEC"
    echo ""
} >"$SUMMARY_LOG"

for i in $(seq 1 "$RUNS"); do
    RUN_LOG="$LOG_DIR/busybox-init-${ARCH}-${STAMP}-run${i}.log"
    RUN_DISK="$LOG_DIR/busybox-init-${ARCH}-${STAMP}-run${i}.img"
    print_info "run ${i}/${RUNS}: capturing log -> $RUN_LOG"
    rm -f "$RUN_DISK"

    set +e
    if [[ -n "$TIMEOUT_CMD" ]]; then
        KERNERS_BUSYBOX="$BUSYBOX_PATH" KERNERS_DISK_IMG="$RUN_DISK" \
            "$TIMEOUT_CMD" "$TIMEOUT_SEC" \
            "$PROJECT_ROOT/run.sh" "$ARCH" 512 1 >"$RUN_LOG" 2>&1
        RUN_EXIT=$?
    else
        KERNERS_BUSYBOX="$BUSYBOX_PATH" KERNERS_DISK_IMG="$RUN_DISK" \
            "$PROJECT_ROOT/run.sh" "$ARCH" 512 1 >"$RUN_LOG" 2>&1 &
        RUN_PID=$!
        ELAPSED=0
        while kill -0 "$RUN_PID" 2>/dev/null; do
            if [[ "$ELAPSED" -ge "$TIMEOUT_SEC" ]]; then
                kill "$RUN_PID" 2>/dev/null || true
                sleep 1
                kill -9 "$RUN_PID" 2>/dev/null || true
                break
            fi
            sleep 1
            ELAPSED=$((ELAPSED + 1))
        done
        wait "$RUN_PID" 2>/dev/null
        RUN_EXIT=$?
    fi
    cleanup_qemu_for_disk "$RUN_DISK"
    rm -f "$RUN_DISK"
    set -e

    RUN_STATUS="FAIL"
    REASONS=()

    COW_FORK_MARKER="N/A"
    COW_FORK_OK=1
    if [[ "$REQUIRE_COW" == "1" && ( "$ARCH" == "aarch64" || "$ARCH" == "riscv64" ) ]]; then
        if grep -q "COW_FORK_TEST: PASS" "$RUN_LOG"; then
            COW_FORK_MARKER="PASS"
        else
            COW_FORK_MARKER="MISSING"
            COW_FORK_OK=0
        fi
    fi

    if grep -q "launched PID1 candidate" "$RUN_LOG" && \
       ! grep -q "no executable init found, falling back to kernel shell" "$RUN_LOG" && \
       ! grep -q "Process 1 exiting with status" "$RUN_LOG" && \
       ! grep -Eq "Kernel panic|Kernels panic" "$RUN_LOG" && \
       [[ "$COW_FORK_OK" -eq 1 ]]; then
        RUN_STATUS="PASS"
    fi

    if grep -q "Unknown syscall:" "$RUN_LOG"; then
        REASONS+=("ENOSYS")
    fi
    # Use word match to avoid false positives from strings like "DEFAULT".
    if grep -wq "EFAULT" "$RUN_LOG"; then
        REASONS+=("EFAULT")
    fi
    if grep -q "failed to start '" "$RUN_LOG"; then
        REASONS+=("EXEC_FAIL")
    fi
    if grep -q "no executable init found, falling back to kernel shell" "$RUN_LOG"; then
        REASONS+=("NO_INIT_FALLBACK")
    fi
    if grep -q "Failed to get \"write\" lock" "$RUN_LOG"; then
        REASONS+=("QEMU_LOCK")
    fi
    if grep -Eq "Kernel panic|Kernels panic" "$RUN_LOG"; then
        REASONS+=("PANIC")
    fi
    if [[ "$REQUIRE_COW" == "1" && ("$ARCH" == "aarch64" || "$ARCH" == "riscv64") && "$COW_FORK_MARKER" == "MISSING" ]]; then
        REASONS+=("COW_FORK_MISSING")
    fi
    if [[ "$RUN_EXIT" -eq 124 ]]; then
        REASONS+=("TIMEOUT")
    fi
    if [[ "${#REASONS[@]}" -eq 0 && "$RUN_STATUS" == "FAIL" ]]; then
        REASONS+=("UNKNOWN")
    fi

    if [[ "$RUN_STATUS" == "PASS" ]]; then
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi

    if [[ "${#REASONS[@]}" -gt 0 ]]; then
        REASON_STR="$(IFS=,; echo "${REASONS[*]}")"
    else
        REASON_STR=""
    fi
    {
        echo "run=$i status=$RUN_STATUS exit=$RUN_EXIT cow_fork=$COW_FORK_MARKER reasons=$REASON_STR"
        echo "log=$RUN_LOG"
        echo ""
    } >>"$SUMMARY_LOG"

    if [[ "$RUN_STATUS" == "PASS" ]]; then
        print_info "run $i PASS (exit=$RUN_EXIT)"
    else
        print_warn "run $i FAIL (exit=$RUN_EXIT, reasons=$REASON_STR)"
    fi
done

{
    echo "total_pass=$PASS_COUNT"
    echo "total_fail=$FAIL_COUNT"
} >>"$SUMMARY_LOG"

print_info "summary log: $SUMMARY_LOG"
print_info "result: PASS=$PASS_COUNT FAIL=$FAIL_COUNT"

if [[ "$FAIL_COUNT" -gt 0 ]]; then
    exit 1
fi
