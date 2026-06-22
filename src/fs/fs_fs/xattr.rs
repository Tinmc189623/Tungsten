// fs/fs_fs/xattr.rs — 扩展属性子系统 (Phase 5: 多节点 B+tree)
// 每 inode 独立 B+tree 存储键值对, 支持 getxattr/setxattr/listxattr/removexattr
// 用于 ACL、加密上下文、安全标签等元数据
// xattr 根节点物理偏移存储在 inode.xattr_root.generation 字段中
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::fs_fs::inode::*;
use crate::fs::ramdisk::get_ramdisk_device;
use crate::fs::types::Ino;
use crate::fs::error::{FsResult, FsError};

// ── 常量 ──

/// 扩展属性名最大长度
pub const XATTR_NAME_MAX: usize = 255;
/// 扩展属性值最大长度
pub const XATTR_VALUE_MAX: usize = 65536;
/// 属性存储节点大小
pub const XATTR_NODE_SIZE: u64 = 4096;
/// 每个节点最多条目数
const XATTR_MAX_ENTRIES: u16 = 64;
/// xattr 根节点魔数 (区别于 extent 魔数)
const XATTR_HEADER_MAGIC: u16 = 0xF30B;

/// 常用属性命名空间前缀
pub const XATTR_SECURITY_PREFIX: &str = "security.";
pub const XATTR_SYSTEM_PREFIX: &str = "system.";
pub const XATTR_TRUSTED_PREFIX: &str = "trusted.";
pub const XATTR_USER_PREFIX: &str = "user.";

// ── 磁盘格式 ──

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct XattrHeader {
    magic: u16,             // XATTR_HEADER_MAGIC
    entries: u16,
    max_entries: u16,
    depth: u8,              // 0=叶节点, >0=内部节点
    _reserved: [u8; 3],
    checksum: u32,
}

impl XattrHeader {
    const fn empty() -> Self {
        XattrHeader { magic: 0, entries: 0, max_entries: 0, depth: 0, _reserved: [0; 3], checksum: 0 }
    }

    fn init(&mut self) {
        self.magic = XATTR_HEADER_MAGIC;
        self.entries = 0;
        self.max_entries = XATTR_MAX_ENTRIES;
        self.depth = 0;
        self.checksum = 0;
    }
}

/// 扩展属性条目: 键值对
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct XattrEntry {
    name_len: u8,
    flags: u8,              // bit 0: 值内联 (1=inline, 0=单独扩展)
    value_len: u16,
    value_offset: u32,      // 内联时在节点内偏移, 或单独扩展物理偏移
    name: [u8; 32],         // 短名内联 (≤31 字节 + NUL)
}

/// xattr 内部节点索引条目
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct XattrIndexEntry {
    key_offset: u32,        // 子节点覆盖的首个名称哈希 (排序键)
    child_physical: u64,    // 子节点物理偏移
}

// ── 内联值最大大小 ──
const XATTR_INLINE_MAX: u16 = 128;

// ── xattr 根节点定位 ──

/// 获取 xattr 根节点物理偏移 (从 inode.xattr_root.generation 读取)
fn get_xattr_root_phys(ino: Ino) -> FsResult<Option<u64>> {
    let mut di = FsDiskInode::empty();
    read_disk_inode(ino, &mut di).map_err(|_| FsError::Eio)?;
    if di.xattr_root.magic != XATTR_HEADER_MAGIC {
        return Ok(None);
    }
    let phys = di.xattr_root.generation as u64;
    if phys == 0 {
        return Ok(None);
    }
    Ok(Some(phys))
}

