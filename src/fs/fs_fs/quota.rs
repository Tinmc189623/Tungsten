// fs/fs_fs/quota.rs — 用户/组/项目配额管理
// 可配置宽限期 (默认 7 天), 支持硬限制/软限制/inode限制
// 配额数据存储在专用 quota inode 中 (超级块字段: quota_inode_user/group/project)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::types::Ino;
use crate::fs::error::{FsResult, FsError};

// ── 配额类型 ──

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum QuotaType {
    User    = 0,
    Group   = 1,
    Project = 2,
}

// ── 配额限制 ──

/// 配额管理器 (内存视图)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct QuotaInfo {
    /// 硬限制 (绝对上限, 字节)
    pub hard_block_limit: u64,
    /// 软限制 (超过后进入宽限期, 字节)
    pub soft_block_limit: u64,
    /// 当前使用字节数
    pub cur_blocks: u64,
    /// 硬 inode 限制
    pub hard_inode_limit: u64,
    /// 软 inode 限制
    pub soft_inode_limit: u64,
    /// 当前使用 inode 数
    pub cur_inodes: u64,
    /// 宽限期开始时间 (Unix 时间戳, 超过软限制时设置)
    pub grace_start: u64,
    /// 标志: bit0=超过软限制, bit1=超过硬限制
    pub flags: u16,
}

impl QuotaInfo {
    pub const fn empty() -> Self {
        QuotaInfo {
            hard_block_limit: 0, soft_block_limit: 0, cur_blocks: 0,
            hard_inode_limit: 0, soft_inode_limit: 0, cur_inodes: 0,
            grace_start: 0, flags: 0,
        }
    }

    /// 检查是否可以分配指定字节数
    pub fn check_block_alloc(&self, want: u64, grace_period: u64, now: u64) -> FsResult<()> {
        if self.hard_block_limit > 0
            && self.cur_blocks + want > self.hard_block_limit
        {
            return Err(FsError::Edquot);
        }
        if self.soft_block_limit > 0
            && self.cur_blocks + want > self.soft_block_limit
        {
            if self.grace_start == 0 {
                // 进入宽限期
                // grace_start 需要在写入时更新
            } else if now > self.grace_start + grace_period {
                return Err(FsError::Edquot);
            }
        }
        Ok(())
    }

    /// 检查是否可以分配 inode
    pub fn check_inode_alloc(&self, now: u64, grace_period: u64) -> FsResult<()> {
        if self.hard_inode_limit > 0
            && self.cur_inodes + 1 > self.hard_inode_limit
        {
            return Err(FsError::Edquot);
        }
        if self.soft_inode_limit > 0
            && self.cur_inodes + 1 > self.soft_inode_limit
        {
            if self.grace_start == 0 {
                // 进入宽限期
            } else if now > self.grace_start + grace_period {
                return Err(FsError::Edquot);
            }
        }
        Ok(())
    }
}

// ── 配额管理器 ──

/// 配额管理器 (每配额类型独立)
pub struct QuotaManager {
    /// 配额类型
    pub qtype: QuotaType,
    /// 存储配额数据的 inode 号
    pub ino: Ino,
    /// 默认宽限期 (秒, 默认 7 天 = 604800)
    pub default_grace: u64,
    /// 配额条目大小
    entry_size: usize,
}

impl QuotaManager {
    /// 创建配额管理器
    pub fn new(qtype: QuotaType, ino: Ino) -> Self {
        QuotaManager {
            qtype,
            ino,
            default_grace: 604800, // 7 days
            entry_size: core::mem::size_of::<QuotaInfo>(),
        }
    }

    /// 读取指定 ID 的配额信息
    pub fn get_quota(&self, id: u32) -> FsResult<QuotaInfo> {
        if self.ino == 0 {
            return Ok(QuotaInfo::empty()); // 无配额文件
        }
        let offset = id as u64 * self.entry_size as u64;
        let mut qi = QuotaInfo::empty();
        let qi_slice = unsafe {
            core::slice::from_raw_parts_mut(
                &mut qi as *mut _ as *mut u8,
                self.entry_size,
            )
        };
        let _read = crate::fs::fs_fs::file::read_file_data(self.ino, offset, qi_slice);
        Ok(qi)
    }

