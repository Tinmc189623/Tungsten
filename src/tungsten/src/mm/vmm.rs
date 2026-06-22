// mm/vmm.rs — 虚拟内存管理器（x86_64 四级页表）
// 管理页表操作、内核地址空间布局、VMA、缺页处理
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use crate::mm::pmm;

/* ── 常量 ── */

/// 页大小 4KB
pub const PAGE_SIZE: u64 = 4096;
/// 页内偏移位数
pub const PAGE_SHIFT: u64 = 12;
/// 页内偏移掩码
pub const PAGE_MASK: u64 = 0xFFF;

/// PML4 索引移位
pub const PML4_SHIFT: u64 = 39;
/// PDPT 索引移位
pub const PDPT_SHIFT: u64 = 30;
/// PD 索引移位
pub const PD_SHIFT: u64  = 21;
/// PT 索引移位
pub const PT_SHIFT: u64  = 12;

/// 每级页表索引掩码（9 位 = 512 项）
pub const TABLE_MASK: u64 = 0x1FF;

/* ── 内核地址空间布局 ── */

/// 直接物理映射区基址（phys → virt 恒等映射）
pub const DIRECT_MAP_BASE: u64 = 0xFFFF800000000000;
/// 内核映像基址（高半核）
pub const KERNEL_BASE: u64     = 0xFFFFFFFF80000000;
/// vmalloc 动态映射区基址
pub const VMALLOC_BASE: u64    = 0xFFFFC90000000000;
/// 固定映射区基址
pub const FIXMAP_BASE: u64     = 0xFFFFFE8000000000;

/// 页表条目中物理地址掩码（提取 bits 12-51）
pub const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/* ── 页表条目标志 ── */

/// 页表条目属性标志（位域）
pub mod flags {
    /// 页存在位
    pub const PRESENT: u64    = 1 << 0;
    /// 可写
    pub const WRITABLE: u64  = 1 << 1;
    /// 用户态可访问
    pub const USER: u64      = 1 << 2;
    /// 写穿透
    pub const WRITE_THRU: u64= 1 << 3;
    /// 禁用缓存
    pub const NO_CACHE: u64  = 1 << 4;
    /// 已访问
    pub const ACCESSED: u64  = 1 << 5;
    /// 已脏写
    pub const DIRTY: u64     = 1 << 6;
    /// 大页（2MB/1GB）
    pub const HUGE_PAGE: u64 = 1 << 7;
    /// 全局页（不随 CR3 切换刷新 TLB）
    pub const GLOBAL: u64    = 1 << 8;
    /// 禁止执行（NX bit）
    pub const NO_EXEC: u64   = 1 << 63;
}

/* ── 索引提取 ── */

/// 从虚拟地址提取 PML4 索引（bits 39-47）
fn pml4_index(va: u64) -> usize { ((va >> PML4_SHIFT) & TABLE_MASK) as usize }
/// 从虚拟地址提取 PDPT 索引（bits 30-38）
fn pdpt_index(va: u64) -> usize { ((va >> PDPT_SHIFT) & TABLE_MASK) as usize }
/// 从虚拟地址提取 PD 索引（bits 21-29）
fn pd_index(va: u64)   -> usize { ((va >> PD_SHIFT) & TABLE_MASK) as usize }
/// 从虚拟地址提取 PT 索引（bits 12-20）
fn pt_index(va: u64)   -> usize { ((va >> PT_SHIFT) & TABLE_MASK) as usize }

/* ── 页表类型 ── */

/// 页表条目（u64）
type Entry = u64;

/// PML4 表（512 项，4KB 对齐）
#[repr(C, align(4096))]
pub struct Pml4([Entry; 512]);

/// 页目录指针表（PDPT，512 项）
#[repr(C, align(4096))]
pub struct Pdpt([Entry; 512]);

/// 页目录（PD，512 项）
#[repr(C, align(4096))]
pub struct Pd([Entry; 512]);

