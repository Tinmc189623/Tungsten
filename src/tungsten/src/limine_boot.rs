// limine_boot.rs — Limine 引导协议适配层，从 Limine 响应构建 BootInfo
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use limine::request::*;

/* ── Limine 请求（放入 .limine_reqs 段） ── */

/// 协议基础版本支持
#[used]
#[unsafe(link_section = ".limine_reqs")]
static BASE_REVISION: limine::BaseRevision = limine::BaseRevision::new();

/// 帧缓冲请求
#[used]
#[unsafe(link_section = ".limine_reqs")]
static FRAMEBUFFER: FramebufferRequest = FramebufferRequest::new();

/// 内存映射请求
#[used]
#[unsafe(link_section = ".limine_reqs")]
static MEMMAP: MemmapRequest = MemmapRequest::new();

/// RSDP 请求
#[used]
#[unsafe(link_section = ".limine_reqs")]
static RSDP: RsdpRequest = RsdpRequest::new();

/// HHDM 请求
#[used]
#[unsafe(link_section = ".limine_reqs")]
static HHDM: HhdmRequest = HhdmRequest::new();

/// 可执行地址请求
#[used]
#[unsafe(link_section = ".limine_reqs")]
static EXEC_ADDR: ExecutableAddressRequest = ExecutableAddressRequest::new();

/// 模块请求 — 用于获取 Limine 额外加载的文件（如 OS 层 .uxi）
#[used]
#[unsafe(link_section = ".limine_reqs")]
static MODULES: ModulesRequest = ModulesRequest::new();

/* ── BootInfo 结构体 ── */

/// 与 PMM/VMM/console 兼容的引导信息结构体
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BootInfo {
    pub fb_addr: u64,
    pub fb_width: u64,
    pub fb_height: u64,
    pub fb_pitch: u64,
    pub fb_bpp: u32,
    pub mmap_entries: u64,
    pub mmap_addr: u64,
    pub rsdp_addr: u64,
    pub hhdm_offset: u64,
    pub kernel_phys_base: u64,  // 内核物理加载基址（供 HHDM 换算）
}

impl BootInfo {
    /// 返回内存映射条目切片（供 PMM/VMM 使用）
    pub fn memory_map(&self) -> &[MemoryMapEntry] {
        unsafe {
            core::slice::from_raw_parts(
                self.mmap_addr as *const MemoryMapEntry,
                self.mmap_entries as usize,
            )
        }
    }
}

/* ── 内存映射条目 ── */

/// 内存映射条目（与 PMM 兼容）
#[derive(Clone, Copy)]
#[repr(C)]
pub struct MemoryMapEntry {
    pub type_: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

impl MemoryMapEntry {
    /// 从 Limine 内存映射条目转换
    fn from_limine(entry: &limine::memmap::Entry) -> Self {
        let mapped_type = match entry.type_ {
            limine::memmap::MEMMAP_USABLE => 7,
            limine::memmap::MEMMAP_BOOTLOADER_RECLAIMABLE => 4,
            limine::memmap::MEMMAP_EXECUTABLE_AND_MODULES => 2,
            limine::memmap::MEMMAP_ACPI_RECLAIMABLE => 3,
            limine::memmap::MEMMAP_ACPI_NVS => 4,
            limine::memmap::MEMMAP_FRAMEBUFFER => 7,
            _ => 0,
        };
        MemoryMapEntry {
            type_: mapped_type,
            physical_start: entry.base,
            virtual_start: 0,
            number_of_pages: entry.length / 4096,
            attribute: 0,
        }
    }
}

/* ── 转换缓冲区 ── */

const MAX_MMAP_ENTRIES: usize = 256;

static mut MMAP_BUFFER: [MemoryMapEntry; MAX_MMAP_ENTRIES] = unsafe {
    #[repr(C)]
    union Zeroed {
        entries: [MemoryMapEntry; MAX_MMAP_ENTRIES],
        bytes: [u8; MAX_MMAP_ENTRIES * core::mem::size_of::<MemoryMapEntry>()],
    }
    Zeroed {
        bytes: [0u8; MAX_MMAP_ENTRIES * core::mem::size_of::<MemoryMapEntry>()],
    }
    .entries
};

/* ── 模块查找 ── */

/// 从 Limine 模块列表中查找指定后缀的模块数据
fn find_module(suffix: &str) -> Option<&'static [u8]> {
    let response = MODULES.response()?;
    for module in response.modules() {
        let path = module.path();
        if path.ends_with(suffix) {
            return Some(module.data());
        }
    }
    None
}

