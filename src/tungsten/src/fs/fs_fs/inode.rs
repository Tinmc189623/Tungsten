// fs/fs_fs/inode.rs — 磁盘 Inode 读写 + 分配/释放
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::fs_fs::superblock::*;
use crate::fs::ramdisk::get_ramdisk_device;
use crate::fs::types::Ino;

/// 获取当前系统时间 (从引导起算的 tick 计数)
/// 后续 RTC 驱动接入后替换为 Unix 时间戳
fn system_time() -> u64 {
    crate::sched::ticks()
}

/// 从设备读取磁盘 inode
pub fn read_disk_inode(ino: Ino, di: &mut FsDiskInode) -> Result<(), ()> {
    if ino >= FS_TOTAL_INODES {
        return Err(());
    }
    let offset = FS_INODE_TABLE_OFFSET + ino * FS_INODE_SIZE;
    let size = core::mem::size_of::<FsDiskInode>();
    if get_ramdisk_device().read_bytes(offset, unsafe {
        core::slice::from_raw_parts_mut(di as *mut _ as *mut u8, size)
    }).is_err() {
        return Err(());
    }
    Ok(())
}

/// 将磁盘 inode 写入设备
pub fn write_disk_inode(ino: Ino, di: &FsDiskInode) -> Result<(), ()> {
    if ino >= FS_TOTAL_INODES {
        return Err(());
    }
    let offset = FS_INODE_TABLE_OFFSET + ino * FS_INODE_SIZE;
    let size = core::mem::size_of::<FsDiskInode>();
    if get_ramdisk_device().write_bytes(offset, unsafe {
        core::slice::from_raw_parts(di as *const _ as *const u8, size)
    }).is_err() {
        return Err(());
    }
    Ok(())
}

/// 分配新 inode (返回 inode 编号)
pub fn alloc_inode(mode: u16) -> Option<Ino> {
    let mut sb = FsSuperBlockV2::empty();
    if sb_read(&mut sb).is_err() {
        return None;
    }
    if sb.free_inodes == 0 {
        return None;
    }
    // 扫描 inode 表查找空闲条目 (mode == 0 表示空闲)
    for ino in 0..sb.inode_count {
        let mut di = FsDiskInode::empty();
        if read_disk_inode(ino, &mut di).is_ok() && di.mode == 0 {
            let now = system_time();
            let new_di = FsDiskInode {
                mode,
                uid: 0, gid: 0, size: 0,
                atime: now, atime_nsec: 0,
                mtime: now, mtime_nsec: 0,
                ctime: now, ctime_nsec: 0,
                btime: now, btime_nsec: 0,
                extent_root: FsExtentHeader { magic: FS_EXTENT_MAGIC, entries: 0, max_entries: 7, depth: 0, generation: 1, checksum: 0 },
                xattr_root: FsExtentHeader::empty(),
                acl_root: FsExtentHeader::empty(),
                encrypt_ctx: [0; 32],
                nlink: 1, flags: 0, generation: 1, project_id: 0,
                checksum: 0, checksum_hi: 0,
                _reserved: [0; 114],
            };
            if write_disk_inode(ino, &new_di).is_err() {
                return None;
            }
            sb.free_inodes -= 1;
            let _ = sb_write(&sb);
            return Some(ino);
        }
    }
    None
}

/// 释放 inode
pub fn free_inode(ino: Ino) {
    if ino >= FS_TOTAL_INODES {
        return;
    }
    let mut di = FsDiskInode::empty();
    if read_disk_inode(ino, &mut di).is_err() {
        return;
    }
    // 清零 inode
    let zero = FsDiskInode::empty();
    let _ = write_disk_inode(ino, &zero);

    let mut sb = FsSuperBlockV2::empty();
    if sb_read(&mut sb).is_ok() {
        sb.free_inodes = sb.free_inodes.saturating_add(1);
        let _ = sb_write(&sb);
    }
}
