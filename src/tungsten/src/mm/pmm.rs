
// mm/pmm.rs — 物理内存管理器（伙伴系统）
// 从 BootInfo 内存映射构建空闲页池，按 Zone 管理
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::bootinfo::BootInfo;
use core::cell::UnsafeCell;

/* ── 常量 ── */

/// 页大小 4KB
pub const PAGE_SIZE: u64 = 4096;
/// 页框号移位（log2 4096）
pub const PAGE_SHIFT: u64 = 12;
/// 最大伙伴阶（4MB 连续块）
pub const MAX_ORDER: usize = 10;
/// Zone 数量
pub const ZONE_COUNT: usize = 3;

/* ── 物理页状态标志 ── */

/// 物理页标志（u32 位掩码）
pub mod page_flags {
    pub const NONE: u32      = 0;
    pub const ALLOCATED: u32 = 1 << 0;
    pub const SLAB: u32      = 1 << 1;
    pub const RESERVED: u32  = 1 << 2;
    pub const BUDDY: u32     = 1 << 3;   // 在伙伴系统空闲链表中
}

/// 检查标志位
pub fn has_flag(flags: u32, flag: u32) -> bool {
    flags & flag != 0
}

/* ── 物理页框描述符 ── */

/// 每 4KB 物理页对应一个描述符
#[derive(Clone, Copy)]
#[repr(C)]
pub struct PhysPage {
    pub flags: u32,
    pub order: u8,              // 伙伴阶（仅 buddy 页有效）
    pub _pad: u8,
    pub refcount: u16,
    _reserved: [u8; 8],
}

impl PhysPage {
    pub const fn new() -> Self {
        PhysPage {
            flags: 0,
            order: 0,
            _pad: 0,
            refcount: 0,
            _reserved: [0; 8],
        }
    }

    pub fn is_free(&self) -> bool {
        has_flag(self.flags, page_flags::BUDDY)
    }

    pub fn set_flag(&mut self, flag: u32) {
        self.flags |= flag;
    }

    pub fn clear_flag(&mut self, flag: u32) {
        self.flags &= !flag;
    }
}

/* ── 空闲链表（侵入式单链表） ── */

/// 空闲页块链表结点（嵌入在空闲页的前 16 字节中）
#[repr(C)]
struct FreeNode {
    next: *mut FreeNode,
    prev: *mut FreeNode,
}

impl FreeNode {
    const fn new() -> Self {
        FreeNode {
            next: core::ptr::null_mut(),
            prev: core::ptr::null_mut(),
        }
    }
}

/// 侵入式链表头
#[repr(C)]
struct FreeList {
    head: FreeNode,
}

impl FreeList {
    const fn new() -> Self {
        FreeList { head: FreeNode::new() }
    }

    fn init(&mut self) {
        self.head.next = &mut self.head as *mut FreeNode;
        self.head.prev = &mut self.head as *mut FreeNode;
    }

    unsafe fn push(&mut self, node: *mut FreeNode) {
        let head = &mut self.head as *mut FreeNode;
        (*node).next = (*head).next;
        (*node).prev = head;
        (*(*head).next).prev = node;
        (*head).next = node;
    }

    unsafe fn remove(&mut self, node: *mut FreeNode) {
        (*(*node).prev).next = (*node).next;
        (*(*node).next).prev = (*node).prev;
    }

    fn pop(&mut self) -> Option<*mut FreeNode> {
        unsafe {
            let head = &mut self.head as *mut FreeNode;
            if (*head).next == head {
                return None;
            }
            let node = (*head).next;
            self.remove(node);
            Some(node)
        }
    }

    fn is_empty(&self) -> bool {
        self.head.next == &self.head as *const FreeNode as *mut FreeNode
    }
}

/* ── 空闲区域（每个阶一个空闲链表） ── */

#[repr(C)]
pub struct FreeArea {
    free_list: FreeList,
    nr_free: u64,
}

impl FreeArea {
    const fn new() -> Self {
        FreeArea {
            free_list: FreeList::new(),
            nr_free: 0,
        }
    }

    fn init(&mut self) {
        self.free_list.init();
        self.nr_free = 0;
    }
}

/* ── 内存区段 ── */

