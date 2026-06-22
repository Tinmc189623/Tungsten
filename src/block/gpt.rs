// block/gpt.rs — GPT 分区表解析
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

#[repr(C, packed)]
pub struct GptHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
    pub current_lba: u64,
    pub backup_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: [u8; 16],
    pub partition_lba: u64,
    pub num_entries: u32,
    pub entry_size: u32,
    pub entries_crc32: u32,
}

#[repr(C, packed)]
pub struct GptEntry {
    pub type_guid: [u8; 16],
    pub part_guid: [u8; 16],
    pub first_lba: u64,
    pub last_lba: u64,
    pub attrs: u64,
    pub name: [u16; 36],
}

const GPT_SIGNATURE: u64 = 0x5452415020494645; // "EFI PART"

/// 解析 GPT 头与分区项
pub fn parse(sector0: &[u8], sector1: &[u8]) -> [(u64, u64); 128] {
    let mut parts = [(0u64, 0u64); 128];
    if sector1.len() < core::mem::size_of::<GptHeader>() {
        return parts;
    }
    let hdr = unsafe {
        core::ptr::read_unaligned(sector1.as_ptr() as *const GptHeader)
    };
    if hdr.signature != GPT_SIGNATURE {
        return parts;
    }
    let count = hdr.num_entries.min(128) as usize;
    let entry_size = hdr.entry_size.max(core::mem::size_of::<GptEntry>() as u32) as usize;
    for i in 0..count {
        let off = i * entry_size;
        if off + core::mem::size_of::<GptEntry>() > sector0.len() {
            break;
        }
        let ent = unsafe {
            core::ptr::read_unaligned(sector0.as_ptr().add(off) as *const GptEntry)
        };
        if ent.first_lba != 0 {
            parts[i] = (ent.first_lba, ent.last_lba - ent.first_lba + 1);
        }
    }
    parts
}

pub fn init() {}
pub fn probe() {}
