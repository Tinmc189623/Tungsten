// fs/fs_fs/extent.rs — 扩展树 B+tree 操作
// 管理文件数据: 逻辑字节偏移 → 物理字节偏移 的映射
// 根节点内联在 inode 中, 深层节点存储在 4KB 设备页中
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::fs_fs::inode::*;
use crate::fs::ramdisk::get_ramdisk_device;
use crate::fs::error::{FsResult, FsError};
use crate::fs::types::Ino;

// ── 常量 ──

/// 扩展树节点大小 (4KB, 与设备页对齐)
pub const NODE_SIZE: usize = 4096;

/// 叶节点最大扩展条目数 (4KB 节点中 FsExtent 数量)
const MAX_LEAF_ENTRIES: u16 = 85;

/// 内部节点最大索引条目数
const MAX_INDEX_ENTRIES: u16 = 200;

/// extent_root 在 FsDiskInode 中的字节偏移
const INODE_EXTENT_OFFSET: u64 = 66;

/// 内联根节点最大条目数 (inode 内可用 317 字节)
const MAX_INLINE_EXTENTS: u16 = 7;
const MAX_INLINE_INDEX: u16 = 19;

// ── 节点缓冲区 ──

type NodeBuf = [u8; NODE_SIZE];

/// 从设备读取 4KB 节点
fn read_node(physical: u64, buf: &mut NodeBuf) -> FsResult<()> {
    get_ramdisk_device().read_bytes(physical, buf)
}

/// 将 4KB 节点写入设备
fn write_node(physical: u64, buf: &NodeBuf) -> FsResult<()> {
    get_ramdisk_device().write_bytes(physical, buf)
}

/// 清零节点缓冲区
fn zero_node(buf: &mut NodeBuf) {
    buf.fill(0);
}

// ── 节点头解析 ──

/// 从节点缓冲区读取 FsExtentHeader
fn read_header(buf: &NodeBuf) -> FsExtentHeader {
    unsafe {
        core::ptr::read_unaligned(buf.as_ptr() as *const FsExtentHeader)
    }
}

/// 将 FsExtentHeader 写入节点缓冲区
fn write_header(buf: &mut NodeBuf, hdr: &FsExtentHeader) {
    unsafe {
        core::ptr::write_unaligned(buf.as_mut_ptr() as *mut FsExtentHeader, *hdr);
    }
}

/// 从叶节点读取第 i 个 FsExtent
fn read_extent(buf: &NodeBuf, i: usize) -> FsExtent {
    let offset = core::mem::size_of::<FsExtentHeader>() + i * core::mem::size_of::<FsExtent>();
    unsafe {
        core::ptr::read_unaligned(buf.as_ptr().add(offset) as *const FsExtent)
    }
}

/// 向叶节点写入第 i 个 FsExtent
fn write_extent(buf: &mut NodeBuf, i: usize, ext: &FsExtent) {
    let offset = core::mem::size_of::<FsExtentHeader>() + i * core::mem::size_of::<FsExtent>();
    unsafe {
        core::ptr::write_unaligned(buf.as_mut_ptr().add(offset) as *mut FsExtent, *ext);
    }
}

/// 从内部节点读取第 i 个 FsExtentIndex
fn read_index(buf: &NodeBuf, i: usize) -> FsExtentIndex {
    let offset = core::mem::size_of::<FsExtentHeader>() + i * core::mem::size_of::<FsExtentIndex>();
    unsafe {
        core::ptr::read_unaligned(buf.as_ptr().add(offset) as *const FsExtentIndex)
    }
}

/// 向内部节点写入第 i 个 FsExtentIndex
fn write_index(buf: &mut NodeBuf, i: usize, idx: &FsExtentIndex) {
    let offset = core::mem::size_of::<FsExtentHeader>() + i * core::mem::size_of::<FsExtentIndex>();
    unsafe {
        core::ptr::write_unaligned(buf.as_mut_ptr().add(offset) as *mut FsExtentIndex, *idx);
    }
}

// ── 内联根节点 I/O (inode 内的紧凑存储) ──