/// 设置 xattr 根节点物理偏移 (写入 inode.xattr_root.generation)
fn set_xattr_root_phys(ino: Ino, phys: u64) -> FsResult<()> {
    let mut di = FsDiskInode::empty();
    read_disk_inode(ino, &mut di).map_err(|_| FsError::Eio)?;
    di.xattr_root.magic = XATTR_HEADER_MAGIC;
    di.xattr_root.entries = 0;
    di.xattr_root.max_entries = XATTR_MAX_ENTRIES;
    di.xattr_root.depth = 0;
    di.xattr_root.generation = phys as u32;
    di.xattr_root.checksum = 0;
    write_disk_inode(ino, &di).map_err(|_| FsError::Eio)
}

// ── API ──

/// 获取扩展属性值
/// 返回值写入 value_buf, 返回写入字节数
pub fn get_xattr(ino: Ino, name: &str, value_buf: &mut [u8]) -> FsResult<usize> {
    if name.len() > XATTR_NAME_MAX || name.is_empty() {
        return Err(FsError::Einval);
    }

    let node_phys = get_xattr_root_phys(ino)?.ok_or(FsError::Enodata)?;

    // 扫描 xattr 树查找名称
    let mut buf = [0u8; XATTR_NODE_SIZE as usize];
    get_ramdisk_device().read_bytes(node_phys, &mut buf).map_err(|_| FsError::Eio)?;

    let header = unsafe {
        core::ptr::read_unaligned(buf.as_ptr() as *const XattrHeader)
    };

    if header.magic != XATTR_HEADER_MAGIC {
        return Err(FsError::Enodata);
    }

    // 在节点中查找
    find_and_copy_value(ino, node_phys, name, value_buf)
}

/// 设置扩展属性 (创建或更新)
pub fn set_xattr(ino: Ino, name: &str, value: &[u8]) -> FsResult<()> {
    if name.len() > XATTR_NAME_MAX || name.is_empty() || value.len() > XATTR_VALUE_MAX {
        return Err(FsError::Einval);
    }

    let mut node_phys = get_xattr_root_phys(ino)?.unwrap_or(0);

    if node_phys == 0 {
        // 分配新 xattr 节点并注册到 inode
        node_phys = alloc_xattr_node()?;
        set_xattr_root_phys(ino, node_phys)?;

        let mut new_buf = [0u8; XATTR_NODE_SIZE as usize];
        new_buf.fill(0);
        let mut header = XattrHeader::empty();
        header.init();
        unsafe {
            core::ptr::write_unaligned(new_buf.as_mut_ptr() as *mut XattrHeader, header);
        }
        get_ramdisk_device().write_bytes(node_phys, &new_buf).map_err(|_| FsError::Eio)?;
    }

    // 加载现有节点
    let mut buf = [0u8; XATTR_NODE_SIZE as usize];
    get_ramdisk_device().read_bytes(node_phys, &mut buf).map_err(|_| FsError::Eio)?;

    let mut header = unsafe {
        core::ptr::read_unaligned(buf.as_ptr() as *const XattrHeader)
    };

    if header.magic != XATTR_HEADER_MAGIC {
        return Err(FsError::Efscorrupt);
    }

    // 扫描现有条目
    let entry_size = core::mem::size_of::<XattrEntry>();
    let header_size = core::mem::size_of::<XattrHeader>();
    let mut existing_idx: Option<usize> = None;

    for i in 0..header.entries as usize {
        let off = header_size + i * entry_size;
        let entry: XattrEntry = unsafe {
            core::ptr::read_unaligned(buf.as_ptr().add(off) as *const XattrEntry)
        };
        let entry_name = unsafe {
            core::str::from_utf8(core::slice::from_raw_parts(entry.name.as_ptr(), entry.name_len as usize))
        }.unwrap_or("");
        if entry_name == name {
            existing_idx = Some(i);
            break;
        }
    }

    // 构建新条目
    let mut new_entry = XattrEntry {
        name_len: name.len() as u8,
        flags: 0,
        value_len: value.len() as u16,
        value_offset: 0,
        name: [0; 32],
    };
    let name_bytes = name.as_bytes();
    let name_copy = name_bytes.len().min(31);
    new_entry.name[..name_copy].copy_from_slice(&name_bytes[..name_copy]);

    if value.len() <= XATTR_INLINE_MAX as usize {
        // 内联值
        new_entry.flags |= 1;
        let inline_off = find_inline_space(&buf, header.entries, value.len());
        new_entry.value_offset = inline_off as u32;
        buf[inline_off..inline_off + value.len()].copy_from_slice(value);
    } else {
        // 单独分配
        let val_phys = alloc_xattr_value(value.len() as u64)?;
        new_entry.value_offset = val_phys as u32;
        get_ramdisk_device().write_bytes(val_phys, value).map_err(|_| FsError::Eio)?;
    }

    if let Some(idx) = existing_idx {
        // 覆盖已有条目
        let off = header_size + idx * entry_size;
        unsafe {
            core::ptr::write_unaligned(buf.as_mut_ptr().add(off) as *mut XattrEntry, new_entry);
        }
    } else {
        // 追加新条目 (可能需要分裂节点)
        if header.entries >= XATTR_MAX_ENTRIES {
            return split_xattr_node(ino, node_phys, &mut header, &new_entry, name, value);
        }
        let off = header_size + header.entries as usize * entry_size;
        unsafe {
            core::ptr::write_unaligned(buf.as_mut_ptr().add(off) as *mut XattrEntry, new_entry);
        }
        header.entries += 1;
        unsafe {
            core::ptr::write_unaligned(buf.as_mut_ptr() as *mut XattrHeader, header);
        }
    }

    get_ramdisk_device().write_bytes(node_phys, &buf).map_err(|_| FsError::Eio)
}

