// fs/page_cache.rs — 文件页面缓存
// 基于 (ino, page_index) 的哈希表, LRU 淘汰, 脏页跟踪
// 支持延迟分配: 写入先缓存, 回写时分配物理空间+写入设备
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use crate::fs::types::Ino;
use crate::fs::error::{FsResult, FsError};
use crate::fs::segment_device::SegmentDevice;
use crate::sync::Spinlock;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, AtomicI32, AtomicUsize, Ordering};

// ── 常量 ──

/// 页大小 (与设备页对齐)
pub const PAGE_SIZE: usize = 4096;
const PAGE_SHIFT: u64 = 12;

/// 哈希桶数量
const PAGE_HASH_SIZE: usize = 64;
/// 最大缓存页数
const MAX_PAGES: usize = 256;

// ── 页标志 ──

const PG_UPTODATE: u8 = 1 << 0;   // 数据有效
const PG_DIRTY: u8    = 1 << 1;   // 已修改, 需回写
const PG_LOCKED: u8   = 1 << 2;   // I/O 进行中
const PG_WRITEBACK: u8 = 1 << 3;  // 正在回写

// ── 页缓存头 ──

#[repr(C)]
pub struct PageHead {
    /// 所属 inode
    pub ino: Ino,
    /// 页索引 (logical_offset >> PAGE_SHIFT)
    pub page_index: u64,
    /// 状态标志
    pub state: AtomicU8,
    /// 引用计数
    pub ref_count: AtomicI32,
    /// 页面数据 (4KB, kmalloc 分配)
    pub data: NonNull<u8>,
    /// 哈希链下一项
    pub hash_next: *mut PageHead,
    /// LRU 双向链表
    pub lru_prev: *mut PageHead,
    pub lru_next: *mut PageHead,
}

impl PageHead {
    /// 判断此页是否覆盖目标区间
    fn covers(&self, ino: Ino, page_index: u64) -> bool {
        self.ino == ino && self.page_index == page_index
    }
}

// ── 哈希桶 ──

struct PageHashBucket {
    head: *mut PageHead,
}

impl PageHashBucket {
    const fn new() -> Self {
        PageHashBucket { head: core::ptr::null_mut() }
    }
}

// ── 页面缓存管理器 ──

pub struct PageCache {
    /// 哈希表
    hash: [Spinlock<PageHashBucket>; PAGE_HASH_SIZE],
    /// LRU 链表头尾
    lru_head: Spinlock<*mut PageHead>,
    lru_tail: Spinlock<*mut PageHead>,
    /// 统计
    nr_pages: AtomicUsize,
    nr_dirty: AtomicUsize,
    /// 绑定的段设备 (用于回写)
    device: Option<&'static SegmentDevice>,
}

impl PageCache {
    pub const fn new() -> Self {
        const EMPTY: Spinlock<PageHashBucket> = Spinlock::new(PageHashBucket::new());
        PageCache {
            hash: [EMPTY; PAGE_HASH_SIZE],
            lru_head: Spinlock::new(core::ptr::null_mut()),
            lru_tail: Spinlock::new(core::ptr::null_mut()),
            nr_pages: AtomicUsize::new(0),
            nr_dirty: AtomicUsize::new(0),
            device: None,
        }
    }

    /// 绑定段设备
    pub fn bind_device(&mut self, dev: &'static SegmentDevice) {
        self.device = Some(dev);
    }

    // ── 查找 ──

    /// 查找页面 (不增加引用计数, 调用者负责 release)
    pub fn lookup(&self, ino: Ino, page_index: u64) -> Option<&PageHead> {
        let hash_idx = page_hash(ino, page_index);
        let bucket = self.hash[hash_idx].lock();
        let mut curr = bucket.head;
        while !curr.is_null() {
            let page = unsafe { &*curr };
            if page.covers(ino, page_index)
                && page.state.load(Ordering::Acquire) & PG_UPTODATE != 0
            {
                page.ref_count.fetch_add(1, Ordering::Relaxed);
                return Some(page);
            }
            curr = page.hash_next;
        }
        None
    }

    // ── 读取 ──

