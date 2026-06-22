// ipc.rs — 进程间通信子系统
// Pipe（单向字节流）、SharedMemory（共享内存）、MessageQueue（消息队列）
// Channel（有界环形缓冲区）+ 路由表
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use crate::sync::Spinlock;
use core::sync::atomic::{AtomicU64, Ordering};

/* ══════════════════════════════════════════════
   消息类型与结构
   ══════════════════════════════════════════════ */

/// IPC 消息类型
#[repr(u64)]
pub enum IpcMsgType {
    /// 空消息
    None = 0,
    /// 数据消息
    Data = 1,
    /// 控制消息（信号、通知）
    Control = 2,
    /// 异常消息（错误传递）
    Exception = 3,
}

/// IPC 消息（64 字节定长，无堆分配）
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Message {
    /// 消息类型（IpcMsgType 数值）
    pub msg_type: u64,
    /// 发送方任务 ID
    pub src_tid: u64,
    /// 接收方任务 ID
    pub dst_tid: u64,
    /// payload 长度（字节）
    pub data_len: u64,
    /// payload（最大 40 字节）
    pub data: [u8; 40],
}

impl Message {
    /// 创建空消息
    pub const fn empty() -> Self {
        Message {
            msg_type: 0,
            src_tid: 0,
            dst_tid: 0,
            data_len: 0,
            data: [0u8; 40],
        }
    }

    /// 构造数据消息
    pub fn data(src: u64, dst: u64, payload: &[u8]) -> Self {
        let mut m = Message::empty();
        m.msg_type = IpcMsgType::Data as u64;
        m.src_tid = src;
        m.dst_tid = dst;
        let len = payload.len().min(40);
        m.data_len = len as u64;
        m.data[..len].copy_from_slice(&payload[..len]);
        m
    }

    /// 构造控制消息
    pub fn control(src: u64, dst: u64, code: u8) -> Self {
        let mut m = Message::empty();
        m.msg_type = IpcMsgType::Control as u64;
        m.src_tid = src;
        m.dst_tid = dst;
        m.data_len = 1;
        m.data[0] = code;
        m
    }
}

/* ══════════════════════════════════════════════
   Channel — 有界环形缓冲区（线程安全 FIFO）
   ══════════════════════════════════════════════ */

/// 通道容量（消息数）
const CHANNEL_CAPACITY: usize = 64;

/// IPC 通道 — 线程安全的有界 FIFO
pub struct Channel {
    buffer: [Message; CHANNEL_CAPACITY],
    head: AtomicU64,
    tail: AtomicU64,
}

impl Channel {
    /// 创建空通道
    pub const fn new() -> Self {
        Channel {
            buffer: [Message::empty(); CHANNEL_CAPACITY],
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
        }
    }

    /// 发送消息（非阻塞），通道满时返回 Err
    pub fn send(&self, msg: &Message) -> Result<(), ()> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= CHANNEL_CAPACITY as u64 {
            return Err(());
        }
        let idx = (tail % CHANNEL_CAPACITY as u64) as usize;
        unsafe {
            (self.buffer.as_ptr() as *mut Message)
                .add(idx)
                .write_volatile(*msg);
        }
        self.tail
            .store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// 接收消息（非阻塞），通道空时返回 None
    pub fn recv(&self) -> Option<Message> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        let idx = (head % CHANNEL_CAPACITY as u64) as usize;
        let msg = unsafe {
            (self.buffer.as_ptr() as *const Message)
                .add(idx)
                .read_volatile()
        };
        self.head
            .store(head.wrapping_add(1), Ordering::Release);
        Some(msg)
    }

    /// 判断通道是否为空
    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Relaxed) == self.tail.load(Ordering::Relaxed)
    }

    /// 判断通道是否已满
    pub fn is_full(&self) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Relaxed);
        tail.wrapping_sub(head) >= CHANNEL_CAPACITY as u64
    }
}

/* ══════════════════════════════════════════════
   Endpoint — Spinlock 保护的通道端点
   ══════════════════════════════════════════════ */

/// 通道端点，持有 Spinlock 保护的 Channel
pub struct Endpoint {
    channel: Spinlock<Channel>,
}

impl Endpoint {
    /// 创建新的通道端点
    pub const fn new() -> Self {
        Endpoint {
            channel: Spinlock::new(Channel::new()),
        }
    }

    /// 通过端点发送消息
    pub fn send(&self, msg: &Message) -> Result<(), ()> {
        self.channel.lock().send(msg)
    }