    /// 设置指定 ID 的配额信息
    pub fn set_quota(&self, id: u32, qi: &QuotaInfo) -> FsResult<()> {
        if self.ino == 0 {
            return Err(FsError::Einval);
        }
        let offset = id as u64 * self.entry_size as u64;
        let qi_slice = unsafe {
            core::slice::from_raw_parts(qi as *const _ as *const u8, self.entry_size)
        };
        crate::fs::fs_fs::file::write_file_data(self.ino, offset, qi_slice);
        Ok(())
    }

    /// 增加已用字节数
    pub fn charge_blocks(&self, id: u32, blocks: u64) -> FsResult<()> {
        if self.ino == 0 { return Ok(()); }
        let mut qi = self.get_quota(id)?;
        qi.cur_blocks = qi.cur_blocks.saturating_add(blocks);
        self.set_quota(id, &qi)
    }

    /// 减少已用字节数
    pub fn uncharge_blocks(&self, id: u32, blocks: u64) -> FsResult<()> {
        if self.ino == 0 { return Ok(()); }
        let mut qi = self.get_quota(id)?;
        qi.cur_blocks = qi.cur_blocks.saturating_sub(blocks);
        self.set_quota(id, &qi)
    }

    /// 增加已用 inode 数
    pub fn charge_inode(&self, id: u32) -> FsResult<()> {
        if self.ino == 0 { return Ok(()); }
        let mut qi = self.get_quota(id)?;
        qi.cur_inodes = qi.cur_inodes.saturating_add(1);
        self.set_quota(id, &qi)
    }

    /// 减少已用 inode 数
    pub fn uncharge_inode(&self, id: u32) -> FsResult<()> {
        if self.ino == 0 { return Ok(()); }
        let mut qi = self.get_quota(id)?;
        qi.cur_inodes = qi.cur_inodes.saturating_sub(1);
        self.set_quota(id, &qi)
    }

    /// 检查是否可以分配 (返回 Ok 或 Edquot)
    pub fn check_alloc(&self, id: u32, want_bytes: u64, now: u64) -> FsResult<()> {
        if self.ino == 0 { return Ok(()); }
        let qi = self.get_quota(id)?;
        qi.check_block_alloc(want_bytes, self.default_grace, now)
    }

    /// 检查是否可以创建 inode
    pub fn check_inode(&self, id: u32, now: u64) -> FsResult<()> {
        if self.ino == 0 { return Ok(()); }
        let qi = self.get_quota(id)?;
        qi.check_inode_alloc(now, self.default_grace)
    }
}

// ── 全局配额管理器 ──

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;

struct QuotaWrapper {
    user: UnsafeCell<MaybeUninit<QuotaManager>>,
    group: UnsafeCell<MaybeUninit<QuotaManager>>,
    project: UnsafeCell<MaybeUninit<QuotaManager>>,
}
unsafe impl Sync for QuotaWrapper {}

static QUOTA: QuotaWrapper = QuotaWrapper {
    user:    UnsafeCell::new(MaybeUninit::uninit()),
    group:   UnsafeCell::new(MaybeUninit::uninit()),
    project: UnsafeCell::new(MaybeUninit::uninit()),
};

/// 获取用户配额管理器
pub fn user_quota() -> &'static mut QuotaManager {
    unsafe { (*QUOTA.user.get()).assume_init_mut() }
}

/// 获取组配额管理器
pub fn group_quota() -> &'static mut QuotaManager {
    unsafe { (*QUOTA.group.get()).assume_init_mut() }
}

/// 获取项目配额管理器
pub fn project_quota() -> &'static mut QuotaManager {
    unsafe { (*QUOTA.project.get()).assume_init_mut() }
}

/// 初始化配额子系统 (从超级块读取配额 inode)
pub fn quota_init(user_ino: Ino, group_ino: Ino, project_ino: Ino) {
    unsafe {
        (*QUOTA.user.get()).write(QuotaManager::new(QuotaType::User, user_ino));
        (*QUOTA.group.get()).write(QuotaManager::new(QuotaType::Group, group_ino));
        (*QUOTA.project.get()).write(QuotaManager::new(QuotaType::Project, project_ino));
    }
    crate::serial::write_str(b"  quota: init done (user=");
    crate::serial_put_u64(user_ino);
    crate::serial::write_str(b" group=");
    crate::serial_put_u64(group_ino);
    crate::serial::write_str(b" project=");
    crate::serial_put_u64(project_ino);
    crate::serial::write_str(b")\n");
}
