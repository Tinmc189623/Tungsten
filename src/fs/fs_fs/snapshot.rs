// fs/fs_fs/snapshot.rs — COW 快照子系统
// 可增长快照池 (自动从全局空闲空间借用)
// 快照通过 B+tree 重定向表跟踪修改的块, 创建/删除/列表/回滚
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::ramdisk::get_ramdisk_device;
use crate::fs::types::Ino;
use crate::fs::error::{FsResult, FsError};

// ── 常量 ──

/// 快照魔数
const SNAPSHOT_MAGIC: u32 = 0x534E_4150; // "SNAP"

/// 快照条目大小
const SNAP_ENTRY_SIZE: usize = 128;

/// 最大快照数
const MAX_SNAPSHOTS: u16 = 64;

/// 每个快照的最大 COW 条目数
const MAX_COW_ENTRIES: u64 = 65536;

// ── 快照磁盘结构 ──

/// 快照文件超级块 (存储在快照 inode 的数据中)
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct SnapshotSuperBlock {
    magic: u32,                     // SNAPSHOT_MAGIC
    version: u16,                   // 1
    snapshot_count: u16,            // 活跃快照数
    next_snapshot_id: u32,          // 下一个快照 ID
    pool_bytes: u64,                // 快照池当前总字节数
    max_pool_bytes: u64,            // 快照池最大字节数 (0=无限制)
    _reserved: [u8; 92],
}

impl SnapshotSuperBlock {
    const fn empty() -> Self {
        SnapshotSuperBlock {
            magic: 0, version: 1, snapshot_count: 0, next_snapshot_id: 1,
            pool_bytes: 0, max_pool_bytes: 0, _reserved: [0; 92],
        }
    }
}

/// 单个快照元数据 (公开, 供外部遍历)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct SnapshotEntry {
    id: u32,                    // 快照 ID
    name: [u8; 32],            // 快照名称 (UTF-8)
    creation_time: u64,        // 创建时间 (Unix 时间戳)
    state: u8,                 // 0=active, 1=deleting, 2=rolled_back
    flags: u8,                 // 标志
    cow_root_offset: u64,      // COW 重定向表物理偏移
    cow_entry_count: u64,      // COW 条目数
    snapshot_size: u64,        // 快照创建时文件系统使用空间
    _reserved: [u8; 6],
}

impl SnapshotEntry {
    const fn empty() -> Self {
        SnapshotEntry {
            id: 0, name: [0; 32], creation_time: 0,
            state: 0, flags: 0, cow_root_offset: 0,
            cow_entry_count: 0, snapshot_size: 0, _reserved: [0; 6],
        }
    }
}

/// COW 重定向条目: 原始物理偏移 → 复制的数据物理偏移
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct CowEntry {
    original_phys: u64,         // 原始数据物理偏移
    copied_phys: u64,           // COW 复制的物理偏移
    length: u64,                // 数据长度
    timestamp: u64,             // COW 发生时间
}

// ── 快照状态 ──

const SNAP_STATE_ACTIVE: u8    = 0;
const SNAP_STATE_DELETING: u8  = 1;
const SNAP_STATE_ROLLED_BACK: u8 = 2;

// ── 快照管理器 ──

/// COW 快照管理器
pub struct SnapshotManager {
    /// 存储快照元数据的 inode
    pub snap_ino: Ino,
    /// 快照池当前大小
    pub pool_bytes: u64,
    /// 快照池最大大小 (0 = 无限制, 自动借用)
    pub max_pool_bytes: u64,
    /// 快照列表缓存
    entries: [SnapshotEntry; MAX_SNAPSHOTS as usize],
    entry_count: u16,
}

impl SnapshotManager {
    /// 创建快照管理器
    pub fn new(snap_ino: Ino) -> Self {
        SnapshotManager {
            snap_ino,
            pool_bytes: 0,
            max_pool_bytes: 0,
            entries: [SnapshotEntry::empty(); MAX_SNAPSHOTS as usize],
            entry_count: 0,
        }
    }