    /// 通过端点接收消息
    pub fn recv(&self) -> Option<Message> {
        self.channel.lock().recv()
    }
}

/* ══════════════════════════════════════════════
   Pipe — 单向字节流管道
   ══════════════════════════════════════════════ */

/// 管道缓冲区大小（4KB）
const PIPE_BUF_SIZE: usize = 4096;

/// 单向字节流管道（环形缓冲区 + Spinlock 保护）
///
/// 生产者写入字节，消费者读取字节。缓冲区满时写操作阻塞（返回实际写入量），
/// 缓冲区空时读操作返回 0 字节。
pub struct Pipe {
    /// 环形缓冲区
    buf: [u8; PIPE_BUF_SIZE],
    /// 读指针
    read_pos: usize,
    /// 写指针
    write_pos: usize,
    /// 缓冲区中有效字节数
    count: usize,
}

impl Pipe {
    /// 创建空管道
    pub const fn new() -> Self {
        Pipe {
            buf: [0u8; PIPE_BUF_SIZE],
            read_pos: 0,
            write_pos: 0,
            count: 0,
        }
    }

    /// 向管道写入数据，返回实际写入的字节数
    pub fn write(&mut self, data: &[u8]) -> usize {
        let available = PIPE_BUF_SIZE - self.count;
        let to_write = data.len().min(available);
        for i in 0..to_write {
            self.buf[self.write_pos] = data[i];
            self.write_pos = (self.write_pos + 1) % PIPE_BUF_SIZE;
        }
        self.count += to_write;
        to_write
    }

    /// 从管道读取数据，返回实际读取的字节数
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let to_read = buf.len().min(self.count);
        for i in 0..to_read {
            buf[i] = self.buf[self.read_pos];
            self.read_pos = (self.read_pos + 1) % PIPE_BUF_SIZE;
        }
        self.count -= to_read;
        to_read
    }

    /// 返回管道中可读的字节数
    pub fn available(&self) -> usize {
        self.count
    }

    /// 判断管道是否为空
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// 判断管道是否已满
    pub fn is_full(&self) -> bool {
        self.count >= PIPE_BUF_SIZE
    }
}

/// Spinlock 保护的管道（线程安全）
pub struct SyncPipe {
    inner: Spinlock<Pipe>,
}

impl SyncPipe {
    /// 创建线程安全的管道
    pub const fn new() -> Self {
        SyncPipe {
            inner: Spinlock::new(Pipe::new()),
        }
    }

    /// 写入数据到管道
    pub fn write(&self, data: &[u8]) -> usize {
        self.inner.lock().write(data)
    }

    /// 从管道读取数据
    pub fn read(&self, buf: &mut [u8]) -> usize {
        self.inner.lock().read(buf)
    }

    /// 返回管道中可读字节数
    pub fn available(&self) -> usize {
        self.inner.lock().available()
    }
}

/* ══════════════════════════════════════════════
   SharedMemory — 共享内存区域
   ══════════════════════════════════════════════ */

/// 最大共享内存区域数量
const MAX_SHM_REGIONS: usize = 32;

/// 共享内存区域描述
pub struct ShmRegion {
    /// 区域 ID
    pub id: u64,
    /// 物理地址基址
    pub phys_addr: u64,
    /// 区域大小（字节）
    pub size: u64,
    /// 引用计数（附加的进程数）
    pub ref_count: u32,
    /// 是否有效
    pub active: bool,
}

impl ShmRegion {
    /// 创建空的共享内存区域
    const fn empty() -> Self {
        ShmRegion {
            id: 0,
            phys_addr: 0,
            size: 0,
            ref_count: 0,
            active: false,
        }
    }
}

/// 共享内存管理器
pub struct ShmManager {
    regions: [ShmRegion; MAX_SHM_REGIONS],
    next_id: u64,
}

impl ShmManager {
    /// 创建空的共享内存管理器
    pub const fn new() -> Self {
        const EMPTY: ShmRegion = ShmRegion::empty();
        ShmManager {
            regions: [EMPTY; MAX_SHM_REGIONS],
            next_id: 1,
        }
    }

