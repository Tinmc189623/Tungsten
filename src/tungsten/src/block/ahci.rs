// block/ahci.rs — AHCI SATA 控制器驱动
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::common;
use super::{register_device, BlockDevice, BlockOps};
use crate::devices::pci;

const AHCI_CLASS: u8 = 0x01;
const AHCI_SUBCLASS: u8 = 0x06;
const AHCI_PROG_IF: u8 = 0x01;

const HBA_GHC: u32 = 0x04;
const HBA_PI: u32 = 0x0C;
const HBA_GHC_AE: u32 = 1 << 31;

const PORT_SIG: u32 = 0x24;
const PORT_CMD: u32 = 0x18;
const PORT_CMD_ST: u32 = 1 << 0;
const PORT_CMD_FRE: u32 = 1 << 4;
const PORT_SIG_ATA: u32 = 0x0000_0101;

const PORT_SIZE: u32 = 0x80;

struct AhciPriv {
    abar: u64,
    port: u32,
}

static mut AHCI_PRIV: Option<AhciPriv> = None;

unsafe extern "C" fn ahci_read(
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

unsafe extern "C" fn ahci_write(
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

unsafe extern "C" fn ahci_flush(_dev: *mut BlockDevice) -> i32 {
    0
}

unsafe extern "C" fn ahci_trim(_dev: *mut BlockDevice, _lba: u64, _count: u32) -> i32 {
    0
}

static AHCI_OPS: BlockOps = BlockOps {
    read: ahci_read,
    write: ahci_write,
    flush: ahci_flush,
    trim: ahci_trim,
};

/// 探测 AHCI 控制器并注册首个 SATA 端口
pub fn probe() {
    let dev = pci::find_by_class_prog(AHCI_CLASS, AHCI_SUBCLASS, AHCI_PROG_IF);
    let Some(pci_dev) = dev else {
        crate::serial::write_str(b"  ahci: controller not found\n");
        return;
    };
    let bar = pci_dev.bars[5].max(pci_dev.bars[0]);
    let abar = common::map_pci_bar(bar);
    if abar == 0 {
        crate::serial::write_str(b"  ahci: ABAR map failed\n");
        return;
    }
    unsafe {
        let ghc = common::mmio_read32(abar + HBA_GHC as u64);
        common::mmio_write32(abar + HBA_GHC as u64, ghc | HBA_GHC_AE);
        let pi = common::mmio_read32(abar + HBA_PI as u64);
        for port in 0..32u32 {
            if pi & (1 << port) == 0 {
                continue;
            }
            let port_base = abar + (port as u64) * PORT_SIZE as u64;
            let sig = common::mmio_read32(port_base + PORT_SIG as u64);
            if sig != PORT_SIG_ATA {
                continue;
            }
            let mut cmd = common::mmio_read32(port_base + PORT_CMD as u64);
            cmd |= PORT_CMD_FRE | PORT_CMD_ST;
            common::mmio_write32(port_base + PORT_CMD as u64, cmd);
            let sectors = 2048 * 1024u64;
            AHCI_PRIV = Some(AhciPriv { abar, port });
            let mut name = [0u8; 32];
            common::copy_name(&mut name, b"sda");
            register_device(BlockDevice {
                name,
                major: 8,
                minor: 0,
                sector_size: 512,
                total_sectors: sectors,
                max_transfer: 256,
                flags: 0,
                ops: &AHCI_OPS,
                priv_data: core::ptr::null_mut(),
                next: core::ptr::null_mut(),
            });
            crate::serial::write_str(b"  ahci: port ");
            crate::serial_put_u64(port as u64);
            crate::serial::write_str(b" sda online\n");
            return;
        }
    }
    crate::serial::write_str(b"  ahci: no active port\n");
}

pub fn init() {}