#[repr(C)]
pub struct Zone {
    pub name: &'static str,
    pub start_pfn: u64,
    pub end_pfn: u64,
    pub managed_pages: u64,
    pub free_areas: [FreeArea; MAX_ORDER + 1],
}

impl Zone {
    const fn new(name: &'static str) -> Self {
        Zone {
            name,
            start_pfn: 0,
            end_pfn: 0,
            managed_pages: 0,
            free_areas: [
                FreeArea::new(), FreeArea::new(), FreeArea::new(),
                FreeArea::new(), FreeArea::new(), FreeArea::new(),
                FreeArea::new(), FreeArea::new(), FreeArea::new(),
                FreeArea::new(), FreeArea::new(),
            ],
        }
    }

    fn init(&mut self, start: u64, end: u64) {
        self.start_pfn = start;
        self.end_pfn = end;
        for i in 0..=MAX_ORDER {
            self.free_areas[i].init();
        }
    }

    fn contains(&self, pfn: u64) -> bool {
        pfn >= self.start_pfn && pfn < self.end_pfn
    }
}

/* ── 伙伴分配器 ── */

#[repr(C)]
pub struct BuddyAllocator {
    pub zones: [Zone; ZONE_COUNT],
    pub total_pages: u64,
    pub free_pages: u64,
    page_array: *mut PhysPage,
    page_array_size: u64,   // 页描述符数组元素数
}

impl BuddyAllocator {
    const fn new() -> Self {
        BuddyAllocator {
            zones: [
                Zone::new("DMA"),
                Zone::new("Normal"),
                Zone::new("High"),
            ],
            total_pages: 0,
            free_pages: 0,
            page_array: core::ptr::null_mut(),
            page_array_size: 0,
        }
    }

    /// 根据物理地址返回对应页描述符
    pub fn page_from_paddr(&self, paddr: u64) -> Option<&PhysPage> {
        let pfn = paddr >> PAGE_SHIFT;
        if pfn >= self.page_array_size {
            return None;
        }
        unsafe { Some(&*self.page_array.add(pfn as usize)) }
    }

    /// 根据物理地址返回可变页描述符引用
    pub fn page_from_paddr_mut(&mut self, paddr: u64) -> Option<&mut PhysPage> {
        let pfn = paddr >> PAGE_SHIFT;
        if pfn >= self.page_array_size {
            return None;
        }
        unsafe { Some(&mut *self.page_array.add(pfn as usize)) }
    }

    /// 返回页框号对应的页描述符
    fn page(&self, pfn: u64) -> &PhysPage {
        unsafe { &*self.page_array.add(pfn as usize) }
    }

    fn page_mut(&mut self, pfn: u64) -> &mut PhysPage {
        unsafe { &mut *self.page_array.add(pfn as usize) }
    }

