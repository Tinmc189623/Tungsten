// proc/mod.rs — Tungsten 进程/线程管理子系统 (POSIX.1-2024)
// 多任务、多用户：进程表 + 用户账户 + 内核线程绑定
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod elf;
pub mod signal;
pub mod rlimit;
pub mod exec;
pub mod task;
pub mod user;

use crate::sync::SpinLock;
use core::sync::atomic::{AtomicU64, Ordering};

pub type Pid = u64;
pub type Tid = u64;

static NEXT_PID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u8)]
pub enum ProcState {
    Embryo = 0,
    Ready = 1,
    Running = 2,
    Blocked = 3,
    Zombie = 4,
    Dead = 5,
    Stopped = 6,
}

#[repr(C)]
pub struct Process {
    pub pid: Pid,
    pub ppid: Pid,
    pub pgid: Pid,
    pub sid: Pid,
    pub name: [u8; 32],
    pub name_len: u8,
    pub state: ProcState,
    pub exit_code: i32,
    pub priority: u8,
    pub nice: i8,
    pub ring: u8,
    pub cpu_time_user: u64,
    pub cpu_time_sys: u64,
    pub start_time: u64,
    pub cr3: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub data_start: u64,
    pub data_end: u64,
    pub bss_start: u64,
    pub bss_end: u64,
    pub stack_top: u64,
    pub stack_size: u64,
    pub heap_start: u64,
    pub heap_cur: u64,
    pub fds: [i32; 256],
    pub fd_count: u8,
    pub cwd: [u8; 256],
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
    pub sig_pending: u64,
    pub sig_blocked: u64,
    pub sig_handlers: [u64; 64],
    pub rlim_cpu: u64,
    pub rlim_fsize: u64,
    pub rlim_nofile: u64,
    pub rlim_nproc: u64,
    pub rlim_stack: u64,
    pub rlim_as: u64,
    pub parent: *mut Process,
    pub children: *mut Process,
    pub next_sibling: *mut Process,
    pub next: *mut Process,
}

#[repr(C)]
pub struct Thread {
    pub tid: Tid,
    pub pid: Pid,
    pub state: ProcState,
    pub priority: u8,
    pub cpu: u8,
    pub rsp: u64,
    pub rip: u64,
    pub rflags: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub kernel_stack: u64,
    pub cpu_time: u64,
    pub owner: *mut Process,
    pub next: *mut Thread,
}

pub struct ProcManager {
    pub init_proc: *mut Process,
    pub active_proc: *mut Process,
    pub proc_list: *mut Process,
    pub proc_count: usize,
    pub thread_count: usize,
}

unsafe impl Send for ProcManager {}

static PROC_MGR: SpinLock<ProcManager> = SpinLock::new(ProcManager {
    init_proc: core::ptr::null_mut(),
    active_proc: core::ptr::null_mut(),
    proc_list: core::ptr::null_mut(),
    proc_count: 0,
    thread_count: 0,
});

/// 初始化进程子系统与用户表
pub fn init() {
    user::init();
    let mut mgr = PROC_MGR.lock();
    let init = task::alloc_process(1, 0, b"init", 0, 0);
    if let Some(init_proc) = init {
        task::attach_process(&mut mgr, init_proc);
        mgr.active_proc = init_proc.as_ptr();
        NEXT_PID.store(2, Ordering::Relaxed);
    }
    crate::serial::write_str(b"proc: init pid=1, multi-user ready\n");
}

/// 分配新 PID
pub fn alloc_pid() -> Pid {
    NEXT_PID.fetch_add(1, Ordering::Relaxed)
}

/// 启动多任务：创建 idle / shell 内核线程并进入调度器
pub fn bootstrap_multitask(
    shell_entry: unsafe extern "C" fn() -> !,
    idle_entry: unsafe extern "C" fn() -> !,
) -> ! {
    let mut mgr = PROC_MGR.lock();
    let init_pid = unsafe {
        mgr.init_proc.as_ref().map(|p| p.pid).unwrap_or(1)
    };
    task::spawn_kthread(&mut mgr, b"idle", idle_entry, init_pid, 0, 0, 8192);
    task::spawn_kthread(&mut mgr, b"kshell", shell_entry, init_pid, 0, 0, 32768);
    drop(mgr);
    crate::serial::write_str(b"proc: entering preemptive scheduler\n");
    crate::sched::start();
}

/// 当前进程 PID
pub fn sys_getpid() -> Pid {
    unsafe {
        PROC_MGR
            .lock()
            .active_proc
            .as_ref()
            .map_or(crate::sched::current_pid(), |p| p.pid)
    }
}

/// 父进程 PID
pub fn sys_getppid() -> Pid {
    unsafe {
        PROC_MGR
            .lock()
            .active_proc
            .as_ref()
            .map_or(0, |p| p.ppid)
    }
}

/// 真实 UID
pub fn sys_getuid() -> u32 {
    crate::sched::current_uid()
}

/// 有效 UID
pub fn sys_geteuid() -> u32 {
    crate::sched::current_euid()
}

/// 真实 GID
pub fn sys_getgid() -> u32 {
    crate::sched::current_gid()
}

/// 进程总数
pub fn proc_count() -> usize {
    PROC_MGR.lock().proc_count
}

/// 按用户名切换当前线程凭证（login / su）
pub fn set_credentials_by_name(name: &str) -> bool {
    if let Some((uid, gid)) = user::lookup_name(name) {
        crate::sched::set_current_credentials(uid, gid);
        let mgr = PROC_MGR.lock();
        if let Some(ap) = unsafe { mgr.active_proc.as_mut() } {
            ap.uid = uid;
            ap.gid = gid;
            ap.euid = uid;
            ap.egid = gid;
        }
        true
    } else {
        false
    }
}

/// 当前用户名
pub fn current_username(buf: &mut [u8]) -> usize {
    let uid = sys_geteuid();
    if let Some(acc) = user::lookup_uid(uid) {
        let name = acc.name_str();
        let n = name.len().min(buf.len());
        buf[..n].copy_from_slice(&name[..n]);
        n
    } else {
        let fallback = b"unknown";
        let n = fallback.len().min(buf.len());
        buf[..n].copy_from_slice(&fallback[..n]);
        n
    }
}

/// 将活动进程指针同步到 proc 子系统
pub fn set_active_proc(proc: *mut Process) {
    let mut mgr = PROC_MGR.lock();
    mgr.active_proc = proc;
}

/// 在进程管理器锁下执行回调（供 service 启动平台线程）
pub(crate) fn with_proc_manager<F, R>(f: F) -> R
where
    F: FnOnce(&mut ProcManager) -> R,
{
    let mut mgr = PROC_MGR.lock();
    f(&mut mgr)
}

/// 将进程/线程列表写入缓冲区
pub fn format_ps(buf: &mut [u8]) -> usize {
    crate::sched::format_task_list(buf)
}