/// 从磁盘 inode 读取内联条目到节点缓冲区
fn read_inline_root(ino: Ino, buf: &mut NodeBuf) -> FsResult<()> {
    let mut di = FsDiskInode::empty();
    if read_disk_inode(ino, &mut di).is_err() {
        return Err(FsError::Eio);
    }
    let hdr = di.extent_root;
    if hdr.magic != 0 && hdr.magic != FS_EXTENT_MAGIC {
        return Err(FsError::Efscorrupt);
    }
    zero_node(buf);
    write_header(buf, &hdr);

    // 计算 inode 中条目数据的位置和大小
    let ino_phys = FS_INODE_TABLE_OFFSET + ino * FS_INODE_SIZE;
    let data_start = INODE_EXTENT_OFFSET + core::mem::size_of::<FsExtentHeader>() as u64;
    let max_data = FS_INODE_SIZE - 114 /* _reserved */ - data_start;
    let max_data = max_data as usize;

    // 条目数据紧接 header 之后在 inode 中
    let data_in_buf = core::mem::size_of::<FsExtentHeader>();
    let copy_len = max_data.min(NODE_SIZE - data_in_buf);
    get_ramdisk_device().read_bytes(
        ino_phys + data_start,
        &mut buf[data_in_buf..data_in_buf + copy_len],
    )?;
    Ok(())
}

/// 将内联条目写回磁盘 inode
fn write_inline_root(ino: Ino, buf: &NodeBuf) -> FsResult<()> {
    let mut di = FsDiskInode::empty();
    if read_disk_inode(ino, &mut di).is_err() {
        return Err(FsError::Eio);
    }
    let hdr = read_header(buf);
    di.extent_root = hdr;

    let ino_phys = FS_INODE_TABLE_OFFSET + ino * FS_INODE_SIZE;
    let data_start = INODE_EXTENT_OFFSET + core::mem::size_of::<FsExtentHeader>() as u64;
    let max_data = FS_INODE_SIZE - 114 - data_start;
    let data_in_buf = core::mem::size_of::<FsExtentHeader>();
    let copy_len = (max_data as usize).min(NODE_SIZE - data_in_buf);

    // 写回条目数据
    get_ramdisk_device().write_bytes(
        ino_phys + data_start,
        &buf[data_in_buf..data_in_buf + copy_len],
    )?;
    // 写回 inode header (含更新后的 extent_root)
    write_disk_inode(ino, &di).map_err(|_| FsError::Eio)
}

// ── 扩展树上下文 ──

/// 扩展树操作上下文
pub struct ExtentTree {
    /// 所属 inode
    pub ino: Ino,
    /// 根节点是否为内联 (inode 内) 存储
    pub root_is_inline: bool,
    /// 根节点物理偏移 (外部节点时有效)
    pub root_physical: u64,
    /// 根节点头缓存
    root_header: FsExtentHeader,
}

/// 扩展映射结果
#[derive(Clone, Copy, Debug)]
pub struct ExtentMap {
    pub logical_offset: u64,
    pub length: u64,
    pub physical_offset: u64,
    pub physical_length: u64,
    pub compression: u8,
}

impl ExtentTree {
    /// 从 inode 加载扩展树
    pub fn load(ino: Ino) -> FsResult<Self> {
        let mut di = FsDiskInode::empty();
        read_disk_inode(ino, &mut di).map_err(|_| FsError::Eio)?;
        let hdr = di.extent_root;

        if hdr.magic == 0 || hdr.entries == 0 {
            // 空树 (新文件)
            let empty_hdr = FsExtentHeader {
                magic: FS_EXTENT_MAGIC,
                entries: 0,
                max_entries: MAX_INLINE_EXTENTS,
                depth: 0,
                generation: 1,
                checksum: 0,
            };
            return Ok(ExtentTree {
                ino,
                root_is_inline: true,
                root_physical: 0,
                root_header: empty_hdr,
            });
        }

        Ok(ExtentTree {
            ino,
            root_is_inline: true,  // 根始终优先内联
            root_physical: 0,
            root_header: hdr,
        })
    }

    /// 获取根节点头引用
    pub fn root_header(&self) -> &FsExtentHeader {
        &self.root_header
    }

    // ── 查找 ──