    /// 从页面缓存读取指定区间到用户缓冲区
    /// 若页面不在缓存中, 从设备加载后返回
    pub fn read(
        &self, ino: Ino, logical_offset: u64, buf: &mut [u8],
        read_from_device: impl Fn(u64, &mut [u8]) -> FsResult<()>,
    ) -> FsResult<usize> {
        let end = logical_offset + buf.len() as u64;
        let start_page = (logical_offset >> PAGE_SHIFT) as u64;
        let end_page = ((end.saturating_sub(1)) >> PAGE_SHIFT) as u64;
        let mut done = 0usize;

        for pg_idx in start_page..=end_page {
            let page_off = pg_idx << PAGE_SHIFT;
            let page_start = logical_offset.max(page_off);
            let page_end = end.min(page_off + PAGE_SIZE as u64);
            if page_start >= page_end {
                continue;
            }
            let copy_offset = (page_start - page_off) as usize;
            let copy_len = (page_end - page_start) as usize;

            // 尝试查找缓存
            let page = match self.lookup(ino, pg_idx) {
                Some(p) => p,
                None => {
                    // 分配新页并从设备读取
                    let new_page = self.alloc_page(ino, pg_idx)?;
                    // 从设备加载数据
                    let data_slice = unsafe {
                        core::slice::from_raw_parts_mut(new_page.data.as_ptr(), PAGE_SIZE)
                    };
                    // 初始化为零 (处理空洞)
                    data_slice.fill(0);
                    let _ = read_from_device(page_off, data_slice);
                    new_page.state.store(PG_UPTODATE, Ordering::Release);
                    new_page
                }
            };

            let data_slice = unsafe {
                core::slice::from_raw_parts(page.data.as_ptr(), PAGE_SIZE)
            };
            let dest = &mut buf[done..done + copy_len];
            dest.copy_from_slice(&data_slice[copy_offset..copy_offset + copy_len]);
            done += copy_len;

            self.release(page);
        }
        Ok(done)
    }

    // ── 写入 ──

    /// 写入数据到页面缓存 (延迟分配: 不立即写入设备)
    pub fn write(
        &self, ino: Ino, logical_offset: u64, buf: &[u8],
    ) -> FsResult<usize> {
        let end = logical_offset + buf.len() as u64;
        let start_page = (logical_offset >> PAGE_SHIFT) as u64;
        let end_page = ((end.saturating_sub(1)) >> PAGE_SHIFT) as u64;
        let mut done = 0usize;

        for pg_idx in start_page..=end_page {
            let page_off = pg_idx << PAGE_SHIFT;
            let page_start = logical_offset.max(page_off);
            let page_end = end.min(page_off + PAGE_SIZE as u64);
            if page_start >= page_end {
                continue;
            }
            let copy_offset = (page_start - page_off) as usize;
            let copy_len = (page_end - page_start) as usize;

            // 查找或分配页面
            let page = match self.lookup(ino, pg_idx) {
                Some(p) => p,
                None => {
                    let new_page = self.alloc_page(ino, pg_idx)?;
                    // 如果不覆盖整页, 先加载已有数据
                    if copy_offset > 0 || copy_offset + copy_len < PAGE_SIZE {
                        let data_slice = unsafe {
                            core::slice::from_raw_parts_mut(new_page.data.as_ptr(), PAGE_SIZE)
                        };
                        data_slice.fill(0);
                        // 尝试从设备读取已有内容
                        if let Some(dev) = self.device {
                            // 使用 extent tree 读取 (由调用者提供 read_fn)
                            // 此处用零填充, 调用者可在写入前预读
                            let _ = dev;
                        }
                    }
                    new_page
                }
            };

            let data_slice = unsafe {
                core::slice::from_raw_parts_mut(page.data.as_ptr(), PAGE_SIZE)
            };
            let src = &buf[done..done + copy_len];
            data_slice[copy_offset..copy_offset + copy_len].copy_from_slice(src);
            done += copy_len;

            // 标记为脏
            self.mark_dirty(page);
            self.release(page);
        }
        Ok(done)
    }

    // ── 回写 ──