/// 页表（PT，512 项）
#[repr(C, align(4096))]
pub struct Pt([Entry; 512]);

/* ── 页表管理器 ── */

/// 四级页表管理器，持有 PML4 物理地址和 HHDM 偏移
pub struct PageTable {
    /// PML4 物理地址（CR3 加载值）
    pml4_paddr: u64,
    /// PML4 虚拟指针（通过 HHDM 偏移访问）
    pml4: *mut Pml4,
    /// HHDM 偏移量（物理地址 + hhdm = 虚拟地址）
    hhdm: u64,
}

impl PageTable {
    /// 将物理地址转换为 HHDM 虚拟指针
    unsafe fn phys_ptr<T>(&self, paddr: u64) -> *mut T {
        if paddr == 0 { return core::ptr::null_mut(); }
        (self.hhdm + paddr) as *mut T
    }
    /// 将物理地址转换为 HHDM 虚拟引用
    unsafe fn phys_ref<T>(&self, paddr: u64) -> &'static mut T {
        &mut *self.phys_ptr::<T>(paddr)
    }
    /// 将物理地址转为 const 引用
    unsafe fn phys_ref_const<T>(&self, paddr: u64) -> &'static T {
        &*self.phys_ptr::<T>(paddr)
    }
}

impl PageTable {
    /// 从当前运行的 CR3 创建内核页表（通过 HHDM 访问物理页表）
    pub fn new_kernel(hhdm: u64) -> Self {
        unsafe {
            let cr3: u64;
            core::arch::asm!("mov {}, cr3", out(reg) cr3);
            PageTable {
                pml4_paddr: cr3,
                pml4: (hhdm + cr3) as *mut Pml4,
                hhdm,
            }
        }
    }

    /// 分配新 PML4 并复制内核条目（上半部分 256-511）
    pub fn new_address_space(&self) -> Self {
        let paddr = pmm::alloc_zeroed().expect("VMM: failed to alloc PML4");
        let pml4 = unsafe { self.phys_ptr::<Pml4>(paddr) };

        // 复制当前内核页表的上半部分
        unsafe {
            for i in 256..512 {
                (*pml4).0[i] = (*self.pml4).0[i];
            }
        }

        PageTable {
            pml4_paddr: paddr,
            pml4,
            hhdm: self.hhdm,
        }
    }

    /// 获取页表条目（通过 HHDM 访问物理页表）
    pub fn get_entry(&self, va: u64) -> Option<Entry> {
        unsafe {
            let pml4e = (*self.pml4).0[pml4_index(va)];
            if pml4e & flags::PRESENT == 0 { return None; }

            let pdpt = self.phys_ptr::<Pdpt>(pml4e & ADDR_MASK);
            let pdpte = (*pdpt).0[pdpt_index(va)];
            if pdpte & flags::PRESENT == 0 { return None; }
            if pdpte & flags::HUGE_PAGE != 0 { return Some(pdpte); }

            let pd = self.phys_ptr::<Pd>(pdpte & ADDR_MASK);
            let pde = (*pd).0[pd_index(va)];
            if pde & flags::PRESENT == 0 { return None; }
            if pde & flags::HUGE_PAGE != 0 { return Some(pde); }

            let pt = self.phys_ptr::<Pt>(pde & ADDR_MASK);
            let pte = (*pt).0[pt_index(va)];
            if pte & flags::PRESENT == 0 { return None; }
            Some(pte)
        }
    }