    /// 分配 2^order 个连续物理页
    pub fn alloc_pages(&mut self, order: u8) -> Option<u64> {
        if order as usize > MAX_ORDER {
            return None;
        }

        // Phase 1: 查找有空闲块的 Zone 和阶
        let mut zone_idx: Option<usize> = None;
        let mut found_order: usize = MAX_ORDER + 1;
        for (idx, zone) in self.zones.iter().enumerate() {
            if zone.start_pfn >= zone.end_pfn { continue; }
            for o in (order as usize)..=MAX_ORDER {
                if zone.free_areas[o].nr_free > 0 {
                    zone_idx = Some(idx);
                    found_order = o;
                    break;
                }
            }
            if zone_idx.is_some() { break; }
        }

        let zi = zone_idx?;
        let o = found_order;

        // Phase 2: 用裸指针分配（避免 &mut self 与 zone 借用冲突）
        let page_array = self.page_array;
        let zone_ptr = &mut self.zones[zi] as *mut Zone;

        unsafe {
            crate::serial::write_str(b"    alloc_pages: zi=");
            crate::serial_put_u64(zi as u64);
            crate::serial::write_str(b" order=");
            crate::serial_put_u64(o as u64);
            crate::serial::write_str(b" nr_free=");
            crate::serial_put_u64((*zone_ptr).free_areas[o].nr_free);
            crate::serial::write_str(b"\n");

            // 从空闲链表取出一个块
            let block = (*zone_ptr).free_areas[o].free_list.pop()
                .expect("nr_free > 0 but pop returned None");

            crate::serial::write_str(b"    alloc_pages: block=");
            crate::serial_put_u64_hex(block as u64);
            crate::serial::write_str(b" page_array=");
            crate::serial_put_u64_hex(page_array as u64);
            crate::serial::write_str(b"\n");

            (*zone_ptr).free_areas[o].nr_free -= 1;

            let pfn = (block as u64 - page_array as u64)
                / core::mem::size_of::<PhysPage>() as u64;

            crate::serial::write_str(b"    alloc_pages: pfn=");
            crate::serial_put_u64(pfn);
            crate::serial::write_str(b" paddr=0x");
            crate::serial_put_u64_hex(pfn << PAGE_SHIFT);
            crate::serial::write_str(b" nr_free_left=");
            crate::serial_put_u64((*zone_ptr).free_areas[o].nr_free);
            crate::serial::write_str(b"\n");

            // 检查 block 节点的 next/prev
            let node = block as *const FreeNode;
            crate::serial::write_str(b"    alloc: node.next=");
            crate::serial_put_u64_hex((*node).next as u64);
            crate::serial::write_str(b" prev=");
            crate::serial_put_u64_hex((*node).prev as u64);
            crate::serial::write_str(b"\n");

            // 分裂：大块拆分为小块直到目标阶
            let mut current_order = o;
            while current_order > order as usize {
                current_order -= 1;
                let buddy_pfn = pfn + (1 << current_order);
                (*page_array.add(buddy_pfn as usize)).flags = page_flags::BUDDY;
                (*page_array.add(buddy_pfn as usize)).order = current_order as u8;
                (*zone_ptr).free_areas[current_order].free_list.push(
                    page_array.add(buddy_pfn as usize) as *mut FreeNode
                );
                (*zone_ptr).free_areas[current_order].nr_free += 1;
            }

            // 标记已分配
            (*page_array.add(pfn as usize)).flags = page_flags::ALLOCATED;
            (*page_array.add(pfn as usize)).order = order;
            (*page_array.add(pfn as usize)).refcount = 0;

            self.free_pages -= 1 << order;
            Some(pfn << PAGE_SHIFT)
        }
    }

    /// 分配单页
    pub fn alloc_one(&mut self) -> Option<u64> {
        self.alloc_pages(0)
    }

    /// 分配并清零
    pub fn alloc_zeroed(&mut self) -> Option<u64> {
        let paddr = self.alloc_pages(0)?;
        // 通过 HHDM 访问清零，不可将物理地址直接当指针用
        unsafe {
            core::ptr::write_bytes((HHDM_OFFSET + paddr) as *mut u8, 0, PAGE_SIZE as usize);
        }
        Some(paddr)
    }

    /// 释放物理页
    pub fn free_pages(&mut self, paddr: u64, order: u8) {
        let pfn = paddr >> PAGE_SHIFT;
        if pfn >= self.page_array_size { return; }

        let zone_idx = match self.find_zone(pfn) {
            Some(z) => z,
            None => return,
        };

        let mut current_pfn = pfn;
        let mut current_order = order as usize;

        // 使用裸指针操作，避免借用冲突
        let page_array = self.page_array;
        let zone = &mut self.zones[zone_idx] as *mut Zone;

        unsafe {
            // 合并伙伴（向上合并）
            while current_order < MAX_ORDER {
                let buddy_pfn = current_pfn ^ (1 << current_order);
                if !(*zone).contains(buddy_pfn) { break; }

                let buddy = &*page_array.add(buddy_pfn as usize);
                if !buddy.is_free() || buddy.order != current_order as u8 { break; }

                // 从空闲链表移除伙伴
                (*zone).free_areas[current_order].free_list.remove(
                    page_array.add(buddy_pfn as usize) as *mut FreeNode
                );
                (*zone).free_areas[current_order].nr_free -= 1;

                if buddy_pfn < current_pfn {
                    current_pfn = buddy_pfn;
                }
                current_order += 1;
            }

            // 将合并后的块加入空闲链表
            (*page_array.add(current_pfn as usize)).flags = page_flags::BUDDY;
            (*page_array.add(current_pfn as usize)).order = current_order as u8;
            (*zone).free_areas[current_order].free_list.push(
                page_array.add(current_pfn as usize) as *mut FreeNode
            );
            (*zone).free_areas[current_order].nr_free += 1;
        }
        self.free_pages += 1 << current_order;
    }

