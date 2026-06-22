// fs/vfs/dentry.rs — VFS 目录项 (路径缓存)
//
// 路径查找缓存 (dcache)，加速路径解析。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::types::MAX_NAME_LEN;
use crate::fs::vfs::inode::Inode;

/// VFS 目录项 — 路径查找缓存
#[repr(C)]
pub struct Dentry {
    pub name: [u8; MAX_NAME_LEN + 1],
    pub name_len: usize,
    pub inode: *mut Inode,
    pub parent: *mut Dentry,
    pub children: *mut Dentry,
    pub next: *mut Dentry,
}

impl Dentry {
    /// 创建空目录项
    pub const fn empty() -> Self {
        Dentry {
            name: [0u8; MAX_NAME_LEN + 1],
            name_len: 0,
            inode: core::ptr::null_mut(),
            parent: core::ptr::null_mut(),
            children: core::ptr::null_mut(),
            next: core::ptr::null_mut(),
        }
    }

    /// 从名称和 inode 创建目录项
    pub fn new(name: &str, inode: *mut Inode) -> Self {
        let mut dentry = Dentry {
            name: [0u8; MAX_NAME_LEN + 1],
            name_len: name.len().min(MAX_NAME_LEN),
            inode,
            parent: core::ptr::null_mut(),
            children: core::ptr::null_mut(),
            next: core::ptr::null_mut(),
        };
        dentry.name[..dentry.name_len].copy_from_slice(&name.as_bytes()[..dentry.name_len]);
        dentry
    }
}