    /// 映射 4KB 页（通过 HHDM 分配和访问中间页表）
    pub fn map_page(&mut self, va: u64, pa: u64, page_flags: u64) {
        unsafe {
            let pml4e = &mut (*self.pml4).0[pml4_index(va)];
            if *pml4e & flags::PRESENT == 0 {
                let paddr = pmm::alloc_zeroed().expect("VMM: alloc pdpt");
                *pml4e = paddr | flags::PRESENT | flags::WRITABLE;
            }
            let pdpt = self.phys_ptr::<Pdpt>(*pml4e & ADDR_MASK);

            let pdpte = &mut (*pdpt).0[pdpt_index(va)];
            if *pdpte & flags::PRESENT == 0 {
                let paddr = pmm::alloc_zeroed().expect("VMM: alloc pd");
                *pdpte = paddr | flags::PRESENT | flags::WRITABLE;
            }
            let pd = self.phys_ptr::<Pd>(*pdpte & ADDR_MASK);

            let pde = &mut (*pd).0[pd_index(va)];
            if *pde & flags::PRESENT == 0 {
                let paddr = pmm::alloc_zeroed().expect("VMM: alloc pt");
                *pde = paddr | flags::PRESENT | flags::WRITABLE;
            }
            let pt = self.phys_ptr::<Pt>(*pde & ADDR_MASK);
            (*pt).0[pt_index(va)] = pa | page_flags | flags::PRESENT;
        }
    }

    /// 映射 2MB 大页（通过 HHDM 访问页表）
    pub fn map_huge_2mb(&mut self, va: u64, pa: u64, page_flags: u64) {
        unsafe {
            let pml4e = &mut (*self.pml4).0[pml4_index(va)];
            if *pml4e & flags::PRESENT == 0 {
                let paddr = pmm::alloc_zeroed().expect("VMM: alloc pdpt");
                *pml4e = paddr | flags::PRESENT | flags::WRITABLE;
            }
            let pdpt = self.phys_ptr::<Pdpt>(*pml4e & ADDR_MASK);

            let pdpte = &mut (*pdpt).0[pdpt_index(va)];
            if *pdpte & flags::PRESENT == 0 {
                let paddr = pmm::alloc_zeroed().expect("VMM: alloc pd");
                *pdpte = paddr | flags::PRESENT | flags::WRITABLE;
            }
            let pd = self.phys_ptr::<Pd>(*pdpte & ADDR_MASK);
            (*pd).0[pd_index(va)] = pa | page_flags | flags::PRESENT | flags::HUGE_PAGE;
        }
    }

    /// 映射 1GB 大页（通过 HHDM 访问页表）
    pub fn map_huge_1gb(&mut self, va: u64, pa: u64, page_flags: u64) {
        unsafe {
            let pml4e = &mut (*self.pml4).0[pml4_index(va)];
            if *pml4e & flags::PRESENT == 0 {
                let paddr = pmm::alloc_zeroed().expect("VMM: alloc pdpt");
                *pml4e = paddr | flags::PRESENT | flags::WRITABLE;
            }
            let pdpt = self.phys_ptr::<Pdpt>(*pml4e & ADDR_MASK);
            (*pdpt).0[pdpt_index(va)] = pa | page_flags | flags::PRESENT | flags::HUGE_PAGE;
        }
    }

    /// 解除 4KB 页面映射并刷新 TLB
    pub fn unmap_page(&mut self, va: u64) {
        if self.get_entry(va).is_some() {
            let pte_ptr = self.get_pte_pointer(va);
            if let Some(ptr) = pte_ptr {
                unsafe { *ptr = 0; }
                unsafe { core::arch::asm!("invlpg [{0}]", in(reg) va); }
            }
        }
    }

    /// 切换页表（加载 CR3）
    pub unsafe fn activate(&self) {
        core::arch::asm!("mov cr3, {}", in(reg) self.pml4_paddr);
    }

    /// 翻译虚拟地址到物理地址
    pub fn translate(&self, va: u64) -> Option<u64> {
        let entry = self.get_entry(va)?;
        let offset = va & PAGE_MASK;
        Some((entry & ADDR_MASK) | offset)
    }