/// 列出所有扩展属性名
/// 每个名称以 NUL 分隔写入 buf, 返回写入字节数
pub fn list_xattr(ino: Ino, buf: &mut [u8]) -> FsResult<usize> {
    let node_phys = match get_xattr_root_phys(ino)? {
        Some(p) => p,
        None => return Ok(0),
    };

    let mut node_buf = [0u8; XATTR_NODE_SIZE as usize];
    get_ramdisk_device().read_bytes(node_phys, &mut node_buf).map_err(|_| FsError::Eio)?;

    let header = unsafe {
        core::ptr::read_unaligned(node_buf.as_ptr() as *const XattrHeader)
    };

    if header.magic != XATTR_HEADER_MAGIC {
        return Ok(0);
    }

    let entry_size = core::mem::size_of::<XattrEntry>();
    let header_size = core::mem::size_of::<XattrHeader>();
    let mut written = 0usize;

    for i in 0..header.entries as usize {
        let off = header_size + i * entry_size;
        let entry: XattrEntry = unsafe {
            core::ptr::read_unaligned(node_buf.as_ptr().add(off) as *const XattrEntry)
        };
        let name_len = entry.name_len as usize;
        if written + name_len + 1 <= buf.len() {
            buf[written..written + name_len].copy_from_slice(&entry.name[..name_len]);
            written += name_len;
            buf[written] = 0;
            written += 1;
        } else {
            break;
        }
    }

    Ok(written)
}