    /// 初始化快照子系统 (从快照 inode 加载元数据)
    pub fn init(&mut self) -> FsResult<()> {
        if self.snap_ino == 0 {
            return Ok(());
        }
        // 读取快照超级块
        let mut ssb_buf = [0u8; core::mem::size_of::<SnapshotSuperBlock>()];
        let _read = crate::fs::fs_fs::file::read_file_data(
            self.snap_ino, 0, &mut ssb_buf,
        );
        let ssb: SnapshotSuperBlock = unsafe {
            core::ptr::read_unaligned(ssb_buf.as_ptr() as *const SnapshotSuperBlock)
        };

        if ssb.magic == SNAPSHOT_MAGIC {
            self.pool_bytes = ssb.pool_bytes;
            self.max_pool_bytes = ssb.max_pool_bytes;
            // 加载快照列表
            let entries_off = core::mem::size_of::<SnapshotSuperBlock>() as u64;
            let mut snap_buf = [0u8; SNAP_ENTRY_SIZE];
            let count = ssb.snapshot_count.min(MAX_SNAPSHOTS);
            for i in 0..count as usize {
                let off = entries_off + i as u64 * SNAP_ENTRY_SIZE as u64;
                if crate::fs::fs_fs::file::read_file_data(self.snap_ino, off, &mut snap_buf) > 0 {
                    let se: SnapshotEntry = unsafe {
                        core::ptr::read_unaligned(snap_buf.as_ptr() as *const SnapshotEntry)
                    };
                    self.entries[i] = se;
                }
            }
            self.entry_count = count;
        } else {
            // 初始化新快照文件
            let new_ssb = SnapshotSuperBlock {
                magic: SNAPSHOT_MAGIC, version: 1,
                snapshot_count: 0, next_snapshot_id: 1,
                pool_bytes: 0, max_pool_bytes: 0,
                _reserved: [0; 92],
            };
            let ssb_slice = unsafe {
                core::slice::from_raw_parts(
                    &new_ssb as *const _ as *const u8,
                    core::mem::size_of::<SnapshotSuperBlock>(),
                )
            };
            crate::fs::fs_fs::file::write_file_data(self.snap_ino, 0, ssb_slice);
        }

        crate::serial::write_str(b"  snapshot: init done, count=");
        crate::serial_put_u64(self.entry_count as u64);
        crate::serial::write_str(b"\n");
        Ok(())
    }

    /// 创建快照
    /// 返回快照 ID, 或错误
    pub fn create_snapshot(&mut self, name: &str) -> FsResult<u32> {
        if self.snap_ino == 0 {
            return Err(FsError::Einval);
        }
        if self.entry_count >= MAX_SNAPSHOTS {
            return Err(FsError::Enospc);
        }

        let ssb = self.read_ssb()?;
        let snap_id = ssb.next_snapshot_id;

        let name_bytes = name.as_bytes();
        let mut snap_name = [0u8; 32];
        let copy_len = name_bytes.len().min(31);
        snap_name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

        let entry = SnapshotEntry {
            id: snap_id,
            name: snap_name,
            creation_time: crate::sched::ticks(),
            state: SNAP_STATE_ACTIVE,
            flags: 0,
            cow_root_offset: 0,
            cow_entry_count: 0,
            snapshot_size: self.pool_bytes,
            _reserved: [0; 6],
        };

        // 写入快照条目
        let entries_off = core::mem::size_of::<SnapshotSuperBlock>() as u64
            + self.entry_count as u64 * SNAP_ENTRY_SIZE as u64;
        let entry_slice = unsafe {
            core::slice::from_raw_parts(
                &entry as *const _ as *const u8,
                SNAP_ENTRY_SIZE,
            )
        };
        crate::fs::fs_fs::file::write_file_data(self.snap_ino, entries_off, entry_slice);

        self.entries[self.entry_count as usize] = entry;
        self.entry_count += 1;

        // 更新快照超级块
        let mut new_ssb = ssb;
        new_ssb.snapshot_count = self.entry_count;
        new_ssb.next_snapshot_id = snap_id + 1;
        self.write_ssb(&new_ssb)?;

        crate::serial::write_str(b"  snapshot: created id=");
        crate::serial_put_u64(snap_id as u64);
        crate::serial::write_str(b"\n");
        Ok(snap_id)
    }