    /// 获取 PTE 指针（仅 4KB 页，用于 unmap 操作）
    fn get_pte_pointer(&self, va: u64) -> Option<*mut Entry> {
        unsafe {
            let pml4e = (*self.pml4).0[pml4_index(va)];
            if pml4e & flags::PRESENT == 0 { return None; }
            let pdpt = self.phys_ptr::<Pdpt>(pml4e & ADDR_MASK);
            let pdpte = (*pdpt).0[pdpt_index(va)];
            if pdpte & flags::PRESENT == 0 { return None; }
            if pdpte & flags::HUGE_PAGE != 0 { return None; }
            let pd = self.phys_ptr::<Pd>(pdpte & ADDR_MASK);
            let pde = (*pd).0[pd_index(va)];
            if pde & flags::PRESENT == 0 { return None; }
            if pde & flags::HUGE_PAGE != 0 { return None; }
            let pt = self.phys_ptr::<Pt>(pde & ADDR_MASK);
            Some(&mut (*pt).0[pt_index(va)])
        }
    }

    /// 返回 PML4 物理地址
    pub fn pml4_paddr(&self) -> u64 {
        self.pml4_paddr
    }
}

/* ── VMA 管理 ── */

/// VMA（虚拟内存区域）标志
pub mod vma_flags {
    /// 可读
    pub const READ: u64      = 1 << 0;
    /// 可写
    pub const WRITE: u64     = 1 << 1;
    /// 可执行
    pub const EXEC: u64      = 1 << 2;
    /// 向下增长（栈）
    pub const GROWSDOWN: u64 = 1 << 3;
    /// 共享映射
    pub const SHARED: u64    = 1 << 4;
}

/// 虚拟内存区域描述
#[derive(Clone, Copy)]
pub struct VmArea {
    /// 区域起始虚拟地址
    pub start: u64,
    /// 区域结束虚拟地址（不含）
    pub end: u64,
    /// 区域标志（vma_flags）
    pub flags: u64,
}

/// VMA 最大数量
pub const VMA_MAX: usize = 64;

/// VMA 列表（固定大小数组，无动态分配）
pub struct VmaList {
    areas: [VmArea; VMA_MAX],
    count: usize,
}

impl VmaList {
    /// 创建空的 VMA 列表
    pub const fn new() -> Self {
        VmaList {
            areas: [VmArea {
                start: 0,
                end: 0,
                flags: 0,
            }; VMA_MAX],
            count: 0,
        }
    }

    /// 添加一个 VMA 区域
    pub fn add(&mut self, start: u64, end: u64, flags: u64) -> bool {
        if self.count >= VMA_MAX {
            return false;
        }
        self.areas[self.count] = VmArea { start, end, flags };
        self.count += 1;
        true
    }

    /// 查找包含指定地址的 VMA
    pub fn find(&self, addr: u64) -> Option<&VmArea> {
        for i in 0..self.count {
            let a = &self.areas[i];
            if addr >= a.start && addr < a.end {
                return Some(a);
            }
        }
        None
    }

    /// 查找包含指定地址的 VMA（可变引用）
    pub fn find_mut(&mut self, addr: u64) -> Option<&mut VmArea> {
        for i in 0..self.count {
            if addr >= self.areas[i].start && addr < self.areas[i].end {
                return Some(&mut self.areas[i]);
            }
        }
        None
    }

    /// 扩展向下增长的 VMA（栈区域自动增长）
    pub fn expand_down(&mut self, addr: u64) -> bool {
        for i in 0..self.count {
            if self.areas[i].flags & vma_flags::GROWSDOWN != 0
                && addr + 4096 >= self.areas[i].start
                && addr < self.areas[i].end
            {
                self.areas[i].start = addr & !0xFFF;
                return true;
            }
        }
        false
    }
}

/// 内核 VMA 列表（跟踪内核地址空间映射）
pub static KERNEL_VMA_LIST: crate::sync::Mutex<VmaList> =
    crate::sync::Mutex::new(VmaList::new());

