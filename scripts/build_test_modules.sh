#!/bin/bash
# 테스트 모듈 일괄 빌드 스크립트
#
# Usage: ./scripts/build_test_modules.sh [ARCH]
#   ARCH: aarch64 (default) or riscv64

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-aarch64}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
print_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# 아키텍처별 타겟
case "$ARCH" in
    aarch64) TARGET="aarch64-unknown-none-softfloat" ;;
    riscv64) TARGET="riscv64gc-unknown-none-elf" ;;
    *) print_error "Unknown architecture: $ARCH"; exit 1 ;;
esac

OUTPUT_DIR="$PROJECT_ROOT/target/modules/$ARCH"
mkdir -p "$OUTPUT_DIR"

# modules/test_* 디렉토리를 순회하며 빌드
BUILT=0
FAILED=0

for module_dir in "$PROJECT_ROOT"/modules/test_*/; do
    [ -d "$module_dir" ] || continue
    [ -f "$module_dir/src/lib.rs" ] || continue

    module_name=$(basename "$module_dir")

    print_info "Building $module_name for $ARCH..."

    cd "$module_dir"

    if rustc --edition 2024 \
        --crate-type cdylib \
        --emit=obj \
        --target "$TARGET" \
        -C relocation-model=pic \
        -C panic=abort \
        -C opt-level=s \
        -o "$OUTPUT_DIR/${module_name}.o" \
        src/lib.rs 2>&1; then

        cp "$OUTPUT_DIR/${module_name}.o" "$OUTPUT_DIR/${module_name}.ko"
        print_info "  → $OUTPUT_DIR/${module_name}.ko"
        BUILT=$((BUILT + 1))
    else
        print_error "  Failed to build $module_name"
        FAILED=$((FAILED + 1))
    fi
done

cd "$PROJECT_ROOT"

echo ""
print_info "Build complete: $BUILT succeeded, $FAILED failed"

if [ "$FAILED" -gt 0 ]; then
    exit 1
fi