/// 检测是否是安装程序引导（模块路径含 INSTALLER.UXI）
pub fn is_installer_boot() -> bool {
    find_module("INSTALLER.UXI").is_some()
}

/// 从 Limine 模块列表中查找 OS 层 .uxi 数据
/// 优先加载 INSTALLER.UXI（安装模式），其次 TUNGSTENOS.UXI（正常模式）
pub fn get_os_module() -> Option<&'static [u8]> {
    if let Some(data) = find_module("INSTALLER.UXI") {
        return Some(data);
    }
    find_module("TUNGSTENOS.UXI")
}

/* ── 初始化 ── */

/// 初始化 BootInfo（从 Limine 响应提取数据）
/// 必须在启动早期调用，确保 boot 信息可用
pub fn build_boot_info() -> BootInfo {
    assert!(BASE_REVISION.is_supported());

    let fb = FRAMEBUFFER.response().expect("no framebuffer response");
    let fb_info = fb.framebuffers()[0];
    let fb_addr = fb_info.address() as u64;

    let mm = MEMMAP.response().expect("no memmap response");
    let entries = mm.entries();
    let count = entries.len().min(MAX_MMAP_ENTRIES);
    unsafe {
        for i in 0..count {
            MMAP_BUFFER[i] = MemoryMapEntry::from_limine(entries[i]);
        }
    }

    let rsdp = RSDP.response().expect("no rsdp response");
    let rsdp_addr = rsdp.address as u64;

    let hhdm = HHDM.response().expect("no hhdm response");

    // 内核物理加载基址（供 PMM 通过 HHDM 换算物理地址）
    // Limine ExecutableAddressResponse: physical_base 即内核在物理内存中的起始地址
    let exec_addr = EXEC_ADDR.response().expect("no exec addr response");
    let kernel_phys_base = exec_addr.physical_base as u64;

    BootInfo {
        fb_addr,
        fb_width: fb_info.width,
        fb_height: fb_info.height,
        fb_pitch: fb_info.pitch,
        fb_bpp: fb_info.bpp as u32,
        mmap_entries: count as u64,
        mmap_addr: &raw const MMAP_BUFFER as *const _ as u64,
        rsdp_addr,
        hhdm_offset: hhdm.offset,
        kernel_phys_base,
    }
}

static mut CACHED_BOOT_INFO: BootInfo = BootInfo {
    fb_addr: 0,
    fb_width: 0,
    fb_height: 0,
    fb_pitch: 0,
    fb_bpp: 0,
    mmap_entries: 0,
    mmap_addr: 0,
    rsdp_addr: 0,
    hhdm_offset: 0,
    kernel_phys_base: 0,
};
static mut CACHED_BOOT_VALID: bool = false;

/// 缓存引导信息供 DRM 等后期子系统读取
pub fn cache_boot_info(info: &BootInfo) {
    unsafe {
        CACHED_BOOT_INFO = *info;
        CACHED_BOOT_VALID = true;
    }
}

/// 获取已缓存的引导信息副本
pub fn cached_boot_info() -> Option<BootInfo> {
    unsafe {
        if CACHED_BOOT_VALID {
            Some(CACHED_BOOT_INFO)
        } else {
            None
        }
    }
}
