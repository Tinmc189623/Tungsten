// task.rs — 进程/线程创建与调度器绑定
// 将 Process 与 sched 内核线程关联，支撑多任务多用户
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{alloc_pid, Process, ProcManager, ProcState, Pid, Tid};
use crate::mm::slab;
use crate::sched;
use crate::sync::SpinLock;
use core::ptr::NonNull;

/// 内核线程入口包装参数
pub struct KthreadArgs {
    pub entry: unsafe extern "C" fn() -> !,
}

/// 分配并初始化 Process 结构
pub fn alloc_process(pid: Pid, ppid: Pid, name: &[u8], uid: u32, gid: u32) -> Option<NonNull<Process>> {
    let ptr = slab::kmalloc(core::mem::size_of::<Process>())?;
    let proc = unsafe { &mut *(ptr.as_ptr() as *mut Process) };
    *proc = Process {
        pid,
        ppid,
        pgid: pid,
        sid: pid,
        name: [0; 32],
        name_len: 0,
        state: ProcState::Ready,
        exit_code: 0,
        priority: 128,
        nice: 0,
        ring: 0,
        cpu_time_user: 0,
        cpu_time_sys: 0,
        start_time: crate::sched::ticks(),
        cr3: 0,
        text_start: 0,
        text_end: 0,
        data_start: 0,
        data_end: 0,
        bss_start: 0,
        bss_end: 0,
        stack_top: 0,
        stack_size: 0,
        heap_start: 0,
        heap_cur: 0,
        fds: [-1; 256],
        fd_count: 0,
        cwd: [0; 256],
        uid,
        gid,
        euid: uid,
        egid: gid,
        sig_pending: 0,
        sig_blocked: 0,
        sig_handlers: [0; 64],
        rlim_cpu: u64::MAX,
        rlim_fsize: u64::MAX,
        rlim_nofile: 256,
        rlim_nproc: 4096,
        rlim_stack: 8 * 1024 * 1024,
        rlim_as: u64::MAX,
        parent: core::ptr::null_mut(),
        children: core::ptr::null_mut(),
        next_sibling: core::ptr::null_mut(),
        next: core::ptr::null_mut(),
    };
    let n = name.len().min(32);
    proc.name[..n].copy_from_slice(&name[..n]);
    proc.name_len = n as u8;
    proc.cwd[..1].copy_from_slice(b"/");
    Some(unsafe { NonNull::new_unchecked(proc) })
}

/// 将进程挂入全局进程表
pub fn attach_process(mgr: &mut ProcManager, proc: NonNull<Process>) {
    unsafe {
        let p = proc.as_ptr();
        (*p).next = mgr.proc_list;
        mgr.proc_list = p;
        mgr.proc_count += 1;
        if mgr.init_proc.is_null() {
            mgr.init_proc = p;
        }
    }
}

/// 创建内核线程并绑定进程
pub fn spawn_kthread(
    mgr: &mut ProcManager,
    name: &[u8],
    entry: unsafe extern "C" fn() -> !,
    ppid: Pid,
    uid: u32,
    gid: u32,
    stack_size: usize,
) -> Option<Tid> {
    let pid = alloc_pid();
    let proc = alloc_process(pid, ppid, name, uid, gid)?;
    attach_process(mgr, proc);
    let proc_raw = proc.as_ptr();
    let tid = sched::spawn_thread(
        entry,
        stack_size,
        name,
        uid,
        gid,
        pid,
        proc_raw as *mut (),
    )?;
    mgr.thread_count += 1;
    Some(tid)
}

/// 创建带资源配额的内核线程（应用服务专用）
pub fn spawn_kthread_with_quota(
    mgr: &mut ProcManager,
    name: &[u8],
    entry: unsafe extern "C" fn() -> !,
    ppid: Pid,
    uid: u32,
    gid: u32,
    stack_size: usize,
    mem_kb: u64,
) -> Option<Tid> {
    let pid = alloc_pid();
    let proc = alloc_process(pid, ppid, name, uid, gid)?;
    unsafe {
        let p = proc.as_ptr();
        (*p).ring = 3;
        (*p).rlim_stack = stack_size as u64;
        (*p).rlim_as = mem_kb.saturating_mul(1024);
        (*p).rlim_cpu = 1_000_000;
        (*p).nice = 5;
    }
    attach_process(mgr, proc);
    let proc_raw = proc.as_ptr();
    let tid = sched::spawn_thread(
        entry,
        stack_size,
        name,
        uid,
        gid,
        pid,
        proc_raw as *mut (),
    )?;
    mgr.thread_count += 1;
    Some(tid)
}

/// 设置当前活动进程（调度器切换时调用）
pub fn set_active(mgr: &mut ProcManager, proc: *mut Process) {
    mgr.active_proc = proc;
}
