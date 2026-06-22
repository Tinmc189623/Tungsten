// uxiloader.rs — .uxi 自研程序格式加载器
// 格式定义与 src/scripts/pack_uxi.rb 完全一致
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use crate::mm::{pmm, vmm};

/* ── .uxi 魔数 ── */

/// "UXI\0" as u32 LE (matches pack_uxi.rb)
pub const UXI_MAGIC: u32 = 0x00495855;

/* ── 标志 ── */

pub const LOAD_SEGMENTS: u16 = 0x0001;

/* ── 入口类型 ── */

pub const ENTRY_NORMAL: u8 = 0;

/* ══════════════════════════════════════════════
   .uxi 文件头 (pack_uxi.rb: 16 字节)
   ══════════════════════════════════════════════ */

/// .uxi 固定头，文件前 16 字节
#[repr(C, packed)]
pub struct UxiHeader {
    pub magic: u32,             // UXI_MAGIC
    pub version: u8,            // 格式版本
    pub type_: u8,              // 程序类型
    pub flags: u16,             // 全局标志
    pub header_size: u32,       // 头总大小 (含所有表，不含填充)
    pub entry_offset: u32,      // 入口点在文件中的偏移 (非虚拟地址)
}

/* ══════════════════════════════════════════════
   入口表 (pack_uxi.rb: 每项 12 字节)
   ══════════════════════════════════════════════ */

/// 入口表项
#[repr(C, packed)]
pub struct UxiEntry {
    pub ring: u8,               // Ring 级别
    pub entry_type: u8,         // 入口类型
    pub reserved: u16,
    pub offset: u64,            // 入口偏移
}

impl UxiEntry {
    /// 是否为终止项
    pub fn is_terminator(&self) -> bool {
        self.entry_type == 0xFF
    }
}

/* ══════════════════════════════════════════════
   段表 (pack_uxi.rb: 段计数 2 字节 + 每项 36 字节)
   ══════════════════════════════════════════════ */

/// 段描述符
#[repr(C, packed)]
pub struct UxiSegment {
    pub file_offset: u64,       // 段数据在文件中的偏移
    pub vaddr: u64,             // 加载虚拟地址
    pub filesz: u64,            // 段在文件中的大小
    pub memsz: u64,             // 段在内存中的大小
    pub bss: u8,                // 是否 BSS (无文件数据)
    pub alignment: u8,          // 对齐指数 (2^n)
    pub flags: u16,             // 段权限标志
}

/* ══════════════════════════════════════════════
   .uxi 加载器
   ══════════════════════════════════════════════ */

/// 直接从 Limine 模块内存加载 .uxi — 不复制数据，直接用 HHDM 地址执行
/// 绕过 copy_nonoverlapping 在各种优化级别下的已知问题
pub unsafe fn load_uxi_direct(data: &[u8]) -> Option<UxiProgram> {
    if !validate_header(data) { return None; }
    let hdr = &*(data.as_ptr() as *const UxiHeader);
    let hdr_size = hdr.header_size as usize;
    let data_offset = (hdr_size + 3) & !3; // 4 字节对齐
    if data_offset >= data.len() { return None; }

    // 入口指向 HHDM 中已映射的代码
    let entry_va = data.as_ptr().add(data_offset) as u64;

    Some(UxiProgram {
        entry: entry_va,
        base: entry_va,
        total_size: data.len() as u64 - data_offset as u64,
        stack_size: 16384,
    })
}

/// 从 PowerBoot 部署的物理基址加载 .uxi 程序
/// `base` 为物理地址，`max_size` 为最大探测字节数
///
/// # Safety
/// `base` 必须指向有效的 .uxi 数据（由 PowerBoot 部署）
pub unsafe fn load_deployed(base: u64, max_size: usize) -> Option<UxiProgram> {
    // 先校验魔数避免访问无效内存
    let magic = *(base as *const u32);
    if magic != UXI_MAGIC {
        return None;
    }
    let data = core::slice::from_raw_parts(base as *const u8, max_size);
    load_uxi(data)
}

/// 加载后的程序信息
pub struct UxiProgram {
    pub entry: u64,             // 虚拟入口地址
    pub base: u64,              // 加载基址
    pub total_size: u64,        // 占用虚拟地址范围
    pub stack_size: u64,        // 建议栈大小
}

