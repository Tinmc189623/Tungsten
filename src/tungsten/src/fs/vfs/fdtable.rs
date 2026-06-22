// fs/vfs/fdtable.rs — VFS 文件描述符表 (每进程一份)
//
// 管理进程级文件描述符到 File 对象的映射。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use core::ptr::NonNull;
use crate::fs::vfs::file::File;
use crate::fs::types::FD_MAX;

/// 文件描述符表
pub struct FdTable {
    files: [Option<NonNull<File>>; FD_MAX],
    next_fd: i32,
}

impl FdTable {
    /// 创建空文件描述符表 (fd 0-2 预留给 stdin/stdout/stderr)
    pub const fn new() -> Self {
        FdTable {
            files: [None; FD_MAX],
            next_fd: 3,
        }
    }

    /// 分配新的文件描述符编号
    pub fn alloc_fd(&mut self) -> i32 {
        let fd = self.next_fd;
        if (fd as usize) >= FD_MAX {
            return -1;
        }
        self.next_fd = fd + 1;
        fd
    }

    /// 设置文件描述符对应的 File 对象
    pub fn set(&mut self, fd: i32, file: NonNull<File>) {
        if (fd as usize) < FD_MAX {
            self.files[fd as usize] = Some(file);
        }
    }

    /// 获取文件描述符对应的 File 引用
    pub fn get(&self, fd: i32) -> Option<&File> {
        if (fd as usize) < FD_MAX {
            self.files[fd as usize].map(|p| unsafe { p.as_ref() })
        } else {
            None
        }
    }

    /// 获取文件描述符对应的可变 File 引用
    pub fn get_mut(&mut self, fd: i32) -> Option<&mut File> {
        if (fd as usize) < FD_MAX {
            self.files[fd as usize].map(|mut p| unsafe { p.as_mut() })
        } else {
            None
        }
    }

    /// 关闭文件描述符
    pub fn close(&mut self, fd: i32) {
        if (fd as usize) < FD_MAX {
            self.files[fd as usize] = None;
        }
    }
}
