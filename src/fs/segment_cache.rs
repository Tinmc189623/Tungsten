// fs/segment_cache.rs — 可变长度段缓存 (SegmentCache)
// 替换传统固定块大小的 BufferCache，支持任意字节区间的缓存
// 2Q-LRU 淘汰策略 + 子区间查找
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use crate::fs::types::DevId;
use crate::fs::segment_device::SegmentDevice;
use crate::fs::error::FsResult;
use crate::sync::Spinlock;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicI32, AtomicU8, AtomicUsize, Ordering};

// ── 常量 ──

/// 哈希桶数量
const SEG_HASH_SIZE: usize = 256;
/// 最大缓存段数
const SEG_MAX_COUNT: usize = 512;
/// 段数据最小对齐 (缓存行)
const SEG_ALIGN: usize = 64;

// ── 段状态 ──

const SEG_UPTODATE: u8 = 1 << 0;  // 数据有效
const SEG_DIRTY: u8    = 1 << 1;  // 已修改, 需回写
const SEG_LOCKED: u8   = 1 << 2;  // I/O 进行中
const SEG_PINNED: u8   = 1 << 3;  // 禁止淘汰

// ── 段缓存头 ──

/// 段缓存条目 (侵入式链表 + 哈希表)
#[repr(C)]
pub struct SegmentHead {
    /// 所属设备
    pub dev: DevId,
    /// 设备上的物理字节偏移
    pub offset: u64,
    /// 缓存数据长度 (字节)
    pub len: u32,
    /// 状态标志
    pub state: AtomicU8,
    /// 引用计数
    pub ref_count: AtomicI32,
    /// 缓存数据指针 (堆分配或直接映射)
    pub data: NonNull<u8>,
    /// 哈希链下一项
    pub hash_next: *mut SegmentHead,
    /// LRU 链表前后指针
    pub lru_prev: *mut SegmentHead,
    pub lru_next: *mut SegmentHead,
    /// 访问次数 (2Q: 1→LRU_1, >=2→LRU_2)
    pub access_count: u8,
}

impl SegmentHead {
    /// 判断此段是否覆盖目标区间
    pub fn covers(&self, dev: DevId, offset: u64, len: usize) -> bool {
        self.dev == dev
            && offset >= self.offset
            && offset + len as u64 <= self.offset + self.len as u64
    }
}

// ── 哈希桶 ──

struct SegHashBucket {
    head: *mut SegmentHead,
}

impl SegHashBucket {
    const fn new() -> Self {
        SegHashBucket { head: core::ptr::null_mut() }
    }
}

// ── 段缓存管理器 ──

pub struct SegmentCache {
    /// 哈希表 (dev, offset) → SegmentHead
    hash: [Spinlock<SegHashBucket>; SEG_HASH_SIZE],
    /// LRU_1: 单次访问队列
    lru1_head: Spinlock<*mut SegmentHead>,
    lru1_tail: Spinlock<*mut SegmentHead>,
    /// LRU_2: 多次访问队列 (热数据)
    lru2_head: Spinlock<*mut SegmentHead>,
    lru2_tail: Spinlock<*mut SegmentHead>,
    /// 统计
    nr_segments: AtomicUsize,
    nr_dirty: AtomicUsize,
    /// 块设备引用 (回写用)
    device: Option<&'static SegmentDevice>,
}

impl SegmentCache {
    /// 创建段缓存
    pub const fn new() -> Self {
        const EMPTY_BUCKET: Spinlock<SegHashBucket> = Spinlock::new(SegHashBucket::new());
        SegmentCache {
            hash: [EMPTY_BUCKET; SEG_HASH_SIZE],
            lru1_head: Spinlock::new(core::ptr::null_mut()),
            lru1_tail: Spinlock::new(core::ptr::null_mut()),
            lru2_head: Spinlock::new(core::ptr::null_mut()),
            lru2_tail: Spinlock::new(core::ptr::null_mut()),
            nr_segments: AtomicUsize::new(0),
            nr_dirty: AtomicUsize::new(0),
            device: None,
        }
    }

    /// 绑定块设备
    pub fn bind_device(&mut self, dev: &'static SegmentDevice) {
        self.device = Some(dev);
    }

