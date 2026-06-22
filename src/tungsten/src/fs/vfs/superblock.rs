// fs/vfs/superblock.rs — VFS 超级块 + 操作接口
//
// 每个挂载的文件系统实例对应一个超级块。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::vfs::inode::Inode;
use crate::fs::vfs::dentry::Dentry;
use crate::fs::types::DevId;

/// VFS 超级块 — 每个挂载的文件系统实例对应一个
#[repr(C)]
pub struct SuperBlock {
    pub s_dev: DevId,
    pub s_magic: u32,
    pub s_root: *mut Dentry,
    pub s_op: &'static SuperOperations,
    pub s_fs_info: *mut (),
}

impl SuperBlock {
    /// 创建超级块
    pub const fn new(dev: DevId, magic: u32, op: &'static SuperOperations) -> Self {
        SuperBlock {
            s_dev: dev,
            s_magic: magic,
            s_root: core::ptr::null_mut(),
            s_op: op,
            s_fs_info: core::ptr::null_mut(),
        }
    }
}

/// VFS 超级块操作
pub struct SuperOperations {
    pub read_inode:  unsafe extern "C" fn(sb: &SuperBlock, ino: u64) -> *mut Inode,
    pub write_inode: unsafe extern "C" fn(sb: &SuperBlock, inode: &Inode) -> i32,
    pub put_inode:   unsafe extern "C" fn(sb: &SuperBlock, inode: *mut Inode),
    pub sync_fs:     unsafe extern "C" fn(sb: &SuperBlock) -> i32,
}

impl SuperOperations {
    /// 创建超级块操作 vtable
    pub const fn new(
        read_inode: unsafe extern "C" fn(&SuperBlock, u64) -> *mut Inode,
        write_inode: unsafe extern "C" fn(&SuperBlock, &Inode) -> i32,
        put_inode: unsafe extern "C" fn(&SuperBlock, *mut Inode),
        sync_fs: unsafe extern "C" fn(&SuperBlock) -> i32,
    ) -> Self {
        SuperOperations { read_inode, write_inode, put_inode, sync_fs }
    }
}