    /// 删除快照
    pub fn delete_snapshot(&mut self, snap_id: u32) -> FsResult<()> {
        let idx = self.find_snapshot_idx(snap_id).ok_or(FsError::Enoent)?;

        // 释放 COW 条目占用的空间
        let entry = &self.entries[idx];
        if entry.cow_root_offset != 0 && entry.cow_entry_count > 0 {
            // 回收 COW 数据
            let cow_phys = entry.cow_root_offset;
            let cow_bytes = entry.cow_entry_count * core::mem::size_of::<CowEntry>() as u64;
            let _ = crate::fs::fs_fs::space::free_bytes_to_space(cow_phys, cow_bytes);
        }

        // 前移后续条目
        for i in idx..self.entry_count as usize - 1 {
            self.entries[i] = self.entries[i + 1];
        }
        self.entries[self.entry_count as usize - 1] = SnapshotEntry::empty();
        self.entry_count -= 1;

        // 更新快照超级块
        let ssb = self.read_ssb()?;
        let mut new_ssb = ssb;
        new_ssb.snapshot_count = self.entry_count;
        self.write_ssb(&new_ssb)?;

        Ok(())
    }

    /// 列出所有快照
    pub fn list_snapshots(&self) -> &[SnapshotEntry] {
        &self.entries[..self.entry_count as usize]
    }

    /// 回滚到指定快照
    /// 将所有文件系统数据恢复到快照创建时的状态
    pub fn rollback_to(&mut self, snap_id: u32) -> FsResult<()> {
        let idx = self.find_snapshot_idx(snap_id).ok_or(FsError::Enoent)?;
        let entry = self.entries[idx];

        // 回滚: 重放 COW 条目, 将原始数据恢复
        if entry.cow_root_offset != 0 && entry.cow_entry_count > 0 {
            let mut cow_buf = [0u8; core::mem::size_of::<CowEntry>()];
            for i in 0..entry.cow_entry_count {
                let off = entry.cow_root_offset + i * core::mem::size_of::<CowEntry>() as u64;
                if crate::fs::fs_fs::file::read_file_data(self.snap_ino, off, &mut cow_buf) == 0 {
                    break;
                }
                let cow: CowEntry = unsafe {
                    core::ptr::read_unaligned(cow_buf.as_ptr() as *const CowEntry)
                };
                // 将 COW 副本写回原始位置
                let mut data = [0u8; 4096];
                if get_ramdisk_device().read_bytes(cow.copied_phys, &mut data[..cow.length as usize]).is_ok() {
                    let _ = get_ramdisk_device().write_bytes(cow.original_phys, &data[..cow.length as usize]);
                }
            }
        }

        // 标记快照为已回滚
        self.entries[idx].state = SNAP_STATE_ROLLED_BACK;

        crate::serial::write_str(b"  snapshot: rolled back to id=");
        crate::serial_put_u64(snap_id as u64);
        crate::serial::write_str(b"\n");
        Ok(())
    }

