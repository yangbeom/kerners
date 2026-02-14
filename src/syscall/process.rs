//! 프로세스 관련 시스템 콜
//!
//! exit, yield, getpid, execve 등

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::kprintln;
use crate::proc;
use crate::sync::Mutex;
use super::errno;

const MAX_EXEC_PATH_LEN: usize = 4096;
const MAX_EXEC_ARG_COUNT: usize = 128;
const MAX_EXEC_ENV_COUNT: usize = 128;
const MAX_EXEC_STR_LEN: usize = 4096;
const MAX_EXEC_ARG_ENV_TOTAL_BYTES: usize = 32 * 1024;
const INIT_PROCESS_TID: proc::Tid = 1;

struct PendingExec {
    tid: proc::Tid,
    image: proc::user::PreparedExecImage,
}

struct BrkRegion {
    tid: proc::Tid,
    base: usize,
    current: usize,
    limit: usize,
    phys_base: usize,
}

struct MmapRegion {
    tid: proc::Tid,
    base: usize,
    phys_base: usize,
    pages: usize,
}

#[derive(Clone, Copy)]
struct ZombieChild {
    parent_tid: proc::Tid,
    child_tid: isize,
    status: i32,
}

struct ProcessInfo {
    tid: proc::Tid,
    parent_tid: proc::Tid,
    pgid: proc::Tid,
    sid: proc::Tid,
    vm_group: u64,
    fs_group: u64,
    files_group: u64,
    sighand_group: u64,
    signal_mask: u64,
    pending_signals: Vec<u32>,
    exit_signal: u32,
}

struct VforkWait {
    parent_tid: proc::Tid,
    child_tid: proc::Tid,
}

#[cfg(target_arch = "aarch64")]
#[repr(C)]
#[derive(Clone, Copy)]
struct ForkChildContext {
    gpr: [u64; 31],
    elr: u64,
    spsr: u64,
    sp_el0: u64,
}

#[cfg(target_arch = "aarch64")]
struct PendingForkChild {
    tid: proc::Tid,
    context: ForkChildContext,
}

/// execve 성공 후 trap 복귀 시 적용할 컨텍스트 전이 정보
pub struct ExecTransition {
    pub entry: usize,
    pub stack_top: usize,
    pub argc: usize,
    pub argv: usize,
    pub envp: usize,
    pub user_stack: Vec<u8>,
}

/// 스레드별 pending exec 리스트
static PENDING_EXECS: Mutex<Vec<PendingExec>> = Mutex::new(Vec::new());
static BRK_REGIONS: Mutex<Vec<BrkRegion>> = Mutex::new(Vec::new());
static MMAP_REGIONS: Mutex<Vec<MmapRegion>> = Mutex::new(Vec::new());
static ZOMBIE_CHILDREN: Mutex<Vec<ZombieChild>> = Mutex::new(Vec::new());
static PROCESS_INFOS: Mutex<Vec<ProcessInfo>> = Mutex::new(Vec::new());
static VFORK_WAITS: Mutex<Vec<VforkWait>> = Mutex::new(Vec::new());
static NEXT_FAKE_CHILD_TID: AtomicUsize = AtomicUsize::new(1000);
static NEXT_RESOURCE_GROUP_ID: AtomicUsize = AtomicUsize::new(1);
#[cfg(target_arch = "aarch64")]
static PENDING_FORK_CHILDREN: Mutex<Vec<PendingForkChild>> = Mutex::new(Vec::new());

const BRK_REGION_SIZE: usize = 16 * 1024 * 1024; // 16MB (static BusyBox init baseline)
#[cfg(target_arch = "aarch64")]
const BRK_REGION_BASE: usize = 0x2000_0000;
#[cfg(target_arch = "aarch64")]
const MMAP_REGION_BASE: usize = 0x3000_0000;
#[cfg(target_arch = "aarch64")]
static NEXT_MMAP_BASE: AtomicUsize = AtomicUsize::new(MMAP_REGION_BASE);

const PROT_READ: usize = 0x1;
const PROT_WRITE: usize = 0x2;
const PROT_EXEC: usize = 0x4;
const MAP_SHARED: usize = 0x01;
const MAP_PRIVATE: usize = 0x02;
const MAP_FIXED: usize = 0x10;
const MAP_ANONYMOUS: usize = 0x20;
const CLONE_VM: usize = 0x00000100;
const CLONE_FS: usize = 0x00000200;
const CLONE_FILES: usize = 0x00000400;
const CLONE_SIGHAND: usize = 0x00000800;
const CLONE_VFORK: usize = 0x00004000;
const CLONE_CSIGNAL_MASK: usize = 0x000000ff;
const WNOHANG: i32 = 0x1;
const WEXITED: i32 = 0x4;
const WNOWAIT: i32 = 0x0100_0000;
const WAITID_IDTYPE_ALL: i32 = 0;
const WAITID_IDTYPE_PID: i32 = 1;
const WAITID_IDTYPE_PGID: i32 = 2;
const SIGNAL_SIGCHLD: u32 = 17;
const SIGINFO_CLD_EXITED: i32 = 1;
const SIGINFO_CLD_KILLED: i32 = 2;
const SIG_BLOCK: i32 = 0;
const SIG_UNBLOCK: i32 = 1;
const SIG_SETMASK: i32 = 2;
const MIN_SIGSET_SIZE: usize = core::mem::size_of::<u64>();

const CLOCK_REALTIME: i32 = 0;
const CLOCK_MONOTONIC: i32 = 1;

#[cfg(target_arch = "aarch64")]
const MIN_USER_VADDR: usize = 0x1000;
#[cfg(target_arch = "aarch64")]
const MAX_USER_VADDR_EXCLUSIVE: usize = crate::proc::user::USER_STACK_BASE;