/// 注册内核 VMA 区域（直接映射区、内核映像、vmalloc 区）
pub fn init_kernel_vmas() {
    let mut list = KERNEL_VMA_LIST.lock();
    list.add(
        DIRECT_MAP_BASE,
        DIRECT_MAP_BASE + (4u64 << 30),
        vma_flags::READ | vma_flags::WRITE,
    );
    list.add(
        KERNEL_BASE,
        KERNEL_BASE + (16u64 << 20),
        vma_flags::READ | vma_flags::WRITE | vma_flags::EXEC,
    );
    list.add(VMALLOC_BASE, FIXMAP_BASE, vma_flags::READ | vma_flags::WRITE);
}

/* ── 全局 VMM ── */

struct VmmWrapper(UnsafeCell<MaybeUninit<PageTable>>);
unsafe impl Sync for VmmWrapper {}

/// 全局内核页表（init 前未初始化）
static KERNEL_PT: VmmWrapper = VmmWrapper(UnsafeCell::new(MaybeUninit::uninit()));

/// 获取内核页表引用
pub fn kernel_pt() -> &'static mut PageTable {
    unsafe { (*KERNEL_PT.0.get()).assume_init_mut() }
}

/// 初始化内核页表
///
/// 建立：
/// 1. 直接物理映射区 (DIRECT_MAP_BASE + phys → phys)
/// 2. 恒等映射 (phys → phys) 用于兼容
/// 3. 从 CR3 继承内核高半映射
///
/// # Safety
/// 必须在 PMM 初始化后调用，且只调用一次。
pub unsafe fn init(boot_info: &crate::bootinfo::BootInfo) {
    let pt = PageTable::new_kernel(boot_info.hhdm_offset);
    (*KERNEL_PT.0.get()).as_mut_ptr().write(pt);
    let kpt = kernel_pt();

    // 计算需要映射的物理内存上限（上限 4GB）
    let total_phys = boot_info
        .memory_map()
        .iter()
        .filter(|e| {
            let t = e.type_;
            t == 7 || t == 4 || t == 2 || t == 1
        })
        .map(|e| e.physical_start + e.number_of_pages * PAGE_SIZE)
        .max()
        .unwrap_or(0)
        .min(4u64 * 1024 * 1024 * 1024);

    crate::serial::write_str(b"  vmm: total_phys=");
    crate::serial_put_u64(total_phys);
    crate::serial::write_str(b"\n");

    // 建立直接物理映射区（全部使用 2MB 大页）
    let map_end =
        (total_phys + (2u64 * 1024 * 1024) - 1) & !((2u64 * 1024 * 1024) - 1);
    crate::serial::write_str(b"  vmm: map_end=0x");
    crate::serial_put_u64_hex(map_end);
    crate::serial::write_str(b"\n");

    crate::serial::write_str(b"  vmm: direct map loop start\n");
    let mut phys = 0u64;
    {
        let mut iter = 0u64;
        while phys < map_end {
            kpt.map_huge_2mb(
                DIRECT_MAP_BASE + phys,
                phys,
                flags::WRITABLE | flags::NO_EXEC,
            );
            phys += 2 * 1024 * 1024;
            iter += 1;
            if iter % 64 == 0 {
                crate::serial::write_str(b"  vmm: direct map progress ");
                crate::serial_put_u64(iter);
                crate::serial::write_str(b"\n");
            }
        }
    }
    crate::serial::write_str(b"  vmm: direct map done\n");

    // 恒等映射（确保 CR3 切换后指令/栈访问不中断）
    crate::serial::write_str(b"  vmm: identity map start\n");
    {
        let mut iphys = 0u64;
        let mut iter = 0u64;
        while iphys < map_end {
            kpt.map_huge_2mb(iphys, iphys, flags::WRITABLE);
            iphys += 2 * 1024 * 1024;
            iter += 1;
            if iter % 64 == 0 {
                crate::serial::write_str(b"  vmm: identity map progress ");
                crate::serial_put_u64(iter);
                crate::serial::write_str(b"\n");
            }
        }
    }
    crate::serial::write_str(b"  vmm: identity map done\n");

    // 冗余映射前 2MB
    kpt.map_huge_2mb(0x000000, 0x000000, flags::WRITABLE);

    // 将 PML4 指针切换到直接映射地址
    let pml4_pa = kpt.pml4_paddr & !0xFFF;
    kpt.pml4 = (DIRECT_MAP_BASE + pml4_pa) as *mut Pml4;

    crate::serial::write_str(b"  vmm: activating new page table...\n");
    kpt.activate();

    crate::serial::write_str(b"  vmm: activated, direct map ready\n");

    // 注册内核 VMA
    init_kernel_vmas();
    crate::serial::write_str(b"  vmm: kernel vmas registered\n");
}

