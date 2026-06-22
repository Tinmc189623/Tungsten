// fs/segment_device.rs — 段设备抽象层
// 提供可变长度字节级 I/O 接口，FS 层通过此 trait 访问所有存储设备
// 扇区对齐由实现层处理，FS 层提交任意字节偏移+长度的请求
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later


#![allow(improper_ctypes_definitions)]

use crate::fs::types::DevId;
use crate::fs::error::FsResult;

// ── 段设备操作方向 ──

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IoDir {
    Read = 0,
    Write = 1,
}

// ── 段设备 I/O 向量 ──

/// 单个 I/O 缓冲区段 (scatter-gather 列表的一项)
#[repr(C)]
pub struct SegmentVec {
    /// 内存缓冲区指针
    pub buf: *mut u8,
    /// 缓冲区长度 (字节)
    pub len: usize,
}

impl SegmentVec {
    pub fn new(buf: *mut u8, len: usize) -> Self {
        SegmentVec { buf, len }
    }

    pub fn from_slice(slice: &mut [u8]) -> Self {
        SegmentVec { buf: slice.as_mut_ptr(), len: slice.len() }
    }

    pub unsafe fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.buf, self.len) }
    }

    pub unsafe fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.buf, self.len) }
    }
}

// ── 段 I/O 请求 ──

/// I/O 完成回调类型
pub type SegmentCompletion = Option<unsafe extern "C" fn(req: &SegmentRequest, status: i32)>;

/// 段 I/O 请求：描述一次读写操作
#[repr(C)]
pub struct SegmentRequest {
    /// 目标设备 ID
    pub dev: DevId,
    /// 设备上的起始字节偏移 (必须扇区对齐)
    pub offset: u64,
    /// 总传输字节数
    pub total_bytes: usize,
    /// 操作方向
    pub dir: IoDir,
    /// 散聚列表
    pub vecs: [SegmentVec; 8],
    /// 散聚表有效条目数
    pub vec_count: u8,
    /// 操作完成回调 (在中断/IPC 上下文调用)
    pub completion: SegmentCompletion,
    /// 请求私有数据 (由提交者使用)
    pub private: *mut (),
    /// 提交时间戳 (用于 I/O 调度)
    pub submit_time: u64,
    /// 请求优先级 (0=正常, 1=同步, 2=回写)
    pub priority: u8,
    /// 标志位
    pub flags: u8,
}

/// 段请求标志
pub const SEG_REQ_SYNC: u8    = 1 << 0;  // 同步等待 (提交者阻塞)
pub const SEG_REQ_BARRIER: u8 = 1 << 1;  // 写屏障 (前序 I/O 须先完成)
pub const SEG_REQ_READAHEAD: u8 = 1 << 2; // 预读 (失败静默忽略)
pub const SEG_REQ_META: u8    = 1 << 3;  // 元数据 I/O (高优先级)

impl SegmentRequest {
    /// 创建简单的单缓冲区读请求
    pub fn read(dev: DevId, offset: u64, buf: &mut [u8]) -> Self {
        SegmentRequest {
            dev,
            offset,
            total_bytes: buf.len(),
            dir: IoDir::Read,
            vecs: [
                SegmentVec::from_slice(buf),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
            ],
            vec_count: 1,
            completion: None,
            private: core::ptr::null_mut(),
            submit_time: 0,
            priority: 0,
            flags: 0,
        }
    }

    /// 创建简单的单缓冲区写请求
    pub fn write(dev: DevId, offset: u64, buf: &[u8]) -> Self {
        SegmentRequest {
            dev,
            offset,
            total_bytes: buf.len(),
            dir: IoDir::Write,
            vecs: [
                SegmentVec::new(buf.as_ptr() as *mut u8, buf.len()),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
                SegmentVec::new(core::ptr::null_mut(), 0),
            ],
            vec_count: 1,
            completion: None,
            private: core::ptr::null_mut(),
            submit_time: 0,
            priority: 0,
            flags: 0,
        }
    }

    /// 设置完成回调
    pub fn with_completion(mut self, cb: unsafe extern "C" fn(&SegmentRequest, i32)) -> Self {
        self.completion = Some(cb);
        self
    }

    /// 设置同步标志
    pub fn with_sync(mut self) -> Self {
        self.flags |= SEG_REQ_SYNC;
        self
    }

    /// 设置写屏障标志
    pub fn with_barrier(mut self) -> Self {
        self.flags |= SEG_REQ_BARRIER;
        self
    }

    /// 设置元数据标志
    pub fn with_meta(mut self) -> Self {
        self.flags |= SEG_REQ_META;
        self.priority = 255;
        self
    }
}

// ── 段设备操作接口 ──

/// 段设备操作 vtable (驱动层实现)
#[repr(C)]
pub struct SegmentDeviceOps {
    /// 提交 I/O 请求 (同步或异步)
    pub submit: unsafe extern "C" fn(dev: &SegmentDevice, req: &mut SegmentRequest) -> FsResult<()>,
    /// 获取设备容量 (字节)
    pub capacity: unsafe extern "C" fn(dev: &SegmentDevice) -> u64,
    /// 获取设备扇区大小
    pub sector_size: unsafe extern "C" fn(dev: &SegmentDevice) -> u32,
    /// 刷新设备缓存
    pub flush: unsafe extern "C" fn(dev: &SegmentDevice) -> FsResult<()>,
    /// 设备 I/O 控制
    pub ioctl: unsafe extern "C" fn(dev: &SegmentDevice, cmd: u32, arg: usize) -> FsResult<usize>,
}

/// 段设备实例
#[repr(C)]
pub struct SegmentDevice {
    pub dev_id: DevId,
    pub name: [u8; 32],
    pub ops: &'static SegmentDeviceOps,
    pub private: *mut (),
    pub removable: bool,
    pub read_only: bool,
}

// 段设备由驱动层实现者保证线程安全
unsafe impl Sync for SegmentDevice {}

impl SegmentDevice {
    /// 创建段设备实例
    pub const fn new(
        dev_id: DevId, name: &[u8; 32], ops: &'static SegmentDeviceOps, read_only: bool,
    ) -> Self {
        SegmentDevice {
            dev_id,
            name: *name,
            ops,
            private: core::ptr::null_mut(),
            removable: false,
            read_only,
        }
    }

    /// 提交 I/O 请求
    pub fn submit(&self, req: &mut SegmentRequest) -> FsResult<()> {
        unsafe { (self.ops.submit)(self, req) }
    }

    /// 获取容量
    pub fn capacity(&self) -> u64 {
        unsafe { (self.ops.capacity)(self) }
    }

    /// 获取扇区大小
    pub fn sector_size(&self) -> u32 {
        unsafe { (self.ops.sector_size)(self) }
    }

    /// 刷新
    pub fn flush(&self) -> FsResult<()> {
        unsafe { (self.ops.flush)(self) }
    }

    /// 执行同步读取
    pub fn read_bytes(&self, offset: u64, buf: &mut [u8]) -> FsResult<()> {
        let mut req = SegmentRequest::read(self.dev_id, offset, buf).with_sync();
        self.submit(&mut req)
    }

    /// 执行同步写入
    pub fn write_bytes(&self, offset: u64, buf: &[u8]) -> FsResult<()> {
        let mut req = SegmentRequest::write(self.dev_id, offset, buf).with_sync();
        self.submit(&mut req)
    }
}
