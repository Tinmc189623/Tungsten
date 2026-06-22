// block/mbr.rs — MBR 分区表解析
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

#[repr(C, packed)]
pub struct MbrPartition {
    pub boot: u8,
    pub chs_start: [u8; 3],
    pub typ: u8,
    pub chs_end: [u8; 3],
    pub lba_start: u32,
    pub sectors: u32,
}

const MBR_MAGIC: u16 = 0xAA55;

/// 解析 MBR 扇区，返回有效分区列表
pub fn parse(sector: &[u8]) -> [(u8, u32, u32); 4] {
    let mut out = [(0u8, 0u32, 0u32); 4];
    if sector.len() < 512 {
        return out;
    }
    let magic = u16::from_le_bytes([sector[510], sector[511]]);
    if magic != MBR_MAGIC {
        return out;
    }
    for i in 0..4 {
        let off = 446 + i * 16;
        let part = unsafe { core::ptr::read_unaligned(sector.as_ptr().add(off) as *const MbrPartition) };
        if part.typ != 0 {
            out[i] = (part.typ, part.lba_start, part.sectors);
        }
    }
    out
}

pub fn init() {}
pub fn probe() {}
