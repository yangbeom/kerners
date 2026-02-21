#!/bin/bash
# Build external static ELF binaries for Phase 15-1 validation (Rust-only, musl targets)
#
# Usage:
#   ./scripts/build_user_static_bins.sh [ARCH] [OUT_DIR]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-aarch64}"
OUT_DIR="${2:-$PROJECT_ROOT/target/user/$ARCH}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_info() { echo -e "${GREEN}[phase15-bin]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[phase15-bin]${NC} $1"; }
print_error() { echo -e "${RED}[phase15-bin]${NC} $1"; }

if ! command -v rustc >/dev/null 2>&1; then
    print_error "rustc is required"
    exit 1
fi

case "$ARCH" in
    aarch64)
        RUST_TARGET="aarch64-unknown-linux-musl"
        ;;
    riscv64)
        RUST_TARGET="riscv64gc-unknown-linux-musl"
        ;;
    *)
        print_error "unsupported arch: $ARCH (expected: aarch64 or riscv64)"
        exit 1
        ;;
esac

mkdir -p "$OUT_DIR"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/phase15-bins.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

HELLO_SRC="$TMP_DIR/hello.rs"
PROBE_SRC="$TMP_DIR/execve_bounds.rs"
HELLO_BIN="$OUT_DIR/hello"
PROBE_BIN="$OUT_DIR/execve_bounds"

cat >"$HELLO_SRC" <<'RS'
fn main() {
    println!("PHASE15_1_HELLO_OK");
    std::process::exit(42);
}
RS

cat >"$PROBE_SRC" <<'RS'
use std::ffi::c_char;
use std::io;
use std::os::raw::{c_int, c_long};
use std::ptr;

unsafe extern "C" {
    fn syscall(num: c_long, ...) -> c_long;
    fn fork() -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char) -> c_int;
    fn _exit(code: c_int) -> !;
}

const SYS_EXECVE: c_long = 221;

unsafe fn raw_execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char) -> c_long {
    unsafe { syscall(SYS_EXECVE, path, argv, envp) }
}

fn wifexited(status: c_int) -> bool {
    (status & 0x7f) == 0
}

fn wexitstatus(status: c_int) -> c_int {
    (status >> 8) & 0xff
}

fn run_case_errno(
    name: &str,
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
    expect_errno: i32,
) -> bool {
    let ret = unsafe { raw_execve(path, argv, envp) };
    let errno = io::Error::last_os_error().raw_os_error().unwrap_or(-1);

    if ret == -1 && errno == expect_errno {
        println!("PHASE15_1_EXECVE_BOUNDS: {name} got=-{errno} expect=-{expect_errno} PASS");
        true
    } else {
        println!(
            "PHASE15_1_EXECVE_BOUNDS: {name} got_ret={ret} got_errno=-{errno} expect=-{expect_errno} FAIL"
        );
        false
    }
}

fn run_exec_wait(name: &str, path: *const c_char, argv: &[*const c_char], expect_exit: i32) -> bool {
    let envp = [ptr::null::<c_char>()];
    let pid = unsafe { fork() };

    if pid < 0 {
        println!("PHASE15_1_EXECVE_BOUNDS: {name} fork FAIL");
        return false;
    }

    if pid == 0 {
        let ret = unsafe { execve(path, argv.as_ptr(), envp.as_ptr()) };
        if ret == -1 {
            let errno = io::Error::last_os_error().raw_os_error().unwrap_or(255);
            unsafe { _exit(200 + (errno & 0x3f)) }
        }
        unsafe { _exit(201) }
    }

    let mut status: c_int = 0;
    let w = unsafe { waitpid(pid, &mut status as *mut c_int, 0) };
    if w != pid {
        println!("PHASE15_1_EXECVE_BOUNDS: {name} waitpid FAIL");
        return false;
    }

    if !wifexited(status) {
        println!("PHASE15_1_EXECVE_BOUNDS: {name} abnormal exit FAIL");
        return false;
    }

    let code = wexitstatus(status);
    if code == expect_exit {
        println!("PHASE15_1_EXECVE_BOUNDS: {name} exit={code} PASS");
        true
    } else {
        println!("PHASE15_1_EXECVE_BOUNDS: {name} exit={code} expect={expect_exit} FAIL");
        false
    }
}

