// mm/slab.rs — SLAB 分配器（通用 kmalloc + 专用对象缓存）
// 基于 PMM 伙伴系统的小块内存分配层
// 支持 10 个标准缓存档位: 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096 字节
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use core::cell::UnsafeCell;
use core::ptr::NonNull;
use crate::mm::pmm;

/* ── 常量 ── */

/// SLAB 最大对象尺寸（超过此尺寸走 PMM 大块分配）
const SLAB_MAX_SIZE: usize = 4096;
/// 标准 kmalloc 缓存档位数
const KMALLOC_SIZES: usize = 10;

/* ── SLAB 页头 ── */

/// SLAB 页头结构，位于每个 SLAB 页的起始位置
#[repr(C)]
struct SlabHead {
    /// 所属缓存
    cache: *mut SlabCache,
    /// 空闲对象链表头
    free_list: *mut u8,
    /// 下一个 SLAB 页
    next: *mut SlabHead,
    /// 已分配对象数
    inuse: u32,
    /// 总对象数
    total: u32,
}

/* ── SLAB 缓存 ── */

/// SLAB 缓存（同尺寸对象池）
pub(crate) struct SlabCache {
    /// 缓存名称
    name: &'static str,
    /// 对象大小
    obj_size: usize,
    /// 对齐要求
    align: usize,
    /// 部分空闲的 SLAB 页链表
    pages: *mut SlabHead,
    /// 已满的 SLAB 页链表
    full_pages: *mut SlabHead,
    /// 分配标志
    gfp_flags: u64,
}

impl SlabCache {
    /// 创建新的空 SLAB 缓存
    const fn new(name: &'static str, obj_size: usize, align: usize) -> Self {
        SlabCache {
            name,
            obj_size,
            align,
            pages: core::ptr::null_mut(),
            full_pages: core::ptr::null_mut(),
            gfp_flags: 0,
        }
    }

    /// 从 PMM 获取新页并构建空闲链表（grow 操作）
    fn grow(&mut self) -> bool {
        let paddr = match pmm::alloc_zeroed() {
            Some(p) => p,
            None => return false,
        };

        unsafe {
            let head = paddr as *mut SlabHead;
            let obj_start = paddr + core::mem::size_of::<SlabHead>() as u64;
            // 对象槽位大小不能小于指针大小（需要存放 next 指针）
            let slot_size = if self.obj_size < core::mem::size_of::<usize>() {
                core::mem::size_of::<usize>()
            } else {
                self.obj_size
            };
            let avail =
                (pmm::PAGE_SIZE as usize - core::mem::size_of::<SlabHead>()) / slot_size;

            (*head).cache = self as *mut SlabCache;
            (*head).total = avail as u32;
            (*head).inuse = 0;
            (*head).next = self.pages;
            self.pages = head;

            // 侵入式空闲链表：每个空闲对象的前 8 字节存下一个空闲对象指针
            let mut prev: *mut u8 = core::ptr::null_mut();
            for i in 0..avail {
                let slot = (obj_start + (i * slot_size) as u64) as *mut u8;
                if i == 0 {
                    (*head).free_list = slot;
                } else {
                    *(prev as *mut *mut u8) = slot;
                }
                *(slot as *mut *mut u8) = core::ptr::null_mut();
                prev = slot;
            }
        }
        true
    }

    /// 从缓存分配一个对象
    fn alloc(&mut self) -> Option<NonNull<u8>> {
        if self.pages.is_null() && !self.grow() {
            return None;
        }
        unsafe {
            let head = self.pages;
            // 从空闲链表头部取出
            let slot = (*head).free_list as *mut *mut u8;
            let obj = slot as *mut u8;
            (*head).free_list = *slot;
            (*head).inuse += 1;

            // 页满后移到 full_pages 链表
            if (*head).inuse >= (*head).total {
                self.pages = (*head).next;
                (*head).next = self.full_pages;
                self.full_pages = head;
            }
            Some(NonNull::new_unchecked(obj))
        }
    }