    /// 执行 COW: 在修改前复制原始数据
    /// 对所有活跃快照记录 COW 条目
    pub fn cow_before_write(&mut self, original_phys: u64, length: u64) -> FsResult<()> {
        if self.entry_count == 0 {
            return Ok(());
        }

        for i in 0..self.entry_count as usize {
            if self.entries[i].state != SNAP_STATE_ACTIVE {
                continue;
            }

            // 分配 COW 副本空间
            let cow_phys = match crate::fs::fs_fs::space::alloc_bytes(length, 0)? {
                Some(p) => p,
                None => {
                    // 快照池自动从全局空间借
                    crate::fs::fs_fs::space::global_space().alloc(length, 0)?
                        .ok_or(FsError::Enospc)?
                }
            };

            // 复制原始数据
            let mut buf = [0u8; 4096];
            let read_len = length.min(4096);
            if get_ramdisk_device().read_bytes(original_phys, &mut buf[..read_len as usize]).is_ok() {
                let _ = get_ramdisk_device().write_bytes(cow_phys, &buf[..read_len as usize]);
            }

            // 写入 COW 条目
            let cow_entry = CowEntry {
                original_phys,
                copied_phys: cow_phys,
                length,
                timestamp: 0,
            };
            let cow_slice = unsafe {
                core::slice::from_raw_parts(
                    &cow_entry as *const _ as *const u8,
                    core::mem::size_of::<CowEntry>(),
                )
            };
            let cow_off = self.entries[i].cow_root_offset
                + self.entries[i].cow_entry_count * core::mem::size_of::<CowEntry>() as u64;
            crate::fs::fs_fs::file::write_file_data(self.snap_ino, cow_off, cow_slice);

            self.entries[i].cow_entry_count += 1;
        }

        Ok(())
    }

    // ── 内部辅助 ──

    fn find_snapshot_idx(&self, snap_id: u32) -> Option<usize> {
        for i in 0..self.entry_count as usize {
            if self.entries[i].id == snap_id {
                return Some(i);
            }
        }
        None
    }

    fn read_ssb(&self) -> FsResult<SnapshotSuperBlock> {
        let mut buf = [0u8; core::mem::size_of::<SnapshotSuperBlock>()];
        crate::fs::fs_fs::file::read_file_data(self.snap_ino, 0, &mut buf);
        Ok(unsafe {
            core::ptr::read_unaligned(buf.as_ptr() as *const SnapshotSuperBlock)
        })
    }

    fn write_ssb(&self, ssb: &SnapshotSuperBlock) -> FsResult<()> {
        let ssb_slice = unsafe {
            core::slice::from_raw_parts(
                ssb as *const _ as *const u8,
                core::mem::size_of::<SnapshotSuperBlock>(),
            )
        };
        crate::fs::fs_fs::file::write_file_data(self.snap_ino, 0, ssb_slice);
        Ok(())
    }
}

// ── 全局快照管理器 ──

use core::cell::UnsafeCell;

struct SnapWrapper(UnsafeCell<SnapshotManager>);
unsafe impl Sync for SnapWrapper {}

static SNAPSHOT_MGR: SnapWrapper = SnapWrapper(UnsafeCell::new(SnapshotManager {
    snap_ino: 0, pool_bytes: 0, max_pool_bytes: 0,
    entries: [SnapshotEntry::empty(); MAX_SNAPSHOTS as usize],
    entry_count: 0,
}));

/// 获取全局快照管理器
pub fn snapshot_manager() -> &'static mut SnapshotManager {
    unsafe { &mut *SNAPSHOT_MGR.0.get() }
}

/// 初始化快照子系统
pub fn snapshot_init(snap_ino: Ino) -> FsResult<()> {
    let mgr = snapshot_manager();
    mgr.snap_ino = snap_ino;
    mgr.init()
}

/// 创建快照便捷函数
pub fn snap_create(name: &str) -> FsResult<u32> {
    snapshot_manager().create_snapshot(name)
}

/// 删除快照便捷函数
pub fn snap_delete(snap_id: u32) -> FsResult<()> {
    snapshot_manager().delete_snapshot(snap_id)
}

/// 回滚到快照
pub fn snap_rollback(snap_id: u32) -> FsResult<()> {
    snapshot_manager().rollback_to(snap_id)
}