    /// 创建新的共享内存区域，返回区域 ID
    pub fn create(&mut self, size: u64) -> Option<u64> {
        use crate::mm::pmm;

        // 分配物理页
        let pages = ((size + pmm::PAGE_SIZE - 1) / pmm::PAGE_SIZE) as usize;
        let order = pages.next_power_of_two().trailing_zeros() as u8;
        let paddr = pmm::alloc_pages(order)?;

        // 清零
        unsafe {
            core::ptr::write_bytes(
                paddr as *mut u8,
                0,
                pages * pmm::PAGE_SIZE as usize,
            );
        }

        // 查找空槽
        for region in self.regions.iter_mut() {
            if !region.active {
                let id = self.next_id;
                self.next_id += 1;
                region.id = id;
                region.phys_addr = paddr;
                region.size = size;
                region.ref_count = 1;
                region.active = true;
                return Some(id);
            }
        }

        // 没有空槽，释放已分配的页
        pmm::free_pages(paddr, order);
        None
    }

    /// 附加到共享内存区域（增加引用计数）
    pub fn attach(&mut self, id: u64) -> Option<u64> {
        for region in self.regions.iter_mut() {
            if region.active && region.id == id {
                region.ref_count += 1;
                return Some(region.phys_addr);
            }
        }
        None
    }

    /// 分离共享内存区域（减少引用计数，为零时释放）
    pub fn detach(&mut self, id: u64) {
        use crate::mm::pmm;

        for region in self.regions.iter_mut() {
            if region.active && region.id == id {
                region.ref_count = region.ref_count.saturating_sub(1);
                if region.ref_count == 0 {
                    let pages = ((region.size + pmm::PAGE_SIZE - 1) / pmm::PAGE_SIZE)
                        as usize;
                    let order =
                        pages.next_power_of_two().trailing_zeros() as u8;
                    pmm::free_pages(region.phys_addr, order);
                    region.active = false;
                    region.phys_addr = 0;
                    region.size = 0;
                }
                return;
            }
        }
    }

    /// 获取共享内存区域的物理地址
    pub fn get_addr(&self, id: u64) -> Option<u64> {
        for region in self.regions.iter() {
            if region.active && region.id == id {
                return Some(region.phys_addr);
            }
        }
        None
    }
}

/* ══════════════════════════════════════════════
   MessageQueue — 消息队列
   ══════════════════════════════════════════════ */

/// 消息队列容量
const MSG_QUEUE_CAPACITY: usize = 128;

/// 消息队列（有界 FIFO，Spinlock 保护）
///
/// 支持多生产者多消费者的消息传递。
pub struct MessageQueue {
    buffer: [Message; MSG_QUEUE_CAPACITY],
    head: usize,
    tail: usize,
    count: usize,
}

