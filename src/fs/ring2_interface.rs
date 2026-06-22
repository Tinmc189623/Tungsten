// fs/ring2_interface.rs — Ring 2 调用门接口 (Phase 9 完整实现)
// FS 子系统运行于 Ring 2, 通过共享环形缓冲区 + IPC 与 Ring 0 通信
// 跨环段 I/O 完成通知通过 syscall 0x1000 异步传递
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::arch::x86_64::gdt::selector;
use core::sync::atomic::{AtomicU64, Ordering};

// ── 调用门常量 ──

pub const RING2_CALL_GATE: u16 = 0x50;
pub const RING2_CS: u16 = selector::RING2_CODE;
pub const RING2_DS: u16 = selector::RING2_DATA;

// ── Ring 2 I/O 请求 ──

const RB_CAPACITY: usize = 64;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Ring2IoRequest {
    pub ino: u64,
    pub op: u8,         // 0=read, 1=write, 2=fsync, 3=truncate
    pub offset: u64,
    pub length: u64,
    pub buf_phys: u64,  // 数据缓冲区物理地址
    pub tag: u64,       // 请求标签 (用于完成通知匹配)
}

impl Ring2IoRequest {
    pub const fn empty() -> Self {
        Ring2IoRequest { ino: 0, op: 0, offset: 0, length: 0, buf_phys: 0, tag: 0 }
    }
}

// ── 共享环形缓冲区 ──

struct Ring2RingBuffer {
    buffer: [Ring2IoRequest; RB_CAPACITY],
    head: AtomicU64,    // Ring 0 写入位置
    tail: AtomicU64,    // Ring 2 读取位置
}

impl Ring2RingBuffer {
    const fn new() -> Self {
        Ring2RingBuffer {
            buffer: [Ring2IoRequest::empty(); RB_CAPACITY],
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
        }
    }

    /// Ring 0 推入请求 (非阻塞)
    fn push(&self, req: Ring2IoRequest) -> Result<(), ()> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= RB_CAPACITY as u64 {
            return Err(());
        }
        let idx = (head % RB_CAPACITY as u64) as usize;
        unsafe {
            (self.buffer.as_ptr() as *mut Ring2IoRequest).add(idx).write_volatile(req);
        }
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// Ring 2 取出请求 (非阻塞)
    fn pop(&self) -> Option<Ring2IoRequest> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == head {
            return None;
        }
        let idx = (tail % RB_CAPACITY as u64) as usize;
        let req = unsafe {
            (self.buffer.as_ptr() as *const Ring2IoRequest).add(idx).read_volatile()
        };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(req)
    }

    fn is_empty(&self) -> bool {
        self.head.load(Ordering::Relaxed) == self.tail.load(Ordering::Relaxed)
    }
}

// ── Ring 2 FS 任务上下文 ──

pub struct Ring2FsTask {
    pub entry: u64,
    pub rsp: u64,
    pub tid: u64,
    pub active: bool,
}

impl Ring2FsTask {
    pub const fn new() -> Self {
        Ring2FsTask { entry: 0, rsp: 0, tid: 0, active: false }
    }
}

// ── 全局 Ring 2 状态 ──

use core::cell::UnsafeCell;

struct Ring2State {
    rb: Ring2RingBuffer,
    fs_task: Ring2FsTask,
    completion_pending: bool,
}

impl Ring2State {
    const fn new() -> Self {
        Ring2State {
            rb: Ring2RingBuffer::new(),
            fs_task: Ring2FsTask::new(),
            completion_pending: false,
        }
    }
}

struct Ring2Wrapper(UnsafeCell<Ring2State>);
unsafe impl Sync for Ring2Wrapper {}

static RING2: Ring2Wrapper = Ring2Wrapper(UnsafeCell::new(Ring2State::new()));

fn ring2_state() -> &'static mut Ring2State {
    unsafe { &mut *RING2.0.get() }
}

// ── Ring 2 入口 ──