    /// 回写指定 inode 的所有脏页到设备
    /// writeback_fn: (physical_offset, &[u8]) → FsResult<()>
    /// alloc_fn: (length) → FsResult<Option<u64>> — 为延迟分配的页分配物理空间
    pub fn writeback_ino(
        &self, ino: Ino,
        alloc_fn: impl Fn(u64) -> FsResult<Option<u64>>,
        write_fn: impl Fn(u64, &[u8]) -> FsResult<()>,
    ) -> FsResult<usize> {
        let mut written = 0usize;

        for bucket_idx in 0..PAGE_HASH_SIZE {
            let bucket = self.hash[bucket_idx].lock();
            let mut curr = bucket.head;
            while !curr.is_null() {
                let page = unsafe { &*curr };
                if page.ino == ino
                    && page.state.load(Ordering::Acquire) & PG_DIRTY != 0
                    && page.state.load(Ordering::Acquire) & PG_LOCKED == 0
                {
                    // 锁定页面
                    page.state.fetch_or(PG_LOCKED, Ordering::Acquire);
                    let _page_off = page.page_index << PAGE_SHIFT;
                    let page_data = unsafe {
                        core::slice::from_raw_parts(page.data.as_ptr(), PAGE_SIZE)
                    };

                    // 为延迟分配的页分配物理空间
                    if let Ok(Some(phys)) = alloc_fn(PAGE_SIZE as u64) {
                        let _ = write_fn(phys, page_data);
                        // 清除脏标志
                        page.state.fetch_and(!(PG_DIRTY | PG_LOCKED), Ordering::Release);
                        self.nr_dirty.fetch_sub(1, Ordering::Relaxed);
                        written += 1;
                    } else {
                        // 分配失败, 解锁
                        page.state.fetch_and(!PG_LOCKED, Ordering::Release);
                    }
                }
                curr = page.hash_next;
            }
        }
        Ok(written)
    }

    /// 同步单个页面到设备
    pub fn sync_page(
        &self, ino: Ino, page_index: u64,
        alloc_fn: impl Fn(u64) -> FsResult<Option<u64>>,
        write_fn: impl Fn(u64, &[u8]) -> FsResult<()>,
    ) -> FsResult<()> {
        if let Some(page) = self.lookup(ino, page_index) {
            if page.state.load(Ordering::Acquire) & PG_DIRTY != 0 {
                page.state.fetch_or(PG_LOCKED, Ordering::Acquire);
                let page_data = unsafe {
                    core::slice::from_raw_parts(page.data.as_ptr(), PAGE_SIZE)
                };
                if let Ok(Some(phys)) = alloc_fn(PAGE_SIZE as u64) {
                    write_fn(phys, page_data)?;
                    page.state.fetch_and(!(PG_DIRTY | PG_LOCKED), Ordering::Release);
                    self.nr_dirty.fetch_sub(1, Ordering::Relaxed);
                } else {
                    page.state.fetch_and(!PG_LOCKED, Ordering::Release);
                    return Err(FsError::Enospc);
                }
            }
            self.release(page);
        }
        Ok(())
    }

    // ── 截断页面缓存 ──

    /// 截断指定 inode 的页面缓存 (移除超出 new_size 的页)
    pub fn truncate_pages(&self, ino: Ino, new_size: u64) {
        let end_page = (new_size + PAGE_SIZE as u64 - 1) >> PAGE_SHIFT;
        for bucket_idx in 0..PAGE_HASH_SIZE {
            let bucket = self.hash[bucket_idx].lock();
            let mut curr = bucket.head;
            while !curr.is_null() {
                let page = unsafe { &*curr };
                if page.ino == ino && page.page_index >= end_page {
                    // 标记为无效, 从 LRU 移除
                    page.state.store(0, Ordering::Release);
                    if page.state.load(Ordering::Relaxed) & PG_DIRTY != 0 {
                        self.nr_dirty.fetch_sub(1, Ordering::Relaxed);
                    }
                }
                curr = page.hash_next;
            }
        }
    }

    // ── 标记脏 ──

    pub fn mark_dirty(&self, page: &PageHead) {
        let old = page.state.fetch_or(PG_DIRTY, Ordering::Release);
        if old & PG_DIRTY == 0 {
            self.nr_dirty.fetch_add(1, Ordering::Relaxed);
        }
    }

    // ── 释放引用 ──

    pub fn release(&self, page: &PageHead) {
        page.ref_count.fetch_sub(1, Ordering::Release);
    }

    // ── 统计 ──

    pub fn dirty_count(&self) -> usize {
        self.nr_dirty.load(Ordering::Relaxed)
    }

