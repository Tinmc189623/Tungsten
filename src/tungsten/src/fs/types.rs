// fs/types.rs — FS 共享类型定义
//
// 所有文件系统模块共享的类型常量。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

/// Inode 编号类型
pub type Ino = u64;

/// 设备 ID 类型
pub type DevId = u64;

/// 物理字节偏移 (设备内)
pub type PhysOffset = u64;

/// 逻辑字节偏移 (文件内)
pub type LogiOffset = u64;

/// 文件模式位 (完整 32 位, POSIX mode_t)
pub type FileMode = u32;

/// 块大小 (仅用于与传统 API 兼容)
pub type BlkSize = u64;

/// 文件名最大长度
pub const MAX_NAME_LEN: usize = 255;
/// 路径最大长度
pub const MAX_PATH_LEN: usize = 4096;
/// 文件描述符最大数量
pub const FD_MAX: usize = 256;

/// 时间戳 (秒 + 纳秒)
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Timespec {
    pub sec: u64,
    pub nsec: u32,
}

impl Timespec {
    /// 创建时间戳
    pub const fn new(sec: u64, nsec: u32) -> Self {
        Timespec { sec, nsec }
    }
}

/// 文件类型
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FileType {
    Unknown = 0,
    Regular = 1,
    Directory = 2,
    Block = 3,
    Char = 4,
    Symlink = 5,
    Fifo = 6,
    Socket = 7,
}

/// 文件权限位 (POSIX)
pub mod perm {
    pub const OWNER_R: u16 = 0o400;
    pub const OWNER_W: u16 = 0o200;
    pub const OWNER_X: u16 = 0o100;
    pub const GROUP_R: u16 = 0o040;
    pub const GROUP_W: u16 = 0o020;
    pub const GROUP_X: u16 = 0o010;
    pub const OTHER_R: u16 = 0o004;
    pub const OTHER_W: u16 = 0o002;
    pub const OTHER_X: u16 = 0o001;
    pub const S_ISUID: u16 = 0o4000;
    pub const S_ISGID: u16 = 0o2000;
    pub const S_ISVTX: u16 = 0o1000;
}

/// 文件打开标志
pub const O_RDONLY: i32  = 0;
pub const O_WRONLY: i32  = 1;
pub const O_RDWR: i32    = 2;
pub const O_CREAT: i32   = 0x40;
pub const O_TRUNC: i32   = 0x200;
pub const O_APPEND: i32  = 0x400;
pub const O_DIRECTORY: i32 = 0x10000;
pub const O_CLOEXEC: i32   = 0x80000;

/// seek 模式
pub const SEEK_SET: i32 = 0;
pub const SEEK_CUR: i32 = 1;
pub const SEEK_END: i32 = 2;
