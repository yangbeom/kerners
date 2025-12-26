#!/bin/bash
# 모듈 빌드 스크립트

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
MODULE_DIR="$SCRIPT_DIR"
MODULE_NAME="hello_module"

# 기본값
ARCH="${1:-aarch64}"

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

print_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# 아키텍처별 설정
case "$ARCH" in
    aarch64)
        TARGET="aarch64-unknown-none-softfloat"
        ;;
    riscv64)
        TARGET="riscv64gc-unknown-none-elf"
        ;;
    *)
        print_error "Unknown architecture: $ARCH (use aarch64 or riscv64)"
        exit 1
        ;;
esac

OUTPUT_DIR="$PROJECT_ROOT/target/modules/$ARCH"

print_info "Building module '$MODULE_NAME' for $ARCH..."

# 출력 디렉토리 생성
mkdir -p "$OUTPUT_DIR"

# 모듈 빌드 - rustc를 직접 사용하여 .o 파일 생성
cd "$MODULE_DIR"

# Rust 소스를 컴파일하여 object 파일 직접 생성
rustc --edition 2024 \
    --crate-type cdylib \
    --emit=obj \
    --target "$TARGET" \
    -C relocation-model=pic \
    -C panic=abort \
    -C opt-level=s \
    -o "$OUTPUT_DIR/${MODULE_NAME}.o" \
    src/lib.rs

if [[ -f "$OUTPUT_DIR/${MODULE_NAME}.o" ]]; then
    # .o 파일을 .ko로 복사 (relocatable object 그대로 사용)
    cp "$OUTPUT_DIR/${MODULE_NAME}.o" "$OUTPUT_DIR/${MODULE_NAME}.ko"
    
    print_info "Module built: $OUTPUT_DIR/${MODULE_NAME}.ko"
    
    # 모듈 정보 출력
    llvm-readelf -h -s "$OUTPUT_DIR/${MODULE_NAME}.ko" 2>/dev/null | head -40 || true
else
    print_error "Failed to build module object file"
    exit 1
fi

print_info "Module build complete!"
