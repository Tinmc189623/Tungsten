
// sched.rs — CFS 调度器 + 上下文切换
// 基于 vruntime 的公平调度, x86_64 长模式
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use core::cell::UnsafeCell;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};
use crate::mm::{pmm, slab};

static RESCHED_PENDING: AtomicBool = AtomicBool::new(false);

/* ── 任务状态 ── */

#[derive(PartialEq)]
#[repr(u8)]
enum TaskState {
    Ready = 0,
    Running = 1,
    Blocked = 2,
    Zombie = 3,
}

/* ── 等待队列 ── */

/// I/O 等待队列 (Phase 9: sleep_on / wake_up)
const WAIT_QUEUE_SIZE: usize = 32;

struct WaitQueue {
    entries: [Option<NonNull<Task>>; WAIT_QUEUE_SIZE],
    tags: [u64; WAIT_QUEUE_SIZE],
    count: usize,
}

impl WaitQueue {
    const fn new() -> Self {
        WaitQueue { entries: [None; WAIT_QUEUE_SIZE], tags: [0; WAIT_QUEUE_SIZE], count: 0 }
    }

    fn enqueue(&mut self, tag: u64, task: NonNull<Task>) {
        if self.count >= WAIT_QUEUE_SIZE { return; }
        self.entries[self.count] = Some(task);
        self.tags[self.count] = tag;
        self.count += 1;
    }

    fn wake(&mut self, tag: u64) -> Option<NonNull<Task>> {
        for i in 0..self.count {
            if self.tags[i] == tag {
                let task = self.entries[i];
                // 前移后续条目
                for j in i..self.count - 1 {
                    self.entries[j] = self.entries[j + 1];
                    self.tags[j] = self.tags[j + 1];
                }
                self.entries[self.count - 1] = None;
                self.tags[self.count - 1] = 0;
                self.count -= 1;
                return task;
            }
        }
        None
    }
}

/* ── 上下文 ── */

/// x86_64 上下文（保存的寄存器）
#[repr(C)]
struct Context {
    r15: u64, r14: u64, r13: u64, r12: u64,
    rbp: u64, rbx: u64,
    rip: u64,
    rsp: u64,
}

/* ── 任务控制块 ── */

/// 任务标识符
pub type Tid = u64;

#[repr(C)]
struct Task {
    id: Tid,
    pid: u64,
    uid: u32,
    gid: u32,
    euid: u32,
    name: [u8; 16],
    name_len: u8,
    state: TaskState,
    vruntime: u64,
    prio: u8,
    context: Context,
    kernel_stack: *mut u8,
    kernel_stack_base: *mut u8,
    /// 绑定的 Process 结构指针
    mm: *mut (),
}

/// Task 中 context.rsp 的偏移（从 Task 起始算起）
const TASK_RSP_OFS: i32 = (core::mem::offset_of!(Task, context) + core::mem::offset_of!(Context, rsp)) as i32;