    /// 查找覆盖目标区间的缓存段
    /// 若找到则提升到 LRU_2 并增加引用计数
    pub fn lookup(&self, dev: DevId, offset: u64, len: usize) -> Option<&SegmentHead> {
        let hash_idx = seg_hash(dev, offset);
        let bucket = self.hash[hash_idx].lock();
        let mut curr = bucket.head;
        while !curr.is_null() {
            let seg = unsafe { &*curr };
            if seg.covers(dev, offset, len)
                && seg.state.load(Ordering::Acquire) & SEG_UPTODATE != 0
            {
                seg.ref_count.fetch_add(1, Ordering::Relaxed);
                return Some(seg);
            }
            curr = seg.hash_next;
        }
        None
    }

    /// 获取或分配段 (类似 getblk)
    /// 若缓存命中返回已缓存段, 否则分配新段并读取
    pub fn get_segment(&self, dev: DevId, offset: u64, len: usize) -> FsResult<&SegmentHead> {
        // 尝试查找
        if let Some(seg) = self.lookup(dev, offset, len) {
            return Ok(seg);
        }

        // 未命中: 分配新段
        let seg = self.alloc_segment(dev, offset, len.max(512))?;

        // 从设备读取
        if let Some(device) = self.device {
            let data_slice = unsafe {
                core::slice::from_raw_parts_mut(seg.data.as_ptr(), seg.len as usize)
            };
            let _ = device.read_bytes(seg.offset, data_slice);
            seg.state.store(SEG_UPTODATE, Ordering::Release);
        }

        Ok(seg)
    }