/// 删除扩展属性
pub fn remove_xattr(ino: Ino, name: &str) -> FsResult<()> {
    let node_phys = get_xattr_root_phys(ino)?.ok_or(FsError::Enodata)?;

    let mut buf = [0u8; XATTR_NODE_SIZE as usize];
    get_ramdisk_device().read_bytes(node_phys, &mut buf).map_err(|_| FsError::Eio)?;

    let mut header = unsafe {
        core::ptr::read_unaligned(buf.as_ptr() as *const XattrHeader)
    };

    if header.magic != XATTR_HEADER_MAGIC {
        return Err(FsError::Enodata);
    }

    let entry_size = core::mem::size_of::<XattrEntry>();
    let header_size = core::mem::size_of::<XattrHeader>();
    let mut found_idx: Option<usize> = None;

    for i in 0..header.entries as usize {
        let off = header_size + i * entry_size;
        let entry: XattrEntry = unsafe {
            core::ptr::read_unaligned(buf.as_ptr().add(off) as *const XattrEntry)
        };
        let entry_name = unsafe {
            core::str::from_utf8(core::slice::from_raw_parts(entry.name.as_ptr(), entry.name_len as usize))
        }.unwrap_or("");
        if entry_name == name {
            found_idx = Some(i);
            break;
        }
    }

    if let Some(idx) = found_idx {
        // 前移后续条目
        for i in idx..header.entries as usize - 1 {
            let src_off = header_size + (i + 1) * entry_size;
            let dst_off = header_size + i * entry_size;
            unsafe {
                core::ptr::copy(buf.as_ptr().add(src_off), buf.as_mut_ptr().add(dst_off), entry_size);
            }
        }
        let last_off = header_size + (header.entries as usize - 1) * entry_size;
        unsafe { core::ptr::write_bytes(buf.as_mut_ptr().add(last_off), 0, entry_size); }

        header.entries -= 1;
        unsafe {
            core::ptr::write_unaligned(buf.as_mut_ptr() as *mut XattrHeader, header);
        }
        get_ramdisk_device().write_bytes(node_phys, &buf).map_err(|_| FsError::Eio)?;
        return Ok(());
    }

    Err(FsError::Enodata)
}

// ── B+tree 辅助 (多节点支持) ──

/// 在 xattr 树中递归查找并复制值
fn find_and_copy_value(_ino: Ino, node_phys: u64, name: &str, value_buf: &mut [u8]) -> FsResult<usize> {
    let mut buf = [0u8; XATTR_NODE_SIZE as usize];
    get_ramdisk_device().read_bytes(node_phys, &mut buf).map_err(|_| FsError::Eio)?;

    let header = unsafe {
        core::ptr::read_unaligned(buf.as_ptr() as *const XattrHeader)
    };

    if header.depth == 0 {
        // 叶节点: 线性扫描
        let entry_size = core::mem::size_of::<XattrEntry>();
        let header_size = core::mem::size_of::<XattrHeader>();
        for i in 0..header.entries as usize {
            let off = header_size + i * entry_size;
            let entry: XattrEntry = unsafe {
                core::ptr::read_unaligned(buf.as_ptr().add(off) as *const XattrEntry)
            };
            let entry_name = unsafe {
                core::str::from_utf8(core::slice::from_raw_parts(entry.name.as_ptr(), entry.name_len as usize))
            }.unwrap_or("");
            if entry_name != name {
                continue;
            }
            return copy_xattr_value(entry, &buf, value_buf);
        }
        Err(FsError::Enodata)
    } else {
        // 内部节点: 按名称哈希二分查找子节点
        let hash = hash_name(name);
        let idx_size = core::mem::size_of::<XattrIndexEntry>();
        let header_size = core::mem::size_of::<XattrHeader>();

        for i in 0..header.entries as usize {
            let off = header_size + i * idx_size;
            let idx: XattrIndexEntry = unsafe {
                core::ptr::read_unaligned(buf.as_ptr().add(off) as *const XattrIndexEntry)
            };
            if hash >= idx.key_offset {
                // 检查是否为最后一个子节点, 或下一个子节点覆盖范围不包含此哈希
                let is_last_or_next_outside = if i + 1 >= header.entries as usize {
                    true
                } else {
                    let next: XattrIndexEntry = unsafe {
                        core::ptr::read_unaligned(
                            buf.as_ptr().add(header_size + (i + 1) * idx_size) as *const XattrIndexEntry
                        )
                    };
                    hash < next.key_offset
                };
                if is_last_or_next_outside {
                    return find_and_copy_value(_ino, idx.child_physical, name, value_buf);
                }
            }
        }
        Err(FsError::Enodata)
    }
}