impl Task {
    /// 创建带用户/进程元数据的内核线程
    fn new_meta(
        id: Tid,
        entry: unsafe extern "C" fn() -> !,
        stack_size: usize,
        name: &[u8],
        uid: u32,
        gid: u32,
        pid: u64,
        proc_ptr: *mut (),
    ) -> Option<NonNull<Task>> {
        let task_ptr = slab::kmalloc(core::mem::size_of::<Task>())?;
        let pages = (stack_size + pmm::PAGE_SIZE as usize - 1) / pmm::PAGE_SIZE as usize;
        let order = pages.next_power_of_two().trailing_zeros() as u8;
        let stack_paddr = pmm::alloc_pages(order)?;
        let stack_top = unsafe { (stack_paddr as *mut u8).add(pages * pmm::PAGE_SIZE as usize) };

        let stack_top_u64 = stack_top as *mut u64;
        unsafe {
            core::ptr::write(stack_top_u64.sub(1), entry as u64);
            core::ptr::write(stack_top_u64.sub(2), 0u64);
            core::ptr::write(stack_top_u64.sub(3), 0u64);
            core::ptr::write(stack_top_u64.sub(4), 0u64);
            core::ptr::write(stack_top_u64.sub(5), 0u64);
            core::ptr::write(stack_top_u64.sub(6), 0u64);
            core::ptr::write(stack_top_u64.sub(7), 0u64);
        }

        let mut tname = [0u8; 16];
        let nlen = name.len().min(16);
        tname[..nlen].copy_from_slice(&name[..nlen]);

        let task = unsafe {
            core::ptr::write(
                task_ptr.as_ptr() as *mut Task,
                Task {
                    id,
                    pid,
                    uid,
                    gid,
                    euid: uid,
                    name: tname,
                    name_len: nlen as u8,
                    state: TaskState::Ready,
                    vruntime: 0,
                    prio: 128,
                    context: Context {
                        r15: 0, r14: 0, r13: 0, r12: 0,
                        rbp: 0, rbx: 0,
                        rip: entry as u64,
                        rsp: (stack_top as u64).wrapping_sub(7 * 8),
                    },
                    kernel_stack: stack_top,
                    kernel_stack_base: stack_paddr as *mut u8,
                    mm: proc_ptr,
                },
            );
            NonNull::new_unchecked(task_ptr.as_ptr() as *mut Task)
        };

        Some(task)
    }

    fn new(id: Tid, entry: unsafe extern "C" fn() -> !, stack_size: usize) -> Option<NonNull<Task>> {
        Self::new_meta(id, entry, stack_size, b"kthread", 0, 0, id, core::ptr::null_mut())
    }
}

/* ── CFS 就绪队列 ── */

/// CFS 就绪队列（简化为排序链表）
struct CfsRq {
    tasks: [Option<NonNull<Task>>; 64],
    count: usize,
    min_vruntime: u64,
}

impl CfsRq {
    const fn new() -> Self {
        CfsRq {
            tasks: [None; 64],
            count: 0,
            min_vruntime: 0,
        }
    }

    /// 插入任务（按 vruntime 升序）
    fn enqueue(&mut self, task: NonNull<Task>) {
        if self.count >= self.tasks.len() { return; }
        let vruntime = unsafe { (*task.as_ptr()).vruntime };
        let mut i = self.count;
        while i > 0 {
            let prev = self.tasks[i - 1].unwrap();
            if unsafe { (*prev.as_ptr()).vruntime } <= vruntime { break; }
            self.tasks[i] = self.tasks[i - 1];
            i -= 1;
        }
        self.tasks[i] = Some(task);
        self.count += 1;
        if vruntime < self.min_vruntime {
            self.min_vruntime = vruntime;
        }
    }

    /// 取出最小 vruntime 的任务
    fn dequeue_min(&mut self) -> Option<NonNull<Task>> {
        if self.count == 0 { return None; }
        let task = self.tasks[0].take();
        self.count -= 1;
        for i in 0..self.count {
            self.tasks[i] = self.tasks[i + 1].take();
        }
        task
    }