fn main() {
    println!("PHASE15_1_EXECVE_BOUNDS: START");

    let mut fail = 0usize;

    let bb_path = b"/mnt/bin/busybox\0";
    let cp_argv0 = b"busybox\0";
    let cp_argv1 = b"cp\0";

    let src_bb = b"/mnt/bin/busybox\0";
    let dst_bb = b"/bin/busybox\0";
    let argv_cp_bb = [
        cp_argv0.as_ptr() as *const c_char,
        cp_argv1.as_ptr() as *const c_char,
        src_bb.as_ptr() as *const c_char,
        dst_bb.as_ptr() as *const c_char,
        ptr::null::<c_char>(),
    ];
    if !run_exec_wait("COPY_BUSYBOX", bb_path.as_ptr() as *const c_char, &argv_cp_bb, 0) {
        fail += 1;
    }

    let src_hello = b"/mnt/bin/hello\0";
    let dst_hello = b"/bin/hello\0";
    let argv_cp_hello = [
        cp_argv0.as_ptr() as *const c_char,
        cp_argv1.as_ptr() as *const c_char,
        src_hello.as_ptr() as *const c_char,
        dst_hello.as_ptr() as *const c_char,
        ptr::null::<c_char>(),
    ];
    if !run_exec_wait("COPY_HELLO", bb_path.as_ptr() as *const c_char, &argv_cp_hello, 0) {
        fail += 1;
    }

    let sh_argv1 = b"sh\0";
    let sh_argv2 = b"-c\0";
    let sh_cmd = b"echo not-an-elf > /bin/not_elf; echo PHASE15_1_BUSYBOX_SHELL_OK\0";
    let argv_sh = [
        cp_argv0.as_ptr() as *const c_char,
        sh_argv1.as_ptr() as *const c_char,
        sh_argv2.as_ptr() as *const c_char,
        sh_cmd.as_ptr() as *const c_char,
        ptr::null::<c_char>(),
    ];
    if !run_exec_wait("BUSYBOX_SHELL", b"/bin/busybox\0".as_ptr() as *const c_char, &argv_sh, 0) {
        fail += 1;
    }

    let path_missing = b"/bin/no_such_binary\0";
    let path_not_elf = b"/bin/not_elf\0";
    let path_hello = b"/bin/hello\0";
    let arg0 = b"probe\0";
    let arg_x = b"x\0";

    let envp = [ptr::null::<c_char>()];
    let argv_ok = [arg0.as_ptr() as *const c_char, ptr::null::<c_char>()];

    let mut argv_big = [ptr::null::<c_char>(); 129];
    for item in argv_big.iter_mut().take(128) {
        *item = arg_x.as_ptr() as *const c_char;
    }

    if !run_case_errno(
        "ENOENT",
        path_missing.as_ptr() as *const c_char,
        argv_ok.as_ptr(),
        envp.as_ptr(),
        2,
    ) {
        fail += 1;
    }

    if !run_case_errno(
        "ENOEXEC",
        path_not_elf.as_ptr() as *const c_char,
        argv_ok.as_ptr(),
        envp.as_ptr(),
        8,
    ) {
        fail += 1;
    }

    if !run_case_errno(
        "E2BIG",
        path_hello.as_ptr() as *const c_char,
        argv_big.as_ptr(),
        envp.as_ptr(),
        7,
    ) {
        fail += 1;
    }

    if !run_case_errno(
        "EFAULT",
        1usize as *const c_char,
        argv_ok.as_ptr(),
        envp.as_ptr(),
        14,
    ) {
        fail += 1;
    }

    let argv_bb_echo = [
        b"busybox\0".as_ptr() as *const c_char,
        b"echo\0".as_ptr() as *const c_char,
        b"PHASE15_1_BUSYBOX_EXEC_OK\0".as_ptr() as *const c_char,
        ptr::null::<c_char>(),
    ];
    if !run_exec_wait("BUSYBOX_EXEC", b"/bin/busybox\0".as_ptr() as *const c_char, &argv_bb_echo, 0) {
        fail += 1;
    }

    let argv_hello = [
        b"hello\0".as_ptr() as *const c_char,
        ptr::null::<c_char>(),
    ];
    if !run_exec_wait("HELLO_EXEC", b"/bin/hello\0".as_ptr() as *const c_char, &argv_hello, 42) {
        fail += 1;
    }

    if fail == 0 {
        println!("PHASE15_1_EXECVE_BOUNDS: PASS");
        std::process::exit(0);
    }

    println!("PHASE15_1_EXECVE_BOUNDS: FAIL");
    std::process::exit(1);
}
RS

RUST_FLAGS=(
    -C linker=rust-lld
    -C target-feature=+crt-static
    -C panic=abort
    -C opt-level=s
)

print_info "building hello (rust) -> $HELLO_BIN"
rustc "$HELLO_SRC" --crate-name phase15_hello --target "$RUST_TARGET" "${RUST_FLAGS[@]}" -o "$HELLO_BIN"

print_info "building execve_bounds (rust) -> $PROBE_BIN"
rustc "$PROBE_SRC" --crate-name phase15_execve_bounds --target "$RUST_TARGET" "${RUST_FLAGS[@]}" -o "$PROBE_BIN"

if command -v file >/dev/null 2>&1; then
    print_info "file info:"
    file "$HELLO_BIN" "$PROBE_BIN"
fi

if command -v shasum >/dev/null 2>&1; then
    print_info "sha256:"
    shasum -a 256 "$HELLO_BIN" "$PROBE_BIN"
fi

print_info "done (arch=$ARCH)"
