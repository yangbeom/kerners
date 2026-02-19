//! ProcFS - 프로세스/시스템 정보 가상 파일시스템
//!
//! 제공 경로:
//! - `/proc/self/`
//! - `/proc/[pid]/status`
//! - `/proc/[pid]/maps`
//! - `/proc/meminfo`
//! - `/proc/cpuinfo`
//! - `/proc/uptime`

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use crate::mm;
use crate::proc;

use super::{DirEntry, FileMode, FileSystem, FsStats, Stat, VfsError, VfsResult, VNode, VNodeType};

/// ProcFS 파일시스템
pub struct ProcFs {
    root: Arc<ProcRoot>,
}

impl ProcFs {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            root: Arc::new(ProcRoot),
        })
    }
}

impl FileSystem for ProcFs {
    fn name(&self) -> &str {
        "procfs"
    }

    fn root(&self) -> Arc<dyn VNode> {
        self.root.clone()
    }

    fn statfs(&self) -> VfsResult<FsStats> {
        Ok(FsStats {
            fs_type: String::from("procfs"),
            block_size: 4096,
            total_blocks: 0,
            free_blocks: 0,
            total_inodes: 0,
            free_inodes: 0,
        })
    }
}

struct ProcRoot;

impl ProcRoot {
    fn lookup_pid(name: &str) -> VfsResult<Arc<dyn VNode>> {
        let Some(pid) = parse_pid(name) else {
            return Err(VfsError::NotFound);
        };
        if !proc::thread_exists(pid) {
            return Err(VfsError::NotFound);
        }
        Ok(Arc::new(ProcPidDir { tid: pid }))
    }
}

impl VNode for ProcRoot {
    fn node_type(&self) -> VNodeType {
        VNodeType::Directory
    }

    fn lookup(&self, name: &str) -> VfsResult<Arc<dyn VNode>> {
        match name {
            "self" => Ok(Arc::new(ProcSelfDir)),
            "meminfo" => Ok(Arc::new(ProcTextFile::new(ProcFileKind::MemInfo))),
            "cpuinfo" => Ok(Arc::new(ProcTextFile::new(ProcFileKind::CpuInfo))),
            "uptime" => Ok(Arc::new(ProcTextFile::new(ProcFileKind::Uptime))),
            _ => Self::lookup_pid(name),
        }
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        let mut entries = Vec::new();
        entries.push(DirEntry {
            name: String::from("self"),
            node_type: VNodeType::Directory,
        });
        entries.push(DirEntry {
            name: String::from("meminfo"),
            node_type: VNodeType::File,
        });
        entries.push(DirEntry {
            name: String::from("cpuinfo"),
            node_type: VNodeType::File,
        });
        entries.push(DirEntry {
            name: String::from("uptime"),
            node_type: VNodeType::File,
        });

        for tid in list_proc_tids() {
            entries.push(DirEntry {
                name: alloc::format!("{}", tid),
                node_type: VNodeType::Directory,
            });
        }
        Ok(entries)
    }

    fn stat(&self) -> VfsResult<Stat> {
        Ok(Stat {
            node_type: VNodeType::Directory,
            mode: FileMode::new(0o555),
            size: 0,
            nlink: 2,
            ..Default::default()
        })
    }
}

struct ProcSelfDir;

impl VNode for ProcSelfDir {
    fn node_type(&self) -> VNodeType {
        VNodeType::Directory
    }

    fn lookup(&self, name: &str) -> VfsResult<Arc<dyn VNode>> {
        let tid = proc::current_tid().ok_or(VfsError::NotFound)?;
        ProcPidDir { tid }.lookup(name)
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        Ok(vec_file_names_status_maps())
    }

    fn stat(&self) -> VfsResult<Stat> {
        Ok(Stat {
            node_type: VNodeType::Directory,
            mode: FileMode::new(0o555),
            size: 0,
            nlink: 2,
            ..Default::default()
        })
    }
}

struct ProcPidDir {
    tid: proc::Tid,
}

impl VNode for ProcPidDir {
    fn node_type(&self) -> VNodeType {
        VNodeType::Directory
    }

    fn lookup(&self, name: &str) -> VfsResult<Arc<dyn VNode>> {
        match name {
            "status" => Ok(Arc::new(ProcTextFile::new(ProcFileKind::PidStatus(self.tid)))),
            "maps" => Ok(Arc::new(ProcTextFile::new(ProcFileKind::PidMaps(self.tid)))),
            _ => Err(VfsError::NotFound),
        }
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        Ok(vec_file_names_status_maps())
    }

    fn stat(&self) -> VfsResult<Stat> {
        Ok(Stat {
            node_type: VNodeType::Directory,
            mode: FileMode::new(0o555),
            size: 0,
            nlink: 2,
            ..Default::default()
        })
    }
}

#[derive(Clone, Copy)]
enum ProcFileKind {
    MemInfo,
    CpuInfo,
    Uptime,
    PidStatus(proc::Tid),
    PidMaps(proc::Tid),
}

struct ProcTextFile {
    kind: ProcFileKind,
}

impl ProcTextFile {
    fn new(kind: ProcFileKind) -> Self {
        Self { kind }
    }

    fn render(&self) -> String {
        match self.kind {
            ProcFileKind::MemInfo => render_meminfo(),
            ProcFileKind::CpuInfo => render_cpuinfo(),
            ProcFileKind::Uptime => render_uptime(),
            ProcFileKind::PidStatus(tid) => render_pid_status(tid),
            ProcFileKind::PidMaps(tid) => render_pid_maps(tid),
        }
    }
}