/// Ring 2 FS 任务入口
/// 在 Ring 2 特权级运行, 循环处理 I/O 请求
#[unsafe(no_mangle)]
pub extern "C" fn ring2_fs_entry() -> ! {
    // 设置 Ring 2 数据段
    unsafe {
        core::arch::asm!(
            "mov ds, {0:x}",
            "mov es, {0:x}",
            "mov fs, {0:x}",
            "mov gs, {0:x}",
            in(reg) RING2_DS as u64,
        );
    }

    // FS 事件循环
    loop {
        let state = ring2_state();
        match state.rb.pop() {
            Some(req) => {
                // 处理 I/O 请求
                match req.op {
                    0 => {
                        // 读操作
                        let mut buf = [0u8; 512];
                        let n = crate::fs::fs_fs::file::read_file_data(req.ino, req.offset, &mut buf[..req.length as usize]);
                        // 将数据写回请求缓冲区
                        if req.buf_phys != 0 && n > 0 {
                            let _ = crate::fs::ramdisk::get_ramdisk_device().write_bytes(req.buf_phys, &buf[..n]);
                        }
                        notify_io_completion(req.tag, n as i32);
                    }
                    1 => {
                        // 写操作: 读取数据 → 写入文件
                        let mut buf = [0u8; 512];
                        let len = req.length.min(512);
                        if req.buf_phys != 0 {
                            let _ = crate::fs::ramdisk::get_ramdisk_device().read_bytes(req.buf_phys, &mut buf[..len as usize]);
                        }
                        let n = crate::fs::fs_fs::file::write_file_data(req.ino, req.offset, &buf[..len as usize]);
                        notify_io_completion(req.tag, n as i32);
                    }
                    2 => {
                        // fsync
                        let _ = crate::fs::fs_fs::file::fsync_file(req.ino);
                        notify_io_completion(req.tag, 0);
                    }
                    3 => {
                        // truncate
                        if let Ok(mut tree) = crate::fs::fs_fs::ExtentTree::load(req.ino) {
                            let _ = tree.truncate(req.length);
                        }
                        notify_io_completion(req.tag, 0);
                    }
                    _ => {
                        notify_io_completion(req.tag, -78); // ENOSYS
                    }
                }
            }
            None => {
                // 队列空, 让出 CPU
                core::hint::spin_loop();
            }
        }
    }
}

// ── Ring 2 初始化 ──

pub fn init_ring2_fs() {
    crate::serial::write_str(b"  ring2: initializing FS task...\n");

    let state = ring2_state();
    state.fs_task.entry = ring2_fs_entry as u64;
    state.fs_task.rsp = 0x9_D000; // TSS RSP2
    state.fs_task.active = true;

    crate::serial::write_str(b"  ring2: FS task registered (entry=ring2_fs_entry)\n");
}

// ── 跨环 I/O 完成通知 ──

/// Ring 2 → Ring 0: 通知 I/O 完成
pub fn notify_io_completion(tag: u64, status: i32) {
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") SYS_RING2_COMPLETION_ID => _,
            in("rdi") tag,
            in("rsi") status as u64,
        );
    }
}

const SYS_RING2_COMPLETION_ID: u64 = 0x1000;

/// Ring 0 侧: 处理 Ring 2 I/O 完成通知
pub fn handle_ring2_completion(tag: u64, status: i32) {
    crate::serial::write_str(b"  ring2: I/O complete tag=");
    crate::serial_put_u64(tag);
    crate::serial::write_str(b" status=");
    crate::serial_put_u64(status as u64);
    crate::serial::write_str(b"\n");

    // 唤醒等待此 I/O 完成的内核任务
    crate::sched::wake_up(tag);
}

// ── Ring 0 → Ring 2: 发送 I/O 请求 ──

pub fn send_io_request(ino: u64, op: u8, offset: u64, length: u64, buf_phys: u64, tag: u64) -> Result<(), ()> {
    let req = Ring2IoRequest { ino, op, offset, length, buf_phys, tag };
    let state = ring2_state();
    state.rb.push(req)
}

/// 同步 I/O: 发送请求并等待完成 (Ring 0 侧)
pub fn sync_io_request(ino: u64, op: u8, offset: u64, length: u64, buf_phys: u64) -> i32 {
    let tag = crate::sched::current_tid();
    if send_io_request(ino, op, offset, length, buf_phys, tag).is_err() {
        return -11; // EAGAIN
    }
    // 阻塞等待完成
    crate::sched::sleep_on(tag);
    0
}