/// 复制 xattr 值到用户缓冲区
fn copy_xattr_value(entry: XattrEntry, node_buf: &[u8; XATTR_NODE_SIZE as usize], value_buf: &mut [u8]) -> FsResult<usize> {
    if entry.flags & 1 != 0 {
        let copy_len = (entry.value_len as usize).min(value_buf.len());
        let val_off = entry.value_offset as usize;
        value_buf[..copy_len].copy_from_slice(&node_buf[val_off..val_off + copy_len]);
        Ok(copy_len)
    } else {
        let copy_len = (entry.value_len as usize).min(value_buf.len());
        get_ramdisk_device().read_bytes(entry.value_offset as u64, &mut value_buf[..copy_len])
            .map_err(|_| FsError::Eio)?;
        Ok(copy_len)
    }
}

/// 节点满时分裂为两个节点并可能提升内部节点
fn split_xattr_node(
    ino: Ino, node_phys: u64, header: &mut XattrHeader,
    new_entry: &XattrEntry, name: &str, value: &[u8],
) -> FsResult<()> {
    // 分配新叶节点
    let new_node_phys = alloc_xattr_node()?;
    let mut new_buf = [0u8; XATTR_NODE_SIZE as usize];
    new_buf.fill(0);
    let mut new_header = XattrHeader::empty();
    new_header.init();

    // 合并所有条目 (现有 + 新条目) 并按名称排序
    let entry_size = core::mem::size_of::<XattrEntry>();
    let header_size = core::mem::size_of::<XattrHeader>();
    let total = header.entries as usize + 1;
    let half = total / 2;

    // 读取所有现有条目
    #[derive(Clone, Copy)]
    struct SortEntry {
        name_h: u32,
        entry: XattrEntry,
        value_data: [u8; XATTR_INLINE_MAX as usize],
        value_external: u64,
        value_external_len: u16,
        is_external: bool,
    }

    let mut sort_entries: [SortEntry; 65] = unsafe { core::mem::zeroed() };

    let mut buf = [0u8; XATTR_NODE_SIZE as usize];
    get_ramdisk_device().read_bytes(node_phys, &mut buf).map_err(|_| FsError::Eio)?;

    for i in 0..header.entries as usize {
        let off = header_size + i * entry_size;
        let entry: XattrEntry = unsafe {
            core::ptr::read_unaligned(buf.as_ptr().add(off) as *const XattrEntry)
        };
        let ename = unsafe {
            core::str::from_utf8(core::slice::from_raw_parts(entry.name.as_ptr(), entry.name_len as usize))
        }.unwrap_or("");
        let mut se = SortEntry {
            name_h: hash_name(ename),
            entry,
            value_data: [0; XATTR_INLINE_MAX as usize],
            value_external: 0,
            value_external_len: 0,
            is_external: false,
        };
        if entry.flags & 1 != 0 {
            let vlen = entry.value_len as usize;
            let voff = entry.value_offset as usize;
            se.value_data[..vlen.min(XATTR_INLINE_MAX as usize)]
                .copy_from_slice(&buf[voff..voff + vlen.min(XATTR_INLINE_MAX as usize)]);
        } else {
            se.is_external = true;
            se.value_external = entry.value_offset as u64;
            se.value_external_len = entry.value_len;
        }
        sort_entries[i] = se;
    }

    // 添加新条目
    sort_entries[header.entries as usize] = {
        let mut se = SortEntry {
            name_h: hash_name(name),
            entry: *new_entry,
            value_data: [0; XATTR_INLINE_MAX as usize],
            value_external: 0,
            value_external_len: 0,
            is_external: false,
        };
        if new_entry.flags & 1 != 0 {
            se.value_data[..value.len().min(XATTR_INLINE_MAX as usize)].copy_from_slice(value);
        } else {
            se.is_external = true;
            se.value_external = new_entry.value_offset as u64;
            se.value_external_len = new_entry.value_len;
        }
        se
    };

    // 按名称哈希排序 (冒泡, 条目数少)
    for i in 0..total {
        for j in i + 1..total {
            if sort_entries[i].name_h > sort_entries[j].name_h {
                let temp = sort_entries[i].clone();
                sort_entries[i] = sort_entries[j].clone();
                sort_entries[j] = temp;
            }
        }
    }

    // 前 half 个条目写入原节点
    buf.fill(0);
    let mut orig_header_h = XattrHeader::empty();
    orig_header_h.init();
    orig_header_h.entries = half as u16;
    orig_header_h.max_entries = XATTR_MAX_ENTRIES;
    orig_header_h.depth = 0;
    unsafe {
        core::ptr::write_unaligned(buf.as_mut_ptr() as *mut XattrHeader, orig_header_h);
    }
    for i in 0..half {
        let se = &sort_entries[i];
        let mut e = se.entry;
        if e.flags & 1 != 0 {
            let inline_off = header_size + half * entry_size + i * XATTR_INLINE_MAX as usize;
            e.value_offset = inline_off as u32;
            let vlen = e.value_len as usize;
            buf[inline_off..inline_off + vlen].copy_from_slice(&se.value_data[..vlen]);
        }
        let off = header_size + i * entry_size;
        unsafe {
            core::ptr::write_unaligned(buf.as_mut_ptr().add(off) as *mut XattrEntry, e);
        }
    }
    get_ramdisk_device().write_bytes(node_phys, &buf).map_err(|_| FsError::Eio)?;

    // 后 total - half 个条目写入新节点
    new_buf.fill(0);
    new_header.entries = (total - half) as u16;
    unsafe {
        core::ptr::write_unaligned(new_buf.as_mut_ptr() as *mut XattrHeader, new_header);
    }
    for i in 0..(total - half) {
        let se = &sort_entries[half + i];
        let mut e = se.entry;
        if e.flags & 1 != 0 {
            let inline_off = header_size + new_header.entries as usize * entry_size + i * XATTR_INLINE_MAX as usize;
            e.value_offset = inline_off as u32;
            let vlen = e.value_len as usize;
            new_buf[inline_off..inline_off + vlen].copy_from_slice(&se.value_data[..vlen]);
        }
        let off = header_size + i * entry_size;
        unsafe {
            core::ptr::write_unaligned(new_buf.as_mut_ptr().add(off) as *mut XattrEntry, e);
        }
    }
    get_ramdisk_device().write_bytes(new_node_phys, &new_buf).map_err(|_| FsError::Eio)?;

    // 如果根节点被分裂, 提升为内部节点或创建新根
    if let Some(root_phys) = get_xattr_root_phys(ino)? {
        if root_phys == node_phys {
            // 原节点是根, 需要创建新的根内部节点
            let new_root_phys = alloc_xattr_node()?;
            let mut root_buf = [0u8; XATTR_NODE_SIZE as usize];
            root_buf.fill(0);
            let mut root_hdr = XattrHeader::empty();
            root_hdr.init();
            root_hdr.depth = 1; // 内部节点
            root_hdr.entries = 2;

            let idx_size = core::mem::size_of::<XattrIndexEntry>();
            let root_header_size = core::mem::size_of::<XattrHeader>();

            // 获取原节点首个条目名称哈希 (从叶节点条目中读取)
            let orig_first_hash: u32 = {
                let e: XattrEntry = unsafe {
                    core::ptr::read_unaligned(
                        buf.as_ptr().add(root_header_size) as *const XattrEntry
                    )
                };
                let ename = unsafe {
                    core::str::from_utf8(core::slice::from_raw_parts(e.name.as_ptr(), e.name_len as usize))
                }.unwrap_or("");
                hash_name(ename)
            };

            // 从新节点读取首个哈希
            let new_first_hash: u32 = {
                let e: XattrEntry = unsafe {
                    core::ptr::read_unaligned(
                        new_buf.as_ptr().add(root_header_size) as *const XattrEntry
                    )
                };
                let ename = unsafe {
                    core::str::from_utf8(core::slice::from_raw_parts(e.name.as_ptr(), e.name_len as usize))
                }.unwrap_or("");
                hash_name(ename)
            };

            unsafe {
                core::ptr::write_unaligned(root_buf.as_mut_ptr() as *mut XattrHeader, root_hdr);
            }

            let idx0 = XattrIndexEntry {
                key_offset: orig_first_hash,
                child_physical: node_phys,
            };
            let idx1 = XattrIndexEntry {
                key_offset: new_first_hash,
                child_physical: new_node_phys,
            };
            unsafe {
                core::ptr::write_unaligned(
                    root_buf.as_mut_ptr().add(root_header_size) as *mut XattrIndexEntry, idx0,
                );
                core::ptr::write_unaligned(
                    root_buf.as_mut_ptr().add(root_header_size + idx_size) as *mut XattrIndexEntry, idx1,
                );
            }

            get_ramdisk_device().write_bytes(new_root_phys, &root_buf).map_err(|_| FsError::Eio)?;
            set_xattr_root_phys(ino, new_root_phys)?;
        }
    }

    Ok(())
}

