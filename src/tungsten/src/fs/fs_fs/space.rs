// fs/fs_fs/space.rs — 空闲空间树管理
// 管理设备空闲区域: 分配/释放/合并, 基于 B+tree
// 键 = 物理字节偏移, 值 = 空闲长度
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::ramdisk::get_ramdisk_device;
use crate::fs::error::{FsResult, FsError};

// ── 常量 ──

/// 空闲空间树节点大小
const SPACE_NODE_SIZE: usize = 4096;

/// 每个叶节点最大空闲条目数
const MAX_FREE_ENTRIES: u16 = 120;

/// 内部节点最大索引条目数
const MAX_FREE_INDEX: u16 = 200;

/// 空闲条目: (物理偏移, 长度)
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct FreeEntry {
    physical_offset: u64,
    length: u64,
}

/// 空闲树索引: 内部节点子指针
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct FreeIndex {
    key_offset: u64,          // 子节点覆盖的最小物理偏移
    child_physical: u64,      // 子节点物理偏移
}

/// 节点头 (与 FsExtentHeader 类似结构)
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct SpaceHeader {
    magic: u16,               // 0xF50A
    entries: u16,
    max_entries: u16,
    depth: u8,
    _reserved: [u8; 7],
}

const SPACE_MAGIC: u16 = 0xF50A;

type SpaceNodeBuf = [u8; SPACE_NODE_SIZE];

// ── 节点 I/O ──

fn read_space_node(physical: u64, buf: &mut SpaceNodeBuf) -> FsResult<()> {
    if physical == 0 {
        return Err(FsError::Einval);
    }
    get_ramdisk_device().read_bytes(physical, buf)
}

fn write_space_node(physical: u64, buf: &SpaceNodeBuf) -> FsResult<()> {
    get_ramdisk_device().write_bytes(physical, buf)
}

fn read_space_header(buf: &SpaceNodeBuf) -> SpaceHeader {
    unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const SpaceHeader) }
}

fn write_space_header(buf: &mut SpaceNodeBuf, hdr: &SpaceHeader) {
    unsafe { core::ptr::write_unaligned(buf.as_mut_ptr() as *mut SpaceHeader, *hdr); }
}

fn read_free_entry(buf: &SpaceNodeBuf, i: usize) -> FreeEntry {
    let off = core::mem::size_of::<SpaceHeader>() + i * core::mem::size_of::<FreeEntry>();
    unsafe { core::ptr::read_unaligned(buf.as_ptr().add(off) as *const FreeEntry) }
}

fn write_free_entry(buf: &mut SpaceNodeBuf, i: usize, entry: &FreeEntry) {
    let off = core::mem::size_of::<SpaceHeader>() + i * core::mem::size_of::<FreeEntry>();
    unsafe { core::ptr::write_unaligned(buf.as_mut_ptr().add(off) as *mut FreeEntry, *entry); }
}

fn read_free_index(buf: &SpaceNodeBuf, i: usize) -> FreeIndex {
    let off = core::mem::size_of::<SpaceHeader>() + i * core::mem::size_of::<FreeIndex>();
    unsafe { core::ptr::read_unaligned(buf.as_ptr().add(off) as *const FreeIndex) }
}

fn write_free_index(buf: &mut SpaceNodeBuf, i: usize, idx: &FreeIndex) {
    let off = core::mem::size_of::<SpaceHeader>() + i * core::mem::size_of::<FreeIndex>();
    unsafe { core::ptr::write_unaligned(buf.as_mut_ptr().add(off) as *mut FreeIndex, *idx); }
}

// ── 空闲空间管理器 ──

/// 空闲空间管理器
pub struct FreeSpaceTree {
    /// 根节点物理偏移 (0 = 未初始化)
    pub root_physical: u64,
}

/// 空闲区域描述
#[derive(Clone, Copy)]
pub struct FreeExtent {
    pub physical_offset: u64,
    pub length: u64,
}

impl FreeSpaceTree {
    pub const fn new(root_physical: u64) -> Self {
        FreeSpaceTree { root_physical }
    }