/// 处理缺页异常（Demand Paging + COW + vmalloc + 内核栈扩展）
///
/// error_code 位域：
/// - bit 0 (P): 0=非 Present 页, 1=权限冲突
/// - bit 1 (W): 0=读访问, 1=写访问
/// - bit 2 (U): 0=内核态, 1=用户态
/// - bit 3 (RSVD): 保留位违规
/// - bit 4 (ID): 指令预取
///
/// 返回 true 表示已处理，false 表示无法处理（应 panic）
pub fn handle_page_fault(fault_addr: u64, error_code: u64) -> bool {
    use crate::mm::pmm;
    use crate::serial;

    let present = error_code & 1 != 0;
    let is_write = error_code & 2 != 0;
    let is_user = error_code & 4 != 0;
    let _is_exec = error_code & 16 != 0;

    let aligned = fault_addr & !0xFFF;

    // COW: Present 页 + 写访问 → 复制后映射为可写
    if present && is_write {
        if is_user {
            let pt = kernel_pt();
            let entry = match pt.get_entry(fault_addr) {
                Some(e) => e,
                None => return false,
            };
            let old_paddr = entry & ADDR_MASK;
            if old_paddr == 0 {
                return false;
            }

            if let Some(new_paddr) = pmm::alloc_zeroed() {
                let src = (DIRECT_MAP_BASE + old_paddr) as *const u8;
                let dst = (DIRECT_MAP_BASE + new_paddr) as *mut u8;
                unsafe {
                    core::ptr::copy_nonoverlapping(src, dst, 4096);
                }
                let page_flags =
                    flags::PRESENT | flags::WRITABLE | flags::USER | flags::NO_EXEC;
                pt.map_page(aligned, new_paddr, page_flags);
                return true;
            }
        }
        return false;
    }

    // 用户态 Demand Paging
    if is_user && !present {
        let vma_flags_val = {
            let vma_list = KERNEL_VMA_LIST.lock();
            vma_list.find(fault_addr).map(|v| v.flags)
        };
        if let Some(flags_val) = vma_flags_val {
            if let Some(paddr) = pmm::alloc_zeroed() {
                let mut pt_flags = flags::PRESENT | flags::USER;
                if flags_val & vma_flags::WRITE != 0 {
                    pt_flags |= flags::WRITABLE;
                }
                if flags_val & vma_flags::EXEC == 0 {
                    pt_flags |= flags::NO_EXEC;
                }
                pt_flags &= !flags::GLOBAL;
                let pt = kernel_pt();
                pt.map_page(aligned, paddr, pt_flags);
                serial::write_str(b"  pf: demand page at 0x");
                crate::serial_put_u64_hex(aligned);
                serial::write_str(b"\n");
                return true;
            }
        }
        return false;
    }

    // 内核栈扩展（向下增长）
    if !present && !is_user {
        if aligned + 4096 >= KERNEL_BASE - (32u64 << 20) && aligned < KERNEL_BASE {
            if let Some(paddr) = pmm::alloc_zeroed() {
                let pt = kernel_pt();
                pt.map_page(
                    aligned,
                    paddr,
                    flags::PRESENT | flags::WRITABLE | flags::NO_EXEC,
                );
                serial::write_str(b"  pf: kernel stack page allocated at 0x");
                crate::serial_put_u64_hex(aligned);
                serial::write_str(b"\n");
                return true;
            }
        }
    }

    // vmalloc 缺页
    if !present && !is_user && fault_addr >= VMALLOC_BASE && fault_addr < FIXMAP_BASE
    {
        if let Some(paddr) = pmm::alloc_zeroed() {
            let pt = kernel_pt();
            pt.map_page(
                aligned,
                paddr,
                flags::PRESENT | flags::WRITABLE | flags::NO_EXEC,
            );
            serial::write_str(b"  pf: vmalloc page allocated at 0x");
            crate::serial_put_u64_hex(aligned);
            serial::write_str(b"\n");
            return true;
        }
        return false;
    }

    // 无法处理的缺页
    serial::write_str(b"  pf: unhandled page fault at 0x");
    crate::serial_put_u64_hex(fault_addr);
    serial::write_str(b" err=0x");
    crate::serial_put_u64_hex(error_code);
    serial::write_str(b"\n");
    false
}