#[inline]
const fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTimespec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTimeval {
    tv_sec: i64,
    tv_usec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTimezone {
    tz_minuteswest: i32,
    tz_dsttime: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxSigInfoHeader {
    si_signo: i32,
    si_errno: i32,
    si_code: i32,
    _pad: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxWaitidSigInfo {
    si_signo: i32,
    si_errno: i32,
    si_code: i32,
    si_pid: i32,
    si_uid: u32,
    si_status: i32,
    si_utime: i64,
    si_stime: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxUtsName {
    sysname: [u8; 65],
    nodename: [u8; 65],
    release: [u8; 65],
    version: [u8; 65],
    machine: [u8; 65],
    domainname: [u8; 65],
}

#[inline]
fn monotonic_time() -> (u64, u64) {
    #[cfg(target_arch = "aarch64")]
    {
        let counter = crate::arch::timer::get_counter();
        let freq = crate::arch::timer::get_frequency();
        if freq == 0 {
            return (0, 0);
        }
        let sec = counter / freq;
        let nsec = (counter % freq) * 1_000_000_000 / freq;
        (sec, nsec)
    }

    #[cfg(target_arch = "riscv64")]
    {
        let counter = crate::arch::timer::get_time();
        let freq = crate::boards::timer_freq();
        if freq == 0 {
            return (0, 0);
        }
        let sec = counter / freq;
        let nsec = (counter % freq) * 1_000_000_000 / freq;
        (sec, nsec)
    }
}

#[cfg(target_arch = "aarch64")]
fn map_user_contiguous(
    virt_base: usize,
    phys_base: usize,
    pages: usize,
    write: bool,
    execute: bool,
) -> Result<(), isize> {
    let page_size = crate::mm::page::PAGE_SIZE;
    for i in 0..pages {
        let va = virt_base + i * page_size;
        let pa = phys_base + i * page_size;
        if crate::arch::mmu::map_user_page_noflush(va, pa, write, execute).is_err() {
            return Err(errno::ENOMEM);
        }
    }
    crate::arch::mmu::flush_tlb_all();
    Ok(())
}

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn fork_child_enter_user(context: *const ForkChildContext) -> !;
}

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    r#"
.section .text
.global fork_child_enter_user
.type fork_child_enter_user, %function
fork_child_enter_user:
    // x0 = *ForkChildContext
    mov x30, x0

    // EL0 복귀 지점/상태 복원
    ldr x9, [x30, #248]   // elr
    ldr x10, [x30, #256]  // spsr
    ldr x11, [x30, #264]  // sp_el0
    msr elr_el1, x9
    msr spsr_el1, x10
    msr sp_el0, x11

    // 사용자 GPR 복원
    ldp x0, x1, [x30, #0]
    ldp x2, x3, [x30, #16]
    ldp x4, x5, [x30, #32]
    ldp x6, x7, [x30, #48]
    ldp x8, x9, [x30, #64]
    ldp x10, x11, [x30, #80]
    ldp x12, x13, [x30, #96]
    ldp x14, x15, [x30, #112]
    ldp x16, x17, [x30, #128]
    ldp x18, x19, [x30, #144]
    ldp x20, x21, [x30, #160]
    ldp x22, x23, [x30, #176]
    ldp x24, x25, [x30, #192]
    ldp x26, x27, [x30, #208]
    ldp x28, x29, [x30, #224]
    ldr x30, [x30, #240]
    eret
"#
);

fn next_resource_group_id() -> u64 {
    NEXT_RESOURCE_GROUP_ID.fetch_add(1, Ordering::SeqCst) as u64
}

fn clone_group_id(parent_group: u64, share_group: bool) -> u64 {
    if share_group {
        parent_group
    } else {
        next_resource_group_id()
    }
}

fn clone_resource_groups(
    flags: usize,
    parent_vm_group: u64,
    parent_fs_group: u64,
    parent_files_group: u64,
    parent_sighand_group: u64,
) -> (u64, u64, u64, u64) {
    (
        clone_group_id(parent_vm_group, flags & CLONE_VM != 0),
        clone_group_id(parent_fs_group, flags & CLONE_FS != 0),
        clone_group_id(parent_files_group, flags & CLONE_FILES != 0),
        clone_group_id(parent_sighand_group, flags & CLONE_SIGHAND != 0),
    )
}

fn encode_wait_status_from_exit_code(exit_code: i32) -> i32 {
    (exit_code & 0xff) << 8
}

fn wait_status_is_exited(wait_status: i32) -> bool {
    (wait_status & 0x7f) == 0
}

fn wait_status_to_si_code(wait_status: i32) -> i32 {
    if wait_status_is_exited(wait_status) {
        SIGINFO_CLD_EXITED
    } else {
        SIGINFO_CLD_KILLED
    }
}

fn wait_status_to_si_status(wait_status: i32) -> i32 {
    if wait_status_is_exited(wait_status) {
        (wait_status >> 8) & 0xff
    } else {
        wait_status & 0x7f
    }
}

fn ensure_process_info_for_tid_locked(processes: &mut Vec<ProcessInfo>, tid: proc::Tid) -> usize {
    if let Some(pos) = processes.iter().position(|p| p.tid == tid) {
        return pos;
    }

    let default_group = if tid == 0 {
        next_resource_group_id()
    } else {
        tid
    };

    processes.push(ProcessInfo {
        tid,
        parent_tid: 0,
        pgid: tid,
        sid: tid,
        vm_group: default_group,
        fs_group: default_group,
        files_group: default_group,
        sighand_group: default_group,
        signal_mask: 0,
        pending_signals: Vec::new(),
        exit_signal: 0,
    });
    processes.len() - 1
}

fn ensure_process_info_for_tid(tid: proc::Tid) {
    let mut processes = PROCESS_INFOS.lock();
    let _ = ensure_process_info_for_tid_locked(&mut processes, tid);
}

fn current_tid_or_zero() -> proc::Tid {
    proc::current_tid().unwrap_or(0)
}

fn signal_to_mask(signum: u32) -> u64 {
    if signum == 0 || signum > 64 {
        return 0;
    }
    1u64 << (signum - 1)
}

fn signal_mask_contains(mask: u64, signum: u32) -> bool {
    (mask & signal_to_mask(signum)) != 0
}

fn enqueue_signal(tid: proc::Tid, signum: u32) {
    if signal_to_mask(signum) == 0 {
        return;
    }
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    processes[idx].pending_signals.push(signum);
}

fn take_pending_signal(tid: proc::Tid, accepted_mask: u64) -> Option<u32> {
    if accepted_mask == 0 {
        return None;
    }

    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    let pos = processes[idx]
        .pending_signals
        .iter()
        .position(|&sig| signal_mask_contains(accepted_mask, sig))?;
    Some(processes[idx].pending_signals.remove(pos))
}

fn complete_vfork_wait(child_tid: proc::Tid) {
    let mut waits = VFORK_WAITS.lock();
    waits.retain(|w| w.child_tid != child_tid);
}

fn add_vfork_wait(parent_tid: proc::Tid, child_tid: proc::Tid) {
    VFORK_WAITS.lock().push(VforkWait {
        parent_tid,
        child_tid,
    });
}

fn wait_vfork_release(parent_tid: proc::Tid, child_tid: proc::Tid) {
    loop {
        let still_waiting = {
            let waits = VFORK_WAITS.lock();
            waits
                .iter()
                .any(|w| w.parent_tid == parent_tid && w.child_tid == child_tid)
        };
        if !still_waiting {
            return;
        }
        proc::yield_now();
    }
}

fn wait_target_matches(wait_pid: isize, child_tid: isize) -> bool {
    wait_pid == -1 || wait_pid == 0 || child_tid == wait_pid
}

fn waitid_target_matches(idtype: i32, id: usize, child_tid: isize, child_pgid: proc::Tid) -> bool {
    match idtype {
        WAITID_IDTYPE_ALL => true,
        WAITID_IDTYPE_PID => child_tid == id as isize,
        WAITID_IDTYPE_PGID => child_pgid == id as proc::Tid,
        _ => false,
    }
}

fn has_matching_child(parent_tid: proc::Tid, wait_pid: isize) -> bool {
    let processes = PROCESS_INFOS.lock();
    processes
        .iter()
        .any(|p| p.parent_tid == parent_tid && wait_target_matches(wait_pid, p.tid as isize))
}

fn has_matching_child_waitid(parent_tid: proc::Tid, idtype: i32, id: usize) -> bool {
    let processes = PROCESS_INFOS.lock();
    processes.iter().any(|p| {
        p.parent_tid == parent_tid && waitid_target_matches(idtype, id, p.tid as isize, p.pgid)
    })
}

fn pop_zombie_child(parent_tid: proc::Tid, wait_pid: isize) -> Option<ZombieChild> {
    let mut children = ZOMBIE_CHILDREN.lock();
    let pos = children
        .iter()
        .position(|c| c.parent_tid == parent_tid && wait_target_matches(wait_pid, c.child_tid))?;
    Some(children.swap_remove(pos))
}

fn find_zombie_child_waitid(parent_tid: proc::Tid, idtype: i32, id: usize) -> Option<ZombieChild> {
    let processes = PROCESS_INFOS.lock();
    let children = ZOMBIE_CHILDREN.lock();
    children
        .iter()
        .find(|c| {
            if c.parent_tid != parent_tid {
                return false;
            }
            let child_pgid = processes
                .iter()
                .find(|p| p.tid == c.child_tid as proc::Tid)
                .map(|p| p.pgid)
                .unwrap_or(0);
            waitid_target_matches(idtype, id, c.child_tid, child_pgid)
        })
        .copied()
}

fn pop_zombie_child_waitid(parent_tid: proc::Tid, idtype: i32, id: usize) -> Option<ZombieChild> {
    let processes = PROCESS_INFOS.lock();
    let mut children = ZOMBIE_CHILDREN.lock();
    let pos = children.iter().position(|c| {
        if c.parent_tid != parent_tid {
            return false;
        }
        let child_pgid = processes
            .iter()
            .find(|p| p.tid == c.child_tid as proc::Tid)
            .map(|p| p.pgid)
            .unwrap_or(0);
        waitid_target_matches(idtype, id, c.child_tid, child_pgid)
    })?;
    Some(children.swap_remove(pos))
}

fn reparent_orphans(exiting_tid: proc::Tid) -> proc::Tid {
    if exiting_tid == INIT_PROCESS_TID {
        return 0;
    }

    let mut processes = PROCESS_INFOS.lock();
    if !processes.iter().any(|p| p.tid == INIT_PROCESS_TID) {
        return 0;
    }

    for process in processes.iter_mut() {
        if process.parent_tid == exiting_tid && process.tid != exiting_tid {
            process.parent_tid = INIT_PROCESS_TID;
        }
    }
    INIT_PROCESS_TID
}

fn reparent_zombie_children(old_parent_tid: proc::Tid, new_parent_tid: proc::Tid) -> usize {
    if new_parent_tid == 0 {
        return 0;
    }

    let mut moved = 0usize;
    let mut children = ZOMBIE_CHILDREN.lock();
    for child in children.iter_mut() {
        if child.parent_tid == old_parent_tid {
            child.parent_tid = new_parent_tid;
            moved += 1;
        }
    }
    moved
}

fn finalize_exit(tid: proc::Tid, status: i32) {
    let orphan_reaper = reparent_orphans(tid);
    let reparented_zombies = reparent_zombie_children(tid, orphan_reaper);
    let wait_status = encode_wait_status_from_exit_code(status);

    let (parent_tid, exit_signal) = {
        let mut processes = PROCESS_INFOS.lock();
        let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
        (processes[idx].parent_tid, processes[idx].exit_signal)
    };

    if parent_tid != 0 {
        ZOMBIE_CHILDREN.lock().push(ZombieChild {
            parent_tid,
            child_tid: tid as isize,
            status: wait_status,
        });

        if exit_signal != 0 {
            enqueue_signal(parent_tid, exit_signal);
        }
    }

    if reparented_zombies != 0 {
        enqueue_signal(orphan_reaper, SIGNAL_SIGCHLD);
    }

    complete_vfork_wait(tid);
}

fn remove_process_info(tid: proc::Tid) {
    let mut processes = PROCESS_INFOS.lock();
    if let Some(pos) = processes.iter().position(|p| p.tid == tid) {
        processes.swap_remove(pos);
    }
}

fn write_user_i32(ptr: *mut i32, value: i32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        // SAFETY: syscall 호출자 계약 상 유효한 사용자 포인터로 가정하고 값을 기록한다.
        core::ptr::write_unaligned(ptr, value);
    }
}

fn read_user_u64(ptr: *const u8) -> u64 {
    unsafe {
        // SAFETY: syscall 호출자 계약 상 유효한 사용자 포인터로 가정하고 값을 읽는다.
        core::ptr::read_unaligned(ptr as *const u64)
    }
}

fn write_user_u64(ptr: *mut u8, value: u64) {
    unsafe {
        // SAFETY: syscall 호출자 계약 상 유효한 사용자 포인터로 가정하고 값을 기록한다.
        core::ptr::write_unaligned(ptr as *mut u64, value);
    }
}

fn write_waitid_siginfo(ptr: *mut u8, child_tid: isize, wait_status: i32) {
    let siginfo = LinuxWaitidSigInfo {
        si_signo: SIGNAL_SIGCHLD as i32,
        si_errno: 0,
        si_code: wait_status_to_si_code(wait_status),
        si_pid: child_tid as i32,
        si_uid: 0,
        si_status: wait_status_to_si_status(wait_status),
        si_utime: 0,
        si_stime: 0,
    };
    unsafe {
        // SAFETY: syscall 호출자 계약 상 유효한 사용자 포인터로 가정하고 값을 기록한다.
        core::ptr::write_unaligned(ptr as *mut LinuxWaitidSigInfo, siginfo);
    }
}

fn clear_waitid_siginfo(ptr: *mut u8) {
    let empty = LinuxWaitidSigInfo {
        si_signo: 0,
        si_errno: 0,
        si_code: 0,
        si_pid: 0,
        si_uid: 0,
        si_status: 0,
        si_utime: 0,
        si_stime: 0,
    };
    unsafe {
        // SAFETY: syscall 호출자 계약 상 유효한 사용자 포인터로 가정하고 값을 기록한다.
        core::ptr::write_unaligned(ptr as *mut LinuxWaitidSigInfo, empty);
    }
}

fn write_uts_field(dst: &mut [u8; 65], value: &str) {
    let bytes = value.as_bytes();
    let len = core::cmp::min(bytes.len(), 64);
    dst[..len].copy_from_slice(&bytes[..len]);
    dst[len] = 0;
}

#[cfg(target_arch = "aarch64")]
fn fork_child_entry() -> ! {
    let tid = current_tid_or_zero();
    let context = {
        let mut pending = PENDING_FORK_CHILDREN.lock();
        let pos = pending.iter().position(|c| c.tid == tid);
        pos.map(|idx| pending.swap_remove(idx).context)
    };

    let Some(context) = context else {
        kprintln!("[syscall] fork child tid={} has no pending context", tid);
        proc::exit();
    };

    unsafe {
        // SAFETY: fork/clone 시 부모 trap context에서 캡처한 유효 EL0 복귀 상태를 복원한다.
        fork_child_enter_user(&context as *const ForkChildContext);
    }
}

/// sys_exit - 프로세스 종료
///
/// # Arguments
/// * `status` - 종료 상태 코드
///
/// # Returns
/// * 반환하지 않음 (하지만 타입 시그니처상 isize 반환)
pub fn sys_exit(status: i32) -> isize {
    let tid = current_tid_or_zero();
    finalize_exit(tid, status);
    kprintln!("[syscall] Process {} exiting with status {}", tid, status);
    proc::exit();
}

/// sys_yield - CPU 양보
///
/// # Returns
/// * 항상 0
pub fn sys_yield() -> isize {
    proc::yield_now();
    0
}

/// sys_getpid - 현재 프로세스 ID 반환
///
/// # Returns
/// * 현재 스레드/프로세스 ID
pub fn sys_getpid() -> isize {
    let tid = current_tid_or_zero();
    ensure_process_info_for_tid(tid);
    tid as isize
}

/// sys_getppid - 현재 프로세스의 부모 PID 반환
///
/// 부모-자식 관계가 없으면 0을 반환한다(PID 1 동작과 동일).
pub fn sys_getppid() -> isize {
    let tid = current_tid_or_zero();
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    processes[idx].parent_tid as isize
}

/// sys_gettid - 현재 스레드 ID 반환
pub fn sys_gettid() -> isize {
    let tid = current_tid_or_zero();
    ensure_process_info_for_tid(tid);
    tid as isize
}

/// sys_getuid - 현재 UID 반환
///
/// 현재 권한 모델이 없으므로 root(0)를 반환한다.
pub fn sys_getuid() -> isize {
    0
}

/// sys_geteuid - 현재 EUID 반환
pub fn sys_geteuid() -> isize {
    0
}

/// sys_getgid - 현재 GID 반환
pub fn sys_getgid() -> isize {
    0
}

/// sys_getegid - 현재 EGID 반환
pub fn sys_getegid() -> isize {
    0
}

/// sys_setuid - UID 변경
///
/// 현재는 root-only 단일 모델로 no-op 성공 처리한다.
pub fn sys_setuid(_uid: u32) -> isize {
    0
}

/// sys_setgid - GID 변경
pub fn sys_setgid(_gid: u32) -> isize {
    0
}

/// sys_set_tid_address - clear_child_tid 포인터 등록
///
/// 현재 커널은 clear_child_tid를 추적하지 않으며,
/// Linux와 동일하게 현재 tid를 반환한다.
pub fn sys_set_tid_address(_tidptr: *mut i32) -> isize {
    proc::current_tid().unwrap_or(0) as isize
}

/// sys_rt_sigaction - 시그널 액션 설정
///
/// 10-1C baseline: signal delivery 미구현, 호출 성공으로 처리한다.
pub fn sys_rt_sigaction(
    _signum: i32,
    _act: *const u8,
    _oldact: *mut u8,
    _sigsetsize: usize,
) -> isize {
    0
}

/// sys_rt_sigprocmask - 시그널 마스크 제어
///
/// 현재 구현은 64비트 시그널 마스크(1~64번)를 프로세스 단위로 추적한다.
pub fn sys_rt_sigprocmask(
    how: i32,
    set: *const u8,
    oldset: *mut u8,
    sigsetsize: usize,
) -> isize {
    if sigsetsize < MIN_SIGSET_SIZE {
        return errno::EINVAL;
    }

    let tid = current_tid_or_zero();
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);

    let old_mask = processes[idx].signal_mask;
    if !oldset.is_null() {
        write_user_u64(oldset, old_mask);
    }

    if set.is_null() {
        return 0;
    }

    let set_bits = read_user_u64(set);
    match how {
        SIG_BLOCK => processes[idx].signal_mask |= set_bits,
        SIG_UNBLOCK => processes[idx].signal_mask &= !set_bits,
        SIG_SETMASK => processes[idx].signal_mask = set_bits,
        _ => return errno::EINVAL,
    }

    0
}

/// sys_nanosleep - 지정 시간 대기
///
/// 고해상도 슬립은 아직 미구현이며, 최소 동작으로 yield를 수행한다.
pub fn sys_nanosleep(_req: *const u8, _rem: *mut u8) -> isize {
    proc::yield_now();
    0
}

/// sys_clock_gettime - 시계 값 조회
///
/// 현재는 MONOTONIC/REALTIME 모두 부팅 이후 monotonic counter 기반으로 제공한다.
pub fn sys_clock_gettime(clock_id: i32, tp: *mut u8) -> isize {
    if tp.is_null() {
        return errno::EFAULT;
    }

    if clock_id != CLOCK_REALTIME && clock_id != CLOCK_MONOTONIC {
        return errno::EINVAL;
    }

    let (sec, nsec) = monotonic_time();
    let ts = LinuxTimespec {
        tv_sec: sec as i64,
        tv_nsec: nsec as i64,
    };

    unsafe {
        // SAFETY: 사용자 공간 포인터가 유효하다는 syscall 호출자 계약 하에 timespec을 기록한다.
        core::ptr::write_unaligned(tp as *mut LinuxTimespec, ts);
    }
    0
}

/// sys_gettimeofday - wallclock 시간 조회
///
/// 현재 커널은 RTC가 없으므로 monotonic 시간을 timeval로 변환해 반환한다.
pub fn sys_gettimeofday(tv: *mut u8, tz: *mut u8) -> isize {
    let (sec, nsec) = monotonic_time();

    if !tv.is_null() {
        let tv_out = LinuxTimeval {
            tv_sec: sec as i64,
            tv_usec: (nsec / 1_000) as i64,
        };
        unsafe {
            // SAFETY: 사용자 공간 포인터가 유효하다는 syscall 호출자 계약 하에 timeval을 기록한다.
            core::ptr::write_unaligned(tv as *mut LinuxTimeval, tv_out);
        }
    }

    if !tz.is_null() {
        let tz_out = LinuxTimezone {
            tz_minuteswest: 0,
            tz_dsttime: 0,
        };
        unsafe {
            // SAFETY: 사용자 공간 포인터가 유효하다는 syscall 호출자 계약 하에 timezone을 기록한다.
            core::ptr::write_unaligned(tz as *mut LinuxTimezone, tz_out);
        }
    }

    0
}

/// sys_rt_sigtimedwait - 지정 시그널 대기
///
/// pending signal queue에서 조건에 맞는 시그널을 하나 꺼내 반환한다.
pub fn sys_rt_sigtimedwait(
    set: *const u8,
    info: *mut u8,
    _timeout: *const u8,
    sigsetsize: usize,
) -> isize {
    if set.is_null() {
        return errno::EFAULT;
    }
    if sigsetsize < MIN_SIGSET_SIZE {
        return errno::EINVAL;
    }

    let accepted = read_user_u64(set);
    let tid = current_tid_or_zero();
    if let Some(signum) = take_pending_signal(tid, accepted) {
        if !info.is_null() {
            let siginfo = LinuxSigInfoHeader {
                si_signo: signum as i32,
                si_errno: 0,
                si_code: 0,
                _pad: 0,
            };
            unsafe {
                // SAFETY: syscall 호출자 계약 상 info 포인터는 siginfo_t 저장 가능한 영역을 가리킨다.
                core::ptr::write_unaligned(info as *mut LinuxSigInfoHeader, siginfo);
            }
        }
        return signum as isize;
    }

    errno::EAGAIN
}

/// sys_socket - 소켓 생성
///
/// 10-1C baseline: 네트워크 스택 미구현 상태를 명시적으로 알린다.
pub fn sys_socket(_domain: i32, _socket_type: i32, _protocol: i32) -> isize {
    errno::EAFNOSUPPORT
}

/// sys_sendto - 소켓 전송
///
/// baseline에서는 소켓을 지원하지 않으므로 EBADF를 반환한다.
pub fn sys_sendto(
    fd: i32,
    _buf: *const u8,
    _len: usize,
    _flags: i32,
    _addr: *const u8,
    _addrlen: u32,
) -> isize {
    if fd < 0 {
        return errno::EBADF;
    }
    errno::EAFNOSUPPORT
}

/// sys_clone - 프로세스/스레드 복제
///
/// trap context가 없는 경로(커널 테스트 모듈 등)에서는 즉시 종료된 가짜 자식을 만든다.
pub fn sys_clone(
    flags: usize,
    _child_stack: usize,
    parent_tid_ptr: *mut u8,
    _tls: usize,
    child_tid_ptr: *mut u8,
) -> isize {
    if flags & CLONE_SIGHAND != 0 && flags & CLONE_VM == 0 {
        return errno::EINVAL;
    }

    let parent_tid = current_tid_or_zero();
    ensure_process_info_for_tid(parent_tid);
    let child_tid = NEXT_FAKE_CHILD_TID.fetch_add(1, Ordering::SeqCst) as isize;
    let child_tid_u64 = child_tid as proc::Tid;
    let exit_signal = (flags & CLONE_CSIGNAL_MASK) as u32;
    {
        let mut processes = PROCESS_INFOS.lock();
        let parent_idx = ensure_process_info_for_tid_locked(&mut processes, parent_tid);

        let parent_pgid = processes[parent_idx].pgid;
        let parent_sid = processes[parent_idx].sid;
        let parent_mask = processes[parent_idx].signal_mask;
        let parent_vm_group = processes[parent_idx].vm_group;
        let parent_fs_group = processes[parent_idx].fs_group;
        let parent_files_group = processes[parent_idx].files_group;
        let parent_sighand_group = processes[parent_idx].sighand_group;
        let (child_vm_group, child_fs_group, child_files_group, child_sighand_group) =
            clone_resource_groups(
                flags,
                parent_vm_group,
                parent_fs_group,
                parent_files_group,
                parent_sighand_group,
            );

        if let Some(pos) = processes.iter().position(|p| p.tid == child_tid_u64) {
            processes.swap_remove(pos);
        }
        processes.push(ProcessInfo {
            tid: child_tid_u64,
            parent_tid,
            pgid: parent_pgid,
            sid: parent_sid,
            vm_group: child_vm_group,
            fs_group: child_fs_group,
            files_group: child_files_group,
            sighand_group: child_sighand_group,
            signal_mask: parent_mask,
            pending_signals: Vec::new(),
            exit_signal,
        });
    }

    ZOMBIE_CHILDREN.lock().push(ZombieChild {
        parent_tid,
        child_tid,
        status: encode_wait_status_from_exit_code(0),
    });

    if exit_signal != 0 {
        enqueue_signal(parent_tid, exit_signal);
    }

    if !parent_tid_ptr.is_null() {
        write_user_i32(parent_tid_ptr as *mut i32, child_tid as i32);
    }
    if !child_tid_ptr.is_null() {
        write_user_i32(child_tid_ptr as *mut i32, child_tid as i32);
    }

    child_tid
}

/// sys_fork - clone(SIGCHLD) 래퍼
pub fn sys_fork() -> isize {
    sys_clone(SIGNAL_SIGCHLD as usize, 0, core::ptr::null_mut(), 0, core::ptr::null_mut())
}

/// sys_vfork - clone(CLONE_VM | CLONE_VFORK | SIGCHLD) 래퍼
pub fn sys_vfork() -> isize {
    sys_clone(
        CLONE_VM | CLONE_VFORK | SIGNAL_SIGCHLD as usize,
        0,
        core::ptr::null_mut(),
        0,
        core::ptr::null_mut(),
    )
}

#[cfg(target_arch = "aarch64")]
pub fn sys_clone_with_user_context(
    flags: usize,
    child_stack: usize,
    parent_tid_ptr: *mut u8,
    _tls: usize,
    child_tid_ptr: *mut u8,
    mut gpr: [u64; 31],
    elr: u64,
    spsr: u64,
    sp_el0: usize,
) -> isize {
    if flags & CLONE_SIGHAND != 0 && flags & CLONE_VM == 0 {
        return errno::EINVAL;
    }

    let parent_tid = current_tid_or_zero();
    ensure_process_info_for_tid(parent_tid);

    let child_tid = proc::spawn("fork-child", fork_child_entry);
    let child_sp = if child_stack == 0 {
        sp_el0 as u64
    } else {
        child_stack as u64
    };

    gpr[0] = 0; // child의 syscall 반환값

    PENDING_FORK_CHILDREN.lock().push(PendingForkChild {
        tid: child_tid,
        context: ForkChildContext {
            gpr,
            elr,
            spsr,
            sp_el0: child_sp,
        },
    });

    let exit_signal = (flags & CLONE_CSIGNAL_MASK) as u32;
    {
        let mut processes = PROCESS_INFOS.lock();
        let parent_idx = ensure_process_info_for_tid_locked(&mut processes, parent_tid);
        let parent_pgid = processes[parent_idx].pgid;
        let parent_sid = processes[parent_idx].sid;
        let parent_mask = processes[parent_idx].signal_mask;
        let parent_vm_group = processes[parent_idx].vm_group;
        let parent_fs_group = processes[parent_idx].fs_group;
        let parent_files_group = processes[parent_idx].files_group;
        let parent_sighand_group = processes[parent_idx].sighand_group;
        let (child_vm_group, child_fs_group, child_files_group, child_sighand_group) =
            clone_resource_groups(
                flags,
                parent_vm_group,
                parent_fs_group,
                parent_files_group,
                parent_sighand_group,
            );
        if let Some(pos) = processes.iter().position(|p| p.tid == child_tid) {
            processes.swap_remove(pos);
        }
        processes.push(ProcessInfo {
            tid: child_tid,
            parent_tid,
            pgid: parent_pgid,
            sid: parent_sid,
            vm_group: child_vm_group,
            fs_group: child_fs_group,
            files_group: child_files_group,
            sighand_group: child_sighand_group,
            signal_mask: parent_mask,
            pending_signals: Vec::new(),
            exit_signal,
        });
    }

    if !parent_tid_ptr.is_null() {
        write_user_i32(parent_tid_ptr as *mut i32, child_tid as i32);
    }
    if !child_tid_ptr.is_null() {
        write_user_i32(child_tid_ptr as *mut i32, child_tid as i32);
    }

    if flags & CLONE_VFORK != 0 {
        add_vfork_wait(parent_tid, child_tid);
        wait_vfork_release(parent_tid, child_tid);
    }

    child_tid as isize
}

#[cfg(target_arch = "aarch64")]
pub fn sys_fork_with_user_context(
    parent_tid_ptr: *mut u8,
    child_tid_ptr: *mut u8,
    gpr: [u64; 31],
    elr: u64,
    spsr: u64,
    sp_el0: usize,
) -> isize {
    sys_clone_with_user_context(
        SIGNAL_SIGCHLD as usize,
        0,
        parent_tid_ptr,
        0,
        child_tid_ptr,
        gpr,
        elr,
        spsr,
        sp_el0,
    )
}

#[cfg(target_arch = "aarch64")]
pub fn sys_vfork_with_user_context(
    parent_tid_ptr: *mut u8,
    child_tid_ptr: *mut u8,
    gpr: [u64; 31],
    elr: u64,
    spsr: u64,
    sp_el0: usize,
) -> isize {
    sys_clone_with_user_context(
        CLONE_VM | CLONE_VFORK | SIGNAL_SIGCHLD as usize,
        0,
        parent_tid_ptr,
        0,
        child_tid_ptr,
        gpr,
        elr,
        spsr,
        sp_el0,
    )
}

/// sys_wait4 - 자식 종료 대기
///
/// 부모의 zombie 자식을 회수한다.
pub fn sys_wait4(pid: isize, status: *mut i32, options: i32, _rusage: *mut u8) -> isize {
    let parent_tid = current_tid_or_zero();
    ensure_process_info_for_tid(parent_tid);

    loop {
        if let Some(child) = pop_zombie_child(parent_tid, pid) {
            write_user_i32(status, child.status);
            remove_process_info(child.child_tid as proc::Tid);
            return child.child_tid;
        }

        if !has_matching_child(parent_tid, pid) {
            return errno::ECHILD;
        }

        if options & WNOHANG != 0 {
            return 0;
        }

        proc::yield_now();
    }
}

/// sys_waitid - 자식 이벤트 대기
///
/// 현재는 `P_ALL/P_PID/P_PGID` + `WEXITED/WNOHANG/WNOWAIT` 최소 호환을 제공한다.
pub fn sys_waitid(idtype: i32, id: usize, infop: *mut u8, options: i32, _rusage: *mut u8) -> isize {
    if infop.is_null() {
        return errno::EFAULT;
    }

    if idtype != WAITID_IDTYPE_ALL && idtype != WAITID_IDTYPE_PID && idtype != WAITID_IDTYPE_PGID {
        return errno::EINVAL;
    }

    let allowed_options = WNOHANG | WEXITED | WNOWAIT;
    if options & !allowed_options != 0 {
        return errno::EINVAL;
    }

    if options & WEXITED == 0 {
        return errno::EINVAL;
    }

    let parent_tid = current_tid_or_zero();
    ensure_process_info_for_tid(parent_tid);
    let target_id = if idtype == WAITID_IDTYPE_PGID && id == 0 {
        let mut processes = PROCESS_INFOS.lock();
        let idx = ensure_process_info_for_tid_locked(&mut processes, parent_tid);
        processes[idx].pgid as usize
    } else {
        id
    };

    loop {
        let zombie = if options & WNOWAIT != 0 {
            find_zombie_child_waitid(parent_tid, idtype, target_id)
        } else {
            pop_zombie_child_waitid(parent_tid, idtype, target_id)
        };

        if let Some(child) = zombie {
            write_waitid_siginfo(infop, child.child_tid, child.status);
            if options & WNOWAIT == 0 {
                remove_process_info(child.child_tid as proc::Tid);
            }
            return 0;
        }

        if !has_matching_child_waitid(parent_tid, idtype, target_id) {
            return errno::ECHILD;
        }

        if options & WNOHANG != 0 {
            clear_waitid_siginfo(infop);
            return 0;
        }

        proc::yield_now();
    }
}

/// sys_uname - 시스템 정보 조회
///
/// Linux `struct utsname` 레이아웃(65바이트 필드)을 사용한다.
pub fn sys_uname(buf: *mut u8) -> isize {
    if buf.is_null() {
        return errno::EFAULT;
    }

    #[cfg(target_arch = "aarch64")]
    let machine = "aarch64";
    #[cfg(target_arch = "riscv64")]
    let machine = "riscv64";
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    let machine = "unknown";

    let mut uts = LinuxUtsName {
        sysname: [0; 65],
        nodename: [0; 65],
        release: [0; 65],
        version: [0; 65],
        machine: [0; 65],
        domainname: [0; 65],
    };

    write_uts_field(&mut uts.sysname, "Kerners");
    write_uts_field(&mut uts.nodename, "kerners");
    write_uts_field(&mut uts.release, "0.1.0");
    write_uts_field(&mut uts.version, "#1 kerners");
    write_uts_field(&mut uts.machine, machine);
    write_uts_field(&mut uts.domainname, "localdomain");

    unsafe {
        // SAFETY: syscall 호출자 계약 상 유효한 사용자 포인터로 가정하고 값을 기록한다.
        core::ptr::write_unaligned(buf as *mut LinuxUtsName, uts);
    }
    0
}

/// sys_setpgid - 프로세스 그룹 설정
///
/// 단순한 프로세스 그룹 추적을 지원한다.
pub fn sys_setpgid(pid: isize, pgid: isize) -> isize {
    let tid = current_tid_or_zero();
    let target_tid = if pid <= 0 { tid } else { pid as proc::Tid };
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, target_tid);
    let new_pgid = if pgid <= 0 { target_tid } else { pgid as proc::Tid };
    processes[idx].pgid = new_pgid;
    0
}

/// sys_getpgid - 프로세스 그룹 조회
///
/// pid가 0이면 현재 프로세스의 pgid를 반환한다.
pub fn sys_getpgid(pid: isize) -> isize {
    let tid = if pid <= 0 {
        current_tid_or_zero()
    } else {
        pid as proc::Tid
    };

    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    processes[idx].pgid as isize
}

/// sys_setsid - 새 세션 생성
///
/// 단순화된 setsid 동작: sid=pgid=current tid
pub fn sys_setsid() -> isize {
    let tid = current_tid_or_zero();
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    processes[idx].sid = tid;
    processes[idx].pgid = tid;
    tid as isize
}

/// sys_getsid - 세션 ID 조회
pub fn sys_getsid(pid: isize) -> isize {
    let tid = if pid <= 0 {
        current_tid_or_zero()
    } else {
        pid as proc::Tid
    };

    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    processes[idx].sid as isize
}

/// sys_reboot - 시스템 재부팅
///
/// 사용자 공간 요청 재부팅은 아직 지원하지 않는다.
pub fn sys_reboot(_magic1: usize, _magic2: usize, _cmd: usize, _arg: usize) -> isize {
    errno::EPERM
}

/// sys_brk - 프로그램 브레이크 조정
///
/// 현재 구현은 스레드별 고정 16MB 영역 내에서만 브레이크를 이동한다.
pub fn sys_brk(addr: usize) -> isize {
    let tid = proc::current_tid().unwrap_or(0);

    let page_size = crate::mm::page::PAGE_SIZE;
    let mut regions = BRK_REGIONS.lock();
    let idx = if let Some(pos) = regions.iter().position(|r| r.tid == tid) {
        pos
    } else {
        let pages = BRK_REGION_SIZE / page_size;
        let phys_base = match crate::mm::page::alloc_frames(pages) {
            Some(v) => v,
            None => return errno::ENOMEM,
        };

        unsafe {
            // SAFETY: page allocator에서 할당받은 유효한 메모리 영역을 0으로 초기화한다.
            core::ptr::write_bytes(phys_base as *mut u8, 0, BRK_REGION_SIZE);
        }

        #[cfg(target_arch = "aarch64")]
        let base = {
            let slot = tid as usize;
            let virt_base = BRK_REGION_BASE + slot.saturating_mul(BRK_REGION_SIZE);
            let virt_end = match virt_base.checked_add(BRK_REGION_SIZE) {
                Some(end) => end,
                None => return errno::ENOMEM,
            };
            if virt_end >= crate::proc::user::USER_STACK_BASE {
                return errno::ENOMEM;
            }
            if map_user_contiguous(virt_base, phys_base, pages, true, false).is_err() {
                unsafe {
                    crate::mm::page::free_frames(phys_base, pages);
                }
                return errno::ENOMEM;
            }
            virt_base
        };

        #[cfg(not(target_arch = "aarch64"))]
        let base = phys_base;

        regions.push(BrkRegion {
            tid,
            base,
            current: base,
            limit: base + BRK_REGION_SIZE,
            phys_base,
        });
        regions.len() - 1
    };

    let region = &mut regions[idx];
    if addr == 0 {
        return region.current as isize;
    }

    if addr < region.base || addr > region.limit {
        // Linux brk와 유사하게 실패 시 현재 break를 반환한다.
        return region.current as isize;
    }

    region.current = align_up(addr, 16);
    region.current as isize
}

/// sys_mmap - 익명(private/shared) 매핑
///
/// 10-1B 최소 범위: MAP_ANONYMOUS 기반 메모리 매핑만 지원.
pub fn sys_mmap(
    _addr: usize,
    len: usize,
    prot: usize,
    flags: usize,
    fd: isize,
    _offset: usize,
) -> isize {
    if len == 0 {
        return errno::EINVAL;
    }

    if prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
        return errno::EINVAL;
    }

    if flags & MAP_FIXED != 0 {
        return errno::ENOSYS;
    }

    if flags & (MAP_PRIVATE | MAP_SHARED) == 0 {
        return errno::EINVAL;
    }

    if flags & MAP_ANONYMOUS == 0 {
        return errno::ENOSYS;
    }

    // anonymous mmap에서는 fd가 -1이어야 한다.
    if fd != -1 {
        return errno::EINVAL;
    }

    let tid = proc::current_tid().unwrap_or(0);

    let page_size = crate::mm::page::PAGE_SIZE;
    let size = align_up(len, page_size);
    let pages = size / page_size;
    let phys_base = match crate::mm::page::alloc_frames(pages) {
        Some(v) => v,
        None => return errno::ENOMEM,
    };

    unsafe {
        // SAFETY: page allocator에서 할당받은 유효한 메모리 영역을 0으로 초기화한다.
        core::ptr::write_bytes(phys_base as *mut u8, 0, size);
    }

    #[cfg(target_arch = "aarch64")]
    let base = {
        let virt_base = NEXT_MMAP_BASE.fetch_add(size, Ordering::SeqCst);
        let virt_end = match virt_base.checked_add(size) {
            Some(end) => end,
            None => return errno::ENOMEM,
        };
        if virt_end >= crate::proc::user::USER_STACK_BASE {
            unsafe {
                crate::mm::page::free_frames(phys_base, pages);
            }
            return errno::ENOMEM;
        }

        let write = (prot & PROT_WRITE) != 0;
        let execute = (prot & PROT_EXEC) != 0;
        if map_user_contiguous(virt_base, phys_base, pages, write, execute).is_err() {
            unsafe {
                crate::mm::page::free_frames(phys_base, pages);
            }
            return errno::ENOMEM;
        }
        virt_base
    };

    #[cfg(not(target_arch = "aarch64"))]
    let base = phys_base;

    MMAP_REGIONS
        .lock()
        .push(MmapRegion { tid, base, phys_base, pages });
    base as isize
}

/// sys_munmap - 매핑 해제
///
/// 현재는 full unmap(전체 길이 해제)만 지원한다.
pub fn sys_munmap(addr: usize, len: usize) -> isize {
    if addr == 0 || len == 0 {
        return errno::EINVAL;
    }

    let tid = proc::current_tid().unwrap_or(0);

    let page_size = crate::mm::page::PAGE_SIZE;
    let pages = align_up(len, page_size) / page_size;

    let mut regions = MMAP_REGIONS.lock();
    let pos = match regions.iter().position(|m| m.tid == tid && m.base == addr) {
        Some(p) => p,
        None => return errno::EINVAL,
    };

    if regions[pos].pages != pages {
        return errno::EINVAL;
    }

    let region = regions.swap_remove(pos);
    unsafe {
        // SAFETY: mmap 등록 시 alloc_frames로 확보한 페이지 범위를 동일 크기로 해제한다.
        crate::mm::page::free_frames(region.phys_base, region.pages);
    }

    0
}

/// sys_execve - 현재 태스크를 새 유저 ELF 이미지로 교체
///
/// # Arguments
/// * `path` - 실행 파일 경로 (NUL 종단 C 문자열)
/// * `argv` - 인자 배열 (`char**`, 마지막은 NULL)
/// * `envp` - 환경변수 배열 (`char**`, 마지막은 NULL)
pub fn sys_execve(path: *const u8, argv: *const *const u8, envp: *const *const u8) -> isize {
    let path_str = match read_user_c_string(path, MAX_EXEC_PATH_LEN) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let (argv_list, argv_bytes) = match read_user_string_array(
        argv,
        MAX_EXEC_ARG_COUNT,
        MAX_EXEC_STR_LEN,
    ) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let (envp_list, envp_bytes) = match read_user_string_array(
        envp,
        MAX_EXEC_ENV_COUNT,
        MAX_EXEC_STR_LEN,
    ) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let arg_env_total = match argv_bytes.checked_add(envp_bytes) {
        Some(v) => v,
        None => return errno::E2BIG,
    };
    if arg_env_total > MAX_EXEC_ARG_ENV_TOTAL_BYTES {
        return errno::E2BIG;
    }

    let image = match proc::user::prepare_exec_image(&path_str, &argv_list, &envp_list) {
        Ok(img) => img,
        Err(e) => return exec_error_to_errno(e),
    };

    let tid = match proc::current_tid() {
        Some(t) => t,
        None => return errno::EPERM,
    };
    ensure_process_info_for_tid(tid);

    {
        let mut pending = PENDING_EXECS.lock();
        if let Some(pos) = pending.iter().position(|x| x.tid == tid) {
            pending.swap_remove(pos);
        }
        pending.push(PendingExec { tid, image });
    }

    kprintln!(
        "[syscall] execve queued for tid={} path='{}' argc={}",
        tid,
        path_str,
        argv_list.len()
    );

    // vfork 부모는 자식이 execve/exit 시점에 해제된다.
    complete_vfork_wait(tid);

    // trap 복귀 경로에서 컨텍스트를 교체한다.
    0
}

/// 현재 스레드에 대해 준비된 exec 전이 정보를 가져온다.
pub fn take_exec_transition_for_current() -> Option<ExecTransition> {
    let tid = proc::current_tid()?;
    let mut pending = PENDING_EXECS.lock();
    let pos = pending.iter().position(|x| x.tid == tid)?;
    let item = pending.swap_remove(pos);

    Some(ExecTransition {
        entry: item.image.entry,
        stack_top: item.image.stack_top,
        argc: item.image.argc,
        argv: item.image.argv,
        envp: item.image.envp,
        user_stack: item.image.user_stack,
    })
}

/// 테스트/디버그용: 현재 태스크의 pending signal 큐에 시그널을 삽입한다.
pub fn test_enqueue_signal_for_current(signum: u32) -> isize {
    let tid = current_tid_or_zero();
    ensure_process_info_for_tid(tid);
    enqueue_signal(tid, signum);
    0
}

fn exec_error_to_errno(err: proc::user::ExecError) -> isize {
    match err {
        proc::user::ExecError::NotFound => errno::ENOENT,
        proc::user::ExecError::IoError => errno::EIO,
        proc::user::ExecError::OutOfMemory => errno::ENOMEM,
        proc::user::ExecError::InvalidArgument => errno::EINVAL,
        proc::user::ExecError::InvalidElf
        | proc::user::ExecError::UnsupportedExecutableType
        | proc::user::ExecError::DynamicElfNotSupported => errno::ENOEXEC,
    }
}

fn read_user_string_array(
    list: *const *const u8,
    max_count: usize,
    max_str_len: usize,
) -> Result<(Vec<String>, usize), isize> {
    let mut out = Vec::new();
    let mut total_bytes = 0usize;
    if list.is_null() {
        return Ok((out, total_bytes));
    }

    let ptr_size = core::mem::size_of::<*const u8>();
    for i in 0..max_count {
        let slot = match (list as usize).checked_add(i * ptr_size) {
            Some(v) => v,
            None => return Err(errno::EFAULT),
        };
        validate_user_pointer(slot, ptr_size)?;

        let ptr = unsafe {
            // SAFETY: 사용자 포인터 배열의 현재 슬롯 범위를 먼저 검증했고, 그 범위를 비정렬 읽기한다.
            core::ptr::read_unaligned(slot as *const *const u8)
        };

        if ptr.is_null() {
            return Ok((out, total_bytes));
        }

        let s = read_user_c_string(ptr, max_str_len)?;
        total_bytes = match total_bytes.checked_add(s.len() + 1) {
            Some(v) => v,
            None => return Err(errno::E2BIG),
        };
        out.push(s);
    }

    // NULL 종단 없이 max_count를 초과하면 인자 크기 초과로 본다.
    Err(errno::E2BIG)
}

fn read_user_c_string(ptr: *const u8, max_len: usize) -> Result<String, isize> {
    if ptr.is_null() {
        return Err(errno::EFAULT);
    }

    let mut len = 0usize;
    loop {
        if len >= max_len {
            return Err(errno::E2BIG);
        }

        let byte_addr = match (ptr as usize).checked_add(len) {
            Some(v) => v,
            None => return Err(errno::EFAULT),
        };
        validate_user_pointer(byte_addr, 1)?;

        let byte = unsafe {
            // SAFETY: 사용자 주소 범위를 검증한 뒤 1바이트를 읽는다.
            core::ptr::read(byte_addr as *const u8)
        };
        if byte == 0 {
            break;
        }
        len += 1;
    }

    validate_user_pointer(ptr as usize, len)?;

    let bytes = unsafe {
        // SAFETY: 위 루프에서 NUL 종단 길이를 계산했고, 동일 범위를 read-only slice로 변환한다.
        core::slice::from_raw_parts(ptr, len)
    };

    let s = core::str::from_utf8(bytes).map_err(|_| errno::EINVAL)?;
    Ok(String::from(s))
}

#[inline]
fn validate_user_pointer(ptr: usize, len: usize) -> Result<(), isize> {
    if user_pointer_in_range(ptr, len) {
        Ok(())
    } else {
        Err(errno::EFAULT)
    }
}

#[inline]
fn user_pointer_in_range(ptr: usize, len: usize) -> bool {
    if ptr == 0 {
        return false;
    }

    if len == 0 {
        return true;
    }

    let Some(end) = ptr.checked_add(len) else {
        return false;
    };

    #[cfg(target_arch = "aarch64")]
    {
        ptr >= MIN_USER_VADDR && end <= MAX_USER_VADDR_EXCLUSIVE
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        ptr >= 0x1000 && end > ptr
    }
}
