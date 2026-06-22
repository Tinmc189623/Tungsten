// block/mod.rs — 块设备子系统 (AHCI/NVMe/IDE/VirtIO)
// 块设备 I/O 调度器、分区表解析 (GPT/MBR)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod ahci;
pub mod common;
pub mod gpt;
pub mod ide;
pub mod io_sched;
pub mod mbr;
pub mod nvme;
pub mod partition;
pub mod virtio_blk;

use crate::sync::SpinLock;

const MAX_BLOCK_DEVS: usize = 16;

/// 块设备描述
#[repr(C)]
#[derive(Clone, Copy)]
pub struct BlockDevice {
    pub name: [u8; 32],
    pub major: u16,
    pub minor: u16,
    pub sector_size: u16,
    pub total_sectors: u64,
    pub max_transfer: u32,
    pub flags: u32,
    pub ops: &'static BlockOps,
    pub priv_data: *mut (),
    pub next: *mut BlockDevice,
}

/// 块设备操作表
#[repr(C)]
pub struct BlockOps {
    pub read: unsafe extern "C" fn(dev: *mut BlockDevice, lba: u64, count: u32, buf: *mut u8) -> i32,
    pub write: unsafe extern "C" fn(dev: *mut BlockDevice, lba: u64, count: u32, buf: *const u8) -> i32,
    pub flush: unsafe extern "C" fn(dev: *mut BlockDevice) -> i32,
    pub trim: unsafe extern "C" fn(dev: *mut BlockDevice, lba: u64, count: u32) -> i32,
}

struct BlockSlot {
    dev: BlockDevice,
    used: bool,
}

struct BlockManager {
    slots: [BlockSlot; MAX_BLOCK_DEVS],
    count: usize,
}

unsafe impl Send for BlockManager {}

static BLOCK_MGR: SpinLock<BlockManager> = SpinLock::new(BlockManager {
    slots: [const {
        BlockSlot {
            dev: BlockDevice {
                name: [0; 32],
                major: 0,
                minor: 0,
                sector_size: 512,
                total_sectors: 0,
                max_transfer: 0,
                flags: 0,
                ops: &NULL_OPS,
                priv_data: core::ptr::null_mut(),
                next: core::ptr::null_mut(),
            },
            used: false,
        }
    }; MAX_BLOCK_DEVS],
    count: 0,
});

static NULL_OPS: BlockOps = BlockOps {
    read: null_read,
    write: null_write,
    flush: null_flush,
    trim: null_trim,
};

unsafe extern "C" fn null_read(_: *mut BlockDevice, _: u64, _: u32, _: *mut u8) -> i32 {
    -19
}
unsafe extern "C" fn null_write(_: *mut BlockDevice, _: u64, _: u32, _: *const u8) -> i32 {
    -19
}
unsafe extern "C" fn null_flush(_: *mut BlockDevice) -> i32 {
    -19
}
unsafe extern "C" fn null_trim(_: *mut BlockDevice, _: u64, _: u32) -> i32 {
    -19
}

/// 注册块设备
pub fn register_device(dev: BlockDevice) -> i32 {
    let mut mgr = BLOCK_MGR.lock();
    if mgr.count >= MAX_BLOCK_DEVS {
        return -12;
    }
    for slot in mgr.slots.iter_mut() {
        if !slot.used {
            slot.dev = dev;
            slot.used = true;
            mgr.count += 1;
            return (mgr.count - 1) as i32;
        }
    }
    -12
}

/// 已注册块设备数量
pub fn device_count() -> usize {
    BLOCK_MGR.lock().count
}

/// 按索引获取块设备引用
pub fn device_by_index(idx: usize) -> Option<BlockDevice> {
    let mgr = BLOCK_MGR.lock();
    let mut n = 0usize;
    for slot in mgr.slots.iter() {
        if slot.used {
            if n == idx {
                return Some(slot.dev);
            }
            n += 1;
        }
    }
    None
}

/// 读取扇区到缓冲区
pub fn block_read_sectors(dev_idx: usize, lba: u64, count: u32, buf: &mut [u8]) -> i32 {
    let mut mgr = BLOCK_MGR.lock();
    let mut n = 0usize;
    for slot in mgr.slots.iter_mut() {
        if !slot.used {
            continue;
        }
        if n == dev_idx {
            let needed = (count as u64).saturating_mul(slot.dev.sector_size as u64) as usize;
            if buf.len() < needed {
                return -22;
            }
            return unsafe {
                (slot.dev.ops.read)(
                    &mut slot.dev as *mut BlockDevice,
                    lba,
                    count,
                    buf.as_mut_ptr(),
                )
            };
        }
        n += 1;
    }
    -19
}

/// 写入扇区
pub fn block_write_sectors(dev_idx: usize, lba: u64, count: u32, buf: &[u8]) -> i32 {
    let mut mgr = BLOCK_MGR.lock();
    let mut n = 0usize;
    for slot in mgr.slots.iter_mut() {
        if !slot.used {
            continue;
        }
        if n == dev_idx {
            return unsafe {
                (slot.dev.ops.write)(
                    &mut slot.dev as *mut BlockDevice,
                    lba,
                    count,
                    buf.as_ptr(),
                )
            };
        }
        n += 1;
    }
    -19
}

/// 刷新块设备缓存
pub fn block_flush(dev_idx: usize) -> i32 {
    let mut mgr = BLOCK_MGR.lock();
    let mut n = 0usize;
    for slot in mgr.slots.iter_mut() {
        if !slot.used {
            continue;
        }
        if n == dev_idx {
            return unsafe { (slot.dev.ops.flush)(&mut slot.dev as *mut BlockDevice) };
        }
        n += 1;
    }
    -19
}

/// 列出块设备到缓冲区
pub fn list_devices(buf: &mut [u8]) -> usize {
    let mgr = BLOCK_MGR.lock();
    let mut pos = 0usize;
    for (i, slot) in mgr.slots.iter().enumerate() {
        if !slot.used {
            continue;
        }
        let end = slot.dev.name.iter().position(|&c| c == 0).unwrap_or(32);
        let line = &slot.dev.name[..end];
        if pos + line.len() + 24 > buf.len() {
            break;
        }
        buf[pos..pos + line.len()].copy_from_slice(line);
        pos += line.len();
        let suffix = b" sectors=";
        buf[pos..pos + suffix.len()].copy_from_slice(suffix);
        pos += suffix.len();
        let mut num = [0u8; 20];
        let mut v = slot.dev.total_sectors;
        let mut j = 20;
        if v == 0 {
            num[j - 1] = b'0';
            j -= 1;
        } else {
            while v > 0 {
                j -= 1;
                num[j] = b'0' + (v % 10) as u8;
                v /= 10;
            }
        }
        let digits = &num[j..];
        buf[pos..pos + digits.len()].copy_from_slice(digits);
        pos += digits.len();
        buf[pos] = b'\n';
        pos += 1;
        let _ = i;
    }
    pos
}

/// 初始化块设备子系统
pub fn init() {
    io_sched::init();
    virtio_blk::probe();
    ahci::probe();
    nvme::probe();
    ide::probe();
    partition::scan_all();
    crate::serial::write_str(b"block: subsystem ready (");
    crate::serial_put_u64(device_count() as u64);
    crate::serial::write_str(b" devices)\n");
}
