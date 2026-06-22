// fs/error.rs — FS 错误类型定义
//
// POSIX errno 兼容的错误码，用于文件系统和段设备操作。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

/// FS 操作结果类型
pub type FsResult<T> = Result<T, FsError>;

/// FS 错误码 (POSIX errno 兼容)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum FsError {
    /// 操作成功 (内部使用)
    Ok = 0,
    /// 没有权限
    Eperm = 1,
    /// 没有此文件或目录
    Enoent = 2,
    /// I/O 错误
    Eio = 5,
    /// 设备不存在
    Enodev = 6,
    /// 参数无效
    Einval = 8,
    /// 文件描述符无效
    Ebadf = 9,
    /// 资源暂时不可用
    Eagain = 11,
    /// 内存不足
    Enomem = 12,
    /// 权限被拒
    Eacces = 13,
    /// 文件已存在
    Eexist = 17,
    /// 不是目录
    Enotdir = 20,
    /// 是目录
    Eisdir = 21,
    /// 文件过大
    Efbig = 27,
    /// 设备空间不足
    Enospc = 28,
    /// 只读文件系统
    Erofs = 30,
    /// 文件名过长
    Enametoolong = 36,
    /// 目录非空
    Enotempty = 39,
    /// 配额已超
    Edquot = 69,
    /// 文件系统已损坏
    Efscorrupt = 71,
    /// 不支持的操作
    Enosys = 78,
    /// 属性不存在
    Enodata = 61,
    /// 不允许操作
    Enotcapable = 93,
}

impl FsError {
    /// 转为 POSIX errno 负数 (用于 syscall 返回值)
    pub fn to_errno(self) -> isize {
        -(self as isize)
    }

    /// 转为 i32 负数
    pub fn to_neg_i32(self) -> i32 {
        -(self as i32)
    }
}

impl From<FsError> for isize {
    fn from(e: FsError) -> isize {
        e.to_errno()
    }
}

impl From<FsError> for i32 {
    fn from(e: FsError) -> i32 {
        e.to_neg_i32()
    }
}
