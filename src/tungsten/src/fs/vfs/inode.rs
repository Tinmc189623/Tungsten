// fs/vfs/inode.rs — VFS 索引节点
//
// 文件系统不感知的抽象 inode，由具体 FS 实现填充。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::types::*;

/// VFS Inode — 文件系统不感知的抽象 inode
#[repr(C)]
pub struct Inode {
    pub ino: Ino,
    pub kind: FileType,
    pub size: u64,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub blocks: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    /// 文件系统私有指针 (FS 内部数据)
    pub fs_priv: *mut (),
}

impl Inode {
    /// 创建新的 VFS inode
    pub const fn new(ino: Ino, kind: FileType) -> Self {
        Inode {
            ino,
            kind,
            size: 0,
            mode: 0o644,
            uid: 0, gid: 0,
            nlink: 1,
            blocks: 0,
            atime: 0, mtime: 0, ctime: 0,
            fs_priv: core::ptr::null_mut(),
        }
    }
}