impl MessageQueue {
    /// 创建空消息队列
    pub const fn new() -> Self {
        MessageQueue {
            buffer: [Message::empty(); MSG_QUEUE_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    /// 发送消息到队列，队列满时返回 Err
    pub fn send(&mut self, msg: &Message) -> Result<(), ()> {
        if self.count >= MSG_QUEUE_CAPACITY {
            return Err(());
        }
        self.buffer[self.tail] = *msg;
        self.tail = (self.tail + 1) % MSG_QUEUE_CAPACITY;
        self.count += 1;
        Ok(())
    }

    /// 从队列接收消息，队列空时返回 None
    pub fn recv(&mut self) -> Option<Message> {
        if self.count == 0 {
            return None;
        }
        let msg = self.buffer[self.head];
        self.head = (self.head + 1) % MSG_QUEUE_CAPACITY;
        self.count -= 1;
        Some(msg)
    }

    /// 窥视队首消息（不取出）
    pub fn peek(&self) -> Option<&Message> {
        if self.count == 0 {
            return None;
        }
        Some(&self.buffer[self.head])
    }

    /// 返回队列中待处理的消息数
    pub fn len(&self) -> usize {
        self.count
    }

    /// 判断队列是否为空
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// 线程安全的消息队列
pub struct SyncMessageQueue {
    inner: Spinlock<MessageQueue>,
}

impl SyncMessageQueue {
    /// 创建线程安全的消息队列
    pub const fn new() -> Self {
        SyncMessageQueue {
            inner: Spinlock::new(MessageQueue::new()),
        }
    }

    /// 发送消息
    pub fn send(&self, msg: &Message) -> Result<(), ()> {
        self.inner.lock().send(msg)
    }

    /// 接收消息
    pub fn recv(&self) -> Option<Message> {
        self.inner.lock().recv()
    }

    /// 返回队列长度
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }
}

/* ══════════════════════════════════════════════
   IPC 路由表（系统级）
   ══════════════════════════════════════════════ */

/// IPC 路由表大小
const IPC_ROUTE_MAX: usize = 32;

/// 路由项：目标 TID → 通道端点
#[derive(Clone, Copy)]
struct IpcRoute {
    tid: u64,
    endpoint: &'static Endpoint,
}

/// IPC 管理器（路由表 + 共享内存管理）
pub struct IpcManager {
    routes: [Option<IpcRoute>; IPC_ROUTE_MAX],
    route_count: usize,
    shm: ShmManager,
}

impl IpcManager {
    /// 创建 IPC 管理器
    pub const fn new() -> Self {
        IpcManager {
            routes: [None; IPC_ROUTE_MAX],
            route_count: 0,
            shm: ShmManager::new(),
        }
    }

    /// 注册通道端点到指定 TID
    pub fn register(&mut self, tid: u64, ep: &'static Endpoint) -> Result<(), ()> {
        if self.route_count >= IPC_ROUTE_MAX {
            return Err(());
        }
        for r in self.routes.iter() {
            if let Some(route) = r {
                if route.tid == tid {
                    return Err(());
                }
            }
        }
        self.routes[self.route_count] = Some(IpcRoute {
            tid,
            endpoint: ep,
        });
        self.route_count += 1;
        Ok(())
    }

    /// 发送消息到目标 TID（通过路由表查找端点）
    pub fn send_to(&self, msg: &Message) -> Result<(), ()> {
        let dst = msg.dst_tid;
        for r in self.routes.iter() {
            if let Some(route) = r {
                if route.tid == dst {
                    return route.endpoint.send(msg);
                }
            }
        }
        Err(())
    }

    /// 创建共享内存区域
    pub fn create_shm(&mut self, size: u64) -> Option<u64> {
        self.shm.create(size)
    }

    /// 附加到共享内存区域
    pub fn attach_shm(&mut self, id: u64) -> Option<u64> {
        self.shm.attach(id)
    }

    /// 分离共享内存区域
    pub fn detach_shm(&mut self, id: u64) {
        self.shm.detach(id)
    }
}

/* ── 全局 IPC 管理器 ── */

use core::cell::UnsafeCell;

struct IpcWrapper(UnsafeCell<IpcManager>);
unsafe impl Sync for IpcWrapper {}

static IPC_MGR: IpcWrapper = IpcWrapper(UnsafeCell::new(IpcManager::new()));

fn ipc_mgr() -> &'static mut IpcManager {
    unsafe { &mut *IPC_MGR.0.get() }
}

/* ── 系统级 IPC API ── */

/// 初始化 IPC 子系统
pub fn init() {
    // IpcManager 已在 const 初始化中完成
}

/// 注册 IPC 端点到指定 TID
pub fn register_endpoint(tid: u64, ep: &'static Endpoint) -> Result<(), ()> {
    ipc_mgr().register(tid, ep)
}

/// 发送 IPC 消息（通过路由表）
pub fn send(msg: &Message) -> Result<(), ()> {
    ipc_mgr().send_to(msg)
}

/// 创建管道，返回 SyncPipe
pub fn create_pipe() -> SyncPipe {
    SyncPipe::new()
}

/// 创建共享内存区域，返回区域 ID
pub fn create_shm(size: u64) -> Option<u64> {
    ipc_mgr().create_shm(size)
}

/// 发送消息到指定目标 TID
pub fn send_msg(dst_tid: u64, data: &[u8]) -> Result<(), ()> {
    let msg = Message::data(0, dst_tid, data);
    send(&msg)
}

/// 从端点接收消息
pub fn recv_msg(ep: &Endpoint) -> Option<Message> {
    ep.recv()
}

/// IPC 总线周期分发（ipc_bus 服务调用）
pub fn dispatch_pending() {
    while let Some(msg) = KERNEL_ENDPOINT.recv() {
        if send(&msg).is_err() {
            break;
        }
    }
}

/* ── 系统调用接口 ── */

/// IPC 发送（syscall 包装）
pub fn sys_ipc_send(dst_tid: u64, data: &[u8]) -> i64 {
    let msg = Message::data(0, dst_tid, data);
    match send(&msg) {
        Ok(()) => 0,
        Err(()) => -1,
    }
}

/// IPC 接收（syscall 包装）
pub fn sys_ipc_recv(ep: &Endpoint) -> Option<Message> {
    ep.recv()
}

/* ── 全局端点 ── */

/// 内核服务端点（供 Ring 2/3 程序向内核请求服务）
pub static KERNEL_ENDPOINT: Endpoint = Endpoint::new();