    // ── 分配 ──

    /// 分配指定长度的物理空间 (最佳匹配)
    /// 返回分配的物理偏移, None 表示空间不足
    pub fn alloc(&mut self, length: u64, _near: u64) -> FsResult<Option<u64>> {
        if self.root_physical == 0 {
            return Ok(None);
        }
        let aligned_len = (length + FS_MIN_ALLOC - 1) & !(FS_MIN_ALLOC - 1);

        let mut root_buf = [0u8; SPACE_NODE_SIZE];
        read_space_node(self.root_physical, &mut root_buf)?;

        // 在树中查找最佳匹配
        let result = self.alloc_from_node(&root_buf, aligned_len)?;

        if let Some((found_offset, found_len)) = result {
            // 从树中移除/缩减找到的空闲区域
            self.remove_range(found_offset, aligned_len)?;

            // 如果有剩余, 插回
            if found_len > aligned_len {
                self.free_internal(found_offset + aligned_len, found_len - aligned_len)?;
            }
            Ok(Some(found_offset))
        } else {
            Ok(None)
        }
    }

    /// 在节点及其子树中查找最佳匹配空闲区域
    fn alloc_from_node(
        &self, buf: &SpaceNodeBuf, length: u64,
    ) -> FsResult<Option<(u64, u64)>> {
        let hdr = read_space_header(buf);

        if hdr.depth == 0 {
            // 叶节点: 最佳匹配搜索
            let mut best: Option<(u64, u64)> = None;
            let mut best_diff: u64 = u64::MAX;
            for i in 0..hdr.entries as usize {
                let entry = read_free_entry(buf, i);
                if entry.length >= length && entry.length - length < best_diff {
                    best = Some((entry.physical_offset, entry.length));
                    best_diff = entry.length - length;
                }
            }
            Ok(best)
        } else {
            // 内部节点: 遍历所有子节点 (简化: 取第一个满足条件的)
            for i in 0..hdr.entries as usize {
                let idx = read_free_index(buf, i);
                let mut child_buf = [0u8; SPACE_NODE_SIZE];
                if read_space_node(idx.child_physical, &mut child_buf).is_ok() {
                    if let Some(result) = self.alloc_from_node(&child_buf, length)? {
                        return Ok(Some(result));
                    }
                }
            }
            Ok(None)
        }
    }

    /// 从空闲树中移除一段物理范围
    fn remove_range(&self, _offset: u64, _length: u64) -> FsResult<()> {
        // 简化: 重新扫描根节点并重建
        // Phase 2 叶节点直接操作, 不需要复杂删除
        if self.root_physical == 0 {
            return Ok(());
        }
        let mut root_buf = [0u8; SPACE_NODE_SIZE];
        read_space_node(self.root_physical, &mut root_buf)?;
        let hdr = read_space_header(&root_buf);
        if hdr.depth > 0 {
            return Ok(()); // 深层树暂不处理
        }

        let mut new_entries = 0u16;
        for i in 0..hdr.entries as usize {
            let entry = read_free_entry(&root_buf, i);
            let entry_end = entry.physical_offset + entry.length;
            let remove_end = _offset + _length;

            if entry_end <= _offset || entry.physical_offset >= remove_end {
                // 无重叠, 保留
                write_free_entry(&mut root_buf, new_entries as usize, &entry);
                new_entries += 1;
            } else if entry.physical_offset < _offset && entry_end > remove_end {
                // 中间移除, 分裂为两段
                let left = FreeEntry {
                    physical_offset: entry.physical_offset,
                    length: _offset - entry.physical_offset,
                };
                write_free_entry(&mut root_buf, new_entries as usize, &left);
                new_entries += 1;

                let right = FreeEntry {
                    physical_offset: remove_end,
                    length: entry_end - remove_end,
                };
                write_free_entry(&mut root_buf, new_entries as usize, &right);
                new_entries += 1;
            } else if entry.physical_offset < _offset {
                // 右半被移除
                let left = FreeEntry {
                    physical_offset: entry.physical_offset,
                    length: _offset - entry.physical_offset,
                };
                write_free_entry(&mut root_buf, new_entries as usize, &left);
                new_entries += 1;
            } else if entry_end > remove_end {
                // 左半被移除
                let right = FreeEntry {
                    physical_offset: remove_end,
                    length: entry_end - remove_end,
                };
                write_free_entry(&mut root_buf, new_entries as usize, &right);
                new_entries += 1;
            }
            // else: 完全覆盖, 丢弃
        }

        let mut new_hdr = hdr;
        new_hdr.entries = new_entries;
        write_space_header(&mut root_buf, &new_hdr);
        write_space_node(self.root_physical, &root_buf)
    }