    /// 查找 logical_offset 对应的物理映射
    /// 返回 None 表示空洞 (sparse 区域, 全零)
    pub fn lookup(&mut self, logical_offset: u64) -> FsResult<Option<ExtentMap>> {
        if self.root_header.entries == 0 {
            return Ok(None);
        }

        let mut node_buf = [0u8; NODE_SIZE];
        if self.root_is_inline {
            read_inline_root(self.ino, &mut node_buf)?;
        } else {
            read_node(self.root_physical, &mut node_buf)?;
        }

        self.lookup_in_node(&node_buf, logical_offset)
    }

    /// 在节点及子树中递归查找
    fn lookup_in_node(&self, buf: &NodeBuf, logical_offset: u64) -> FsResult<Option<ExtentMap>> {
        let hdr = read_header(buf);

        if hdr.depth == 0 {
            // 叶节点: 遍历 FsExtent 条目
            for i in 0..hdr.entries as usize {
                let ext = read_extent(buf, i);
                if logical_offset >= ext.logical_offset
                    && logical_offset < ext.logical_offset + ext.length
                {
                    let delta = logical_offset - ext.logical_offset;
                    return Ok(Some(ExtentMap {
                        logical_offset: ext.logical_offset + delta,
                        length: ext.length - delta,
                        physical_offset: ext.physical_offset + delta,
                        physical_length: ext.physical_length.saturating_sub(delta),
                        compression: ext.compression,
                    }));
                }
            }
            Ok(None) // 空洞
        } else {
            // 内部节点: 二分查找合适的子节点
            let child = self.find_child_index(buf, logical_offset, hdr.entries as usize)?;
            let idx_entry = read_index(buf, child);

            let mut child_buf = [0u8; NODE_SIZE];
            read_node(idx_entry.child_physical, &mut child_buf)?;
            self.lookup_in_node(&child_buf, logical_offset)
        }
    }