    /// 释放对象回缓存
    fn free(&mut self, ptr: NonNull<u8>) {
        unsafe {
            let addr = ptr.as_ptr() as u64;
            let head_addr = addr & !(pmm::PAGE_SIZE - 1);
            let head = head_addr as *mut SlabHead;

            // 用对象前 8 字节存 free_list 指针
            let slot = ptr.as_ptr() as *mut *mut u8;
            *slot = (*head).free_list;
            (*head).free_list = ptr.as_ptr();
            (*head).inuse -= 1;

            // 页从 full 变 partial，移回 pages 链表
            if (*head).inuse + 1 >= (*head).total {
                Self::unlink_full(self, head);
                (*head).next = self.pages;
                self.pages = head;
            }
        }
    }

    /// 从 full_pages 链表移除指定页
    unsafe fn unlink_full(this: &mut SlabCache, target: *mut SlabHead) {
        let mut curr = this.full_pages;
        let mut prev: *mut SlabHead = core::ptr::null_mut();
        while !curr.is_null() {
            if curr == target {
                if prev.is_null() {
                    this.full_pages = (*curr).next;
                } else {
                    (*prev).next = (*curr).next;
                }
                return;
            }
            prev = curr;
            curr = (*curr).next;
        }
    }
}

/* ── kmalloc 标准缓存档位 ── */

/// 标准缓存尺寸表（10 档）
const KMALLOC_SIZES_TABLE: [usize; KMALLOC_SIZES] = [
    8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096,
];

/// 标准缓存初始值
const KMALLOC_CACHES_INIT: [SlabCache; KMALLOC_SIZES] = [
    SlabCache::new("kmalloc-8",    8,    8),
    SlabCache::new("kmalloc-16",   16,   8),
    SlabCache::new("kmalloc-32",   32,   8),
    SlabCache::new("kmalloc-64",   64,   8),
    SlabCache::new("kmalloc-128",  128,  16),
    SlabCache::new("kmalloc-256",  256,  16),
    SlabCache::new("kmalloc-512",  512,  32),
    SlabCache::new("kmalloc-1024", 1024, 64),
    SlabCache::new("kmalloc-2048", 2048, 64),
    SlabCache::new("kmalloc-4096", 4096, 64),
];

struct KmallocCaches(UnsafeCell<[SlabCache; KMALLOC_SIZES]>);
unsafe impl Sync for KmallocCaches {}

static KMALLOC: KmallocCaches = KmallocCaches(UnsafeCell::new(KMALLOC_CACHES_INIT));

/// 获取 kmalloc 缓存数组可变引用
fn kmalloc_slab() -> &'static mut [SlabCache; KMALLOC_SIZES] {
    unsafe { &mut *KMALLOC.0.get() }
}

/// 查找适配指定尺寸的最小缓存索引
fn kmalloc_index(size: usize) -> Option<usize> {
    for (i, s) in KMALLOC_SIZES_TABLE.iter().enumerate() {
        if *s >= size {
            return Some(i);
        }
    }
    None
}

/* ── 公开 API ── */

/// 初始化所有 kmalloc 标准缓存（每档预分配一页）
///
/// # Safety
/// 必须在 PMM 初始化之后调用，且只调用一次。
pub unsafe fn init() {
    crate::serial::write_str(b"  slab: init start\n");
    for i in 0..KMALLOC_SIZES {
        crate::serial::write_str(b"  slab: growing cache ");
        crate::serial_put_u64(i as u64);
        crate::serial::write_str(b" size=");
        crate::serial_put_u64(KMALLOC_SIZES_TABLE[i] as u64);
        crate::serial::write_str(b"\n");
        let ok = kmalloc_slab()[i].grow();
        if ok {
            crate::serial::write_str(b"  slab: grow ok\n");
        } else {
            crate::serial::write_str(b"  slab: grow FAILED\n");
        }
    }
    crate::serial::write_str(b"  slab: init done\n");
}

/// 分配内核内存
///
/// 尺寸 <= 4096 走 SLAB 缓存，> 4096 走 PMM 大块分配。
pub fn kmalloc(size: usize) -> Option<NonNull<u8>> {
    if size == 0 {
        return None;
    }
    if size > SLAB_MAX_SIZE {
        let pages =
            (size + pmm::PAGE_SIZE as usize - 1) / pmm::PAGE_SIZE as usize;
        let order: u8 = pages.next_power_of_two().trailing_zeros() as u8;
        let order = order.min(super::pmm::MAX_ORDER as u8);
        let paddr = pmm::alloc_pages(order)?;
        Some(unsafe { NonNull::new_unchecked(paddr as *mut u8) })
    } else {
        let idx = kmalloc_index(size)?;
        kmalloc_slab()[idx].alloc()
    }
}

