// block/common.rs — 块设备公共 MMIO 与缓冲区辅助
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::mm::pmm;
use crate::mm::vmm;

/// MMIO 读 32 位
#[inline]
pub unsafe fn mmio_read32(addr: u64) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

/// MMIO 写 32 位
#[inline]
pub unsafe fn mmio_write32(addr: u64, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

/// MMIO 读 64 位
#[inline]
pub unsafe fn mmio_read64(addr: u64) -> u64 {
    core::ptr::read_volatile(addr as *const u64)
}

/// MMIO 写 64 位
#[inline]
pub unsafe fn mmio_write64(addr: u64, val: u64) {
    core::ptr::write_volatile(addr as *mut u64, val);
}

/// 将 PCI BAR 解码为 MMIO 物理地址并映射
pub fn map_pci_bar(bar: u32) -> u64 {
    if bar == 0 {
        return 0;
    }
    let phys = if bar & 1 != 0 {
        return 0;
    } else {
        (bar & 0xFFFF_FFF0) as u64
    };
    vmm::map_mmio(phys, 0x10000)
}

/// 分配 DMA 对齐物理页并返回直接映射虚拟地址
pub fn alloc_dma_buffer(size: usize) -> Option<u64> {
    let pages = (size + 4095) / 4096;
    let mut base = 0u64;
    for _ in 0..pages {
        let paddr = pmm::alloc_zeroed()?;
        if base == 0 {
            base = paddr;
        }
    }
    Some(vmm::phys_to_virt(base))
}

/// 复制设备名到固定缓冲区
pub fn copy_name(dst: &mut [u8; 32], src: &[u8]) {
    let n = src.len().min(31);
    dst[..n].copy_from_slice(&src[..n]);
    dst[n] = 0;
}