    /// 释放单页
    pub fn free_one(&mut self, paddr: u64) {
        self.free_pages(paddr, 0);
    }

    /// 查找页框号所属 Zone
    fn find_zone(&self, pfn: u64) -> Option<usize> {
        for (i, zone) in self.zones.iter().enumerate() {
            if zone.contains(pfn) {
                return Some(i);
            }
        }
        None
    }
}

/* ── 全局 PMM 实例 ── */

/// 单线程包裹（内核早期启动阶段仅单核运行）
struct PmmWrapper(UnsafeCell<BuddyAllocator>);
unsafe impl Sync for PmmWrapper {}

/// 伙伴系统分配器实例
static PMM: PmmWrapper = PmmWrapper(UnsafeCell::new(BuddyAllocator::new()));
/// 全局 HHDM 偏移量（在 init 中设置），供 alloc_zeroed 清零页时使用
static mut HHDM_OFFSET: u64 = 0;

/// 获取 PMM 可变引用
fn pmm() -> &'static mut BuddyAllocator {
    unsafe { &mut *PMM.0.get() }
}

/// UEFI 内存类型常量
const EFI_CONVENTIONAL_MEMORY: u32 = 7;
const EFI_BOOT_SERVICES_DATA: u32 = 4;
const EFI_LOADER_DATA: u32 = 2;
const EFI_LOADER_CODE: u32 = 1;