    /// 在内部节点中二分查找覆盖目标偏移的子节点索引
    fn find_child_index(
        &self, buf: &NodeBuf, logical_offset: u64, n_entries: usize,
    ) -> FsResult<usize> {
        // 内部节点条目按 logical_offset 升序排列
        // 找最后一个 logical_offset <= target 的条目
        let mut lo: isize = 0;
        let mut hi: isize = n_entries as isize - 1;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let idx = read_index(buf, mid as usize);
            if idx.logical_offset <= logical_offset {
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        let result = if hi < 0 { 0usize } else { hi as usize };
        Ok(result.min(n_entries.saturating_sub(1)))
    }

    // ── bmap: 文件逻辑偏移 → 设备物理偏移 ──

    /// 将文件内逻辑字节偏移映射到设备物理偏移
    /// 返回 (physical_offset, contiguous_bytes_available)
    pub fn bmap(&mut self, logical_offset: u64) -> FsResult<Option<(u64, u64)>> {
        match self.lookup(logical_offset)? {
            Some(map) => {
                let avail = map.physical_length;
                Ok(Some((map.physical_offset, avail)))
            }
            None => Ok(None),
        }
    }

    // ── 插入 ──

    /// 插入新扩展条目, 自动与相邻条目合并
    pub fn insert(
        &mut self,
        logical_offset: u64,
        length: u64,
        physical_offset: u64,
        physical_length: u64,
        compression: u8,
    ) -> FsResult<()> {
        if length == 0 || physical_length == 0 {
            return Err(FsError::Einval);
        }

        let new_ext = FsExtent {
            logical_offset,
            length,
            physical_offset,
            physical_length,
            compression,
            flags: 0,
        };

        let mut node_buf = [0u8; NODE_SIZE];
        if self.root_is_inline {
            read_inline_root(self.ino, &mut node_buf)?;
        } else {
            read_node(self.root_physical, &mut node_buf)?;
        }

        let hdr = read_header(&node_buf);

        if hdr.depth == 0 {
            // 叶节点: 直接插入
            self.insert_into_leaf(&mut node_buf, &new_ext)?;
        } else {
            // 递归插入到合适的子节点
            self.insert_recursive(&mut node_buf, &new_ext)?;
        }

        // 更新内存中的根头缓存
        self.root_header = read_header(&node_buf);

        // 写回
        if self.root_is_inline {
            write_inline_root(self.ino, &node_buf)?;
        } else {
            write_node(self.root_physical, &node_buf)?;
        }
        Ok(())
    }

    /// 在叶节点中插入 (处理合并与分裂)
    fn insert_into_leaf(&self, buf: &mut NodeBuf, ext: &FsExtent) -> FsResult<()> {
        let mut hdr = read_header(buf);
        let max_entries = if self.root_is_inline { MAX_INLINE_EXTENTS } else { MAX_LEAF_ENTRIES };

        // 尝试与现有条目合并
        for i in 0..hdr.entries as usize {
            let existing = read_extent(buf, i);
            // 相邻且物理连续且压缩标志相同 → 合并
            if existing.logical_offset + existing.length == ext.logical_offset
                && existing.physical_offset + existing.physical_length == ext.physical_offset
                && existing.compression == ext.compression
            {
                let merged = FsExtent {
                    logical_offset: existing.logical_offset,
                    length: existing.length + ext.length,
                    physical_offset: existing.physical_offset,
                    physical_length: existing.physical_length + ext.physical_length,
                    compression: existing.compression,
                    flags: 0,
                };
                write_extent(buf, i, &merged);
                // 更新条目计数不变, 仅修改内容
                return Ok(());
            }
        }

        if hdr.entries >= max_entries {
            // 节点已满, 需要扩展
            if self.root_is_inline && hdr.entries >= MAX_INLINE_EXTENTS {
                // 内联根节点已满, 需要迁移到外部节点
                return self.migrate_and_insert(buf, ext);
            }
            return Err(FsError::Enospc);
        }

        // 按 logical_offset 插入到正确位置
        let pos = self.find_insert_pos(buf, ext.logical_offset, hdr.entries as usize);
        // 后移后续条目
        for j in (pos..hdr.entries as usize).rev() {
            let moved = read_extent(buf, j);
            write_extent(buf, j + 1, &moved);
        }
        write_extent(buf, pos, ext);
        hdr.entries += 1;
        write_header(buf, &hdr);
        Ok(())
    }

    /// 查找插入位置 (保持 logical_offset 升序)
    fn find_insert_pos(&self, buf: &NodeBuf, logical_offset: u64, n_entries: usize) -> usize {
        for i in 0..n_entries {
            let ext = read_extent(buf, i);
            if ext.logical_offset > logical_offset {
                return i;
            }
        }
        n_entries
    }

    /// 内联根节点满时迁移到外部 4KB 节点
    fn migrate_and_insert(&self, buf: &mut NodeBuf, ext: &FsExtent) -> FsResult<()> {
        // 分配外部节点
        let new_phys = crate::fs::fs_fs::space::alloc_extent_node()?;

        let mut new_buf = [0u8; NODE_SIZE];
        zero_node(&mut new_buf);

        let new_hdr = FsExtentHeader {
            magic: FS_EXTENT_MAGIC,
            entries: 0,
            max_entries: MAX_LEAF_ENTRIES,
            depth: 0,
            generation: 1,
            checksum: 0,
        };
        write_header(&mut new_buf, &new_hdr);

        // 复制旧条目 + 新条目到新节点
        let old_hdr = read_header(buf);
        let mut all_extents: [FsExtent; MAX_LEAF_ENTRIES as usize] = [FsExtent::empty(); MAX_LEAF_ENTRIES as usize];
        let mut count = 0usize;

        // 合并旧条目和新条目 (按 logical_offset 排序插入)
        let mut old_i = 0usize;
        let mut ext_inserted = false;
        while old_i < old_hdr.entries as usize || !ext_inserted {
            let use_new = if old_i >= old_hdr.entries as usize {
                true
            } else if ext_inserted {
                false
            } else {
                ext.logical_offset < read_extent(buf, old_i).logical_offset
            };

            if use_new {
                all_extents[count] = *ext;
                count += 1;
                ext_inserted = true;
            } else {
                all_extents[count] = read_extent(buf, old_i);
                count += 1;
                old_i += 1;
            }
        }

        // 写入新节点
        let mut final_hdr = read_header(&mut new_buf);
        for i in 0..count {
            write_extent(&mut new_buf, i, &all_extents[i]);
        }
        final_hdr.entries = count as u16;
        write_header(&mut new_buf, &final_hdr);
        write_node(new_phys, &new_buf)?;

        // 将原内联根变为内部节点, 指向新叶节点
        zero_node(buf);
        let root_hdr = FsExtentHeader {
            magic: FS_EXTENT_MAGIC,
            entries: 1,
            max_entries: MAX_INLINE_INDEX,
            depth: 1,  // 深度+1
            generation: 1,
            checksum: 0,
        };
        write_header(buf, &root_hdr);
        write_index(buf, 0, &FsExtentIndex {
            logical_offset: all_extents[0].logical_offset,
            child_physical: new_phys,
        });

        // 更新 inode 中的根节点物理偏移 (此时根变为外部)
        // 注意: self 是 &self, 无法直接修改 root_physical
        // 这里我们保持 root_is_inline=true, 但根已在 inode 内作为内部节点
        Ok(())
    }

    /// 递归插入到子节点 (B+tree 递归, Phase 9: 完整实现)
    fn insert_recursive(&self, buf: &mut NodeBuf, ext: &FsExtent) -> FsResult<()> {
        let hdr = read_header(buf);
        let n_entries = hdr.entries as usize;

        // 找到合适的子节点
        let child_idx = self.find_child_index(buf, ext.logical_offset, n_entries)?;
        let idx_entry = read_index(buf, child_idx);

        // 加载子节点
        let mut child_buf = [0u8; NODE_SIZE];
        read_node(idx_entry.child_physical, &mut child_buf)?;

        let child_hdr = read_header(&child_buf);
        if child_hdr.depth == 0 {
            // 子节点是叶节点: 直接插入
            let max_leaf = MAX_LEAF_ENTRIES;
            if child_hdr.entries >= max_leaf {
                // 叶节点已满, 分裂
                self.split_leaf_and_insert(buf, child_idx, &mut child_buf, ext)?;
            } else {
                // 插入到叶节点
                self.insert_ext_into_node(&mut child_buf, ext, max_leaf)?;
                write_node(idx_entry.child_physical, &child_buf)?;
            }
        } else {
            // 子节点是内部节点: 递归
            self.insert_recursive(&mut child_buf, ext)?;
            write_node(idx_entry.child_physical, &child_buf)?;
        }

        Ok(())
    }

    /// 分裂叶节点并插入新条目, 可能有父节点也需要分裂
    fn split_leaf_and_insert(
        &self, parent_buf: &mut NodeBuf, child_idx: usize,
        child_buf: &mut NodeBuf, ext: &FsExtent,
    ) -> FsResult<()> {
        let child_hdr = read_header(child_buf);

        // 合并现有条目 + 新条目
        let mut all: [FsExtent; MAX_LEAF_ENTRIES as usize + 1] =
            [FsExtent::empty(); MAX_LEAF_ENTRIES as usize + 1];
        let mut count = 0usize;

        let mut inserted = false;
        for i in 0..child_hdr.entries as usize {
            if !inserted && ext.logical_offset < read_extent(child_buf, i).logical_offset {
                all[count] = *ext;
                count += 1;
                inserted = true;
            }
            all[count] = read_extent(child_buf, i);
            count += 1;
        }
        if !inserted {
            all[count] = *ext;
            count += 1;
        }

        let half = count / 2;

        // 写回前半到原子节点
        let mut new_child_hdr = child_hdr;
        new_child_hdr.entries = half as u16;
        write_header(child_buf, &new_child_hdr);
        for i in 0..half {
            write_extent(child_buf, i, &all[i]);
        }
        let child_phys = read_index(parent_buf, child_idx).child_physical;
        write_node(child_phys, child_buf)?;

        // 后半写入新分配的节点
        let new_node_phys = crate::fs::fs_fs::space::alloc_extent_node()?;
        let mut new_buf = [0u8; NODE_SIZE];
        let new_hdr = FsExtentHeader {
            magic: FS_EXTENT_MAGIC,
            entries: (count - half) as u16,
            max_entries: MAX_LEAF_ENTRIES,
            depth: 0,
            generation: 1,
            checksum: 0,
        };
        write_header(&mut new_buf, &new_hdr);
        for i in 0..(count - half) {
            write_extent(&mut new_buf, i, &all[half + i]);
        }
        write_node(new_node_phys, &new_buf)?;

        // 在父节点中插入指向新子节点的索引
        let parent_hdr = read_header(parent_buf);
        let max_parent = if self.root_is_inline { MAX_INLINE_INDEX } else { MAX_INDEX_ENTRIES };
        if parent_hdr.entries >= max_parent {
            return Err(FsError::Enospc); // 父节点也需要分裂 (Phase 9+)
        }
        // 在 child_idx+1 处插入新索引
        for j in ((child_idx + 1)..parent_hdr.entries as usize).rev() {
            let moved = read_index(parent_buf, j);
            write_index(parent_buf, j + 1, &moved);
        }
        write_index(parent_buf, child_idx + 1, &FsExtentIndex {
            logical_offset: all[half].logical_offset,
            child_physical: new_node_phys,
        });

        let mut new_parent_hdr = parent_hdr;
        new_parent_hdr.entries += 1;
        write_header(parent_buf, &new_parent_hdr);

        Ok(())
    }

    /// 在节点内按顺序插入扩展条目 (简单叶插入)
    fn insert_ext_into_node(&self, buf: &mut NodeBuf, ext: &FsExtent, max_entries: u16) -> FsResult<()> {
        let mut hdr = read_header(buf);
        if hdr.entries >= max_entries {
            return Err(FsError::Enospc);
        }

        let pos = self.find_insert_pos(buf, ext.logical_offset, hdr.entries as usize);
        for j in (pos..hdr.entries as usize).rev() {
            let moved = read_extent(buf, j);
            write_extent(buf, j + 1, &moved);
        }
        write_extent(buf, pos, ext);
        hdr.entries += 1;
        write_header(buf, &hdr);
        Ok(())
    }

    // ── 截断 ──

    /// 截断到指定大小, 移除超出范围的扩展
    pub fn truncate(&mut self, new_size: u64) -> FsResult<()> {
        if self.root_header.entries == 0 {
            return Ok(());
        }

        let mut node_buf = [0u8; NODE_SIZE];
        if self.root_is_inline {
            read_inline_root(self.ino, &mut node_buf)?;
        } else {
            read_node(self.root_physical, &mut node_buf)?;
        }

        let mut hdr = read_header(&node_buf);
        if hdr.depth > 0 {
            // 非叶根: 遍历子节点并截断 (Phase 9)
            for i in 0..hdr.entries as usize {
                let idx = read_index(&node_buf, i);
                let mut child_buf = [0u8; NODE_SIZE];
                if read_node(idx.child_physical, &mut child_buf).is_ok() {
                    self.truncate_leaf_range(&mut child_buf, new_size);
                    let _ = write_node(idx.child_physical, &child_buf);
                }
            }
            // 更新根节点头
            self.root_header = hdr;
            let mut di = FsDiskInode::empty();
            if read_disk_inode(self.ino, &mut di).is_err() { return Err(FsError::Eio); }
            di.size = new_size;
            return write_disk_inode(self.ino, &di).map_err(|_| FsError::Eio);
        }

        // 叶节点: 移除 logical_offset >= new_size 的条目
        // 同时截断跨越 new_size 边界的条目
        let mut new_count = 0u16;
        for i in 0..hdr.entries as usize {
            let ext = read_extent(&node_buf, i);
            if ext.logical_offset >= new_size {
                continue; // 完全超出, 丢弃
            }
            if ext.logical_offset + ext.length > new_size {
                // 跨越边界, 截断此条目
                let truncated = FsExtent {
                    logical_offset: ext.logical_offset,
                    length: new_size - ext.logical_offset,
                    physical_offset: ext.physical_offset,
                    physical_length: ext.physical_length.saturating_sub(
                        (ext.logical_offset + ext.length) - new_size,
                    ),
                    compression: ext.compression,
                    flags: ext.flags,
                };
                write_extent(&mut node_buf, new_count as usize, &truncated);
                new_count += 1;
            } else {
                write_extent(&mut node_buf, new_count as usize, &ext);
                new_count += 1;
            }
        }

        hdr.entries = new_count;
        write_header(&mut node_buf, &hdr);
        self.root_header = hdr;

        if self.root_is_inline {
            write_inline_root(self.ino, &node_buf)?;
        } else {
            write_node(self.root_physical, &node_buf)?;
        }

        // 更新 inode 大小
        let mut di = FsDiskInode::empty();
        if read_disk_inode(self.ino, &mut di).is_err() {
            return Err(FsError::Eio);
        }
        di.size = new_size;
        write_disk_inode(self.ino, &di).map_err(|_| FsError::Eio)
    }

    // ── 删除 ──

    /// 从指定逻辑偏移开始移除一段范围 (punch hole / 释放空间)
    pub fn remove(&mut self, logical_offset: u64, length: u64) -> FsResult<()> {
        if length == 0 || self.root_header.entries == 0 {
            return Ok(());
        }
        let end = logical_offset + length;

        let mut node_buf = [0u8; NODE_SIZE];
        if self.root_is_inline {
            read_inline_root(self.ino, &mut node_buf)?;
        } else {
            read_node(self.root_physical, &mut node_buf)?;
        }

        let mut hdr = read_header(&node_buf);
        if hdr.depth > 0 {
            // 非叶根: 遍历所有子叶节点处理 (Phase 9)
            for i in 0..hdr.entries as usize {
                let idx = read_index(&node_buf, i);
                let mut child_buf = [0u8; NODE_SIZE];
                if read_node(idx.child_physical, &mut child_buf).is_ok() {
                    let child_hdr = read_header(&child_buf);
                    if child_hdr.depth == 0 {
                        self.remove_range_from_leaf(&mut child_buf, logical_offset, length);
                        let _ = write_node(idx.child_physical, &child_buf);
                    }
                }
            }
            self.root_header = hdr;
            return if self.root_is_inline {
                write_inline_root(self.ino, &node_buf)
            } else {
                write_node(self.root_physical, &node_buf)
            };
        }

        let mut new_count = 0u16;
        for i in 0..hdr.entries as usize {
            let ext = read_extent(&node_buf, i);
            let ext_end = ext.logical_offset + ext.length;

            if ext_end <= logical_offset || ext.logical_offset >= end {
                // 无重叠, 保留
                write_extent(&mut node_buf, new_count as usize, &ext);
                new_count += 1;
            } else if ext.logical_offset < logical_offset && ext_end > end {
                // 中间掏空, 分裂为两个条目
                let left = FsExtent {
                    logical_offset: ext.logical_offset,
                    length: logical_offset - ext.logical_offset,
                    physical_offset: ext.physical_offset,
                    physical_length: logical_offset - ext.logical_offset,
                    compression: ext.compression,
                    flags: ext.flags,
                };
                write_extent(&mut node_buf, new_count as usize, &left);
                new_count += 1;

                let right = FsExtent {
                    logical_offset: end,
                    length: ext_end - end,
                    physical_offset: ext.physical_offset + (end - ext.logical_offset),
                    physical_length: ext.physical_length.saturating_sub(
                        (end - ext.logical_offset) + (logical_offset - ext.logical_offset),
                    ),
                    compression: ext.compression,
                    flags: ext.flags,
                };
                write_extent(&mut node_buf, new_count as usize, &right);
                new_count += 1;
            } else if ext.logical_offset < logical_offset {
                // 右半被移除, 保留左侧
                let left = FsExtent {
                    logical_offset: ext.logical_offset,
                    length: logical_offset - ext.logical_offset,
                    physical_offset: ext.physical_offset,
                    physical_length: logical_offset - ext.logical_offset,
                    compression: ext.compression,
                    flags: ext.flags,
                };
                write_extent(&mut node_buf, new_count as usize, &left);
                new_count += 1;
            } else if ext_end > end {
                // 左半被移除, 保留右侧
                let right = FsExtent {
                    logical_offset: end,
                    length: ext_end - end,
                    physical_offset: ext.physical_offset + (end - ext.logical_offset),
                    physical_length: ext.physical_length.saturating_sub(end - ext.logical_offset),
                    compression: ext.compression,
                    flags: ext.flags,
                };
                write_extent(&mut node_buf, new_count as usize, &right);
                new_count += 1;
            }
            // else: 完全被覆盖, 丢弃
        }

        hdr.entries = new_count;
        write_header(&mut node_buf, &hdr);
        self.root_header = hdr;

        if self.root_is_inline {
            write_inline_root(self.ino, &node_buf)
        } else {
            write_node(self.root_physical, &node_buf)
        }
    }

    /// 在叶节点内截断范围
    fn truncate_leaf_range(&self, buf: &mut NodeBuf, new_size: u64) {
        let mut hdr = read_header(buf);
        if hdr.depth > 0 { return; }

        let mut new_count = 0u16;
        for i in 0..hdr.entries as usize {
            let ext = read_extent(buf, i);
            if ext.logical_offset >= new_size { continue; }
            if ext.logical_offset + ext.length > new_size {
                let truncated = FsExtent {
                    logical_offset: ext.logical_offset,
                    length: new_size - ext.logical_offset,
                    physical_offset: ext.physical_offset,
                    physical_length: ext.physical_length.saturating_sub(
                        (ext.logical_offset + ext.length) - new_size,
                    ),
                    compression: ext.compression,
                    flags: ext.flags,
                };
                write_extent(buf, new_count as usize, &truncated);
                new_count += 1;
            } else {
                write_extent(buf, new_count as usize, &ext);
                new_count += 1;
            }
        }
        hdr.entries = new_count;
        write_header(buf, &hdr);
    }

    /// 从叶节点中移除一段范围 (punch hole, Phase 9)
    fn remove_range_from_leaf(&self, buf: &mut NodeBuf, logical_offset: u64, length: u64) {
        let end = logical_offset + length;
        let mut hdr = read_header(buf);
        if hdr.entries == 0 { return; }

        let mut new_count = 0u16;
        for i in 0..hdr.entries as usize {
            let ext = read_extent(buf, i);
            let ext_end = ext.logical_offset + ext.length;
            if ext_end <= logical_offset || ext.logical_offset >= end {
                write_extent(buf, new_count as usize, &ext); new_count += 1;
            } else if ext.logical_offset < logical_offset && ext_end > end {
                let left = FsExtent { logical_offset: ext.logical_offset, length: logical_offset - ext.logical_offset, physical_offset: ext.physical_offset, physical_length: logical_offset - ext.logical_offset, compression: ext.compression, flags: ext.flags };
                write_extent(buf, new_count as usize, &left); new_count += 1;
                let right = FsExtent { logical_offset: end, length: ext_end - end, physical_offset: ext.physical_offset + (end - ext.logical_offset), physical_length: ext.physical_length.saturating_sub((end - ext.logical_offset) + (logical_offset - ext.logical_offset)), compression: ext.compression, flags: ext.flags };
                write_extent(buf, new_count as usize, &right); new_count += 1;
            } else if ext.logical_offset < logical_offset {
                let left = FsExtent { logical_offset: ext.logical_offset, length: logical_offset - ext.logical_offset, physical_offset: ext.physical_offset, physical_length: logical_offset - ext.logical_offset, compression: ext.compression, flags: ext.flags };
                write_extent(buf, new_count as usize, &left); new_count += 1;
            } else if ext_end > end {
                let right = FsExtent { logical_offset: end, length: ext_end - end, physical_offset: ext.physical_offset + (end - ext.logical_offset), physical_length: ext.physical_length.saturating_sub(end - ext.logical_offset), compression: ext.compression, flags: ext.flags };
                write_extent(buf, new_count as usize, &right); new_count += 1;
            }
        }
        hdr.entries = new_count;
        write_header(buf, &hdr);
    }

    /// 获取文件物理占用字节数
    pub fn physical_size(&mut self) -> FsResult<u64> {
        if self.root_header.entries == 0 {
            return Ok(0);
        }
        let mut node_buf = [0u8; NODE_SIZE];
        if self.root_is_inline {
            read_inline_root(self.ino, &mut node_buf)?;
        } else {
            read_node(self.root_physical, &mut node_buf)?;
        }
        let hdr = read_header(&node_buf);
        let mut total: u64 = 0;
        for i in 0..hdr.entries as usize {
            let ext = read_extent(&node_buf, i);
            total = total.saturating_add(ext.physical_length);
        }
        Ok(total)
    }

    /// 获取文件逻辑大小 (从 inode size)
    pub fn logical_size(&self) -> FsResult<u64> {
        let mut di = FsDiskInode::empty();
        read_disk_inode(self.ino, &mut di).map_err(|_| FsError::Eio)?;
        Ok(di.size)
    }
}
