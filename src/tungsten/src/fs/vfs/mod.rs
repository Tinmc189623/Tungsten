// fs/vfs/mod.rs — VFS 抽象层入口
//
// 提供文件系统不感知的虚拟 inode/dentry/file/superblock 抽象。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod inode;
pub mod dentry;
pub mod file;
pub mod superblock;
pub mod fdtable;
pub mod mount;
pub mod pathwalk;

pub use inode::Inode;
pub use dentry::Dentry;
pub use file::{File, FileOperations};
pub use superblock::{SuperBlock, SuperOperations};
pub use fdtable::FdTable;
pub use mount::{Mount, MountTable};
pub use pathwalk::path_walk;
