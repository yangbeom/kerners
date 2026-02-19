//! 프로세스 관련 시스템 콜
//!
//! exit, yield, getpid, execve 등

use super::errno;
use crate::fs;
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
use crate::fs::fd;
use crate::kprintln;
use crate::proc;
use crate::sync::Mutex;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

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

#[derive(Clone)]
struct BrkRegion {
    vm_group: u64,
    base: usize,
    current: usize,
    limit: usize,
    direct_phys: bool,
    pages: Vec<Option<usize>>,
}

#[derive(Clone)]
struct FileMapBacking {
    vnode: Arc<dyn fs::VNode>,
    stable_id: u64,
    file_offset: usize,
    map_len: usize,
    shared: bool,
}

#[derive(Clone)]
enum MmapBacking {
    Anonymous,
    File(FileMapBacking),
}

#[derive(Clone)]
struct MmapRegion {
    vm_group: u64,
    base: usize,
    len: usize,
    requested_len: usize,
    prot: usize,
    flags: usize,
    direct_phys: bool,
    pages: Vec<usize>,
    backing: MmapBacking,
}

struct VmSpace {
    vm_group: u64,
    root_table: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CowOrigin {
    Fork,
    PrivateMap,
}

#[derive(Clone, Copy)]
struct CowMeta {
    vm_group: u64,
    addr: usize,
    frame: usize,
    execute: bool,
    origin: CowOrigin,
}

struct FilePageCacheEntry {
    stable_id: u64,
    page_index: usize,
    frame: usize,
    vnode: Arc<dyn fs::VNode>,
    dirty: bool,
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
    sigtimedwait_mask: u64,
    pending_signals: Vec<u32>,
    exit_signal: u32,
}

#[derive(Clone, Copy)]
struct SignalActionGroup {
    sighand_group: u64,
    actions: [LinuxSigAction; MAX_SIGNAL_COUNT],
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

#[cfg(target_arch = "riscv64")]
#[repr(C)]
#[derive(Clone, Copy)]
struct ForkChildContext {
    gpr: [u64; 32],
    mstatus: u64,
    mepc: u64,
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
struct PendingForkChild {
    tid: proc::Tid,
    ready: bool,
    context: ForkChildContext,
}

#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy)]
struct PendingSigReturnAarch64 {
    tid: proc::Tid,
    gpr: [u64; 31],
    elr: u64,
    spsr: u64,
    sp_el0: u64,
    signal_mask: u64,
}

#[cfg(target_arch = "riscv64")]
#[derive(Clone, Copy)]
struct PendingSigReturnRiscv {
    tid: proc::Tid,
    gpr: [u64; 32],
    mstatus: u64,
    mepc: u64,
    signal_mask: u64,
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

/// `/proc/[pid]/status`용 상태 스냅샷
#[derive(Clone)]
pub struct ProcStatusSnapshot {
    pub tid: proc::Tid,
    pub parent_tid: proc::Tid,
    pub pgid: proc::Tid,
    pub sid: proc::Tid,
    pub vm_group: u64,
    pub signal_mask: u64,
    pub pending_signals: Vec<u32>,
}

/// `/proc/[pid]/maps`용 매핑 스냅샷
#[derive(Clone, Copy)]
pub struct ProcMapSnapshot {
    pub start: usize,
    pub end: usize,
    pub prot: usize,
    pub shared: bool,
    pub file_backed: bool,
}

/// 스레드별 pending exec 리스트
static PENDING_EXECS: Mutex<Vec<PendingExec>> = Mutex::new(Vec::new());
static BRK_REGIONS: Mutex<Vec<BrkRegion>> = Mutex::new(Vec::new());
static MMAP_REGIONS: Mutex<Vec<MmapRegion>> = Mutex::new(Vec::new());
static VM_SPACES: Mutex<Vec<VmSpace>> = Mutex::new(Vec::new());
static COW_PAGES: Mutex<Vec<CowMeta>> = Mutex::new(Vec::new());
static FILE_PAGE_CACHE: Mutex<Vec<FilePageCacheEntry>> = Mutex::new(Vec::new());
static ZOMBIE_CHILDREN: Mutex<Vec<ZombieChild>> = Mutex::new(Vec::new());
static PROCESS_INFOS: Mutex<Vec<ProcessInfo>> = Mutex::new(Vec::new());
static SIGNAL_ACTION_GROUPS: Mutex<Vec<SignalActionGroup>> = Mutex::new(Vec::new());
static VFORK_WAITS: Mutex<Vec<VforkWait>> = Mutex::new(Vec::new());
static NEXT_FAKE_CHILD_TID: AtomicUsize = AtomicUsize::new(1000);
static NEXT_RESOURCE_GROUP_ID: AtomicUsize = AtomicUsize::new(2);
static COW_FORK_TEST_REPORTED: AtomicUsize = AtomicUsize::new(0);
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
static PENDING_FORK_CHILDREN: Mutex<Vec<PendingForkChild>> = Mutex::new(Vec::new());
#[cfg(target_arch = "aarch64")]
static PENDING_SIGRETURN_AARCH64: Mutex<Vec<PendingSigReturnAarch64>> = Mutex::new(Vec::new());
#[cfg(target_arch = "riscv64")]
static PENDING_SIGRETURN_RISCV64: Mutex<Vec<PendingSigReturnRiscv>> = Mutex::new(Vec::new());

const BRK_REGION_SIZE: usize = 16 * 1024 * 1024; // 16MB (static BusyBox init baseline)
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const BRK_REGION_BASE: usize = 0x2000_0000;
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const MMAP_REGION_BASE: usize = 0x3000_0000;
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const MMAP_REGION_END: usize =
    crate::proc::user::USER_STACK_BASE - crate::proc::user::USER_STACK_SIZE;

const PROT_READ: usize = 0x1;
const PROT_WRITE: usize = 0x2;
const PROT_EXEC: usize = 0x4;
const MAP_TYPE_MASK: usize = 0x0f;
const MAP_SHARED: usize = 0x01;
const MAP_PRIVATE: usize = 0x02;
const MAP_FIXED: usize = 0x10;
const MAP_ANONYMOUS: usize = 0x20;
const CLONE_VM: usize = 0x00000100;
const CLONE_FS: usize = 0x00000200;
const CLONE_FILES: usize = 0x00000400;
const CLONE_SIGHAND: usize = 0x00000800;
const CLONE_VFORK: usize = 0x00004000;
const CLONE_PARENT_SETTID: usize = 0x00100000;
const CLONE_CHILD_SETTID: usize = 0x01000000;
const CLONE_CSIGNAL_MASK: usize = 0x000000ff;
const WNOHANG: i32 = 0x1;
const WEXITED: i32 = 0x4;
const WNOWAIT: i32 = 0x0100_0000;
const WAITID_IDTYPE_ALL: i32 = 0;
const WAITID_IDTYPE_PID: i32 = 1;
const WAITID_IDTYPE_PGID: i32 = 2;
const SIGNAL_SIGKILL: u32 = 9;
const SIGNAL_SIGSEGV: u32 = 11;
const SIGNAL_SIGTERM: u32 = 15;
const SIGNAL_SIGCHLD: u32 = 17;
const SIGNAL_SIGCONT: u32 = 18;
const SIGNAL_SIGSTOP: u32 = 19;
const SIGINFO_CLD_EXITED: i32 = 1;
const SIGINFO_CLD_KILLED: i32 = 2;
const SIG_BLOCK: i32 = 0;
const SIG_UNBLOCK: i32 = 1;
const SIG_SETMASK: i32 = 2;
const MIN_SIGSET_SIZE: usize = core::mem::size_of::<u64>();
const MAX_SIGNAL_COUNT: usize = 64;
const SIG_DFL: u64 = 0;
const SIG_IGN: u64 = 1;
const SA_SIGINFO: u64 = 0x0000_0004;
const SA_NODEFER: u64 = 0x4000_0000;
const SA_RESTART: u64 = 0x1000_0000;
const SIGFRAME_MAGIC: u64 = 0x5349_4746_5241_4d45;

const CLOCK_REALTIME: i32 = 0;
const CLOCK_MONOTONIC: i32 = 1;

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const MIN_USER_VADDR: usize = 0x1000;
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const MAX_USER_VADDR_EXCLUSIVE: usize = crate::proc::user::USER_STACK_BASE;

#[inline]
const fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

#[inline]
fn page_count_for_len(len: usize, page_size: usize) -> usize {
    align_up(len, page_size) / page_size
}

#[cfg(target_arch = "riscv64")]
#[inline]
fn use_riscv_kernel_direct_vm() -> bool {
    !super::in_user_syscall_context()
}

#[cfg(not(target_arch = "riscv64"))]
#[inline]
fn use_riscv_kernel_direct_vm() -> bool {
    false
}

#[inline]
fn mmap_region_end(region: &MmapRegion) -> usize {
    region.base + region.len
}

fn mmap_overlaps(region: &MmapRegion, start: usize, end: usize) -> bool {
    region.base < end && start < mmap_region_end(region)
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
struct LinuxSigAction {
    sa_handler: u64,
    sa_flags: u64,
    sa_restorer: u64,
    sa_mask: u64,
}

impl LinuxSigAction {
    const fn empty() -> Self {
        Self {
            sa_handler: SIG_DFL,
            sa_flags: 0,
            sa_restorer: 0,
            sa_mask: 0,
        }
    }
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

#[cfg(target_arch = "aarch64")]
#[repr(C)]
#[derive(Clone, Copy)]
struct KernelSigFrameAarch64 {
    magic: u64,
    signum: u64,
    old_mask: u64,
    _pad: u64,
    gpr: [u64; 31],
    elr: u64,
    spsr: u64,
    sp_el0: u64,
}

#[cfg(target_arch = "riscv64")]
#[repr(C)]
#[derive(Clone, Copy)]
struct KernelSigFrameRiscv64 {
    magic: u64,
    signum: u64,
    old_mask: u64,
    _pad: u64,
    gpr: [u64; 32],
    mstatus: u64,
    mepc: u64,
}

#[inline]
fn monotonic_time() -> (u64, u64) {
    let ns = crate::time::monotonic_now_ns();
    (ns / 1_000_000_000, ns % 1_000_000_000)
}

#[inline]
fn realtime_time() -> (u64, u64) {
    let ns = crate::time::realtime_now_ns();
    (ns / 1_000_000_000, ns % 1_000_000_000)
}

#[inline]
fn alloc_zeroed_frame() -> Option<usize> {
    let frame = crate::mm::page::alloc_frame()?;
    unsafe {
        // SAFETY: alloc_frame로 확보한 유효한 4KB 프레임을 zero-fill 한다.
        core::ptr::write_bytes(frame as *mut u8, 0, crate::mm::page::PAGE_SIZE);
    }
    Some(frame)
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn reprotect_user_pages_noflush(
    virt_base: usize,
    pages: usize,
    write: bool,
    execute: bool,
) -> Result<(), isize> {
    let page_size = crate::mm::page::PAGE_SIZE;
    for i in 0..pages {
        let va = virt_base + i * page_size;
        if crate::arch::mmu::update_user_page_flags_noflush(va, write, execute).is_err() {
            return Err(errno::EINVAL);
        }
    }
    Ok(())
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
#[inline]
fn flush_user_tlb() {
    crate::arch::mmu::flush_tlb_all();
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
#[inline]
fn flush_user_tlb() {}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
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

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(
    r#"
.section .text
.global fork_child_enter_user
.type fork_child_enter_user, @function
fork_child_enter_user:
    // a0 = *ForkChildContext
    mv t6, a0

    // mret 복귀 컨텍스트 복원
    ld t0, 256(t6)   // mstatus
    ld t1, 264(t6)   // mepc
    csrw mstatus, t0
    csrw mepc, t1

    // 사용자 GPR 복원 (x0 제외)
    ld x1, 8(t6)
    ld x2, 16(t6)
    ld x3, 24(t6)
    ld x4, 32(t6)
    ld x5, 40(t6)
    ld x6, 48(t6)
    ld x7, 56(t6)
    ld x8, 64(t6)
    ld x9, 72(t6)
    ld x10, 80(t6)
    ld x11, 88(t6)
    ld x12, 96(t6)
    ld x13, 104(t6)
    ld x14, 112(t6)
    ld x15, 120(t6)
    ld x16, 128(t6)
    ld x17, 136(t6)
    ld x18, 144(t6)
    ld x19, 152(t6)
    ld x20, 160(t6)
    ld x21, 168(t6)
    ld x22, 176(t6)
    ld x23, 184(t6)
    ld x24, 192(t6)
    ld x25, 200(t6)
    ld x26, 208(t6)
    ld x27, 216(t6)
    ld x28, 224(t6)
    ld x29, 232(t6)
    ld x30, 240(t6)
    ld x31, 248(t6)
    mret
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

    let default_group = if tid == 0 { 0 } else { tid };

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
        sigtimedwait_mask: 0,
        pending_signals: Vec::new(),
        exit_signal: 0,
    });
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        let _ = ensure_vm_space_root(default_group);
    }
    processes.len() - 1
}

fn ensure_process_info_for_tid(tid: proc::Tid) {
    let mut processes = PROCESS_INFOS.lock();
    let _ = ensure_process_info_for_tid_locked(&mut processes, tid);
}

fn current_tid_or_zero() -> proc::Tid {
    proc::current_tid().unwrap_or(0)
}

fn vm_group_for_tid(tid: proc::Tid) -> u64 {
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    processes[idx].vm_group
}

fn current_vm_group() -> u64 {
    vm_group_for_tid(current_tid_or_zero())
}

/// tid에 해당하는 프로세스 상태 스냅샷을 반환한다.
pub fn proc_status_snapshot(tid: proc::Tid) -> Option<ProcStatusSnapshot> {
    let processes = PROCESS_INFOS.lock();
    let idx = processes.iter().position(|p| p.tid == tid)?;
    Some(ProcStatusSnapshot {
        tid: processes[idx].tid,
        parent_tid: processes[idx].parent_tid,
        pgid: processes[idx].pgid,
        sid: processes[idx].sid,
        vm_group: processes[idx].vm_group,
        signal_mask: processes[idx].signal_mask,
        pending_signals: processes[idx].pending_signals.clone(),
    })
}

/// tid에 해당하는 프로세스 가상 메모리 매핑 스냅샷을 반환한다.
pub fn proc_maps_snapshot(tid: proc::Tid) -> Vec<ProcMapSnapshot> {
    let vm_group = {
        let processes = PROCESS_INFOS.lock();
        let Some(idx) = processes.iter().position(|p| p.tid == tid) else {
            return Vec::new();
        };
        processes[idx].vm_group
    };

    let mut maps = Vec::new();

    {
        let brk_regions = BRK_REGIONS.lock();
        for region in brk_regions.iter() {
            if region.vm_group == vm_group && region.current > region.base {
                maps.push(ProcMapSnapshot {
                    start: region.base,
                    end: region.current,
                    prot: PROT_READ | PROT_WRITE,
                    shared: false,
                    file_backed: false,
                });
            }
        }
    }

    {
        let mmap_regions = MMAP_REGIONS.lock();
        for region in mmap_regions.iter() {
            if region.vm_group != vm_group {
                continue;
            }
            maps.push(ProcMapSnapshot {
                start: region.base,
                end: region.base.saturating_add(region.len),
                prot: region.prot,
                shared: region.flags & MAP_SHARED != 0,
                file_backed: matches!(region.backing, MmapBacking::File(_)),
            });
        }
    }

    maps.sort_by_key(|m| m.start);
    maps
}

fn process_count_in_vm_group(vm_group: u64) -> usize {
    let processes = PROCESS_INFOS.lock();
    processes
        .iter()
        .filter(|process| process.vm_group == vm_group)
        .count()
}

fn process_count_in_sighand_group(sighand_group: u64) -> usize {
    let processes = PROCESS_INFOS.lock();
    processes
        .iter()
        .filter(|process| process.sighand_group == sighand_group)
        .count()
}

fn sighand_group_for_tid(tid: proc::Tid) -> u64 {
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    processes[idx].sighand_group
}

fn ensure_signal_action_group_locked(
    groups: &mut Vec<SignalActionGroup>,
    sighand_group: u64,
) -> usize {
    if let Some(pos) = groups
        .iter()
        .position(|group| group.sighand_group == sighand_group)
    {
        return pos;
    }
    groups.push(SignalActionGroup {
        sighand_group,
        actions: [LinuxSigAction::empty(); MAX_SIGNAL_COUNT],
    });
    groups.len() - 1
}

fn signal_action_for_group(sighand_group: u64, signum: u32) -> LinuxSigAction {
    if signum == 0 || signum > MAX_SIGNAL_COUNT as u32 {
        return LinuxSigAction::empty();
    }

    let mut groups = SIGNAL_ACTION_GROUPS.lock();
    let idx = ensure_signal_action_group_locked(&mut groups, sighand_group);
    groups[idx].actions[(signum - 1) as usize]
}

fn set_signal_action_for_group(sighand_group: u64, signum: u32, action: LinuxSigAction) {
    if signum == 0 || signum > MAX_SIGNAL_COUNT as u32 {
        return;
    }

    let mut groups = SIGNAL_ACTION_GROUPS.lock();
    let idx = ensure_signal_action_group_locked(&mut groups, sighand_group);
    groups[idx].actions[(signum - 1) as usize] = action;
}

fn clone_signal_actions_if_needed(parent_group: u64, child_group: u64) {
    if parent_group == child_group {
        return;
    }

    let mut groups = SIGNAL_ACTION_GROUPS.lock();
    let parent_idx = ensure_signal_action_group_locked(&mut groups, parent_group);
    let child_idx = ensure_signal_action_group_locked(&mut groups, child_group);
    groups[child_idx].actions = groups[parent_idx].actions;
}

fn remove_signal_actions_if_unused(sighand_group: u64) {
    if process_count_in_sighand_group(sighand_group) != 0 {
        return;
    }
    let mut groups = SIGNAL_ACTION_GROUPS.lock();
    if let Some(pos) = groups
        .iter()
        .position(|group| group.sighand_group == sighand_group)
    {
        groups.swap_remove(pos);
    }
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn ensure_vm_space_root(vm_group: u64) -> usize {
    let mut spaces = VM_SPACES.lock();
    if let Some(space) = spaces.iter().find(|s| s.vm_group == vm_group) {
        return space.root_table;
    }
    let root =
        crate::proc::current_user_root_table().unwrap_or(crate::arch::mmu::current_root_table());
    spaces.push(VmSpace {
        vm_group,
        root_table: root,
    });
    root
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn vm_root_for_group(vm_group: u64) -> usize {
    ensure_vm_space_root(vm_group)
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn set_vm_root_for_group(vm_group: u64, root_table: usize) {
    let mut spaces = VM_SPACES.lock();
    if let Some(space) = spaces.iter_mut().find(|s| s.vm_group == vm_group) {
        space.root_table = root_table;
        return;
    }
    spaces.push(VmSpace {
        vm_group,
        root_table,
    });
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn vm_root_for_group_if_present(vm_group: u64) -> Option<usize> {
    let spaces = VM_SPACES.lock();
    spaces
        .iter()
        .find(|space| space.vm_group == vm_group)
        .map(|space| space.root_table)
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn remove_vm_space(vm_group: u64) {
    let mut spaces = VM_SPACES.lock();
    if let Some(pos) = spaces.iter().position(|space| space.vm_group == vm_group) {
        spaces.swap_remove(pos);
    }
}

fn flush_shared_writeback_for_vm_group(vm_group: u64) {
    let regions = MMAP_REGIONS.lock();
    for region in regions.iter().filter(|region| region.vm_group == vm_group) {
        let _ = flush_file_region_pages(region, 0, region.pages.len());
    }
}

fn cleanup_vm_group_resources(vm_group: u64) {
    flush_shared_writeback_for_vm_group(vm_group);

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    let root_table = vm_root_for_group_if_present(vm_group);
    let page_size = crate::mm::page::PAGE_SIZE;

    {
        let mut brk_regions = BRK_REGIONS.lock();
        let mut i = 0usize;
        while i < brk_regions.len() {
            if brk_regions[i].vm_group != vm_group {
                i += 1;
                continue;
            }
            let region = brk_regions.swap_remove(i);
            for (idx, frame_opt) in region.pages.iter().enumerate() {
                let Some(frame) = *frame_opt else {
                    continue;
                };
                #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
                if !region.direct_phys {
                    if let Some(root) = root_table {
                        let va = region.base + idx * page_size;
                        let _ = crate::arch::mmu::unmap_user_page_for_root_noflush(root, va);
                    }
                }
                remove_cow_meta(vm_group, region.base + idx * page_size);
                unsafe {
                    // SAFETY: vm_group에서 소유하던 frame 참조를 해제한다.
                    crate::mm::page::free_frame(frame);
                }
            }
        }
    }

    {
        let mut mmap_regions = MMAP_REGIONS.lock();
        let mut i = 0usize;
        while i < mmap_regions.len() {
            if mmap_regions[i].vm_group != vm_group {
                i += 1;
                continue;
            }
            let region = mmap_regions.swap_remove(i);
            for (idx, frame) in region.pages.iter().copied().enumerate() {
                let va = region.base + idx * page_size;
                #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
                if !region.direct_phys {
                    if let Some(root) = root_table {
                        let _ = crate::arch::mmu::unmap_user_page_for_root_noflush(root, va);
                    }
                }
                remove_cow_meta(vm_group, va);
                unsafe {
                    // SAFETY: vm_group에서 소유하던 frame 참조를 해제한다.
                    crate::mm::page::free_frame(frame);
                }
            }
        }
    }

    remove_cow_for_vm_group(vm_group);

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        if root_table.is_some() {
            flush_user_tlb();
        }
        remove_vm_space(vm_group);
    }
}

fn set_cow_meta(vm_group: u64, addr: usize, frame: usize, execute: bool, origin: CowOrigin) {
    let mut cows = COW_PAGES.lock();
    if let Some(item) = cows
        .iter_mut()
        .find(|item| item.vm_group == vm_group && item.addr == addr)
    {
        item.frame = frame;
        item.execute = execute;
        item.origin = origin;
        return;
    }
    cows.push(CowMeta {
        vm_group,
        addr,
        frame,
        execute,
        origin,
    });
}

fn remove_cow_meta(vm_group: u64, addr: usize) {
    let mut cows = COW_PAGES.lock();
    if let Some(pos) = cows
        .iter()
        .position(|item| item.vm_group == vm_group && item.addr == addr)
    {
        cows.swap_remove(pos);
    }
}

fn remove_cow_for_vm_group(vm_group: u64) {
    let mut cows = COW_PAGES.lock();
    cows.retain(|item| item.vm_group != vm_group);
}

fn mark_file_cache_dirty(stable_id: u64, page_index: usize) {
    let mut cache = FILE_PAGE_CACHE.lock();
    if let Some(item) = cache
        .iter_mut()
        .find(|entry| entry.stable_id == stable_id && entry.page_index == page_index)
    {
        item.dirty = true;
    }
}

fn clear_file_cache_dirty(stable_id: u64, page_index: usize) {
    let mut cache = FILE_PAGE_CACHE.lock();
    if let Some(item) = cache
        .iter_mut()
        .find(|entry| entry.stable_id == stable_id && entry.page_index == page_index)
    {
        item.dirty = false;
    }
}

fn get_or_create_file_cache_page(
    vnode: &Arc<dyn fs::VNode>,
    stable_id: u64,
    page_index: usize,
    file_offset: usize,
) -> Result<usize, isize> {
    {
        let cache = FILE_PAGE_CACHE.lock();
        if let Some(item) = cache
            .iter()
            .find(|entry| entry.stable_id == stable_id && entry.page_index == page_index)
        {
            let _ = crate::mm::page::retain_frame(item.frame);
            return Ok(item.frame);
        }
    }

    let frame = match alloc_zeroed_frame() {
        Some(frame) => frame,
        None => return Err(errno::ENOMEM),
    };

    let read_buf = unsafe {
        // SAFETY: frame은 유효한 4KB 페이지 프레임이며 임시 read 버퍼로 사용한다.
        core::slice::from_raw_parts_mut(frame as *mut u8, crate::mm::page::PAGE_SIZE)
    };
    if vnode.read(file_offset, read_buf).is_err() {
        unsafe {
            // SAFETY: 방금 할당한 frame을 실패 경로에서 반환한다.
            crate::mm::page::free_frame(frame);
        }
        return Err(errno::EIO);
    }

    {
        let mut cache = FILE_PAGE_CACHE.lock();
        cache.push(FilePageCacheEntry {
            stable_id,
            page_index,
            frame,
            vnode: vnode.clone(),
            dirty: false,
        });
    }

    let _ = crate::mm::page::retain_frame(frame);
    Ok(frame)
}

fn flush_file_region_pages(
    region: &MmapRegion,
    start_page: usize,
    pages: usize,
) -> Result<(), isize> {
    let MmapBacking::File(backing) = &region.backing else {
        return Ok(());
    };
    if !backing.shared {
        return Ok(());
    }

    for rel in 0..pages {
        let page = start_page + rel;
        if page >= region.pages.len() {
            break;
        }
        let page_start = page * crate::mm::page::PAGE_SIZE;
        if page_start >= backing.map_len {
            break;
        }
        let bytes = core::cmp::min(crate::mm::page::PAGE_SIZE, backing.map_len - page_start);
        let write_off = backing.file_offset + page_start;
        let frame = region.pages[page];
        let buf = unsafe {
            // SAFETY: frame은 유효한 매핑 프레임이며 writeback 시 읽기 전용으로 참조한다.
            core::slice::from_raw_parts(frame as *const u8, bytes)
        };
        let page_index = backing.file_offset / crate::mm::page::PAGE_SIZE + page;
        mark_file_cache_dirty(backing.stable_id, page_index);
        match backing.vnode.write(write_off, buf) {
            Ok(n) if n == bytes => {}
            _ => return Err(errno::EIO),
        }
        clear_file_cache_dirty(backing.stable_id, page_index);
    }

    Ok(())
}

fn replace_vm_page_frame(vm_group: u64, va: usize, new_frame: usize) {
    let page_size = crate::mm::page::PAGE_SIZE;
    {
        let mut brk = BRK_REGIONS.lock();
        for region in brk.iter_mut() {
            if region.vm_group != vm_group {
                continue;
            }
            if va < region.base || va >= region.limit {
                continue;
            }
            let idx = (va - region.base) / page_size;
            if idx < region.pages.len() {
                region.pages[idx] = Some(new_frame);
                return;
            }
        }
    }

    let mut mmaps = MMAP_REGIONS.lock();
    for region in mmaps.iter_mut() {
        if region.vm_group != vm_group {
            continue;
        }
        if va < region.base || va >= mmap_region_end(region) {
            continue;
        }
        let idx = (va - region.base) / page_size;
        if idx < region.pages.len() {
            region.pages[idx] = new_frame;
            return;
        }
    }
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn setup_cow_pair(
    parent_vm_group: u64,
    child_vm_group: u64,
    parent_root: usize,
    child_root: usize,
    va: usize,
    frame: usize,
    execute: bool,
) -> Result<(), isize> {
    if crate::arch::mmu::update_user_page_flags_for_root_noflush(parent_root, va, false, execute)
        .is_err()
    {
        return Err(errno::EINVAL);
    }
    if crate::arch::mmu::update_user_page_flags_for_root_noflush(child_root, va, false, execute)
        .is_err()
    {
        return Err(errno::EINVAL);
    }
    set_cow_meta(parent_vm_group, va, frame, execute, CowOrigin::Fork);
    set_cow_meta(child_vm_group, va, frame, execute, CowOrigin::Fork);
    Ok(())
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn handle_user_page_fault_write(far: usize) -> bool {
    let page_size = crate::mm::page::PAGE_SIZE;
    let va = far & !(page_size - 1);
    if va < MIN_USER_VADDR || va >= MAX_USER_VADDR_EXCLUSIVE {
        return false;
    }

    let current_vm_group = current_vm_group();
    let active_root =
        crate::proc::current_user_root_table().unwrap_or(crate::arch::mmu::current_root_table());
    let (vm_group, cow) = {
        let cows = COW_PAGES.lock();
        if let Some(item) = cows
            .iter()
            .find(|item| item.vm_group == current_vm_group && item.addr == va)
            .copied()
        {
            (item.vm_group, Some(item))
        } else {
            let mut fallback: Option<CowMeta> = None;
            for item in cows.iter().filter(|item| item.addr == va) {
                if vm_root_for_group_if_present(item.vm_group) == Some(active_root) {
                    fallback = Some(*item);
                    break;
                }
            }
            (
                fallback
                    .map(|item| item.vm_group)
                    .unwrap_or(current_vm_group),
                fallback,
            )
        }
    };
    let Some(cow) = cow else {
        return false;
    };

    let root_table = vm_root_for_group(vm_group);
    let mapped_frame = match crate::arch::mmu::get_user_page_phys_for_root(root_table, va) {
        Ok(frame) => frame,
        Err(_) => return false,
    };
    let source_frame = if mapped_frame != 0 {
        mapped_frame
    } else {
        cow.frame
    };
    let refcount = crate::mm::page::frame_refcount(source_frame);
    if refcount == 0 {
        return false;
    }

    if refcount > 1 {
        let new_frame = match alloc_zeroed_frame() {
            Some(frame) => frame,
            None => return false,
        };
        unsafe {
            // SAFETY: source/new frame은 모두 PAGE_SIZE 크기의 유효한 프레임이다.
            core::ptr::copy_nonoverlapping(
                source_frame as *const u8,
                new_frame as *mut u8,
                page_size,
            );
        }
        if crate::arch::mmu::map_user_page_for_root_noflush(
            root_table,
            va,
            new_frame,
            true,
            cow.execute,
        )
        .is_err()
        {
            unsafe {
                // SAFETY: map 실패 시 새 프레임만 즉시 반납한다.
                crate::mm::page::free_frame(new_frame);
            }
            return false;
        }
        replace_vm_page_frame(vm_group, va, new_frame);
        unsafe {
            // SAFETY: 기존 공유 프레임에 대한 현재 매핑 참조를 해제한다.
            crate::mm::page::free_frame(source_frame);
        }
    } else if crate::arch::mmu::update_user_page_flags_for_root_noflush(
        root_table,
        va,
        true,
        cow.execute,
    )
    .is_err()
    {
        return false;
    }

    if cow.origin == CowOrigin::Fork
        && COW_FORK_TEST_REPORTED
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        kprintln!(
            "COW_FORK_TEST: PASS (tid={}, va={:#x}, refcount_before={})",
            current_tid_or_zero(),
            va,
            refcount
        );
    }

    remove_cow_meta(vm_group, va);
    flush_user_tlb();
    true
}

#[cfg(target_arch = "aarch64")]
pub fn handle_user_page_fault_aarch64(far: usize, esr: u64) -> bool {
    const ISS_WNR_BIT: u64 = 1 << 6;
    if (esr & ISS_WNR_BIT) == 0 {
        return false;
    }
    handle_user_page_fault_write(far)
}

#[cfg(target_arch = "riscv64")]
pub fn handle_user_page_fault_riscv64(far: usize, cause: u64) -> bool {
    const STORE_PAGE_FAULT: u64 = 15;
    if cause != STORE_PAGE_FAULT {
        return false;
    }
    handle_user_page_fault_write(far)
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

fn signal_is_unmaskable(signum: u32) -> bool {
    signum == SIGNAL_SIGKILL || signum == SIGNAL_SIGSTOP
}

fn signal_is_blocked(mask: u64, signum: u32) -> bool {
    if signal_is_unmaskable(signum) {
        false
    } else {
        signal_mask_contains(mask, signum)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DefaultSignalDisposition {
    Ignore,
    Terminate,
    Stop,
    Continue,
}

fn default_signal_disposition(signum: u32) -> DefaultSignalDisposition {
    match signum {
        SIGNAL_SIGCHLD => DefaultSignalDisposition::Ignore,
        SIGNAL_SIGSTOP => DefaultSignalDisposition::Stop,
        SIGNAL_SIGCONT => DefaultSignalDisposition::Continue,
        SIGNAL_SIGKILL | SIGNAL_SIGTERM | SIGNAL_SIGSEGV => DefaultSignalDisposition::Terminate,
        _ => DefaultSignalDisposition::Terminate,
    }
}

fn set_sigtimedwait_mask_for_tid(tid: proc::Tid, mask: u64) {
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    let unwaitable = signal_to_mask(SIGNAL_SIGKILL) | signal_to_mask(SIGNAL_SIGSTOP);
    processes[idx].sigtimedwait_mask = mask & !unwaitable;
}

fn clear_sigtimedwait_mask_for_tid(tid: proc::Tid) {
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    processes[idx].sigtimedwait_mask = 0;
}

fn enqueue_signal(tid: proc::Tid, signum: u32) {
    if signal_to_mask(signum) == 0 {
        return;
    }

    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    let mask = processes[idx].signal_mask;
    let sigtimedwait_mask = processes[idx].sigtimedwait_mask;

    if signum == SIGNAL_SIGCONT {
        processes[idx]
            .pending_signals
            .retain(|&pending| pending != SIGNAL_SIGSTOP);
    } else if signum == SIGNAL_SIGSTOP {
        processes[idx]
            .pending_signals
            .retain(|&pending| pending != SIGNAL_SIGCONT);
    }

    processes[idx].pending_signals.push(signum);
    let wake_for_sigtimedwait = signal_mask_contains(sigtimedwait_mask, signum);
    let wake_for_signal =
        wake_for_sigtimedwait || !signal_is_blocked(mask, signum) || signum == SIGNAL_SIGCONT;
    drop(processes);

    if wake_for_signal {
        let _ = proc::wake_thread_for_signal(tid);
    }
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

fn has_unmasked_pending_signal(tid: proc::Tid) -> bool {
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    let mask = processes[idx].signal_mask;
    processes[idx]
        .pending_signals
        .iter()
        .copied()
        .any(|signum| !signal_is_blocked(mask, signum))
}

fn pop_pending_unmasked_signal(tid: proc::Tid) -> Option<u32> {
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    let mask = processes[idx].signal_mask;
    let pos = processes[idx]
        .pending_signals
        .iter()
        .position(|&signum| !signal_is_blocked(mask, signum))?;
    Some(processes[idx].pending_signals.remove(pos))
}

fn set_signal_mask_for_tid(tid: proc::Tid, mask: u64) {
    let mut processes = PROCESS_INFOS.lock();
    let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
    let unmaskable = signal_to_mask(SIGNAL_SIGKILL) | signal_to_mask(SIGNAL_SIGSTOP);
    processes[idx].signal_mask = mask & !unmaskable;
}

fn prepare_pending_signal_delivery(tid: proc::Tid) -> Option<(u32, LinuxSigAction, u64)> {
    loop {
        let signum = pop_pending_unmasked_signal(tid)?;
        let sighand_group = sighand_group_for_tid(tid);
        let action = signal_action_for_group(sighand_group, signum);

        if action.sa_handler == SIG_IGN {
            continue;
        }

        if action.sa_handler == SIG_DFL {
            match default_signal_disposition(signum) {
                DefaultSignalDisposition::Ignore | DefaultSignalDisposition::Continue => {
                    continue;
                }
                DefaultSignalDisposition::Stop => {
                    let _ = proc::block_thread_for_signal_stop(tid);
                    proc::yield_now();
                    continue;
                }
                DefaultSignalDisposition::Terminate => {
                    finalize_exit_by_signal(tid, signum);
                    kprintln!(
                        "[signal] tid={} default action terminate by signal {}",
                        tid,
                        signum
                    );
                    if tid == current_tid_or_zero() {
                        proc::exit();
                    }
                    let _ = proc::terminate_thread_for_signal(tid);
                    return None;
                }
            }
        }

        let old_mask = {
            let mut processes = PROCESS_INFOS.lock();
            let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
            let old = processes[idx].signal_mask;
            let mut next_mask = old | action.sa_mask;
            if (action.sa_flags & SA_NODEFER) == 0 {
                next_mask |= signal_to_mask(signum);
            }
            let unmaskable = signal_to_mask(SIGNAL_SIGKILL) | signal_to_mask(SIGNAL_SIGSTOP);
            processes[idx].signal_mask = next_mask & !unmaskable;
            old
        };

        return Some((signum, action, old_mask));
    }
}

#[cfg(target_arch = "aarch64")]
pub fn deliver_pending_signal_aarch64(ctx: &mut crate::arch::exception::ExceptionContext) -> bool {
    let tid = current_tid_or_zero();
    let Some((signum, action, old_mask)) = prepare_pending_signal_delivery(tid) else {
        return false;
    };

    if action.sa_restorer == 0 {
        finalize_exit_by_signal(tid, SIGNAL_SIGSEGV);
        proc::exit();
    }

    let mut sp_el0: usize;
    unsafe {
        // SAFETY: EL1 예외 컨텍스트에서 현재 EL0 스택 포인터를 읽는다.
        core::arch::asm!(
            "mrs {sp}, sp_el0",
            sp = out(reg) sp_el0,
            options(nostack, nomem)
        );
    }

    let frame_size = core::mem::size_of::<KernelSigFrameAarch64>();
    let frame_sp = sp_el0.saturating_sub(frame_size) & !0xF;
    if validate_user_pointer(frame_sp, frame_size).is_err() {
        finalize_exit_by_signal(tid, SIGNAL_SIGSEGV);
        proc::exit();
    }

    let frame = KernelSigFrameAarch64 {
        magic: SIGFRAME_MAGIC,
        signum: signum as u64,
        old_mask,
        _pad: 0,
        gpr: ctx.gpr,
        elr: ctx.elr,
        spsr: ctx.spsr,
        sp_el0: sp_el0 as u64,
    };
    unsafe {
        // SAFETY: 사용자 스택 영역 유효성을 검증한 뒤 sigframe을 기록한다.
        core::ptr::write_unaligned(frame_sp as *mut KernelSigFrameAarch64, frame);
        core::arch::asm!(
            "msr sp_el0, {sp}",
            sp = in(reg) frame_sp,
            options(nostack, nomem)
        );
    }

    ctx.gpr[0] = signum as u64;
    ctx.gpr[1] = 0;
    ctx.gpr[2] = 0;
    ctx.gpr[30] = action.sa_restorer;
    ctx.elr = action.sa_handler;
    true
}

#[cfg(target_arch = "riscv64")]
pub fn deliver_pending_signal_riscv64(ctx: &mut crate::arch::trap::TrapContext) -> bool {
    let tid = current_tid_or_zero();
    let Some((signum, action, old_mask)) = prepare_pending_signal_delivery(tid) else {
        return false;
    };

    if action.sa_restorer == 0 {
        finalize_exit_by_signal(tid, SIGNAL_SIGSEGV);
        proc::exit();
    }

    let user_sp = ctx.gpr[2] as usize;
    let frame_size = core::mem::size_of::<KernelSigFrameRiscv64>();
    let frame_sp = user_sp.saturating_sub(frame_size) & !0xF;
    if validate_user_pointer(frame_sp, frame_size).is_err() {
        finalize_exit_by_signal(tid, SIGNAL_SIGSEGV);
        proc::exit();
    }

    let frame = KernelSigFrameRiscv64 {
        magic: SIGFRAME_MAGIC,
        signum: signum as u64,
        old_mask,
        _pad: 0,
        gpr: ctx.gpr,
        mstatus: ctx.mstatus,
        mepc: ctx.mepc,
    };
    unsafe {
        // SAFETY: 사용자 스택 영역 유효성을 검증한 뒤 sigframe을 기록한다.
        core::ptr::write_unaligned(frame_sp as *mut KernelSigFrameRiscv64, frame);
    }

    ctx.gpr[10] = signum as u64; // a0
    ctx.gpr[11] = 0;
    ctx.gpr[12] = 0;
    ctx.gpr[1] = action.sa_restorer; // ra
    ctx.gpr[2] = frame_sp as u64; // sp
    ctx.mepc = action.sa_handler;
    true
}

#[cfg(target_arch = "aarch64")]
pub fn sys_rt_sigreturn_aarch64(_gpr: [u64; 31], _elr: u64, _spsr: u64, sp_el0: usize) -> isize {
    let frame_size = core::mem::size_of::<KernelSigFrameAarch64>();
    if validate_user_pointer(sp_el0, frame_size).is_err() {
        return errno::EFAULT;
    }

    let frame = unsafe {
        // SAFETY: 사용자 포인터 범위를 검증한 뒤 sigframe을 읽는다.
        core::ptr::read_unaligned(sp_el0 as *const KernelSigFrameAarch64)
    };
    if frame.magic != SIGFRAME_MAGIC {
        return errno::EFAULT;
    }

    let tid = current_tid_or_zero();
    set_signal_mask_for_tid(tid, frame.old_mask);

    let mut pending = PENDING_SIGRETURN_AARCH64.lock();
    if let Some(pos) = pending.iter().position(|item| item.tid == tid) {
        pending.swap_remove(pos);
    }
    pending.push(PendingSigReturnAarch64 {
        tid,
        gpr: frame.gpr,
        elr: frame.elr,
        spsr: frame.spsr,
        sp_el0: frame.sp_el0,
        signal_mask: frame.old_mask,
    });
    0
}

#[cfg(target_arch = "riscv64")]
pub fn sys_rt_sigreturn_riscv(gpr: [u64; 32], _mstatus: u64, _mepc: u64) -> isize {
    let sp = gpr[2] as usize;
    let frame_size = core::mem::size_of::<KernelSigFrameRiscv64>();
    if validate_user_pointer(sp, frame_size).is_err() {
        return errno::EFAULT;
    }

    let frame = unsafe {
        // SAFETY: 사용자 포인터 범위를 검증한 뒤 sigframe을 읽는다.
        core::ptr::read_unaligned(sp as *const KernelSigFrameRiscv64)
    };
    if frame.magic != SIGFRAME_MAGIC {
        return errno::EFAULT;
    }

    let tid = current_tid_or_zero();
    set_signal_mask_for_tid(tid, frame.old_mask);

    let mut pending = PENDING_SIGRETURN_RISCV64.lock();
    if let Some(pos) = pending.iter().position(|item| item.tid == tid) {
        pending.swap_remove(pos);
    }
    pending.push(PendingSigReturnRiscv {
        tid,
        gpr: frame.gpr,
        mstatus: frame.mstatus,
        mepc: frame.mepc,
        signal_mask: frame.old_mask,
    });
    0
}

pub fn sys_rt_sigreturn() -> isize {
    errno::ENOSYS
}

#[cfg(target_arch = "aarch64")]
pub fn apply_pending_sigreturn_aarch64(ctx: &mut crate::arch::exception::ExceptionContext) -> bool {
    let tid = current_tid_or_zero();
    let mut pending = PENDING_SIGRETURN_AARCH64.lock();
    let pos = match pending.iter().position(|item| item.tid == tid) {
        Some(pos) => pos,
        None => return false,
    };
    let entry = pending.swap_remove(pos);
    drop(pending);

    set_signal_mask_for_tid(tid, entry.signal_mask);
    ctx.gpr = entry.gpr;
    ctx.elr = entry.elr;
    ctx.spsr = entry.spsr;
    unsafe {
        // SAFETY: 저장해둔 사용자 스택 포인터를 EL0 복귀 전에 복원한다.
        core::arch::asm!(
            "msr sp_el0, {sp}",
            sp = in(reg) entry.sp_el0 as usize,
            options(nostack, nomem)
        );
    }
    true
}

#[cfg(target_arch = "riscv64")]
pub fn apply_pending_sigreturn_riscv64(ctx: &mut crate::arch::trap::TrapContext) -> bool {
    let tid = current_tid_or_zero();
    let mut pending = PENDING_SIGRETURN_RISCV64.lock();
    let pos = match pending.iter().position(|item| item.tid == tid) {
        Some(pos) => pos,
        None => return false,
    };
    let entry = pending.swap_remove(pos);
    drop(pending);

    set_signal_mask_for_tid(tid, entry.signal_mask);
    ctx.gpr = entry.gpr;
    ctx.mstatus = entry.mstatus;
    ctx.mepc = entry.mepc;
    true
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

fn finalize_exit_with_wait_status(tid: proc::Tid, wait_status: i32) {
    let orphan_reaper = reparent_orphans(tid);
    let reparented_zombies = reparent_zombie_children(tid, orphan_reaper);

    let (parent_tid, exit_signal, vm_group) = {
        let mut processes = PROCESS_INFOS.lock();
        let idx = ensure_process_info_for_tid_locked(&mut processes, tid);
        (
            processes[idx].parent_tid,
            processes[idx].exit_signal,
            processes[idx].vm_group,
        )
    };

    flush_shared_writeback_for_vm_group(vm_group);
    if process_count_in_vm_group(vm_group) == 1 {
        cleanup_vm_group_resources(vm_group);
    }

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

fn finalize_exit(tid: proc::Tid, status: i32) {
    finalize_exit_with_wait_status(tid, encode_wait_status_from_exit_code(status));
}

fn finalize_exit_by_signal(tid: proc::Tid, signum: u32) {
    let wait_status = (signum as i32) & 0x7f;
    finalize_exit_with_wait_status(tid, wait_status);
}

fn remove_process_info(tid: proc::Tid) {
    let mut processes = PROCESS_INFOS.lock();
    let removed = if let Some(pos) = processes.iter().position(|p| p.tid == tid) {
        let info = processes.swap_remove(pos);
        Some((info.vm_group, info.sighand_group))
    } else {
        None
    };
    drop(processes);

    if let Some((vm_group, sighand_group)) = removed {
        if process_count_in_vm_group(vm_group) == 0 {
            cleanup_vm_group_resources(vm_group);
        }
        remove_signal_actions_if_unused(sighand_group);
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

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn fork_child_entry() -> ! {
    let tid = current_tid_or_zero();
    let context = loop {
        let context = {
            let mut pending = PENDING_FORK_CHILDREN.lock();
            let pos = pending.iter().position(|c| c.tid == tid && c.ready);
            pos.map(|idx| pending.swap_remove(idx).context)
        };
        if let Some(context) = context {
            break context;
        }
        proc::yield_now();
    };

    unsafe {
        // SAFETY: fork/clone 시 부모 trap context에서 캡처한 유효 사용자 복귀 상태를 복원한다.
        fork_child_enter_user(&context as *const ForkChildContext);
    }
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn mark_pending_fork_child_ready(child_tid: proc::Tid) {
    let mut pending = PENDING_FORK_CHILDREN.lock();
    if let Some(child) = pending.iter_mut().find(|child| child.tid == child_tid) {
        child.ready = true;
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
/// sighand_group 단위로 시그널 핸들러를 등록/조회한다.
pub fn sys_rt_sigaction(signum: i32, act: *const u8, oldact: *mut u8, sigsetsize: usize) -> isize {
    if sigsetsize < MIN_SIGSET_SIZE {
        return errno::EINVAL;
    }
    if signum <= 0 || signum > MAX_SIGNAL_COUNT as i32 {
        return errno::EINVAL;
    }
    if !oldact.is_null()
        && validate_user_pointer(oldact as usize, core::mem::size_of::<LinuxSigAction>()).is_err()
    {
        return errno::EFAULT;
    }
    if !act.is_null()
        && validate_user_pointer(act as usize, core::mem::size_of::<LinuxSigAction>()).is_err()
    {
        return errno::EFAULT;
    }

    let tid = current_tid_or_zero();
    let sighand_group = sighand_group_for_tid(tid);
    let sig = signum as u32;
    let current = signal_action_for_group(sighand_group, sig);

    if !oldact.is_null() {
        unsafe {
            // SAFETY: 사용자 포인터 범위를 검증한 뒤 sigaction 구조체를 기록한다.
            core::ptr::write_unaligned(oldact as *mut LinuxSigAction, current);
        }
    }

    if act.is_null() {
        return 0;
    }

    if sig == SIGNAL_SIGKILL || sig == SIGNAL_SIGSTOP {
        return errno::EINVAL;
    }

    let mut next = unsafe {
        // SAFETY: 사용자 포인터 범위를 검증한 뒤 sigaction 구조체를 읽는다.
        core::ptr::read_unaligned(act as *const LinuxSigAction)
    };
    // 현재 단계에서는 핵심 플래그만 유지한다.
    next.sa_flags &= SA_SIGINFO | SA_NODEFER | SA_RESTART;
    set_signal_action_for_group(sighand_group, sig, next);
    0
}

/// sys_rt_sigprocmask - 시그널 마스크 제어
///
/// 현재 구현은 64비트 시그널 마스크(1~64번)를 프로세스 단위로 추적한다.
pub fn sys_rt_sigprocmask(how: i32, set: *const u8, oldset: *mut u8, sigsetsize: usize) -> isize {
    if sigsetsize < MIN_SIGSET_SIZE {
        return errno::EINVAL;
    }
    if !oldset.is_null() && validate_user_pointer(oldset as usize, MIN_SIGSET_SIZE).is_err() {
        return errno::EFAULT;
    }
    if !set.is_null() && validate_user_pointer(set as usize, MIN_SIGSET_SIZE).is_err() {
        return errno::EFAULT;
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
    let unmaskable_bits = signal_to_mask(SIGNAL_SIGKILL) | signal_to_mask(SIGNAL_SIGSTOP);
    match how {
        SIG_BLOCK => processes[idx].signal_mask |= set_bits & !unmaskable_bits,
        SIG_UNBLOCK => processes[idx].signal_mask &= !(set_bits & !unmaskable_bits),
        SIG_SETMASK => processes[idx].signal_mask = set_bits & !unmaskable_bits,
        _ => return errno::EINVAL,
    }

    drop(processes);
    if has_unmasked_pending_signal(tid) {
        let _ = proc::wake_thread_for_signal(tid);
    }

    0
}

/// sys_nanosleep - 지정 시간 대기
///
/// timespec 기반 슬립 + EINTR/rem 지원.
pub fn sys_nanosleep(req: *const u8, rem: *mut u8) -> isize {
    if req.is_null() {
        return errno::EFAULT;
    }
    if validate_user_pointer(req as usize, core::mem::size_of::<LinuxTimespec>()).is_err() {
        return errno::EFAULT;
    }
    if !rem.is_null()
        && validate_user_pointer(rem as usize, core::mem::size_of::<LinuxTimespec>()).is_err()
    {
        return errno::EFAULT;
    }

    let req_ts = unsafe {
        // SAFETY: 사용자 포인터 범위를 검증한 뒤 timespec을 읽는다.
        core::ptr::read_unaligned(req as *const LinuxTimespec)
    };
    if req_ts.tv_sec < 0 || req_ts.tv_nsec < 0 || req_ts.tv_nsec >= 1_000_000_000 {
        return errno::EINVAL;
    }

    let req_ns = (req_ts.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(req_ts.tv_nsec as u64);
    if req_ns == 0 {
        if !rem.is_null() {
            let zero = LinuxTimespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            unsafe {
                // SAFETY: 사용자 포인터 범위를 검증한 뒤 timespec을 기록한다.
                core::ptr::write_unaligned(rem as *mut LinuxTimespec, zero);
            }
        }
        return 0;
    }

    let tid = current_tid_or_zero();
    if has_unmasked_pending_signal(tid) {
        if !rem.is_null() {
            unsafe {
                // SAFETY: 사용자 포인터 범위를 검증한 뒤 timespec을 기록한다.
                core::ptr::write_unaligned(rem as *mut LinuxTimespec, req_ts);
            }
        }
        return errno::EINTR;
    }

    let start_ns = crate::time::monotonic_now_ns();
    let deadline_ns = start_ns.saturating_add(req_ns);
    loop {
        let now = crate::time::monotonic_now_ns();
        if now >= deadline_ns {
            if !rem.is_null() {
                let zero = LinuxTimespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                unsafe {
                    // SAFETY: 사용자 포인터 범위를 검증한 뒤 timespec을 기록한다.
                    core::ptr::write_unaligned(rem as *mut LinuxTimespec, zero);
                }
            }
            return 0;
        }

        let wake_reason = proc::sleep_current_until(deadline_ns);
        if wake_reason == proc::SleepWakeReason::Signal && has_unmasked_pending_signal(tid) {
            if !rem.is_null() {
                let now2 = crate::time::monotonic_now_ns();
                let remaining = deadline_ns.saturating_sub(now2);
                let rem_ts = LinuxTimespec {
                    tv_sec: (remaining / 1_000_000_000) as i64,
                    tv_nsec: (remaining % 1_000_000_000) as i64,
                };
                unsafe {
                    // SAFETY: 사용자 포인터 범위를 검증한 뒤 timespec을 기록한다.
                    core::ptr::write_unaligned(rem as *mut LinuxTimespec, rem_ts);
                }
            }
            return errno::EINTR;
        }
    }
}

/// sys_clock_gettime - 시계 값 조회
///
/// 현재는 MONOTONIC/REALTIME 모두 부팅 이후 monotonic counter 기반으로 제공한다.
pub fn sys_clock_gettime(clock_id: i32, tp: *mut u8) -> isize {
    if tp.is_null() {
        return errno::EFAULT;
    }
    if validate_user_pointer(tp as usize, core::mem::size_of::<LinuxTimespec>()).is_err() {
        return errno::EFAULT;
    }

    if clock_id != CLOCK_REALTIME && clock_id != CLOCK_MONOTONIC {
        return errno::EINVAL;
    }

    let (sec, nsec) = if clock_id == CLOCK_REALTIME {
        realtime_time()
    } else {
        monotonic_time()
    };
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

/// sys_clock_getres - 시계 해상도 조회
pub fn sys_clock_getres(clock_id: i32, tp: *mut u8) -> isize {
    if clock_id != CLOCK_REALTIME && clock_id != CLOCK_MONOTONIC {
        return errno::EINVAL;
    }
    if tp.is_null() {
        return 0;
    }
    if validate_user_pointer(tp as usize, core::mem::size_of::<LinuxTimespec>()).is_err() {
        return errno::EFAULT;
    }

    let res_ns = crate::time::clock_res_ns();
    let ts = LinuxTimespec {
        tv_sec: (res_ns / 1_000_000_000) as i64,
        tv_nsec: (res_ns % 1_000_000_000) as i64,
    };
    unsafe {
        // SAFETY: 사용자 포인터 범위를 검증한 뒤 timespec을 기록한다.
        core::ptr::write_unaligned(tp as *mut LinuxTimespec, ts);
    }
    0
}

/// sys_gettimeofday - wallclock 시간 조회
///
/// realtime 시간을 timeval로 변환해 반환한다.
pub fn sys_gettimeofday(tv: *mut u8, tz: *mut u8) -> isize {
    let (sec, nsec) = realtime_time();

    if !tv.is_null() {
        if validate_user_pointer(tv as usize, core::mem::size_of::<LinuxTimeval>()).is_err() {
            return errno::EFAULT;
        }
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
        if validate_user_pointer(tz as usize, core::mem::size_of::<LinuxTimezone>()).is_err() {
            return errno::EFAULT;
        }
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

/// sys_kill - 프로세스 시그널 전송
pub fn sys_kill(pid: isize, sig: i32) -> isize {
    if sig < 0 || sig > MAX_SIGNAL_COUNT as i32 {
        return errno::EINVAL;
    }
    if pid < 0 {
        return errno::ESRCH;
    }

    let target_tid = if pid == 0 {
        current_tid_or_zero()
    } else {
        pid as proc::Tid
    };
    let exists = {
        let mut processes = PROCESS_INFOS.lock();
        let _ = ensure_process_info_for_tid_locked(&mut processes, current_tid_or_zero());
        processes.iter().any(|p| p.tid == target_tid)
    };
    if !exists {
        if !proc::thread_exists(target_tid) {
            return errno::ESRCH;
        }
        ensure_process_info_for_tid(target_tid);
    }
    if sig != 0 {
        enqueue_signal(target_tid, sig as u32);
    }
    0
}

/// sys_tkill - 스레드 단위 시그널 전송
pub fn sys_tkill(tid: isize, sig: i32) -> isize {
    if sig < 0 || sig > MAX_SIGNAL_COUNT as i32 {
        return errno::EINVAL;
    }
    if tid < 0 {
        return errno::ESRCH;
    }
    let target_tid = if tid == 0 {
        current_tid_or_zero()
    } else {
        tid as proc::Tid
    };
    let exists = {
        let processes = PROCESS_INFOS.lock();
        processes.iter().any(|p| p.tid == target_tid)
    };
    if !exists {
        if !proc::thread_exists(target_tid) {
            return errno::ESRCH;
        }
        ensure_process_info_for_tid(target_tid);
    }
    if sig != 0 {
        enqueue_signal(target_tid, sig as u32);
    }
    0
}

/// sys_tgkill - 쓰레드 그룹 + 쓰레드 지정 시그널 전송
pub fn sys_tgkill(tgid: isize, tid: isize, sig: i32) -> isize {
    if sig < 0 || sig > MAX_SIGNAL_COUNT as i32 {
        return errno::EINVAL;
    }
    if tgid < 0 || tid < 0 {
        return errno::ESRCH;
    }
    let mapped_tgid = if tgid == 0 {
        current_tid_or_zero() as isize
    } else {
        tgid
    };
    let mapped_tid = if tid == 0 {
        current_tid_or_zero() as isize
    } else {
        tid
    };
    if mapped_tgid != mapped_tid {
        return errno::ESRCH;
    }
    sys_tkill(mapped_tid, sig)
}

fn write_sigtimedwait_info(info: *mut u8, signum: u32) {
    if info.is_null() {
        return;
    }

    let siginfo = LinuxSigInfoHeader {
        si_signo: signum as i32,
        si_errno: 0,
        si_code: 0,
        _pad: 0,
    };
    unsafe {
        // SAFETY: 호출자가 전달한 사용자 포인터 범위를 선검증한 뒤 siginfo 헤더를 기록한다.
        core::ptr::write_unaligned(info as *mut LinuxSigInfoHeader, siginfo);
    }
}

/// sys_rt_sigtimedwait - 지정 시그널 대기
///
/// pending signal queue에서 조건에 맞는 시그널을 하나 꺼내 반환한다.
pub fn sys_rt_sigtimedwait(
    set: *const u8,
    info: *mut u8,
    timeout: *const u8,
    sigsetsize: usize,
) -> isize {
    if set.is_null() {
        return errno::EFAULT;
    }
    if sigsetsize < MIN_SIGSET_SIZE {
        return errno::EINVAL;
    }
    if validate_user_pointer(set as usize, MIN_SIGSET_SIZE).is_err() {
        return errno::EFAULT;
    }
    if !info.is_null()
        && validate_user_pointer(info as usize, core::mem::size_of::<LinuxSigInfoHeader>()).is_err()
    {
        return errno::EFAULT;
    }

    let timeout_deadline_ns = if timeout.is_null() {
        None
    } else {
        if validate_user_pointer(timeout as usize, core::mem::size_of::<LinuxTimespec>()).is_err() {
            return errno::EFAULT;
        }
        let ts = unsafe {
            // SAFETY: 사용자 포인터 범위를 검증한 뒤 timespec을 읽는다.
            core::ptr::read_unaligned(timeout as *const LinuxTimespec)
        };
        if ts.tv_sec < 0 || ts.tv_nsec < 0 || ts.tv_nsec >= 1_000_000_000 {
            return errno::EINVAL;
        }
        let timeout_ns = (ts.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(ts.tv_nsec as u64);
        Some(crate::time::monotonic_now_ns().saturating_add(timeout_ns))
    };

    let unwaitable = signal_to_mask(SIGNAL_SIGKILL) | signal_to_mask(SIGNAL_SIGSTOP);
    let accepted = read_user_u64(set) & !unwaitable;
    let tid = current_tid_or_zero();
    clear_sigtimedwait_mask_for_tid(tid);

    if let Some(signum) = take_pending_signal(tid, accepted) {
        write_sigtimedwait_info(info, signum);
        return signum as isize;
    }

    if let Some(deadline_ns) = timeout_deadline_ns {
        if crate::time::monotonic_now_ns() >= deadline_ns {
            return errno::EAGAIN;
        }
    }

    loop {
        set_sigtimedwait_mask_for_tid(tid, accepted);

        if let Some(signum) = take_pending_signal(tid, accepted) {
            clear_sigtimedwait_mask_for_tid(tid);
            write_sigtimedwait_info(info, signum);
            return signum as isize;
        }

        let sleep_deadline = timeout_deadline_ns.unwrap_or(u64::MAX);
        if sleep_deadline != u64::MAX && crate::time::monotonic_now_ns() >= sleep_deadline {
            clear_sigtimedwait_mask_for_tid(tid);
            return errno::EAGAIN;
        }

        let wake_reason = proc::sleep_current_until(sleep_deadline);
        clear_sigtimedwait_mask_for_tid(tid);

        if let Some(signum) = take_pending_signal(tid, accepted) {
            write_sigtimedwait_info(info, signum);
            return signum as isize;
        }

        if let Some(deadline_ns) = timeout_deadline_ns {
            if crate::time::monotonic_now_ns() >= deadline_ns {
                return errno::EAGAIN;
            }
        }

        if wake_reason == proc::SleepWakeReason::Signal {
            return errno::EINTR;
        }
    }
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
            sigtimedwait_mask: 0,
            pending_signals: Vec::new(),
            exit_signal,
        });
        clone_signal_actions_if_needed(parent_sighand_group, child_sighand_group);
    }

    ZOMBIE_CHILDREN.lock().push(ZombieChild {
        parent_tid,
        child_tid,
        status: encode_wait_status_from_exit_code(0),
    });

    if exit_signal != 0 {
        enqueue_signal(parent_tid, exit_signal);
    }

    if flags & CLONE_PARENT_SETTID != 0 && !parent_tid_ptr.is_null() {
        write_user_i32(parent_tid_ptr as *mut i32, child_tid as i32);
    }
    if flags & CLONE_CHILD_SETTID != 0 && !child_tid_ptr.is_null() {
        write_user_i32(child_tid_ptr as *mut i32, child_tid as i32);
    }

    child_tid
}

/// sys_fork - clone(SIGCHLD) 래퍼
pub fn sys_fork() -> isize {
    sys_clone(
        SIGNAL_SIGCHLD as usize,
        0,
        core::ptr::null_mut(),
        0,
        core::ptr::null_mut(),
    )
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

fn finalize_clone_with_vm_setup(
    flags: usize,
    parent_tid: proc::Tid,
    child_tid: proc::Tid,
    parent_tid_ptr: *mut u8,
    child_tid_ptr: *mut u8,
) -> isize {
    let exit_signal = (flags & CLONE_CSIGNAL_MASK) as u32;
    let (parent_vm_group, child_vm_group) = {
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
            sigtimedwait_mask: 0,
            pending_signals: Vec::new(),
            exit_signal,
        });
        clone_signal_actions_if_needed(parent_sighand_group, child_sighand_group);

        (parent_vm_group, child_vm_group)
    };

    if flags & CLONE_VM != 0 {
        let root = vm_root_for_group(parent_vm_group);
        set_vm_root_for_group(child_vm_group, root);
        let _ = proc::set_thread_user_root_table(child_tid, root);
    } else {
        let parent_root = vm_root_for_group(parent_vm_group);
        let child_root = match crate::arch::mmu::clone_root_table(parent_root) {
            Ok(root) => root,
            Err(_) => return errno::ENOMEM,
        };
        set_vm_root_for_group(child_vm_group, child_root);
        let _ = proc::set_thread_user_root_table(child_tid, child_root);

        let page_size = crate::mm::page::PAGE_SIZE;
        let mut needs_tlb_flush = false;

        {
            let mut brk_regions = BRK_REGIONS.lock();
            let mut child_regions: Vec<BrkRegion> = Vec::new();
            for region in brk_regions
                .iter()
                .filter(|region| region.vm_group == parent_vm_group)
            {
                let mut cloned = region.clone();
                cloned.vm_group = child_vm_group;
                for (idx, frame_opt) in region.pages.iter().enumerate() {
                    let Some(frame) = *frame_opt else {
                        continue;
                    };
                    let _ = crate::mm::page::retain_frame(frame);
                    if !region.direct_phys {
                        let va = region.base + idx * page_size;
                        if setup_cow_pair(
                            parent_vm_group,
                            child_vm_group,
                            parent_root,
                            child_root,
                            va,
                            frame,
                            false,
                        )
                        .is_err()
                        {
                            return errno::ENOMEM;
                        }
                        needs_tlb_flush = true;
                    }
                }
                child_regions.push(cloned);
            }
            if !child_regions.is_empty() {
                brk_regions.extend(child_regions);
            }
        }

        {
            let mut mmap_regions = MMAP_REGIONS.lock();
            let mut child_regions: Vec<MmapRegion> = Vec::new();
            for region in mmap_regions
                .iter()
                .filter(|region| region.vm_group == parent_vm_group)
            {
                let mut cloned = region.clone();
                cloned.vm_group = child_vm_group;
                child_regions.push(cloned);

                let private_mapping = (region.flags & MAP_TYPE_MASK) == MAP_PRIVATE;
                let writable = (region.prot & PROT_WRITE) != 0;
                let execute = (region.prot & PROT_EXEC) != 0;

                for (idx, frame) in region.pages.iter().copied().enumerate() {
                    let _ = crate::mm::page::retain_frame(frame);
                    if !region.direct_phys && private_mapping && writable {
                        let va = region.base + idx * page_size;
                        if setup_cow_pair(
                            parent_vm_group,
                            child_vm_group,
                            parent_root,
                            child_root,
                            va,
                            frame,
                            execute,
                        )
                        .is_err()
                        {
                            return errno::ENOMEM;
                        }
                        needs_tlb_flush = true;
                    }
                }
            }
            if !child_regions.is_empty() {
                mmap_regions.extend(child_regions);
            }
        }

        if needs_tlb_flush {
            flush_user_tlb();
        }
    }

    if flags & CLONE_PARENT_SETTID != 0 && !parent_tid_ptr.is_null() {
        write_user_i32(parent_tid_ptr as *mut i32, child_tid as i32);
    }
    if flags & CLONE_CHILD_SETTID != 0 && !child_tid_ptr.is_null() {
        write_user_i32(child_tid_ptr as *mut i32, child_tid as i32);
    }

    mark_pending_fork_child_ready(child_tid);

    if flags & CLONE_VFORK != 0 {
        add_vfork_wait(parent_tid, child_tid);
        wait_vfork_release(parent_tid, child_tid);
    }

    child_tid as isize
}

#[cfg(target_arch = "riscv64")]
pub fn sys_clone_with_user_context_riscv(
    flags: usize,
    child_stack: usize,
    parent_tid_ptr: *mut u8,
    _tls: usize,
    child_tid_ptr: *mut u8,
    mut gpr: [u64; 32],
    mstatus: u64,
    mepc: u64,
) -> isize {
    if flags & CLONE_SIGHAND != 0 && flags & CLONE_VM == 0 {
        return errno::EINVAL;
    }

    let parent_tid = current_tid_or_zero();
    ensure_process_info_for_tid(parent_tid);

    let child_tid = proc::spawn("fork-child", fork_child_entry);
    if child_stack != 0 {
        gpr[2] = child_stack as u64; // child user sp
    }
    gpr[10] = 0; // child syscall return value (a0)

    PENDING_FORK_CHILDREN.lock().push(PendingForkChild {
        tid: child_tid,
        ready: false,
        context: ForkChildContext { gpr, mstatus, mepc },
    });

    finalize_clone_with_vm_setup(flags, parent_tid, child_tid, parent_tid_ptr, child_tid_ptr)
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
        ready: false,
        context: ForkChildContext {
            gpr,
            elr,
            spsr,
            sp_el0: child_sp,
        },
    });

    let exit_signal = (flags & CLONE_CSIGNAL_MASK) as u32;
    let (parent_vm_group, child_vm_group) = {
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
            sigtimedwait_mask: 0,
            pending_signals: Vec::new(),
            exit_signal,
        });
        clone_signal_actions_if_needed(parent_sighand_group, child_sighand_group);

        (parent_vm_group, child_vm_group)
    };

    if flags & CLONE_VM != 0 {
        let root = vm_root_for_group(parent_vm_group);
        set_vm_root_for_group(child_vm_group, root);
        let _ = proc::set_thread_user_root_table(child_tid, root);
    } else {
        let parent_root = vm_root_for_group(parent_vm_group);
        let child_root = match crate::arch::mmu::clone_root_table(parent_root) {
            Ok(root) => root,
            Err(_) => return errno::ENOMEM,
        };
        set_vm_root_for_group(child_vm_group, child_root);
        let _ = proc::set_thread_user_root_table(child_tid, child_root);

        let page_size = crate::mm::page::PAGE_SIZE;
        let mut needs_tlb_flush = false;

        {
            let mut brk_regions = BRK_REGIONS.lock();
            let mut child_regions: Vec<BrkRegion> = Vec::new();
            for region in brk_regions
                .iter()
                .filter(|region| region.vm_group == parent_vm_group)
            {
                let mut cloned = region.clone();
                cloned.vm_group = child_vm_group;
                for (idx, frame_opt) in region.pages.iter().enumerate() {
                    let Some(frame) = *frame_opt else {
                        continue;
                    };
                    let _ = crate::mm::page::retain_frame(frame);
                    if !region.direct_phys {
                        let va = region.base + idx * page_size;
                        if setup_cow_pair(
                            parent_vm_group,
                            child_vm_group,
                            parent_root,
                            child_root,
                            va,
                            frame,
                            false,
                        )
                        .is_err()
                        {
                            return errno::ENOMEM;
                        }
                        needs_tlb_flush = true;
                    }
                }
                child_regions.push(cloned);
            }
            if !child_regions.is_empty() {
                brk_regions.extend(child_regions);
            }
        }

        {
            let mut mmap_regions = MMAP_REGIONS.lock();
            let mut child_regions: Vec<MmapRegion> = Vec::new();
            for region in mmap_regions
                .iter()
                .filter(|region| region.vm_group == parent_vm_group)
            {
                let mut cloned = region.clone();
                cloned.vm_group = child_vm_group;
                child_regions.push(cloned);

                let private_mapping = (region.flags & MAP_TYPE_MASK) == MAP_PRIVATE;
                let writable = (region.prot & PROT_WRITE) != 0;
                let execute = (region.prot & PROT_EXEC) != 0;

                for (idx, frame) in region.pages.iter().copied().enumerate() {
                    let _ = crate::mm::page::retain_frame(frame);
                    if !region.direct_phys && private_mapping && writable {
                        let va = region.base + idx * page_size;
                        if setup_cow_pair(
                            parent_vm_group,
                            child_vm_group,
                            parent_root,
                            child_root,
                            va,
                            frame,
                            execute,
                        )
                        .is_err()
                        {
                            return errno::ENOMEM;
                        }
                        needs_tlb_flush = true;
                    }
                }
            }
            if !child_regions.is_empty() {
                mmap_regions.extend(child_regions);
            }
        }

        if needs_tlb_flush {
            flush_user_tlb();
        }
    }

    if flags & CLONE_PARENT_SETTID != 0 && !parent_tid_ptr.is_null() {
        write_user_i32(parent_tid_ptr as *mut i32, child_tid as i32);
    }
    if flags & CLONE_CHILD_SETTID != 0 && !child_tid_ptr.is_null() {
        write_user_i32(child_tid_ptr as *mut i32, child_tid as i32);
    }

    mark_pending_fork_child_ready(child_tid);

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
    let new_pgid = if pgid <= 0 {
        target_tid
    } else {
        pgid as proc::Tid
    };
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

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn brk_base_for_vm_group(vm_group: u64) -> Option<usize> {
    let slot = vm_group as usize;
    let base = BRK_REGION_BASE.checked_add(slot.checked_mul(BRK_REGION_SIZE)?)?;
    let end = base.checked_add(BRK_REGION_SIZE)?;
    if end >= crate::proc::user::USER_STACK_BASE {
        None
    } else {
        Some(base)
    }
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn find_mmap_base_locked(regions: &[MmapRegion], vm_group: u64, size: usize) -> Option<usize> {
    let page_size = crate::mm::page::PAGE_SIZE;
    let mut cursor = align_up(MMAP_REGION_BASE, page_size);

    loop {
        let end = cursor.checked_add(size)?;
        if end > MMAP_REGION_END {
            return None;
        }

        let mut collided = false;
        let mut next_cursor = cursor;
        for region in regions.iter().filter(|r| r.vm_group == vm_group) {
            if mmap_overlaps(region, cursor, end) {
                collided = true;
                let region_end = align_up(mmap_region_end(region), page_size);
                if region_end > next_cursor {
                    next_cursor = region_end;
                }
            }
        }

        if !collided {
            return Some(cursor);
        }
        if next_cursor <= cursor {
            return None;
        }
        cursor = align_up(next_cursor, page_size);
    }
}

#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
fn unmap_mmap_range_locked(
    regions: &mut Vec<MmapRegion>,
    vm_group: u64,
    root_table: usize,
    start: usize,
    end: usize,
) -> Result<bool, isize> {
    let page_size = crate::mm::page::PAGE_SIZE;
    let mut changed = false;
    let mut new_regions = Vec::new();
    let mut i = 0usize;

    while i < regions.len() {
        if regions[i].vm_group != vm_group || !mmap_overlaps(&regions[i], start, end) {
            i += 1;
            continue;
        }

        let region_base = regions[i].base;
        let region_end = mmap_region_end(&regions[i]);
        let unmap_start = core::cmp::max(region_base, start);
        let unmap_end = core::cmp::min(region_end, end);
        if unmap_start >= unmap_end {
            i += 1;
            continue;
        }

        let first_page = (unmap_start - region_base) / page_size;
        let unmap_pages = (unmap_end - unmap_start) / page_size;
        if unmap_pages == 0 {
            i += 1;
            continue;
        }

        let removed: Vec<usize> = regions[i].pages[first_page..first_page + unmap_pages].to_vec();
        let write = (regions[i].prot & PROT_WRITE) != 0;
        let execute = (regions[i].prot & PROT_EXEC) != 0;
        let direct_phys = regions[i].direct_phys;

        if write && matches!(regions[i].backing, MmapBacking::File(_)) {
            if flush_file_region_pages(&regions[i], first_page, unmap_pages).is_err() {
                return Err(errno::EIO);
            }
        }

        if !direct_phys {
            let mut unmapped = 0usize;
            for page in 0..removed.len() {
                let va = unmap_start + page * page_size;
                remove_cow_meta(vm_group, va);
                if crate::arch::mmu::unmap_user_page_for_root_noflush(root_table, va).is_err() {
                    for rolled in 0..unmapped {
                        let rollback_va = unmap_start + rolled * page_size;
                        let rollback_pa = removed[rolled];
                        let _ = crate::arch::mmu::map_user_page_for_root_noflush(
                            root_table,
                            rollback_va,
                            rollback_pa,
                            write,
                            execute,
                        );
                    }
                    flush_user_tlb();
                    return Err(errno::EINVAL);
                }
                unmapped += 1;
            }
        } else {
            for page in 0..removed.len() {
                let va = unmap_start + page * page_size;
                remove_cow_meta(vm_group, va);
            }
        }

        for frame in removed.iter() {
            unsafe {
                // SAFETY: 해당 페이지는 mmap 메타데이터가 소유하고 있으며 방금 페이지 테이블에서 해제했다.
                crate::mm::page::free_frame(*frame);
            }
        }

        changed = true;

        if first_page == 0 && unmap_pages == regions[i].pages.len() {
            regions.swap_remove(i);
            continue;
        }

        if first_page == 0 {
            let removed_bytes = unmap_pages * page_size;
            regions[i].pages.drain(0..unmap_pages);
            regions[i].base = unmap_end;
            regions[i].len = regions[i].pages.len() * page_size;
            regions[i].requested_len = regions[i].requested_len.saturating_sub(removed_bytes);
            if let MmapBacking::File(backing) = &mut regions[i].backing {
                backing.file_offset = backing.file_offset.saturating_add(removed_bytes);
                backing.map_len = backing.map_len.saturating_sub(removed_bytes);
            }
            i += 1;
            continue;
        }

        if first_page + unmap_pages == regions[i].pages.len() {
            let kept_bytes = first_page * page_size;
            regions[i].pages.truncate(first_page);
            regions[i].len = regions[i].pages.len() * page_size;
            regions[i].requested_len = core::cmp::min(regions[i].requested_len, kept_bytes);
            if let MmapBacking::File(backing) = &mut regions[i].backing {
                backing.map_len = core::cmp::min(backing.map_len, kept_bytes);
            }
            i += 1;
            continue;
        }

        let original_requested_len = regions[i].requested_len;
        let mut right_backing = regions[i].backing.clone();
        if let MmapBacking::File(left_backing) = &mut regions[i].backing {
            let left_kept = first_page * page_size;
            left_backing.map_len = core::cmp::min(left_backing.map_len, left_kept);
        }
        if let MmapBacking::File(right_file) = &mut right_backing {
            let right_shift = (first_page + unmap_pages) * page_size;
            right_file.file_offset = right_file.file_offset.saturating_add(right_shift);
            right_file.map_len = right_file.map_len.saturating_sub(right_shift);
        }

        let right_pages = regions[i].pages.split_off(first_page + unmap_pages);
        regions[i].pages.truncate(first_page);
        regions[i].len = regions[i].pages.len() * page_size;
        regions[i].requested_len = core::cmp::min(regions[i].requested_len, first_page * page_size);
        new_regions.push(MmapRegion {
            vm_group: regions[i].vm_group,
            base: unmap_end,
            len: right_pages.len() * page_size,
            requested_len: original_requested_len
                .saturating_sub((first_page + unmap_pages) * page_size),
            prot: regions[i].prot,
            flags: regions[i].flags,
            direct_phys: regions[i].direct_phys,
            pages: right_pages,
            backing: right_backing,
        });
        i += 1;
    }

    if !new_regions.is_empty() {
        regions.extend(new_regions);
    }

    if changed {
        flush_user_tlb();
    }

    Ok(changed)
}

/// sys_brk - 프로그램 브레이크 조정
///
/// 스레드별 고정 16MB 영역을 페이지 단위로 동적 확장/축소한다.
pub fn sys_brk(addr: usize) -> isize {
    let vm_group = current_vm_group();
    let page_size = crate::mm::page::PAGE_SIZE;

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        let mut regions = BRK_REGIONS.lock();
        let idx = if let Some(pos) = regions.iter().position(|r| r.vm_group == vm_group) {
            pos
        } else {
            let pages = BRK_REGION_SIZE / page_size;
            let phys_base = match crate::mm::page::alloc_frames(pages) {
                Some(v) => v,
                None => return errno::ENOMEM,
            };
            let mut page_vec = Vec::new();
            page_vec.resize(pages, None);
            for (i, slot) in page_vec.iter_mut().enumerate() {
                *slot = Some(phys_base + i * page_size);
            }
            regions.push(BrkRegion {
                vm_group,
                base: phys_base,
                current: phys_base,
                limit: phys_base + BRK_REGION_SIZE,
                direct_phys: true,
                pages: page_vec,
            });
            regions.len() - 1
        };

        let region = &mut regions[idx];
        if region.direct_phys {
            if addr == 0 {
                return region.current as isize;
            }
            if addr < region.base || addr > region.limit {
                return region.current as isize;
            }
            region.current = align_up(addr, 16);
            return region.current as isize;
        }
        if addr == 0 {
            return region.current as isize;
        }
        if addr < region.base || addr > region.limit {
            return region.current as isize;
        }
        region.current = align_up(addr, 16);
        return region.current as isize;
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        #[cfg(target_arch = "riscv64")]
        if use_riscv_kernel_direct_vm() {
            let mut regions = BRK_REGIONS.lock();
            let idx = if let Some(pos) = regions.iter().position(|r| r.vm_group == vm_group) {
                pos
            } else {
                let pages = BRK_REGION_SIZE / page_size;
                let phys_base = match crate::mm::page::alloc_frames(pages) {
                    Some(v) => v,
                    None => return errno::ENOMEM,
                };
                let mut page_vec = Vec::new();
                page_vec.resize(pages, None);
                for (i, slot) in page_vec.iter_mut().enumerate() {
                    *slot = Some(phys_base + i * page_size);
                }
                regions.push(BrkRegion {
                    vm_group,
                    base: phys_base,
                    current: phys_base,
                    limit: phys_base + BRK_REGION_SIZE,
                    direct_phys: true,
                    pages: page_vec,
                });
                regions.len() - 1
            };

            let region = &mut regions[idx];
            if addr == 0 {
                return region.current as isize;
            }
            if addr < region.base || addr > region.limit {
                return region.current as isize;
            }
            region.current = align_up(addr, 16);
            return region.current as isize;
        }

        let root_table = vm_root_for_group(vm_group);
        let mut regions = BRK_REGIONS.lock();
        let idx = if let Some(pos) = regions.iter().position(|r| r.vm_group == vm_group) {
            pos
        } else {
            let Some(base) = brk_base_for_vm_group(vm_group) else {
                return errno::ENOMEM;
            };
            let pages = BRK_REGION_SIZE / page_size;
            let mut page_vec = Vec::new();
            page_vec.resize(pages, None);
            regions.push(BrkRegion {
                vm_group,
                base,
                current: base,
                limit: base + BRK_REGION_SIZE,
                direct_phys: false,
                pages: page_vec,
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

        let requested = align_up(addr, 16);
        let old_pages = page_count_for_len(region.current.saturating_sub(region.base), page_size);
        let new_pages = page_count_for_len(requested.saturating_sub(region.base), page_size);

        if new_pages > old_pages {
            let mut staged: Vec<(usize, usize)> = Vec::new();
            for page_idx in old_pages..new_pages {
                let frame = match alloc_zeroed_frame() {
                    Some(v) => v,
                    None => {
                        for (rollback_idx, rollback_frame) in staged.drain(..).rev() {
                            let va = region.base + rollback_idx * page_size;
                            let _ =
                                crate::arch::mmu::unmap_user_page_for_root_noflush(root_table, va);
                            region.pages[rollback_idx] = None;
                            unsafe {
                                // SAFETY: 실패 경로에서 이번 호출 중 확보한 프레임만 반환한다.
                                crate::mm::page::free_frame(rollback_frame);
                            }
                        }
                        flush_user_tlb();
                        return region.current as isize;
                    }
                };

                let va = region.base + page_idx * page_size;
                if crate::arch::mmu::map_user_page_for_root_noflush(
                    root_table, va, frame, true, false,
                )
                .is_err()
                {
                    unsafe {
                        // SAFETY: map 실패한 방금 할당 프레임을 즉시 반환한다.
                        crate::mm::page::free_frame(frame);
                    }
                    for (rollback_idx, rollback_frame) in staged.drain(..).rev() {
                        let rollback_va = region.base + rollback_idx * page_size;
                        let _ = crate::arch::mmu::unmap_user_page_for_root_noflush(
                            root_table,
                            rollback_va,
                        );
                        region.pages[rollback_idx] = None;
                        unsafe {
                            // SAFETY: 실패 경로에서 이번 호출 중 확보한 프레임만 반환한다.
                            crate::mm::page::free_frame(rollback_frame);
                        }
                    }
                    flush_user_tlb();
                    return region.current as isize;
                }

                region.pages[page_idx] = Some(frame);
                staged.push((page_idx, frame));
            }
            if !staged.is_empty() {
                flush_user_tlb();
            }
        } else if new_pages < old_pages {
            let mut removed: Vec<(usize, usize)> = Vec::new();
            for page_idx in new_pages..old_pages {
                let Some(frame) = region.pages[page_idx].take() else {
                    continue;
                };
                let va = region.base + page_idx * page_size;
                if crate::arch::mmu::unmap_user_page_for_root_noflush(root_table, va).is_err() {
                    region.pages[page_idx] = Some(frame);
                    for (rollback_idx, rollback_frame) in removed.drain(..).rev() {
                        let rollback_va = region.base + rollback_idx * page_size;
                        let _ = crate::arch::mmu::map_user_page_for_root_noflush(
                            root_table,
                            rollback_va,
                            rollback_frame,
                            true,
                            false,
                        );
                        region.pages[rollback_idx] = Some(rollback_frame);
                    }
                    flush_user_tlb();
                    return region.current as isize;
                }
                removed.push((page_idx, frame));
            }

            if !removed.is_empty() {
                flush_user_tlb();
                for (_idx, frame) in removed {
                    unsafe {
                        // SAFETY: 페이지 테이블 엔트리를 제거한 프레임만 allocator로 반환한다.
                        crate::mm::page::free_frame(frame);
                    }
                }
            }
        }

        region.current = requested;
        return region.current as isize;
    }
}

/// sys_mmap - 익명(private/shared) 매핑
///
/// MAP_ANONYMOUS 기반 매핑 + MAP_FIXED를 지원한다.
pub fn sys_mmap(
    addr: usize,
    len: usize,
    prot: usize,
    flags: usize,
    fd: isize,
    offset: usize,
) -> isize {
    if len == 0 {
        return errno::EINVAL;
    }
    if prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
        return errno::EINVAL;
    }
    let map_type = flags & MAP_TYPE_MASK;
    if map_type != MAP_PRIVATE && map_type != MAP_SHARED {
        return errno::EINVAL;
    }
    let anonymous = (flags & MAP_ANONYMOUS) != 0;
    let page_size = crate::mm::page::PAGE_SIZE;
    let size = align_up(len, page_size);
    let pages = page_count_for_len(size, page_size);
    let vm_group = current_vm_group();

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        if !anonymous {
            return errno::ENOSYS;
        }
        if fd != -1 || offset != 0 {
            return errno::EINVAL;
        }
        let phys = match crate::mm::page::alloc_frames(pages) {
            Some(v) => v,
            None => return errno::ENOMEM,
        };
        let mut page_vec = Vec::new();
        page_vec.reserve(pages);
        for i in 0..pages {
            page_vec.push(phys + i * page_size);
        }
        MMAP_REGIONS.lock().push(MmapRegion {
            vm_group,
            base: phys,
            len: size,
            requested_len: len,
            prot,
            flags,
            direct_phys: true,
            pages: page_vec,
            backing: MmapBacking::Anonymous,
        });
        return phys as isize;
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        #[cfg(target_arch = "riscv64")]
        if use_riscv_kernel_direct_vm() {
            if !anonymous {
                if fd < 0 {
                    return errno::EBADF;
                }
                return errno::ENOSYS;
            }
            if fd != -1 || offset != 0 {
                return errno::EINVAL;
            }

            let mut regions = MMAP_REGIONS.lock();

            if flags & MAP_FIXED != 0 {
                if addr == 0 || addr & (page_size - 1) != 0 {
                    return errno::EINVAL;
                }
                let end = match addr.checked_add(size) {
                    Some(v) => v,
                    None => return errno::ENOMEM,
                };
                if addr < MIN_USER_VADDR || end > MMAP_REGION_END {
                    return errno::ENOMEM;
                }

                if let Some(region) = regions.iter_mut().find(|region| {
                    region.vm_group == vm_group
                        && region.direct_phys
                        && region.base == addr
                        && region.len == size
                }) {
                    for frame in region.pages.iter().copied() {
                        unsafe {
                            // SAFETY: direct_phys anonymous mmap 프레임은 4KB 페이지 단위로 zero-fill 가능하다.
                            core::ptr::write_bytes(frame as *mut u8, 0, page_size);
                        }
                    }
                    region.prot = prot;
                    region.flags = flags;
                    region.requested_len = len;
                    return addr as isize;
                }

                return errno::ENOMEM;
            }

            let phys = match crate::mm::page::alloc_frames(pages) {
                Some(v) => v,
                None => return errno::ENOMEM,
            };
            let mut page_vec = Vec::new();
            page_vec.reserve(pages);
            for i in 0..pages {
                page_vec.push(phys + i * page_size);
            }
            regions.push(MmapRegion {
                vm_group,
                base: phys,
                len: size,
                requested_len: len,
                prot,
                flags,
                direct_phys: true,
                pages: page_vec,
                backing: MmapBacking::Anonymous,
            });
            return phys as isize;
        }

        let root_table = vm_root_for_group(vm_group);
        let file_backing = if anonymous {
            if fd != -1 || offset != 0 {
                return errno::EINVAL;
            }
            None
        } else {
            if fd < 0 {
                return errno::EBADF;
            }
            if offset & (page_size - 1) != 0 {
                return errno::EINVAL;
            }

            let table = match fd::kernel_fd_table() {
                Ok(table) => table,
                Err(_) => return errno::EBADF,
            };
            let file = match table.get(fd as i32) {
                Ok(file) => file,
                Err(_) => return errno::EBADF,
            };
            let stat = match file.vnode.stat() {
                Ok(stat) => stat,
                Err(_) => return errno::EIO,
            };
            let file_size = stat.size as usize;
            let Some(end) = offset.checked_add(len) else {
                return errno::EINVAL;
            };
            if end > file_size {
                return errno::EINVAL;
            }

            Some(FileMapBacking {
                vnode: file.vnode.clone(),
                stable_id: file.vnode.stable_id(),
                file_offset: offset,
                map_len: len,
                shared: map_type == MAP_SHARED,
            })
        };

        let mut regions = MMAP_REGIONS.lock();
        let base = if flags & MAP_FIXED != 0 {
            if addr == 0 || addr & (page_size - 1) != 0 {
                return errno::EINVAL;
            }
            let end = match addr.checked_add(size) {
                Some(v) => v,
                None => return errno::ENOMEM,
            };
            if addr < MIN_USER_VADDR || end > MMAP_REGION_END {
                return errno::ENOMEM;
            }
            if unmap_mmap_range_locked(&mut regions, vm_group, root_table, addr, end).is_err() {
                return errno::EINVAL;
            }
            addr
        } else {
            match find_mmap_base_locked(&regions, vm_group, size) {
                Some(v) => v,
                None => return errno::ENOMEM,
            }
        };

        let write_requested = (prot & PROT_WRITE) != 0;
        let execute = (prot & PROT_EXEC) != 0;
        let mut mapped_pages = Vec::new();
        mapped_pages.reserve(pages);

        for i in 0..pages {
            let va = base + i * page_size;
            let frame = match &file_backing {
                Some(backing) => {
                    let page_index = backing.file_offset / page_size + i;
                    let page_file_offset = backing.file_offset + i * page_size;
                    match get_or_create_file_cache_page(
                        &backing.vnode,
                        backing.stable_id,
                        page_index,
                        page_file_offset,
                    ) {
                        Ok(frame) => frame,
                        Err(e) => {
                            for (rollback_idx, rollback_frame) in mapped_pages.iter().enumerate() {
                                let rollback_va = base + rollback_idx * page_size;
                                remove_cow_meta(vm_group, rollback_va);
                                let _ = crate::arch::mmu::unmap_user_page_for_root_noflush(
                                    root_table,
                                    rollback_va,
                                );
                                unsafe {
                                    // SAFETY: 실패 경로에서 현재 mmap 호출이 획득한 참조를 되돌린다.
                                    crate::mm::page::free_frame(*rollback_frame);
                                }
                            }
                            flush_user_tlb();
                            return e;
                        }
                    }
                }
                None => match alloc_zeroed_frame() {
                    Some(v) => v,
                    None => {
                        for (rollback_idx, rollback_frame) in mapped_pages.iter().enumerate() {
                            let rollback_va = base + rollback_idx * page_size;
                            remove_cow_meta(vm_group, rollback_va);
                            let _ = crate::arch::mmu::unmap_user_page_for_root_noflush(
                                root_table,
                                rollback_va,
                            );
                            unsafe {
                                // SAFETY: 실패 경로에서 현재 mmap 호출이 확보한 프레임만 반납한다.
                                crate::mm::page::free_frame(*rollback_frame);
                            }
                        }
                        flush_user_tlb();
                        return errno::ENOMEM;
                    }
                },
            };

            let map_write = match &file_backing {
                Some(backing) if !backing.shared => false,
                _ => write_requested,
            };

            if crate::arch::mmu::map_user_page_for_root_noflush(
                root_table, va, frame, map_write, execute,
            )
            .is_err()
            {
                unsafe {
                    // SAFETY: map 실패한 방금 할당 프레임을 즉시 반납한다.
                    crate::mm::page::free_frame(frame);
                }
                for (rollback_idx, rollback_frame) in mapped_pages.iter().enumerate() {
                    let rollback_va = base + rollback_idx * page_size;
                    remove_cow_meta(vm_group, rollback_va);
                    let _ =
                        crate::arch::mmu::unmap_user_page_for_root_noflush(root_table, rollback_va);
                    unsafe {
                        // SAFETY: 실패 경로에서 현재 mmap 호출이 확보한 프레임만 반납한다.
                        crate::mm::page::free_frame(*rollback_frame);
                    }
                }
                flush_user_tlb();
                return errno::ENOMEM;
            }

            if let Some(backing) = &file_backing {
                if !backing.shared && write_requested {
                    set_cow_meta(vm_group, va, frame, execute, CowOrigin::PrivateMap);
                }
            }
            mapped_pages.push(frame);
        }
        flush_user_tlb();

        let backing = match file_backing {
            Some(backing) => MmapBacking::File(backing),
            None => MmapBacking::Anonymous,
        };
        regions.push(MmapRegion {
            vm_group,
            base,
            len: size,
            requested_len: len,
            prot,
            flags,
            direct_phys: false,
            pages: mapped_pages,
            backing,
        });
        base as isize
    }
}

/// sys_munmap - 매핑 해제
///
/// 부분/전체 unmap을 지원한다.
pub fn sys_munmap(addr: usize, len: usize) -> isize {
    if addr == 0 || len == 0 {
        return errno::EINVAL;
    }

    let page_size = crate::mm::page::PAGE_SIZE;
    if addr & (page_size - 1) != 0 {
        return errno::EINVAL;
    }
    let size = align_up(len, page_size);
    let end = match addr.checked_add(size) {
        Some(v) => v,
        None => return errno::EINVAL,
    };

    let vm_group = current_vm_group();

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        let mut regions = MMAP_REGIONS.lock();
        let mut i = 0usize;
        while i < regions.len() {
            if regions[i].vm_group != vm_group || !mmap_overlaps(&regions[i], addr, end) {
                i += 1;
                continue;
            }

            for frame in regions[i].pages.iter() {
                unsafe {
                    // SAFETY: mmap 메타데이터가 소유 중인 프레임만 반환한다.
                    crate::mm::page::free_frame(*frame);
                }
            }
            regions.swap_remove(i);
        }
        return 0;
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        if addr < MIN_USER_VADDR || end > MAX_USER_VADDR_EXCLUSIVE {
            return errno::EINVAL;
        }
        let root_table = vm_root_for_group(vm_group);
        let mut regions = MMAP_REGIONS.lock();
        match unmap_mmap_range_locked(&mut regions, vm_group, root_table, addr, end) {
            Ok(_) => 0,
            Err(e) => e,
        }
    }
}

/// sys_mprotect - 매핑된 페이지 권한 변경
pub fn sys_mprotect(addr: usize, len: usize, prot: usize) -> isize {
    if len == 0 {
        return errno::EINVAL;
    }
    if prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
        return errno::EINVAL;
    }
    if prot & (PROT_READ | PROT_WRITE | PROT_EXEC) == 0 {
        return errno::EINVAL;
    }

    let page_size = crate::mm::page::PAGE_SIZE;
    if addr == 0 || addr & (page_size - 1) != 0 {
        return errno::EINVAL;
    }

    let size = align_up(len, page_size);
    let end = match addr.checked_add(size) {
        Some(v) => v,
        None => return errno::EINVAL,
    };

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        let _ = (addr, end);
        return errno::ENOSYS;
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        if addr < MIN_USER_VADDR || end > MAX_USER_VADDR_EXCLUSIVE {
            return errno::EINVAL;
        }

        let vm_group = current_vm_group();
        let root_table = vm_root_for_group(vm_group);
        let mut regions = MMAP_REGIONS.lock();
        let write = (prot & PROT_WRITE) != 0;
        let execute = (prot & PROT_EXEC) != 0;

        let mut cursor = addr;
        while cursor < end {
            let found_idx = regions.iter().position(|r| {
                r.vm_group == vm_group && r.base <= cursor && cursor < mmap_region_end(r)
            });
            let Some(idx) = found_idx else {
                return errno::ENOMEM;
            };

            let region_end = core::cmp::min(end, mmap_region_end(&regions[idx]));
            let pages = (region_end - cursor) / page_size;
            if pages == 0 {
                return errno::EINVAL;
            }

            let force_cow_ro = matches!(
                regions[idx].backing,
                MmapBacking::File(FileMapBacking { shared: false, .. })
            ) && write;
            let map_write = if force_cow_ro { false } else { write };

            let start_page = (cursor - regions[idx].base) / page_size;
            for page in 0..pages {
                let va = cursor + page * page_size;
                if !regions[idx].direct_phys {
                    if crate::arch::mmu::update_user_page_flags_for_root_noflush(
                        root_table, va, map_write, execute,
                    )
                    .is_err()
                    {
                        return errno::EINVAL;
                    }
                }

                if force_cow_ro {
                    let page_idx = start_page + page;
                    if let Some(&frame) = regions[idx].pages.get(page_idx) {
                        set_cow_meta(vm_group, va, frame, execute, CowOrigin::PrivateMap);
                    }
                } else {
                    remove_cow_meta(vm_group, va);
                }
            }

            if cursor == regions[idx].base && region_end == mmap_region_end(&regions[idx]) {
                regions[idx].prot = prot;
            }

            cursor = region_end;
        }

        flush_user_tlb();
        0
    }
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

    let (argv_list, argv_bytes) =
        match read_user_string_array(argv, MAX_EXEC_ARG_COUNT, MAX_EXEC_STR_LEN) {
            Ok(v) => v,
            Err(e) => return e,
        };

    let (envp_list, envp_bytes) =
        match read_user_string_array(envp, MAX_EXEC_ENV_COUNT, MAX_EXEC_STR_LEN) {
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

    flush_shared_writeback_for_vm_group(current_vm_group());

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

/// 테스트/디버그용: 지정한 tid의 pending signal 큐에 시그널을 삽입한다.
pub fn test_enqueue_signal_for_tid(tid: isize, signum: u32) -> isize {
    if signum == 0 || signum > MAX_SIGNAL_COUNT as u32 {
        return errno::EINVAL;
    }
    if tid < 0 {
        return errno::ESRCH;
    }

    let target_tid = tid as proc::Tid;
    let exists = {
        let processes = PROCESS_INFOS.lock();
        processes.iter().any(|p| p.tid == target_tid)
    };
    if !exists && !proc::thread_exists(target_tid) {
        return errno::ESRCH;
    }

    ensure_process_info_for_tid(target_tid);
    enqueue_signal(target_tid, signum);
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

    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        ptr >= MIN_USER_VADDR && end <= MAX_USER_VADDR_EXCLUSIVE
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    {
        ptr >= 0x1000 && end > ptr
    }
}