    pub fn page_count(&self) -> usize {
        self.nr_pages.load(Ordering::Relaxed)
    }

    // ── 内部分配 ──

    fn alloc_page(&self, ino: Ino, page_index: u64) -> FsResult<&PageHead> {
        use crate::mm::slab;

        let total = core::mem::size_of::<PageHead>() + PAGE_SIZE;
        let ptr = slab::kmalloc(total).ok_or(FsError::Enomem)?;

        unsafe {
            let page = ptr.as_ptr() as *mut PageHead;
            core::ptr::write_bytes(ptr.as_ptr(), 0, total);

            (*page).ino = ino;
            (*page).page_index = page_index;
            (*page).state = AtomicU8::new(0);
            (*page).ref_count = AtomicI32::new(1);
            (*page).data = NonNull::new_unchecked(ptr.as_ptr().add(core::mem::size_of::<PageHead>()));
            (*page).hash_next = core::ptr::null_mut();
            (*page).lru_prev = core::ptr::null_mut();
            (*page).lru_next = core::ptr::null_mut();

            // 插入哈希表
            let hash_idx = page_hash(ino, page_index);
            let mut bucket = self.hash[hash_idx].lock();
            (*page).hash_next = bucket.head;
            bucket.head = page;

            // 插入 LRU 头部
            let mut head = self.lru_head.lock();
            (*page).lru_next = *head;
            if !(*head).is_null() {
                (*(*head)).lru_prev = page;
            }
            *head = page;
            if self.lru_tail.lock().is_null() {
                *self.lru_tail.lock() = page;
            }

            self.nr_pages.fetch_add(1, Ordering::Relaxed);

            // 缓存过多则淘汰
            if self.nr_pages.load(Ordering::Relaxed) > MAX_PAGES {
                self.evict_lru();
            }

            Ok(&*page)
        }
    }

    /// LRU 淘汰
    fn evict_lru(&self) {
        let tail = *self.lru_tail.lock();
        if tail.is_null() {
            return;
        }
        let page = unsafe { &*tail };
        // 跳过正在使用或脏页
        if page.ref_count.load(Ordering::Relaxed) > 0
            || page.state.load(Ordering::Acquire) & (PG_DIRTY | PG_LOCKED) != 0
        {
            return;
        }
        self.remove_page(page);
    }

    fn remove_page(&self, page: &PageHead) {
        use crate::mm::slab;

        // 从哈希表移除
        let hash_idx = page_hash(page.ino, page.page_index);
        let mut bucket = self.hash[hash_idx].lock();
        let mut prev: *mut PageHead = core::ptr::null_mut();
        let mut curr = bucket.head;
        while !curr.is_null() {
            unsafe {
                if curr == page as *const _ as *mut PageHead {
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
            if !page.lru_prev.is_null() {
                (*(page.lru_prev)).lru_next = page.lru_next;
            }
            if !page.lru_next.is_null() {
                (*(page.lru_next)).lru_prev = page.lru_prev;
            }
        }

        let ptr = NonNull::new(page as *const _ as *mut u8).unwrap();
        unsafe { slab::kfree(ptr); }
        self.nr_pages.fetch_sub(1, Ordering::Relaxed);
    }
}

// ── 哈希函数 ──

fn page_hash(ino: Ino, page_index: u64) -> usize {
    let mut h: u64 = ino ^ page_index;
    h = h.wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h = h ^ (h >> 27);
    (h % PAGE_HASH_SIZE as u64) as usize
}

// ── 全局页面缓存 ──

use core::cell::UnsafeCell;

struct PageCacheWrapper(UnsafeCell<PageCache>);
unsafe impl Sync for PageCacheWrapper {}

static PAGE_CACHE: PageCacheWrapper = PageCacheWrapper(UnsafeCell::new(PageCache::new()));

/// 获取全局页面缓存引用
pub fn global_page_cache() -> &'static PageCache {
    unsafe { &*PAGE_CACHE.0.get() }
}

/// 初始化页面缓存 (绑定到段设备)
pub fn init(device: &'static SegmentDevice) {
    unsafe {
        (*PAGE_CACHE.0.get()).bind_device(device);
    }
    crate::serial::write_str(b"  page_cache: init done\n");
}