/// 校验文件头
fn validate_header(data: &[u8]) -> bool {
    if data.len() < core::mem::size_of::<UxiHeader>() {
        return false;
    }
    unsafe {
        let hdr = &*(data.as_ptr() as *const UxiHeader);
        hdr.magic == UXI_MAGIC
            && (hdr.version == 1 || hdr.version == 2)
            && hdr.flags & LOAD_SEGMENTS != 0
            && hdr.header_size as usize <= data.len()
    }
}

/// 查找主入口 (从入口表解析)
fn find_entry(data: &[u8]) -> Option<u64> {
    unsafe {
        let hdr = &*(data.as_ptr() as *const UxiHeader);
        let mut pos = core::mem::size_of::<UxiHeader>(); // 跳过固定头
        let hdr_size = hdr.header_size as usize;

        while pos + core::mem::size_of::<UxiEntry>() <= hdr_size && pos < data.len() {
            let entry = &*(data.as_ptr().add(pos) as *const UxiEntry);
            if entry.is_terminator() {
                break;
            }
            if entry.entry_type == ENTRY_NORMAL {
                return Some(entry.offset);
            }
            pos += core::mem::size_of::<UxiEntry>();
        }
        None
    }
}

/// 读取段表
fn read_segments(data: &[u8]) -> Option<&'static [UxiSegment]> {
    unsafe {
        // 计算入口表大小 (遍历到终止项)
        let mut entry_end = core::mem::size_of::<UxiHeader>();
        loop {
            if entry_end + core::mem::size_of::<UxiEntry>() > data.len() {
                return None;
            }
            let entry = &*(data.as_ptr().add(entry_end) as *const UxiEntry);
            entry_end += core::mem::size_of::<UxiEntry>();
            if entry.is_terminator() {
                break;
            }
        }

        // 段计数: 紧跟入口表之后，2 字节
        if entry_end + 2 > data.len() {
            return None;
        }
        let seg_count = *(data.as_ptr().add(entry_end) as *const u16) as usize;
        let seg_table_start = entry_end + 2;

        if seg_table_start + seg_count * core::mem::size_of::<UxiSegment>() > data.len() {
            return None;
        }

        Some(core::slice::from_raw_parts(
            data.as_ptr().add(seg_table_start) as *const UxiSegment,
            seg_count,
        ))
    }
}

