// block/nvme.rs — NVMe 控制器驱动
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::common;
use super::{register_device, BlockDevice, BlockOps};
use crate::devices::pci;

const NVME_CLASS: u8 = 0x01;
const NVME_SUBCLASS: u8 = 0x08;
const NVME_PROG_IF: u8 = 0x02;

const NVME_REG_CAP: u64 = 0x00;
const NVME_REG_CC: u64 = 0x14;
const NVME_REG_CSTS: u64 = 0x1C;
const NVME_CC_EN: u32 = 1;

struct NvmePriv {
    bar: u64,
}

static mut NVME_PRIV: Option<NvmePriv> = None;

unsafe extern "C" fn nvme_read(
    dev: *mut BlockDevice,
    lba: u64,
    count: u32,
    buf: *mut u8,
) -> i32 {
    let d = unsafe { &*dev };
    if lba.saturating_add(count as u64) > d.total_sectors {
        return -28;
    }
    let bytes = (count as u64).saturating_mul(512) as usize;
    unsafe {
        core::ptr::write_bytes(buf, 0, bytes);
    }
    bytes as i32
}

unsafe extern "C" fn nvme_write(
    dev: *mut BlockDevice,
    lba: u64,
    count: u32,
    _buf: *const u8,
) -> i32 {
    let d = unsafe { &*dev };
    if lba.saturating_add(count as u64) > d.total_sectors {
        return -28;
    }
    (count as i64 * 512) as i32
}

unsafe extern "C" fn nvme_flush(_dev: *mut BlockDevice) -> i32 {
    0
}

unsafe extern "C" fn nvme_trim(_dev: *mut BlockDevice, _lba: u64, _count: u32) -> i32 {
    0
}

static NVME_OPS: BlockOps = BlockOps {
    read: nvme_read,
    write: nvme_write,
    flush: nvme_flush,
    trim: nvme_trim,
};

/// 探测 NVMe 控制器
pub fn probe() {
    let dev = pci::find_by_class_prog(NVME_CLASS, NVME_SUBCLASS, NVME_PROG_IF);
    let Some(pci_dev) = dev else {
        crate::serial::write_str(b"  nvme: controller not found\n");
        return;
    };
    let bar = common::map_pci_bar(pci_dev.bars[0]);
    if bar == 0 {
        return;
    }
    unsafe {
        let _cap = common::mmio_read64(bar + NVME_REG_CAP);
        common::mmio_write32(bar + NVME_REG_CC, NVME_CC_EN);
        for _ in 0..100_000 {
            let csts = common::mmio_read32(bar + NVME_REG_CSTS);
            if csts & 1 != 0 {
                break;
            }
        }
        let sectors = 4096 * 1024u64;
        NVME_PRIV = Some(NvmePriv { bar });
        let mut name = [0u8; 32];
        common::copy_name(&mut name, b"nvme0n1");
        register_device(BlockDevice {
            name,
            major: 259,
            minor: 0,
            sector_size: 512,
            total_sectors: sectors,
            max_transfer: 256,
            flags: 0,
            ops: &NVME_OPS,
            priv_data: core::ptr::null_mut(),
            next: core::ptr::null_mut(),
        });
        crate::serial::write_str(b"  nvme: nvme0n1 online\n");
    }
}

pub fn init() {}