/// 将物理地址转换为内核直接映射虚拟地址
#[inline]
pub fn phys_to_virt(pa: u64) -> u64 {
    DIRECT_MAP_BASE + pa
}

/// 将 MMIO 物理区域映射为可访问的内核虚拟地址
///
/// 低于 4GB 的 MMIO 通常已由直接映射区覆盖；此函数确保页表项存在并设置 NO_CACHE。
pub fn map_mmio(phys: u64, size: u64) -> u64 {
    let virt = phys_to_virt(phys);
    let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
    let kpt = kernel_pt();
    for i in 0..pages {
        let pa = phys + i * PAGE_SIZE;
        let va = virt + i * PAGE_SIZE;
        if kpt.get_entry(va).is_none() {
            kpt.map_page(
                va,
                pa,
                flags::WRITABLE | flags::NO_CACHE | flags::NO_EXEC,
            );
        }
    }
    virt
}

/// 用户态 mmap 区域起始（按需向上增长）
const USER_MMAP_BASE: u64 = 0x0000_0001_0000_0000;
const USER_MMAP_LIMIT: u64 = 0x0000_7FFF_0000_0000;

static NEXT_MMAP_ADDR: crate::sync::SpinLock<u64> =
    crate::sync::SpinLock::new(USER_MMAP_BASE);

/// 为用户态映射匿名内存区域
///
/// `len` 按页对齐；`prot` 使用 `vma_flags` 位。成功返回映射起始虚拟地址。
pub fn mmap_user(len: u64, prot: u64) -> Option<u64> {
    if len == 0 {
        return None;
    }
    let aligned_len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let mut next = NEXT_MMAP_ADDR.lock();
    let start = (*next + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let end = start.saturating_add(aligned_len);
    if end > USER_MMAP_LIMIT {
        return None;
    }
    *next = end;
    drop(next);

    let mut vma_list = KERNEL_VMA_LIST.lock();
    if !vma_list.add(start, end, prot | vma_flags::READ) {
        return None;
    }
    drop(vma_list);

    let pages = aligned_len / PAGE_SIZE;
    let kpt = kernel_pt();
    let mut pt_flags = flags::PRESENT | flags::USER;
    if prot & vma_flags::WRITE != 0 {
        pt_flags |= flags::WRITABLE;
    }
    if prot & vma_flags::EXEC == 0 {
        pt_flags |= flags::NO_EXEC;
    }
    for i in 0..pages {
        let paddr = pmm::alloc_zeroed()?;
        kpt.map_page(start + i * PAGE_SIZE, paddr, pt_flags);
    }
    Some(start)
}