    // ── 释放 ──

    /// 释放物理空间 (自动与前后相邻空闲区域合并)
    pub fn free(&mut self, physical_offset: u64, length: u64) -> FsResult<()> {
        self.free_internal(physical_offset, length)
    }

    fn free_internal(&self, physical_offset: u64, length: u64) -> FsResult<()> {
        if length == 0 || self.root_physical == 0 {
            return Ok(());
        }

        let mut root_buf = [0u8; SPACE_NODE_SIZE];
        read_space_node(self.root_physical, &mut root_buf)?;
        let hdr = read_space_header(&root_buf);

        if hdr.depth > 0 {
            return Ok(()); // 深层树暂不支持
        }

        let mut new_entry = FreeEntry { physical_offset, length };

        // 尝试与相邻条目合并
        let mut merged_entries: [FreeEntry; MAX_FREE_ENTRIES as usize] =
            [FreeEntry { physical_offset: 0, length: 0 }; MAX_FREE_ENTRIES as usize];
        let mut count = 0usize;
        let mut inserted = false;

        for i in 0..hdr.entries as usize {
            let entry = read_free_entry(&root_buf, i);

            if !inserted && entry.physical_offset >= new_entry.physical_offset + new_entry.length {
                // 在当前条目之前插入, 检查是否与 new_entry 相邻
                if entry.physical_offset == new_entry.physical_offset + new_entry.length {
                    // 合并: new_entry 吸收 entry
                    new_entry.length += entry.length;
                    continue;
                }
                // 检查 new_entry 是否与上一个条目相邻
                if count > 0 {
                    let prev = merged_entries[count - 1];
                    if prev.physical_offset + prev.length == new_entry.physical_offset {
                        merged_entries[count - 1].length += new_entry.length;
                        inserted = true;
                        // 仍需处理当前条目
                        merged_entries[count] = entry;
                        count += 1;
                        continue;
                    }
                }
                merged_entries[count] = new_entry;
                count += 1;
                inserted = true;
                merged_entries[count] = entry;
                count += 1;
            } else if !inserted && entry.physical_offset + entry.length <= new_entry.physical_offset {
                // entry 完全在 new_entry 前面
                if entry.physical_offset + entry.length == new_entry.physical_offset {
                    // 合并 entry → new_entry
                    new_entry.physical_offset = entry.physical_offset;
                    new_entry.length += entry.length;
                } else {
                    merged_entries[count] = entry;
                    count += 1;
                }
            } else if !inserted {
                // entry 在 new_entry 之后或有重叠
                if entry.physical_offset == new_entry.physical_offset + new_entry.length {
                    new_entry.length += entry.length;
                } else {
                    merged_entries[count] = new_entry;
                    count += 1;
                    inserted = true;
                    merged_entries[count] = entry;
                    count += 1;
                }
            } else {
                merged_entries[count] = entry;
                count += 1;
            }
        }

        if !inserted {
            // 检查最后一个合并
            if count > 0 {
                let prev = merged_entries[count - 1];
                if prev.physical_offset + prev.length == new_entry.physical_offset {
                    merged_entries[count - 1].length += new_entry.length;
                } else {
                    merged_entries[count] = new_entry;
                    count += 1;
                }
            } else {
                merged_entries[count] = new_entry;
                count += 1;
            }
        }

        // 写回
        let mut new_hdr = hdr;
        new_hdr.entries = count as u16;
        write_space_header(&mut root_buf, &new_hdr);
        for i in 0..count {
            write_free_entry(&mut root_buf, i, &merged_entries[i]);
        }
        write_space_node(self.root_physical, &root_buf)
    }

