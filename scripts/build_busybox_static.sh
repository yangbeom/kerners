#!/bin/bash
# BusyBox static ELF 빌드 스크립트 (zig 기반 크로스 컴파일)
#
# Usage:
#   ./scripts/build_busybox_static.sh [ARCH] [BUSYBOX_SRC] [OUTPUT_BIN]
#
# Examples:
#   ./scripts/build_busybox_static.sh aarch64 /Users/yangbeom/github/busybox
#   ./scripts/build_busybox_static.sh riscv64 /Users/yangbeom/github/busybox ./out/busybox-rv64

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-aarch64}"
BUSYBOX_SRC="${2:-/Users/yangbeom/github/busybox}"
OUTPUT_BIN="${3:-$PROJECT_ROOT/target/user/$ARCH/busybox}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[busybox-build]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[busybox-build]${NC} $1"; }
print_error() { echo -e "${RED}[busybox-build]${NC} $1"; }

if [[ ! -d "$BUSYBOX_SRC" ]]; then
    print_error "BusyBox source not found: $BUSYBOX_SRC"
    exit 1
fi

if ! command -v zig >/dev/null 2>&1; then
    print_error "zig is required (not found in PATH)"
    exit 1
fi

case "$ARCH" in
    aarch64)
        BB_ARCH="arm64"
        ZIG_TARGET="aarch64-linux-musl"
        TOOL_PREFIX="aarch64-linux-musl-"
        FILE_ARCH_KEY="ARM aarch64"
        ;;
    riscv64)
        BB_ARCH="riscv"
        ZIG_TARGET="riscv64-linux-musl"
        TOOL_PREFIX="riscv64-linux-musl-"
        FILE_ARCH_KEY="UCB RISC-V"
        ;;
    *)
        print_error "Unsupported arch: $ARCH (expected: aarch64 or riscv64)"
        exit 1
        ;;
esac

CPU_COUNT="$(
    (
        getconf _NPROCESSORS_ONLN 2>/dev/null ||
        sysctl -n hw.ncpu 2>/dev/null ||
        echo 4
    ) | head -n1
)"

BUILD_ROOT="$PROJECT_ROOT/target/busybox-build/$ARCH"
WORK_DIR="$BUILD_ROOT/work"
WRAP_DIR="$BUILD_ROOT/toolchain/bin"
WRAP_PREFIX="$WRAP_DIR/$TOOL_PREFIX"
CACHE_LOCAL="$BUILD_ROOT/zig-cache/local"
CACHE_GLOBAL="$BUILD_ROOT/zig-cache/global"

mkdir -p "$WRAP_DIR"
mkdir -p "$CACHE_LOCAL" "$CACHE_GLOBAL"
mkdir -p "$(dirname "$OUTPUT_BIN")"

# zig 기본 캐시(~/.cache/zig)는 샌드박스에서 권한 문제가 날 수 있으므로 워크스페이스 내로 고정
export ZIG_LOCAL_CACHE_DIR="$CACHE_LOCAL"
export ZIG_GLOBAL_CACHE_DIR="$CACHE_GLOBAL"

find_llvm_tool() {
    local tool="$1"
    local candidates=(
        "/opt/homebrew/opt/llvm/bin/llvm-$tool"
        "$(command -v llvm-$tool 2>/dev/null || true)"
        "$(command -v $tool 2>/dev/null || true)"
    )

    local cand
    for cand in "${candidates[@]}"; do
        if [[ -n "$cand" && -x "$cand" ]]; then
            echo "$cand"
            return 0
        fi
    done
    return 1
}

write_wrapper() {
    local name="$1"
    local body="$2"
    local path="$WRAP_DIR/${TOOL_PREFIX}${name}"
    cat >"$path" <<EOF
#!/bin/bash
set -e
$body
EOF
    chmod +x "$path"
}

LLVM_NM="$(find_llvm_tool nm || true)"
LLVM_STRIP="$(find_llvm_tool strip || true)"
LLVM_OBJCOPY="$(find_llvm_tool objcopy || true)"
LLVM_OBJDUMP="$(find_llvm_tool objdump || true)"

if [[ -z "$LLVM_NM" || -z "$LLVM_STRIP" || -z "$LLVM_OBJCOPY" || -z "$LLVM_OBJDUMP" ]]; then
    print_error "required llvm/binutils tools not found (nm/strip/objcopy/objdump)"
    exit 1
fi

# zig cc가 일부 GNU ld 플래그(--warn-common)를 거부하므로 래퍼에서 필터링
cat >"$WRAP_DIR/${TOOL_PREFIX}gcc" <<EOF
#!/bin/bash
set -e
args=()
for arg in "\$@"; do
    case "\$arg" in
        -Wl,--warn-common|--warn-common|-Wl,-Map,*|-Wl,--verbose)
            continue
            ;;
    esac
    args+=("\$arg")
done
exec zig cc -target $ZIG_TARGET "\${args[@]}"
EOF
chmod +x "$WRAP_DIR/${TOOL_PREFIX}gcc"