/// 从 BootInfo 内存映射初始化伙伴系统
///
/// # Safety
/// 必须在 Phase 2 初始化调用，且只调用一次。
pub unsafe fn init(boot_info: &BootInfo) {
    HHDM_OFFSET = boot_info.hhdm_offset;
    let mmap = boot_info.memory_map();
    crate::serial::write_str(b"  pmm: mmap entries=");
    crate::serial_put_u64(boot_info.mmap_entries);
    crate::serial::write_str(b"\n");

    // Phase 1: 扫描内存映射，找出最大可用页框号
    let mut max_pfn: u64 = 0;
    for entry in mmap {
        if entry.type_ == EFI_CONVENTIONAL_MEMORY
            || entry.type_ == EFI_BOOT_SERVICES_DATA
            || entry.type_ == EFI_LOADER_DATA
            || entry.type_ == EFI_LOADER_CODE
        {
            let end_pfn = (entry.physical_start + entry.number_of_pages * PAGE_SIZE) >> PAGE_SHIFT;
            if end_pfn > max_pfn {
                max_pfn = end_pfn;
            }
        }
    }

    crate::serial::write_str(b"  pmm: max_pfn=");
    crate::serial_put_u64(max_pfn);
    crate::serial::write_str(b"\n");

    // Phase 2: 为 PhysPage 数组分配物理页面（从 Limine 内存映射中找 USABLE 区域）
    let array_bytes = (max_pfn as usize) * core::mem::size_of::<PhysPage>();
    let array_pages = (array_bytes as u64 + PAGE_SIZE - 1) >> PAGE_SHIFT;

    // 扫描内存映射，找到足够大的 USABLE 区域放置页描述符数组
    let mut phys_array_base: u64 = 0;
    for entry in boot_info.memory_map() {
        if entry.type_ != 7 { continue; }  // 7 = USABLE
        let start = entry.physical_start;
        let end = start + entry.number_of_pages * PAGE_SIZE;
        if end - start >= array_bytes as u64 + PAGE_SIZE {
            phys_array_base = (start + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            break;
        }
    }
    assert!(phys_array_base > 0, "no usable memory for page array");

    // 通过 HHDM 窗口访问页描述符数组
    let hhdm_array_base = boot_info.hhdm_offset + phys_array_base;

    let pmm = pmm();
    pmm.page_array = hhdm_array_base as *mut PhysPage;
    pmm.page_array_size = max_pfn;

    // 清零页描述符数组（HHDM 映射保证可访问）
    core::ptr::write_bytes(pmm.page_array, 0, array_bytes);

    crate::serial::write_str(b"  pmm: array_bytes=");
    crate::serial_put_u64(array_bytes as u64);
    crate::serial::write_str(b", phys_base=0x");
    crate::serial_put_u64_hex(phys_array_base);
    crate::serial::write_str(b", hhdm=0x");
    crate::serial_put_u64_hex(hhdm_array_base);
    crate::serial::write_str(b"\n");

    // Phase 3: 设置 Zone 边界
    crate::serial::write_str(b"  pmm: zone init\n");
    // DMA Zone: 0 ~ 0xFFFFFF (16MB)
    // Normal Zone: 0x1000000 ~ 0xFFFFFFFF (4GB)
    // 计算实际最大可用物理内存
    let dma_end = (16 * 1024 * 1024) >> PAGE_SHIFT;   // 16MB
    let normal_end = (4096 * 1024 * 1024u64) >> PAGE_SHIFT; // 4GB

    pmm.zones[0].init(0, dma_end.min(max_pfn));
    pmm.zones[1].init(dma_end, normal_end.min(max_pfn));
    pmm.zones[2].init(normal_end, max_pfn);

    // Phase 4: build free list — push each free page to zone order-0 list
    pmm.total_pages = 0;
    let pmm_ptr: *mut BuddyAllocator = pmm;
    crate::serial::write_str(b"  pmm: phase4\n");

    // 先打印各区的 PFN 范围
    unsafe {
        for i in 0..ZONE_COUNT {
            crate::serial::write_str(b"  pmm: zone");
            crate::serial_put_u64(i as u64);
            crate::serial::write_str(b" pfn_range=");
            crate::serial_put_u64((*pmm_ptr).zones[i].start_pfn);
            crate::serial::write_str(b"-");
            crate::serial_put_u64((*pmm_ptr).zones[i].end_pfn);
            crate::serial::write_str(b"\n");
        }
    }

    // 打印落在 Zone 0 范围内的所有可用 mmap 条目
    unsafe {
        crate::serial::write_str(b"  pmm: zone0 mmap entries (usable):\n");
        for entry in mmap {
            let usable = entry.type_ == EFI_CONVENTIONAL_MEMORY
                || entry.type_ == EFI_BOOT_SERVICES_DATA;
            if !usable { continue; }
            let start_pfn = entry.physical_start >> PAGE_SHIFT;
            let end_pfn = ((entry.physical_start + entry.number_of_pages * PAGE_SIZE) >> PAGE_SHIFT)
                .min(max_pfn);
            // 检查是否与 Zone 0 有交集
            if start_pfn < (*pmm_ptr).zones[0].end_pfn && end_pfn > (*pmm_ptr).zones[0].start_pfn {
                let z0_start = if start_pfn < (*pmm_ptr).zones[0].start_pfn { (*pmm_ptr).zones[0].start_pfn } else { start_pfn };
                let z0_end = if end_pfn > (*pmm_ptr).zones[0].end_pfn { (*pmm_ptr).zones[0].end_pfn } else { end_pfn };
                crate::serial::write_str(b"    type=");
                crate::serial_put_u64(entry.type_ as u64);
                crate::serial::write_str(b" pfn=");
                crate::serial_put_u64(z0_start);
                crate::serial::write_str(b"-");
                crate::serial_put_u64(z0_end);
                crate::serial::write_str(b" pages=");
                crate::serial_put_u64(z0_end - z0_start);
                crate::serial::write_str(b"\n");
            }
        }
    }

    let mut z0_count = 0u64;
    for entry in mmap {
        let usable = entry.type_ == EFI_CONVENTIONAL_MEMORY
            || entry.type_ == EFI_BOOT_SERVICES_DATA;
        if !usable { continue; }
        let mut pfn = entry.physical_start >> PAGE_SHIFT;
        let end_pfn = ((entry.physical_start + entry.number_of_pages * PAGE_SIZE) >> PAGE_SHIFT)
            .min(max_pfn);
        while pfn < end_pfn {
            // Skip page array pages (phys_array_base 是物理地址，需换算为 pfn)
            if pfn >= (phys_array_base >> PAGE_SHIFT)
                && pfn < (((phys_array_base + array_bytes as u64 + 0xFFF) >> PAGE_SHIFT).min(max_pfn))
            {
                pfn = ((phys_array_base + array_bytes as u64 + 0xFFF) >> PAGE_SHIFT).min(max_pfn);
                continue;
            }
            unsafe {
                // Find owning zone via raw pointer (avoid &mut conflict)
                let mut zi = ZONE_COUNT;
                for i in 0..ZONE_COUNT {
                    if pfn >= (*pmm_ptr).zones[i].start_pfn && pfn < (*pmm_ptr).zones[i].end_pfn {
                        zi = i;
                        break;
                    }
                }
                if zi >= ZONE_COUNT { pfn += 1; continue; }

                // Mark as BUDDY, order 0, and push to zone free list
                let page = (*pmm_ptr).page_array.add(pfn as usize);
                (*page).flags = page_flags::BUDDY;
                (*page).order = 0;
                if zi == 0 && z0_count < 160 {
                    crate::serial::write_str(b"    push z0 pfn=");
                    crate::serial_put_u64(pfn);
                    crate::serial::write_str(b"\n");
                }
                (*pmm_ptr).zones[zi].free_areas[0].free_list.push(page as *mut FreeNode);
                (*pmm_ptr).zones[zi].free_areas[0].nr_free += 1;
                (*pmm_ptr).total_pages += 1;
                (*pmm_ptr).free_pages += 1;
                if zi == 0 { z0_count += 1; }
            }
            pfn += 1;
        }
    }
    crate::serial::write_str(b"  pmm: zone0 actual pushes=");
    crate::serial_put_u64(z0_count);
    crate::serial::write_str(b"\n");
    crate::serial::write_str(b"  pmm: done pages=");
    unsafe { crate::serial_put_u64((*pmm_ptr).total_pages); }
    crate::serial::write_str(b"\n");

    // 验证 free list 节点数
    unsafe {
        for zi in 0..3 {
            if (*pmm_ptr).zones[zi].start_pfn >= (*pmm_ptr).zones[zi].end_pfn { continue; }
            let mut count = 0u64;
            let head = &(*pmm_ptr).zones[zi].free_areas[0].free_list.head as *const FreeNode as *mut FreeNode;
            let mut curr = (*head).next;
            while curr != head {
                count += 1;
                if count > 200000 { break; }
                curr = (*curr).next;
            }
            crate::serial::write_str(b"  pmm: zone ");
            crate::serial_put_u64(zi as u64);
            crate::serial::write_str(b" link_count=");
            crate::serial_put_u64(count);
            crate::serial::write_str(b" nr_free=");
            crate::serial_put_u64((*pmm_ptr).zones[zi].free_areas[0].nr_free);
            crate::serial::write_str(b"\n");

            // 打印前 10 个节点 PFN
            crate::serial::write_str(b"  pmm: zone ");
            crate::serial_put_u64(zi as u64);
            crate::serial::write_str(b" first PTEs: ");
            curr = (*head).next;
            for _ in 0..10 {
                if curr == head { break; }
                let pfn = (curr as u64 - (*pmm_ptr).page_array as u64)
                    / core::mem::size_of::<PhysPage>() as u64;
                crate::serial_put_u64(pfn);
                crate::serial::write_str(b" ");
                curr = (*curr).next;
            }
            crate::serial::write_str(b"\n");
        }
    }
}

/* ── 高层封装 ── */

/// 分配 2^order 个连续物理页（返回物理地址）
pub fn alloc_pages(order: u8) -> Option<u64> {
    pmm().alloc_pages(order)
}

/// 分配单页（4KB）
pub fn alloc_one() -> Option<u64> {
    pmm().alloc_one()
}

/// 分配并清零单页
pub fn alloc_zeroed() -> Option<u64> {
    pmm().alloc_zeroed()
}

/// 释放物理页
pub fn free_pages(paddr: u64, order: u8) {
    pmm().free_pages(paddr, order)
}

/// 释放单页
pub fn free_one(paddr: u64) {
    pmm().free_one(paddr)
}

/// 返回内存统计：(总量, 已用, 空闲) 字节
pub fn memory_stats() -> (u64, u64, u64) {
    let p = pmm();
    let total = p.total_pages * PAGE_SIZE;
    let free = p.free_pages * PAGE_SIZE;
    let used = total.saturating_sub(free);
    (total, used, free)
}