    // ── 查询 ──

    /// 查询空闲空间总量
    pub fn free_bytes(&self) -> FsResult<u64> {
        if self.root_physical == 0 {
            return Ok(0);
        }
        let mut root_buf = [0u8; SPACE_NODE_SIZE];
        read_space_node(self.root_physical, &mut root_buf)?;
        let hdr = read_space_header(&root_buf);
        let mut total: u64 = 0;
        for i in 0..hdr.entries as usize {
            let entry = read_free_entry(&root_buf, i);
            total = total.saturating_add(entry.length);
        }
        Ok(total)
    }

    /// 预留空间
    pub fn reserve(&mut self, length: u64) -> FsResult<()> {
        match self.alloc(length, 0)? {
            Some(_) => Ok(()),
            None => Err(FsError::Enospc),
        }
    }
}

// ── 全局空闲空间树 ──

use core::cell::UnsafeCell;

struct SpaceWrapper(UnsafeCell<FreeSpaceTree>);
unsafe impl Sync for SpaceWrapper {}

static FREE_SPACE: SpaceWrapper = SpaceWrapper(UnsafeCell::new(FreeSpaceTree::new(0)));

/// 获取全局空闲空间树的可变引用
pub fn global_space() -> &'static mut FreeSpaceTree {
    unsafe { &mut *FREE_SPACE.0.get() }
}

/// 从全局空闲空间分配字节 (通用接口)
pub fn alloc_bytes(length: u64, near: u64) -> FsResult<Option<u64>> {
    global_space().alloc(length, near)
}

/// 释放字节回全局空闲空间 (通用接口)
pub fn free_bytes_to_space(physical_offset: u64, length: u64) -> FsResult<()> {
    global_space().free(physical_offset, length)
}

/// 初始化空闲空间树 (在超级块格式化后调用)
pub fn init_free_space(root_physical: u64) {
    unsafe {
        *FREE_SPACE.0.get() = FreeSpaceTree::new(root_physical);
    }
    crate::serial::write_str(b"  space: init done\n");
}

/// 从空闲空间分配一个 4KB 节点 (供扩展树使用)
pub fn alloc_extent_node() -> FsResult<u64> {
    let space = global_space();
    match space.alloc(SPACE_NODE_SIZE as u64, 0)? {
        Some(phys) => {
            // 清零新节点
            let zero_buf = [0u8; SPACE_NODE_SIZE];
            get_ramdisk_device().write_bytes(phys, &zero_buf)?;
            Ok(phys)
        }
        None => Err(FsError::Enospc),
    }
}

/// 释放扩展树节点回空闲空间
pub fn free_extent_node(physical: u64) -> FsResult<()> {
    global_space().free(physical, SPACE_NODE_SIZE as u64)
}

/// 查询空闲字节数
pub fn free_bytes() -> FsResult<u64> {
    global_space().free_bytes()
}

/// 创建空的空闲空间树根节点并写入设备
pub fn create_free_space_root(root_phys: u64, data_start: u64, total_bytes: u64) -> FsResult<()> {
    let mut root_buf = [0u8; SPACE_NODE_SIZE];
    let hdr = SpaceHeader {
        magic: SPACE_MAGIC,
        entries: 1,
        max_entries: MAX_FREE_ENTRIES,
        depth: 0,
        _reserved: [0; 7],
    };
    write_space_header(&mut root_buf, &hdr);

    let free_len = total_bytes.saturating_sub(data_start);
    write_free_entry(&mut root_buf, 0, &FreeEntry {
        physical_offset: data_start,
        length: free_len,
    });

    write_space_node(root_phys, &root_buf)?;
    init_free_space(root_phys);
    Ok(())
}