    /// 标记段为脏 (修改后调用)
    pub fn mark_dirty(&self, seg: &SegmentHead) {
        let old = seg.state.fetch_or(SEG_DIRTY, Ordering::Release);
        if old & SEG_DIRTY == 0 {
            self.nr_dirty.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 释放段引用
    pub fn release(&self, seg: &SegmentHead) {
        seg.ref_count.fetch_sub(1, Ordering::Release);
    }

    /// 同步所有脏段到设备
    pub fn sync_all(&self) -> FsResult<()> {
        if let Some(device) = self.device {
            for bucket_idx in 0..SEG_HASH_SIZE {
                let bucket = self.hash[bucket_idx].lock();
                let mut curr = bucket.head;
                while !curr.is_null() {
                    let seg = unsafe { &*curr };
                    if seg.state.load(Ordering::Acquire) & SEG_DIRTY != 0 {
                        let data_slice = unsafe {
                            core::slice::from_raw_parts(seg.data.as_ptr(), seg.len as usize)
                        };
                        device.write_bytes(seg.offset, data_slice)?;
                        seg.state.fetch_and(!SEG_DIRTY, Ordering::Release);
                        self.nr_dirty.fetch_sub(1, Ordering::Relaxed);
                    }
                    curr = seg.hash_next;
                }
            }
        }
        if let Some(device) = self.device {
            device.flush()?;
        }
        Ok(())
    }

    /// 分配新段: 段头从专用 SLAB 缓存, 数据缓冲区从 kmalloc
    fn alloc_segment(&self, dev: DevId, offset: u64, len: usize) -> FsResult<&SegmentHead> {
        use crate::mm::slab;

        // 对齐到缓存行
        let alloc_len = (len + SEG_ALIGN - 1) & !(SEG_ALIGN - 1);

        // 段头: 专用 SLAB 缓存
        let seg_ptr = slab::alloc_segment_head().ok_or(crate::fs::error::FsError::Enomem)?;
        // 数据: 通用 kmalloc
        let data_ptr = slab::kmalloc(alloc_len).ok_or_else(|| {
            // 回滚段头分配
            unsafe { slab::free_segment_head(seg_ptr); }
            crate::fs::error::FsError::Enomem
        })?;

        unsafe {
            let seg = seg_ptr.as_ptr() as *mut SegmentHead;

            (*seg).dev = dev;
            (*seg).offset = offset;
            (*seg).len = alloc_len as u32;
            (*seg).state = AtomicU8::new(0);
            (*seg).ref_count = AtomicI32::new(1);
            (*seg).data = data_ptr;
            (*seg).hash_next = core::ptr::null_mut();
            (*seg).lru_prev = core::ptr::null_mut();
            (*seg).lru_next = core::ptr::null_mut();
            (*seg).access_count = 1;

            // 插入哈希表
            let hash_idx = seg_hash(dev, offset);
            let mut bucket = self.hash[hash_idx].lock();
            (*seg).hash_next = bucket.head;
            bucket.head = seg;

            // 插入 LRU_1
            let mut head = self.lru1_head.lock();
            (*seg).lru_next = *head;
            if !(*head).is_null() {
                (*(*head)).lru_prev = seg;
            }
            *head = seg;

            self.nr_segments.fetch_add(1, Ordering::Relaxed);

            // 缓存过多则淘汰
            if self.nr_segments.load(Ordering::Relaxed) > SEG_MAX_COUNT {
                self.evict_lru();
            }

            Ok(&*seg)
        }
    }

    /// LRU 淘汰
    pub fn evict_lru(&self) {
        // 简化: 从 LRU_1 尾部淘汰非脏段
        let tail = *self.lru1_tail.lock();
        if !tail.is_null() {
            let seg = unsafe { &*tail };
            if seg.ref_count.load(Ordering::Relaxed) == 0
                && seg.state.load(Ordering::Acquire) & SEG_DIRTY == 0
                && seg.state.load(Ordering::Acquire) & SEG_PINNED == 0
            {
                self.remove_segment(seg);
            }
        }
    }

    /// 从缓存中移除段并释放内存
    fn remove_segment(&self, seg: &SegmentHead) {
        use crate::mm::slab;

        // 从哈希表移除
        let hash_idx = seg_hash(seg.dev, seg.offset);
        let mut bucket = self.hash[hash_idx].lock();
        let mut prev: *mut SegmentHead = core::ptr::null_mut();
        let mut curr = bucket.head;
        while !curr.is_null() {
            unsafe {
                if curr == seg as *const _ as *mut SegmentHead {
                    if prev.is_null() {
                        bucket.head = (*curr).hash_next;
                    } else {
                        (*prev).hash_next = (*curr).hash_next;
                    }
                    break;
                }
                prev = curr;
                curr = (*curr).hash_next;
            }
        }
        drop(bucket);

        // 从 LRU 链表移除
        unsafe {
            if !seg.lru_prev.is_null() {
                (*(seg.lru_prev)).lru_next = seg.lru_next;
            }
            if !seg.lru_next.is_null() {
                (*(seg.lru_next)).lru_prev = seg.lru_prev;
            }
        }

        // 释放数据缓冲区
        unsafe { slab::kfree(seg.data); }
        // 释放段头回专用缓存
        unsafe {
            slab::free_segment_head(NonNull::new_unchecked(
                seg as *const SegmentHead as *mut u8,
            ));
        }

        self.nr_segments.fetch_sub(1, Ordering::Relaxed);
    }
}

// ── 全局段缓存 ──

use core::cell::UnsafeCell;

struct SegCacheWrapper(UnsafeCell<SegmentCache>);
unsafe impl Sync for SegCacheWrapper {}

static SEG_CACHE: SegCacheWrapper = SegCacheWrapper(UnsafeCell::new(SegmentCache::new()));

/// 获取全局段缓存引用
pub fn global_cache() -> &'static SegmentCache {
    unsafe { &*SEG_CACHE.0.get() }
}

/// 初始化段缓存 (绑定到段设备)
pub fn init(device: &'static SegmentDevice) {
    unsafe {
        (*SEG_CACHE.0.get()).bind_device(device);
    }
    crate::serial::write_str(b"  seg_cache: init done\n");
}

/// 段缓存周期维护（vfsd 调用）
pub fn evict_lru() {
    global_cache().evict_lru();
    let _ = global_cache().sync_all();
}

// ── 哈希函数 ──

fn seg_hash(dev: DevId, offset: u64) -> usize {
    let mut h: u64 = dev;
    h ^= offset;
    h = h.wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h = h ^ (h >> 27);
    (h % SEG_HASH_SIZE as u64) as usize
}