/// 分配并清零内核内存
pub fn kzalloc(size: usize) -> Option<NonNull<u8>> {
    let ptr = kmalloc(size)?;
    unsafe {
        core::ptr::write_bytes(ptr.as_ptr(), 0, size);
    }
    Some(ptr)
}

/// 释放内核内存
///
/// # Safety
/// `ptr` 必须来自 `kmalloc`/`kzalloc`，且未释放过。
pub unsafe fn kfree(ptr: NonNull<u8>) {
    let addr = ptr.as_ptr() as u64;
    let head_addr = addr & !(pmm::PAGE_SIZE - 1);
    let head = head_addr as *const SlabHead;
    let cache = (*head).cache;
    if cache.is_null() {
        // PMM 大块分配 — 释放单页
        pmm::free_one(addr & !(pmm::PAGE_SIZE - 1));
    } else {
        (*cache).free(ptr);
    }
}

/// 返回分配对象的实际大小
pub fn ksize(ptr: NonNull<u8>) -> usize {
    let addr = ptr.as_ptr() as u64;
    let head_addr = addr & !(pmm::PAGE_SIZE - 1);
    unsafe {
        let head = head_addr as *const SlabHead;
        let cache = (*head).cache;
        if cache.is_null() {
            pmm::PAGE_SIZE as usize
        } else {
            (*cache).obj_size
        }
    }
}

/* ── 专用对象缓存 API ── */

/// 创建专用 SLAB 缓存（供内核子系统使用）
pub(crate) fn create_cache(
    name: &'static str,
    obj_size: usize,
    _align: usize,
) -> Option<NonNull<SlabCache>> {
    let ptr = kmalloc(core::mem::size_of::<SlabCache>())?;
    unsafe {
        core::ptr::write(
            ptr.as_ptr() as *mut SlabCache,
            SlabCache::new(name, obj_size, _align),
        );
        let cache = &mut *(ptr.as_ptr() as *mut SlabCache);
        if !cache.grow() {
            kfree(ptr);
            return None;
        }
        Some(ptr.cast())
    }
}

/// 从专用缓存分配对象
pub(crate) unsafe fn cache_alloc(cache: &mut SlabCache) -> Option<NonNull<u8>> {
    cache.alloc()
}

/// 释放对象回专用缓存
pub(crate) unsafe fn cache_free(cache: &mut SlabCache, ptr: NonNull<u8>) {
    cache.free(ptr)
}

/* ── FS 专用缓存 ── */

struct SegHeadCache(UnsafeCell<SlabCache>);
unsafe impl Sync for SegHeadCache {}

static SEG_HEAD_CACHE: SegHeadCache = SegHeadCache(UnsafeCell::new(SlabCache::new(
    "segment-head",
    core::mem::size_of::<crate::fs::segment_cache::SegmentHead>(),
    8,
)));

/// 分配 SegmentHead（从专用 SLAB 缓存）
pub fn alloc_segment_head() -> Option<NonNull<u8>> {
    unsafe { SEG_HEAD_CACHE.0.get().as_mut().unwrap().alloc() }
}

/// 释放 SegmentHead 回专用 SLAB 缓存
pub unsafe fn free_segment_head(ptr: NonNull<u8>) {
    unsafe { SEG_HEAD_CACHE.0.get().as_mut().unwrap().free(ptr) }
}

/// 初始化 FS 专用 SLAB 缓存
pub fn init_fs_caches() {
    unsafe {
        let cache = &mut *SEG_HEAD_CACHE.0.get();
        cache.grow();
    }
    crate::serial::write_str(b"  slab: fs caches init done\n");
}

/* ── 内核全局分配器（桥接 alloc crate） ── */

/// 内核全局分配器，将 Rust `alloc` crate 的分配请求路由到 SLAB
pub struct KernelAllocator;

unsafe impl core::alloc::GlobalAlloc for KernelAllocator {
    /// 分配内存，size 取 layout.size() 和 layout.align() 的较大值
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size().max(layout.align());
        match kmalloc(size) {
            Some(ptr) => ptr.as_ptr(),
            None => core::ptr::null_mut(),
        }
    }

    /// 释放内存
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: core::alloc::Layout) {
        if let Some(nn) = NonNull::new(ptr) {
            unsafe {
                kfree(nn);
            }
        }
    }
}