impl VNode for ProcTextFile {
    fn node_type(&self) -> VNodeType {
        VNodeType::File
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> VfsResult<usize> {
        let content = self.render();
        let bytes = content.as_bytes();
        if offset >= bytes.len() {
            return Ok(0);
        }
        let n = core::cmp::min(buf.len(), bytes.len() - offset);
        buf[..n].copy_from_slice(&bytes[offset..offset + n]);
        Ok(n)
    }

    fn stat(&self) -> VfsResult<Stat> {
        let size = self.render().len() as u64;
        Ok(Stat {
            node_type: VNodeType::File,
            mode: FileMode::new(0o444),
            size,
            nlink: 1,
            ..Default::default()
        })
    }
}

fn parse_pid(name: &str) -> Option<proc::Tid> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    name.parse::<u64>().ok()
}

fn list_proc_tids() -> Vec<proc::Tid> {
    let mut tids: Vec<proc::Tid> = proc::thread_snapshots()
        .into_iter()
        .map(|t| t.tid)
        .filter(|tid| *tid > 0)
        .collect();
    tids.sort_unstable();
    tids.dedup();
    tids
}

fn vec_file_names_status_maps() -> Vec<DirEntry> {
    let mut entries = Vec::new();
    entries.push(DirEntry {
        name: String::from("status"),
        node_type: VNodeType::File,
    });
    entries.push(DirEntry {
        name: String::from("maps"),
        node_type: VNodeType::File,
    });
    entries
}

fn render_meminfo() -> String {
    let heap = mm::heap::stats();
    let page = mm::page::stats();
    let mem_total_kb = page.total_pages.saturating_mul(mm::page::PAGE_SIZE) / 1024;
    let mem_free_kb = page.free_pages.saturating_mul(mm::page::PAGE_SIZE) / 1024;
    let heap_total_kb = heap.size / 1024;
    let heap_free_kb = heap.free / 1024;

    alloc::format!(
        "MemTotal:\t{} kB\nMemFree:\t{} kB\nMemAvailable:\t{} kB\nKernHeapTotal:\t{} kB\nKernHeapFree:\t{} kB\n",
        mem_total_kb,
        mem_free_kb,
        mem_free_kb,
        heap_total_kb,
        heap_free_kb
    )
}

fn render_cpuinfo() -> String {
    let total = proc::percpu::total_count();
    let online = proc::percpu::online_count();
    let mut out = String::new();

    for cpu in 0..total {
        let pc = proc::percpu::get(cpu);
        let ticks = pc.tick_count.load(Ordering::Relaxed);
        let is_online = if cpu < online { "yes" } else { "no" };
        out.push_str(&alloc::format!(
            "processor\t: {}\nonline\t\t: {}\nticks\t\t: {}\n\n",
            cpu, is_online, ticks
        ));
    }

    if out.is_empty() {
        out.push_str("processor\t: 0\nonline\t\t: yes\nticks\t\t: 0\n");
    }
    out
}

fn render_uptime() -> String {
    let ns = crate::time::monotonic_now_ns();
    let secs = ns / 1_000_000_000;
    let centis = (ns % 1_000_000_000) / 10_000_000;
    alloc::format!("{}.{:02} {}.{:02}\n", secs, centis, secs, centis)
}

fn render_pid_status(tid: proc::Tid) -> String {
    let thread = proc::thread_snapshots().into_iter().find(|t| t.tid == tid);

    let proc_status = crate::syscall::proc_status_snapshot(tid);
    let (ppid, pgid, sid, vm_group, sigblk, sigpnd) = match proc_status {
        Some(p) => {
            let mut pending_mask = 0u64;
            for signum in p.pending_signals {
                if signum > 0 && signum <= 64 {
                    pending_mask |= 1u64 << (signum - 1);
                }
            }
            (
                p.parent_tid,
                p.pgid,
                p.sid,
                p.vm_group,
                p.signal_mask,
                pending_mask,
            )
        }
        None => (0, tid, tid, 0, 0, 0),
    };

    let (name, state_char, state_name) = match thread {
        Some(t) => {
            let (ch, st) = match t.state {
                proc::ThreadState::Running | proc::ThreadState::Ready => ('R', "running"),
                proc::ThreadState::Blocked => ('S', "sleeping"),
                proc::ThreadState::Terminated => ('Z', "terminated"),
            };
            (t.name, ch, st)
        }
        None => (String::from("unknown"), 'S', "sleeping"),
    };

    alloc::format!(
        "Name:\t{}\nPid:\t{}\nPPid:\t{}\nTgid:\t{}\nPgid:\t{}\nSid:\t{}\nState:\t{} ({})\nVmGroup:\t{}\nSigBlk:\t{:016x}\nSigPnd:\t{:016x}\n",
        name,
        tid,
        ppid,
        tid,
        pgid,
        sid,
        state_char,
        state_name,
        vm_group,
        sigblk,
        sigpnd
    )
}

fn render_pid_maps(tid: proc::Tid) -> String {
    let maps = crate::syscall::proc_maps_snapshot(tid);
    if maps.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for m in maps {
        let mut perms = ['-'; 4];
        if m.prot & 0x1 != 0 {
            perms[0] = 'r';
        }
        if m.prot & 0x2 != 0 {
            perms[1] = 'w';
        }
        if m.prot & 0x4 != 0 {
            perms[2] = 'x';
        }
        perms[3] = if m.shared { 's' } else { 'p' };
        let tag = if m.file_backed { "[file]" } else { "[anon]" };
        out.push_str(&alloc::format!(
            "{:016x}-{:016x} {}{}{}{} 00000000 00:00 0 {}\n",
            m.start, m.end, perms[0], perms[1], perms[2], perms[3], tag
        ));
    }
    out
}

pub fn create_procfs() -> Arc<ProcFs> {
    ProcFs::new()
}
