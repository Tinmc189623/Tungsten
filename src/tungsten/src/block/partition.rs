// block/partition.rs — 分区扫描与分区块设备注册
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::common;
use super::gpt;
use super::mbr;
use super::{
    block_read_sectors, block_write_sectors, device_by_index, device_count, register_device,
    BlockDevice, BlockOps,
};
use crate::sync::SpinLock;

const MAX_PARTITIONS: usize = 64;

/// 分区私有数据：父设备索引 + LBA 偏移
#[derive(Clone, Copy)]
struct PartPriv {
    parent_idx: usize,
    start_lba: u64,
}

struct PartTable {
    slots: [Option<PartPriv>; MAX_PARTITIONS],
    count: usize,
}

static PART_TABLE: SpinLock<PartTable> = SpinLock::new(PartTable {
    slots: [None; MAX_PARTITIONS],
    count: 0,
});

/// 分配分区私有数据槽
fn alloc_part_priv(parent_idx: usize, start_lba: u64) -> Option<usize> {
    let mut tbl = PART_TABLE.lock();
    for (i, slot) in tbl.slots.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(PartPriv {
                parent_idx,
                start_lba,
            });
            tbl.count += 1;
            return Some(i);
        }
    }
    None
}

/// 根据 private_data 索引获取分区私有数据
fn part_priv(idx: usize) -> Option<PartPriv> {
    PART_TABLE.lock().slots.get(idx).and_then(|s| *s)
}

/// 分区读：转发至父设备并加上 LBA 偏移
unsafe extern "C" fn part_read(
    dev: *mut BlockDevice,
    lba: u64,
    count: u32,
    buf: *mut u8,
) -> i32 {
    let d = unsafe { &*dev };
    let idx = d.priv_data as usize;
    let Some(p) = part_priv(idx) else {
        return -19;
    };
    if lba.saturating_add(count as u64) > d.total_sectors {
        return -28;
    }
    let byte_len = (count as u64).saturating_mul(512) as usize;
    let kbuf = unsafe { core::slice::from_raw_parts_mut(buf, byte_len) };
    block_read_sectors(p.parent_idx, lba + p.start_lba, count, kbuf)
}

/// 分区写
unsafe extern "C" fn part_write(
    dev: *mut BlockDevice,
    lba: u64,
    count: u32,
    buf: *const u8,
) -> i32 {
    let d = unsafe { &*dev };
    let idx = d.priv_data as usize;
    let Some(p) = part_priv(idx) else {
        return -19;
    };
    if lba.saturating_add(count as u64) > d.total_sectors {
        return -28;
    }
    let byte_len = (count as u64).saturating_mul(512) as usize;
    let kbuf = unsafe { core::slice::from_raw_parts(buf, byte_len) };
    block_write_sectors(p.parent_idx, lba + p.start_lba, count, kbuf)
}

unsafe extern "C" fn part_flush(dev: *mut BlockDevice) -> i32 {
    let d = unsafe { &*dev };
    let idx = d.priv_data as usize;
    let Some(p) = part_priv(idx) else {
        return -19;
    };
    super::block_flush(p.parent_idx)
}

unsafe extern "C" fn part_trim(_dev: *mut BlockDevice, _lba: u64, _count: u32) -> i32 {
    0
}

static PART_OPS: BlockOps = BlockOps {
    read: part_read,
    write: part_write,
    flush: part_flush,
    trim: part_trim,
};

/// 注册单个分区为独立块设备
fn register_partition(
    parent_idx: usize,
    parent_name: &[u8],
    part_no: u32,
    start: u64,
    len: u64,
    gpt_style: bool,
) {
    if len == 0 {
        return;
    }
    let priv_idx = match alloc_part_priv(parent_idx, start) {
        Some(i) => i,
        None => return,
    };
    let mut name = [0u8; 32];
    let mut tmp = [0u8; 40];
    let plen = parent_name.iter().position(|&c| c == 0).unwrap_or(parent_name.len());
    let base = &parent_name[..plen.min(16)];
    if gpt_style {
        let suffix = b"p";
        let mut pos = 0usize;
        tmp[pos..pos + base.len()].copy_from_slice(base);
        pos += base.len();
        tmp[pos..pos + suffix.len()].copy_from_slice(suffix);
        pos += suffix.len();
        let mut n = part_no;
        let mut digits = [0u8; 8];
        let mut j = 8;
        if n == 0 {
            digits[j - 1] = b'0';
            j -= 1;
        } else {
            while n > 0 {
                j -= 1;
                digits[j] = b'0' + (n % 10) as u8;
                n /= 10;
            }
        }
        let d = &digits[j..];
        tmp[pos..pos + d.len()].copy_from_slice(d);
        pos += d.len();
        common::copy_name(&mut name, &tmp[..pos]);
    } else {
        let mut pos = 0usize;
        tmp[pos..pos + base.len()].copy_from_slice(base);
        pos += base.len();
        let mut n = part_no;
        let mut digits = [0u8; 8];
        let mut j = 8;
        if n == 0 {
            digits[j - 1] = b'0';
            j -= 1;
        } else {
            while n > 0 {
                j -= 1;
                digits[j] = b'0' + (n % 10) as u8;
                n /= 10;
            }
        }
        let d = &digits[j..];
        tmp[pos..pos + d.len()].copy_from_slice(d);
        pos += d.len();
        common::copy_name(&mut name, &tmp[..pos]);
    }
    register_device(BlockDevice {
        name,
        major: 8,
        minor: part_no as u16,
        sector_size: 512,
        total_sectors: len,
        max_transfer: 128,
        flags: 1,
        ops: &PART_OPS,
        priv_data: priv_idx as *mut (),
        next: core::ptr::null_mut(),
    });
    crate::serial::write_str(b"  partition: registered ");
    let end = name.iter().position(|&c| c == 0).unwrap_or(32);
    crate::serial::write_str(&name[..end]);
    crate::serial::write_str(b"\n");
}

/// 扫描所有块设备分区表并注册分区设备
pub fn scan_all() {
    crate::serial::write_str(b"partition: scanning block devices...\n");
    let count = device_count();
    if count == 0 {
        crate::serial::write_str(b"partition: no block devices\n");
        return;
    }
    for i in 0..count {
        let mut sector = [0u8; 512];
        if block_read_sectors(i, 0, 1, &mut sector) < 0 {
            continue;
        }
        let parent_name = device_by_index(i).map(|d| d.name).unwrap_or([0; 32]);
        let mbr_parts = mbr::parse(&sector);
        let mut found = 0usize;
        for (idx, (typ, start, len)) in mbr_parts.iter().enumerate() {
            if *typ == 0 {
                continue;
            }
            found += 1;
            register_partition(i, &parent_name, (idx + 1) as u32, *start as u64, *len as u64, false);
        }
        if found == 0 {
            let mut gpt_lba = [0u8; 512];
            if block_read_sectors(i, 1, 1, &mut gpt_lba) >= 0 {
                let gpt_parts = gpt::parse(&sector, &gpt_lba);
                for (idx, (start, len)) in gpt_parts.iter().enumerate() {
                    if *start == 0 {
                        continue;
                    }
                    register_partition(
                        i,
                        &parent_name,
                        (idx + 1) as u32,
                        *start,
                        *len,
                        true,
                    );
                }
            }
        }
    }
}

pub fn init() {}
pub fn probe() {}