    /// 查看最小 vruntime 任务
    fn peek_min(&self) -> Option<NonNull<Task>> {
        self.tasks[0]
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/* ── 调度器 ── */

struct Scheduler {
    rq: CfsRq,
    current: Option<NonNull<Task>>,
    next_tid: Tid,
    ticks: u64,
    wait_queue: WaitQueue,
    /// 时间片计数，到期触发抢占
    slice_ticks: u32,
}

impl Scheduler {
    const fn new() -> Self {
        Scheduler {
            rq: CfsRq::new(),
            current: None,
            next_tid: 1,
            ticks: 0,
            wait_queue: WaitQueue::new(),
            slice_ticks: 0,
        }
    }

    /// 创建内核任务（带进程/用户元数据）
    fn spawn_thread(
        &mut self,
        entry: unsafe extern "C" fn() -> !,
        stack_size: usize,
        name: &[u8],
        uid: u32,
        gid: u32,
        pid: u64,
        proc_ptr: *mut (),
    ) -> Option<Tid> {
        let tid = self.next_tid;
        self.next_tid += 1;
        let task = Task::new_meta(tid, entry, stack_size, name, uid, gid, pid, proc_ptr)?;
        self.rq.enqueue(task);
        Some(tid)
    }

    /// 创建内核任务
    fn spawn(&mut self, entry: unsafe extern "C" fn() -> !, stack_size: usize) -> Option<Tid> {
        let tid = self.next_tid;
        self.next_tid += 1;
        let task = Task::new(tid, entry, stack_size)?;
        self.rq.enqueue(task);
        Some(tid)
    }

    /// 时钟滴答（由定时器中断调用，满时间片标记抢占）
    fn tick(&mut self) {
        self.ticks += 1;
        if let Some(current) = self.current {
            unsafe {
                (*current.as_ptr()).vruntime += 1;
            }
            self.slice_ticks += 1;
        }
    }

    /// 是否应抢占当前任务
    fn need_resched(&self) -> bool {
        self.slice_ticks >= 50 && self.rq.count > 0
    }

    fn clear_slice(&mut self) {
        self.slice_ticks = 0;
    }
}

/* ── 上下文切换 ── */

/// 保存当前任务 context，切换到下一个任务。
/// 永不返回 —— 恢复 next 的保存现场后从它上次中断点继续执行。
unsafe fn context_switch(current: *mut Task, next: *mut Task) -> ! {
    core::arch::asm!(
        // 保存当前任务的 callee-saved 寄存器
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        // 保存 RSP 到 current->context.rsp
        "mov [{cur} + {rsp_ofs}], rsp",
        // 从 next->context.rsp 恢复 RSP
        "mov rsp, [{nxt} + {rsp_ofs}]",
        // 恢复下一个任务的寄存器
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "ret",
        cur = in(reg) current,
        nxt = in(reg) next,
        rsp_ofs = const TASK_RSP_OFS,
        options(noreturn),
    )
}

/// 启动第一个任务（不保存当前上下文，因为没有上一个任务）。
unsafe fn start_first_task(next: *mut Task) -> ! {
    core::arch::asm!(
        "mov rsp, [{nxt} + {rsp_ofs}]",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "ret",
        nxt = in(reg) next,
        rsp_ofs = const TASK_RSP_OFS,
        options(noreturn),
    )
}

/* ── 全局调度器 ── */

struct SchedWrapper(UnsafeCell<Scheduler>);
unsafe impl Sync for SchedWrapper {}

static SCHED: SchedWrapper = SchedWrapper(UnsafeCell::new(Scheduler::new()));

fn sched() -> &'static mut Scheduler {
    unsafe { &mut *SCHED.0.get() }
}

/* ── 导出的 API ── */

/// 初始化调度器
pub fn init() {
    // 当前执行上下文成为初始 "current"（创建 idle 任务太早，暂时留空）
    // 第一个 spawn 后调用 start() 开始调度
}

/// 创建带用户/进程信息的内核线程
pub fn spawn_thread(
    entry: unsafe extern "C" fn() -> !,
    stack_size: usize,
    name: &[u8],
    uid: u32,
    gid: u32,
    pid: u64,
    proc_ptr: *mut (),
) -> Option<Tid> {
    sched().spawn_thread(entry, stack_size, name, uid, gid, pid, proc_ptr)
}

/// 创建内核线程
pub fn spawn(entry: unsafe extern "C" fn() -> !, stack_size: usize) -> Option<Tid> {
    sched().spawn(entry, stack_size)
}

/// 时钟滴答；满时间片时标记需要重新调度
pub fn tick() {
    let s = sched();
    s.tick();
    if s.need_resched() {
        RESCHED_PENDING.store(true, Ordering::Relaxed);
    }
}

/// 查询并清除抢占标记
pub fn take_resched() -> bool {
    RESCHED_PENDING.swap(false, Ordering::Relaxed)
}

/// 切换任务时绑定活动进程
fn bind_active_process(task: NonNull<Task>) {
    unsafe {
        let proc = (*task.as_ptr()).mm as *mut crate::proc::Process;
        crate::proc::set_active_proc(proc);
    }
}

/// 主动让出 CPU
pub fn yield_now() {
    let sched = sched();
    sched.clear_slice();
    if sched.rq.is_empty() {
        return;
    }

    let current = match sched.current.take() {
        Some(c) => c,
        None => {
            if let Some(next) = sched.pick_next() {
                unsafe { (*next.as_ptr()).state = TaskState::Running; }
                sched.current = Some(next);
                bind_active_process(next);
                unsafe { start_first_task(next.as_ptr()); }
            }
            return;
        }
    };

    unsafe { (*current.as_ptr()).state = TaskState::Ready; }
    sched.rq.enqueue(current);

    let next = match sched.pick_next() {
        Some(n) => n,
        None => {
            unsafe { (*current.as_ptr()).state = TaskState::Running; }
            sched.current = Some(current);
            return;
        }
    };

    if next.as_ptr() == current.as_ptr() {
        unsafe { (*current.as_ptr()).state = TaskState::Running; }
        sched.current = Some(current);
        return;
    }

    unsafe { (*next.as_ptr()).state = TaskState::Running; }
    sched.current = Some(next);
    bind_active_process(next);
    unsafe { context_switch(current.as_ptr(), next.as_ptr()); }
}

/// 返回当前任务 TID（无任务时返回 0）
pub fn current_tid() -> Tid {
    sched().current.as_ref().map_or(0, |c| unsafe { (*c.as_ptr()).id })
}

/// 当前任务绑定的 PID
pub fn current_pid() -> u64 {
    sched().current.as_ref().map_or(0, |c| unsafe { (*c.as_ptr()).pid })
}

/// 当前线程真实 UID
pub fn current_uid() -> u32 {
    sched().current.as_ref().map_or(0, |c| unsafe { (*c.as_ptr()).uid })
}

/// 当前线程有效 UID
pub fn current_euid() -> u32 {
    sched().current.as_ref().map_or(0, |c| unsafe { (*c.as_ptr()).euid })
}

/// 当前线程 GID
pub fn current_gid() -> u32 {
    sched().current.as_ref().map_or(0, |c| unsafe { (*c.as_ptr()).gid })
}

/// 切换当前线程用户凭证（login / su）
pub fn set_current_credentials(uid: u32, gid: u32) -> bool {
    if let Some(cur) = sched().current {
        unsafe {
            let t = cur.as_ptr();
            (*t).uid = uid;
            (*t).gid = gid;
            (*t).euid = uid;
            if !(*t).mm.is_null() {
                let p = (*t).mm as *mut crate::proc::Process;
                (*p).uid = uid;
                (*p).gid = gid;
                (*p).euid = uid;
                (*p).egid = gid;
            }
        }
        true
    } else {
        false
    }
}

/// 降低当前任务调度优先级（占用过多 CPU 时调用）
pub fn penalize_current(amount: u64) {
    if let Some(cur) = sched().current {
        unsafe {
            (*cur.as_ptr()).vruntime = (*cur.as_ptr()).vruntime.saturating_add(amount);
        }
    }
}

/// 降低指定 TID 的调度优先级
pub fn penalize_tid(tid: Tid, amount: u64) {
    let s = sched();
    if let Some(cur) = s.current {
        if unsafe { (*cur.as_ptr()).id } == tid {
            penalize_current(amount);
            return;
        }
    }
    for i in 0..s.rq.count {
        if let Some(t) = s.rq.tasks[i] {
            if unsafe { (*t.as_ptr()).id } == tid {
                unsafe {
                    (*t.as_ptr()).vruntime = (*t.as_ptr()).vruntime.saturating_add(amount);
                }
                return;
            }
        }
    }
}

/// 列出所有就绪/运行中任务（tid pid uid name state）
pub fn format_task_list(buf: &mut [u8]) -> usize {
    let s = sched();
    let mut pos = 0;
    let mut push_line = |line: &[u8]| {
        if pos + line.len() + 1 > buf.len() {
            return;
        }
        buf[pos..pos + line.len()].copy_from_slice(line);
        pos += line.len();
        buf[pos] = b'\n';
        pos += 1;
    };
    push_line(b"TID  PID  UID  NAME         ST");
    if let Some(cur) = s.current {
        let mut line = [b' '; 40];
        fill_task_line(&mut line, unsafe { cur.as_ptr() }, b'R');
        push_line(&line);
    }
    for i in 0..s.rq.count {
        if let Some(t) = s.rq.tasks[i] {
            let mut line = [b' '; 40];
            fill_task_line(&mut line, unsafe { t.as_ptr() }, b'Q');
            push_line(&line);
        }
    }
    pos
}

/// 填充任务信息行
fn fill_task_line(line: &mut [u8; 40], task: *const Task, st: u8) {
    let t = unsafe { &*task };
    write_field(&mut line[0..6], t.id);
    write_field(&mut line[6..12], t.pid);
    write_field(&mut line[12..17], t.euid as u64);
    let n = t.name_len as usize;
    line[17..17 + n].copy_from_slice(&t.name[..n]);
    line[37] = st;
}

/// 将数字写入固定宽度字段
fn write_field(buf: &mut [u8], val: u64) {
    let mut tmp = [0u8; 20];
    let mut n = val;
    let mut len = 0usize;
    if n == 0 {
        buf[0] = b'0';
        return;
    }
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    let start = buf.len().saturating_sub(len);
    for i in 0..len {
        if start + i < buf.len() {
            buf[start + i] = tmp[len - 1 - i];
        }
    }
}

/// 获取系统 tick 计数 (供 inode 时间戳等使用)
pub fn ticks() -> u64 {
    sched().ticks
}

/// 退出当前任务
pub fn exit(_code: i32) -> ! {
    let sched = sched();
    if let Some(current) = sched.current.take() {
        unsafe { (*current.as_ptr()).state = TaskState::Zombie; }
    }
    // 选择下一个任务运行
    loop {
        if let Some(next) = sched.pick_next() {
            unsafe { (*next.as_ptr()).state = TaskState::Running; }
            sched.current = Some(next);
            unsafe { start_first_task(next.as_ptr()); }
        }
        // 无可用任务 — halt
        unsafe { core::arch::asm!("cli; hlt"); }
    }
}

/// 让当前任务在 tag 上睡眠, 等待 wake_up(tag)
pub fn sleep_on(tag: u64) {
    let sched = sched();
    if let Some(current) = sched.current.take() {
        unsafe { (*current.as_ptr()).state = TaskState::Blocked; }
        sched.wait_queue.enqueue(tag, current);
    }
    // 选择下一个任务运行
    if let Some(next) = sched.pick_next() {
        unsafe { (*next.as_ptr()).state = TaskState::Running; }
        sched.current = Some(next);
        unsafe { start_first_task(next.as_ptr()); }
    } else {
        unsafe { core::arch::asm!("cli; hlt"); }
    }
}

/// 唤醒等待 tag 的任务
pub fn wake_up(tag: u64) {
    let sched = sched();
    if let Some(task) = sched.wait_queue.wake(tag) {
        unsafe { (*task.as_ptr()).state = TaskState::Ready; }
        sched.rq.enqueue(task);
    }
}

/// 开始调度（从第一个任务启动，永不返回）
pub fn start() -> ! {
    let sched = sched();
    let next = sched.pick_next().expect("sched::start: no tasks");
    unsafe {
        (*next.as_ptr()).state = TaskState::Running;
        sched.current = Some(next);
        bind_active_process(next);
        start_first_task(next.as_ptr());
    }
}

/* ── 调度选择（内部） ── */

impl Scheduler {
    fn pick_next(&mut self) -> Option<NonNull<Task>> {
        self.rq.dequeue_min()
    }
}

/// 返回当前任务数量
pub fn task_count() -> usize {
    sched().rq.count
}

/// 返回调度器 tick 计数（系统运行时间）
pub fn uptime_ticks() -> u64 {
    sched().ticks
}