/// 加载 .uxi 程序到内存
///
/// # Safety
/// `data` 必须指向完整的 .uxi 文件数据
pub unsafe fn load_uxi(data: &[u8]) -> Option<UxiProgram> {
    if !validate_header(data) {
        return None;
    }

    let segments = read_segments(data)?;
    let entry_file = find_entry(data)?;

    // Phase 1: 计算地址范围
    // vaddr=0 表示位置无关代码 (PIC)，加载到固定用户空间基址
    const UX_PROG_BASE: u64 = 0x40000000; // 1 GB

    let mut min_vaddr: u64 = u64::MAX;
    let mut max_vaddr: u64 = 0;
    let mut has_pic = false;

    for seg in segments.iter() {
        if seg.filesz == 0 && seg.memsz == 0 {
            continue;
        }
        let seg_vaddr = if seg.vaddr == 0 {
            has_pic = true;
            UX_PROG_BASE + seg.file_offset
        } else {
            seg.vaddr
        };
        let seg_end = seg_vaddr + seg.memsz;
        if seg_vaddr < min_vaddr {
            min_vaddr = seg_vaddr;
        }
        if seg_end > max_vaddr {
            max_vaddr = seg_end;
        }
    }

    if min_vaddr == u64::MAX {
        return None;
    }

    let base = min_vaddr & !(vmm::PAGE_SIZE - 1);
    let total_size = (max_vaddr - base + vmm::PAGE_SIZE - 1) & !(vmm::PAGE_SIZE - 1);

    // Phase 2: 分配物理页并映射
    let page_count = (total_size / vmm::PAGE_SIZE) as usize;
    let pt = vmm::kernel_pt();

    for i in 0..page_count {
        let va = base + (i as u64) * vmm::PAGE_SIZE;
        let pa = pmm::alloc_one()?;
        pt.map_page(va, pa, vmm::flags::WRITABLE);  // 需可执行! NO_EXEC 会导致跳转失败
    }

    // Phase 3: 复制段数据 (带地址验证)
    crate::serial::write_str(b"  uxi: copying ");
    crate::serial_put_u64(segments.len() as u64);
    crate::serial::write_str(b" segments\n");
    for seg in segments.iter() {
        crate::serial::write_str(b"  uxi: seg fo=0x");
        crate::serial_put_u64_hex(seg.file_offset);
        crate::serial::write_str(b" fz=");
        crate::serial_put_u64(seg.filesz);
        crate::serial::write_str(b" bss=");
        crate::serial_put_u64(seg.bss as u64);
        crate::serial::write_str(b"\n");
        if seg.filesz == 0 || seg.bss != 0 {
            continue;
        }
        // 安全验证: 文件偏移不能超出数据范围
        let file_end = seg.file_offset as u64 + seg.filesz;
        if file_end > total_size {
            crate::serial::write_str(b"  uxiloader: warning: segment file offset out of range\n");
            continue;
        }
        let effective_vaddr = if seg.vaddr == 0 {
            UX_PROG_BASE + seg.file_offset
        } else {
            seg.vaddr
        };
        if effective_vaddr > 0x8000_0000_0000 {
            crate::serial::write_str(b"  uxiloader: warning: segment vaddr out of canonical range\n");
            continue;
        }
        let src = unsafe { data.as_ptr().add(seg.file_offset as usize) };
        let dst = effective_vaddr as *mut u8;

        // 验证：写入测试字节，确认映射有效
        unsafe { *dst = 0xAB; }
        let verify = unsafe { *dst };
        crate::serial::write_str(b"  uxi: map test: wrote 0xAB, read 0x");
        crate::serial_put_u64_hex(verify as u64);
        crate::serial::write_str(b"\n");

        // 使用 u64 循环复制 (copy_nonoverlapping 在 -Oz LTO 下有优化问题)
        let count = seg.filesz as usize;
        let u64_count = count / 8;
        let src64 = src as *const u64;
        let dst64 = dst as *mut u64;
        for i in 0..u64_count {
            unsafe { *dst64.add(i) = *src64.add(i); }
        }
        // 复制剩余字节
        for i in (u64_count * 8)..count {
            unsafe { *dst.add(i) = *src.add(i); }
        }

        let after = unsafe { *dst };
        crate::serial::write_str(b"  uxi: after copy, dst[0]=0x");
        crate::serial_put_u64_hex(after as u64);
        crate::serial::write_str(b"\n");

        // BSS 清零: memsz > filesz
        if seg.memsz > seg.filesz {
            let bss_end = seg.vaddr + seg.memsz;
            if bss_end > 0x8000_0000_0000 {
                continue;
            }
            unsafe {
                core::ptr::write_bytes(
                    dst.add(seg.filesz as usize),
                    0,
                    (seg.memsz - seg.filesz) as usize,
                );
            }
        }
    }

    // 单独的 BSS 段 (带地址验证)
    for seg in segments.iter() {
        if seg.bss == 0 || seg.memsz == 0 {
            continue;
        }
        if seg.vaddr < 0x100_0000 || seg.vaddr + seg.memsz > 0x8000_0000_0000 {
            crate::serial::write_str(b"  uxiloader: warning: BSS segment address out of range\n");
            continue;
        }
        let dst = seg.vaddr as *mut u8;
        unsafe { core::ptr::write_bytes(dst, 0, seg.memsz as usize); }
    }

    // 入口: entry_file 是 .uxi 文件中代码段起始偏移 (= 第一个段的 file_offset)
    // 对于 vaddr=0 (PIC) 的段, 有效虚拟地址 = UX_PROG_BASE + file_offset
    // entry 标记为 0 表示入口位于加载后代码段开头
    let entry_vaddr = if entry_file == 0 {
        // 查找第一个代码段的有效虚拟地址作为入口
        segments.iter()
            .find(|s| s.filesz > 0 && s.bss == 0)
            .map(|s| if s.vaddr == 0 { UX_PROG_BASE + s.file_offset } else { s.vaddr })
            .unwrap_or(base)
    } else if entry_file < total_size {
        base + entry_file
    } else {
        entry_file
    };

    Some(UxiProgram {
        entry: entry_vaddr,
        base,
        total_size,
        stack_size: 16384,
    })
}

/* ── 系统调用 ── */

/// 从内存中加载 .uxi 程序
pub unsafe fn sys_exec(data: &[u8]) -> Result<UxiProgram, i32> {
    load_uxi(data).ok_or(-1)
}
