// fs/vfs/file.rs — VFS 文件对象
//
// 打开的文件实例 + 文件操作 vtable。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::vfs::inode::Inode;
use crate::fs::vfs::dentry::Dentry;

/// 文件操作接口 (vtable)
pub struct FileOperations {
    pub read:  unsafe extern "C" fn(file: &mut File, buf: *mut u8, count: usize) -> isize,
    pub write: unsafe extern "C" fn(file: &mut File, buf: *const u8, count: usize) -> isize,
    pub lseek: unsafe extern "C" fn(file: &mut File, offset: i64, whence: i32) -> i64,
    pub close: unsafe extern "C" fn(file: &mut File) -> i32,
    pub fsync: unsafe extern "C" fn(file: &mut File) -> i32,
    pub fallocate: unsafe extern "C" fn(file: &mut File, offset: u64, len: u64) -> i32,
}

impl FileOperations {
    /// 基础构造器 (fsync/fallocate 使用默认实现)
    pub const fn new(
        read: unsafe extern "C" fn(&mut File, *mut u8, usize) -> isize,
        write: unsafe extern "C" fn(&mut File, *const u8, usize) -> isize,
        lseek: unsafe extern "C" fn(&mut File, i64, i32) -> i64,
        close: unsafe extern "C" fn(&mut File) -> i32,
    ) -> Self {
        FileOperations { read, write, lseek, close, fsync: default_fsync, fallocate: default_fallocate }
    }

    /// 完整构造器 (含 fsync/fallocate)
    pub const fn new_full(
        read: unsafe extern "C" fn(&mut File, *mut u8, usize) -> isize,
        write: unsafe extern "C" fn(&mut File, *const u8, usize) -> isize,
        lseek: unsafe extern "C" fn(&mut File, i64, i32) -> i64,
        close: unsafe extern "C" fn(&mut File) -> i32,
        fsync: unsafe extern "C" fn(&mut File) -> i32,
        fallocate: unsafe extern "C" fn(&mut File, u64, u64) -> i32,
    ) -> Self {
        FileOperations { read, write, lseek, close, fsync, fallocate }
    }
}

/// 默认 fsync 实现 (无操作)
unsafe extern "C" fn default_fsync(_file: &mut File) -> i32 { 0 }

/// 默认 fallocate 实现 (不支持)
unsafe extern "C" fn default_fallocate(_file: &mut File, _offset: u64, _len: u64) -> i32 { -1 }

/// 打开的文件实例
#[derive(Clone, Copy)]
#[repr(C)]
pub struct File {
    pub fd: i32,
    pub flags: i32,
    pub pos: i64,
    pub inode: *mut Inode,
    pub dentry: *mut Dentry,
    pub f_op: &'static FileOperations,
    pub private_data: *mut (),
}

impl File {
    /// 创建打开的文件实例
    pub fn new(
        fd: i32, inode: *mut Inode, dentry: *mut Dentry,
        f_op: &'static FileOperations, flags: i32,
    ) -> Self {
        File {
            fd, flags, pos: 0, inode, dentry, f_op,
            private_data: core::ptr::null_mut(),
        }
    }
}