/// 计算名称的简单哈希值 (用于 B+tree 排序)
fn hash_name(name: &str) -> u32 {
    let bytes = name.as_bytes();
    let mut h: u32 = 0x811C_9DC5;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

// ── ACL 便捷访问 ──

/// 获取文件的 NFSv4 ACL (xattr 形式)
pub fn get_acl_xattr(ino: Ino) -> FsResult<[u8; 4096]> {
    let mut buf = [0u8; 4096];
    let _len = get_xattr(ino, "system.nfs4_acl", &mut buf).unwrap_or(0);
    Ok(buf)
}

/// 设置文件的 NFSv4 ACL
pub fn set_acl_xattr(ino: Ino, acl_data: &[u8]) -> FsResult<()> {
    set_xattr(ino, "system.nfs4_acl", acl_data)
}

/// 获取加密上下文
pub fn get_encrypt_ctx(ino: Ino) -> FsResult<[u8; 64]> {
    let mut buf = [0u8; 64];
    let len = get_xattr(ino, "system.encrypt.context", &mut buf)?;
    let mut ctx = [0u8; 64];
    ctx[..len.min(64)].copy_from_slice(&buf[..len.min(64)]);
    Ok(ctx)
}

/// 设置加密上下文
pub fn set_encrypt_ctx(ino: Ino, ctx: &[u8]) -> FsResult<()> {
    set_xattr(ino, "system.encrypt.context", ctx)
}

// ── 内部辅助 ──

/// 在 xattr 节点内找到可容纳 value_len 字节的内联空间
fn find_inline_space(_buf: &[u8; XATTR_NODE_SIZE as usize], entry_count: u16, need: usize) -> usize {
    let header_size = core::mem::size_of::<XattrHeader>();
    let entry_size = core::mem::size_of::<XattrEntry>();
    let used_by_entries = header_size + entry_count as usize * entry_size;

    // 放在所有条目之后
    let candidate = used_by_entries;
    if candidate + need <= XATTR_NODE_SIZE as usize {
        candidate
    } else {
        header_size
    }
}

/// 分配 xattr 节点 (4KB)
fn alloc_xattr_node() -> FsResult<u64> {
    use crate::fs::fs_fs::space;
    space::global_space().alloc(XATTR_NODE_SIZE, 0)?
        .ok_or(FsError::Enospc)
}

/// 分配 xattr 值存储空间
fn alloc_xattr_value(len: u64) -> FsResult<u64> {
    use crate::fs::fs_fs::space;
    space::global_space().alloc(len, 0)?
        .ok_or(FsError::Enospc)
}