cat >"$WRAP_DIR/${TOOL_PREFIX}g++" <<EOF
#!/bin/bash
set -e
args=()
for arg in "\$@"; do
    case "\$arg" in
        -Wl,--warn-common|--warn-common|-Wl,-Map,*|-Wl,--verbose)
            continue
            ;;
    esac
    args+=("\$arg")
done
exec zig c++ -target $ZIG_TARGET "\${args[@]}"
EOF
chmod +x "$WRAP_DIR/${TOOL_PREFIX}g++"

write_wrapper "cpp" "exec zig cc -E -target $ZIG_TARGET \"\$@\""
write_wrapper "ar" "exec zig ar \"\$@\""
write_wrapper "ranlib" "exec zig ranlib \"\$@\""
write_wrapper "ld" "exec zig cc -target $ZIG_TARGET \"\$@\""
write_wrapper "nm" "exec \"$LLVM_NM\" \"\$@\""
write_wrapper "strip" "exec \"$LLVM_STRIP\" \"\$@\""
write_wrapper "objcopy" "exec \"$LLVM_OBJCOPY\" \"\$@\""
write_wrapper "objdump" "exec \"$LLVM_OBJDUMP\" \"\$@\""

print_info "source: $BUSYBOX_SRC"
print_info "arch: $ARCH (busybox ARCH=$BB_ARCH, zig target=$ZIG_TARGET)"
print_info "build dir: $WORK_DIR"
print_info "output: $OUTPUT_BIN"

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR"

make -C "$BUSYBOX_SRC" O="$WORK_DIR" ARCH="$BB_ARCH" CROSS_COMPILE="$WRAP_PREFIX" defconfig >/dev/null

if [[ ! -f "$WORK_DIR/.config" ]]; then
    print_error "busybox .config was not generated"
    exit 1
fi

set_bool_config() {
    local key="$1"
    local value="$2" # y|n
    if [[ "$value" == "y" ]]; then
        if grep -q "^# $key is not set" "$WORK_DIR/.config"; then
            sed -i.bak "s/^# $key is not set/$key=y/" "$WORK_DIR/.config"
        elif grep -q "^$key=" "$WORK_DIR/.config"; then
            sed -i.bak "s/^$key=.*/$key=y/" "$WORK_DIR/.config"
        else
            echo "$key=y" >> "$WORK_DIR/.config"
        fi
    else
        if grep -q "^$key=" "$WORK_DIR/.config"; then
            sed -i.bak "s/^$key=.*/# $key is not set/" "$WORK_DIR/.config"
        elif ! grep -q "^# $key is not set" "$WORK_DIR/.config"; then
            echo "# $key is not set" >> "$WORK_DIR/.config"
        fi
    fi
}

# static + non-PIE(ET_EXEC) 중심으로 고정
set_bool_config "CONFIG_STATIC" "y"
set_bool_config "CONFIG_PIE" "n"

# zig + musl cross 빌드에서 충돌이 잦은 항목 비활성화
set_bool_config "CONFIG_TC" "n"
set_bool_config "CONFIG_SHA1_HWACCEL" "n"
set_bool_config "CONFIG_SHA256_HWACCEL" "n"
set_bool_config "CONFIG_SHA1_HWACCEL_X86_SHA_NI" "n"
set_bool_config "CONFIG_SHA256_HWACCEL_X86" "n"
set_bool_config "CONFIG_SHA256_HWACCEL_X86_SSE4" "n"

rm -f "$WORK_DIR/.config.bak"

# BusyBox는 olddefconfig 타겟이 없을 수 있으므로 oldconfig를 non-interactive로 수행
set +o pipefail
yes "" | make -C "$BUSYBOX_SRC" O="$WORK_DIR" ARCH="$BB_ARCH" CROSS_COMPILE="$WRAP_PREFIX" oldconfig >/dev/null
set -o pipefail
make -C "$BUSYBOX_SRC" O="$WORK_DIR" ARCH="$BB_ARCH" CROSS_COMPILE="$WRAP_PREFIX" \
    CFLAGS="-static -no-pie" LDFLAGS="-static -no-pie" -j"$CPU_COUNT"

if [[ ! -f "$WORK_DIR/busybox" ]]; then
    print_error "build output not found: $WORK_DIR/busybox"
    exit 1
fi

cp "$WORK_DIR/busybox" "$OUTPUT_BIN"
chmod +x "$OUTPUT_BIN"

FILE_INFO="$(file "$OUTPUT_BIN")"
print_info "file: $FILE_INFO"

if [[ "$FILE_INFO" != *"ELF 64-bit"* ]]; then
    print_error "output is not ELF64"
    exit 1
fi

if [[ "$FILE_INFO" != *"$FILE_ARCH_KEY"* ]]; then
    print_error "unexpected ELF architecture"
    exit 1
fi

if [[ "$FILE_INFO" != *"statically linked"* ]]; then
    print_error "output is not statically linked"
    exit 1
fi

if [[ "$FILE_INFO" == *"pie executable"* || "$FILE_INFO" == *"shared object"* ]]; then
    print_error "output is PIE/shared; kerners phase 10-1 baseline requires ET_EXEC"
    exit 1
fi

print_info "busybox static build complete: $OUTPUT_BIN"
