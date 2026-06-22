// fs/fs_fs/acl.rs — NFSv4 风格访问控制列表
// ACE 类型: ALLOW/DENY/AUDIT/ALARM, 继承标志, 首次匹配胜出
// 存储在 xattr "system.nfs4_acl" 中
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::fs_fs::inode::*;
use crate::fs::types::Ino;
use crate::fs::error::{FsResult, FsError};

// ── ACE 类型 ──

pub const ACE_TYPE_ALLOW: u8 = 0;
pub const ACE_TYPE_DENY: u8  = 1;
pub const ACE_TYPE_AUDIT: u8 = 2;
pub const ACE_TYPE_ALARM: u8 = 3;

// ── ACE 标志 ──

pub const ACE_FLAG_FILE_INHERIT: u8     = 1 << 0;  // 子文件继承
pub const ACE_FLAG_DIR_INHERIT: u8      = 1 << 1;  // 子目录继承
pub const ACE_FLAG_NO_PROPAGATE: u8     = 1 << 2;  // 仅直接子项
pub const ACE_FLAG_INHERIT_ONLY: u8     = 1 << 3;  // 不应用于自身
pub const ACE_FLAG_SUCCESSFUL_ACCESS: u8 = 1 << 6;  // AUDIT/ALARM 成功访问
pub const ACE_FLAG_FAILED_ACCESS: u8    = 1 << 7;  // AUDIT/ALARM 失败访问

// ── 访问掩码 (NFSv4) ──

pub const ACE_READ_DATA: u32         = 1 << 0;
pub const ACE_LIST_DIRECTORY: u32    = 1 << 0;
pub const ACE_WRITE_DATA: u32        = 1 << 1;
pub const ACE_ADD_FILE: u32          = 1 << 1;
pub const ACE_APPEND_DATA: u32       = 1 << 2;
pub const ACE_ADD_SUBDIRECTORY: u32  = 1 << 2;
pub const ACE_READ_NAMED_ATTRS: u32  = 1 << 3;
pub const ACE_WRITE_NAMED_ATTRS: u32 = 1 << 4;
pub const ACE_EXECUTE: u32           = 1 << 5;
pub const ACE_DELETE_CHILD: u32      = 1 << 6;
pub const ACE_READ_ATTRIBUTES: u32   = 1 << 7;
pub const ACE_WRITE_ATTRIBUTES: u32  = 1 << 8;
pub const ACE_DELETE: u32            = 1 << 9;
pub const ACE_READ_ACL: u32          = 1 << 10;
pub const ACE_WRITE_ACL: u32         = 1 << 11;
pub const ACE_WRITE_OWNER: u32       = 1 << 12;
pub const ACE_SYNCHRONIZE: u32       = 1 << 13;

// 复合掩码
pub const ACE_GENERIC_READ: u32  = ACE_READ_DATA | ACE_READ_ATTRIBUTES | ACE_READ_ACL | ACE_SYNCHRONIZE;
pub const ACE_GENERIC_WRITE: u32 = ACE_WRITE_DATA | ACE_APPEND_DATA | ACE_WRITE_ATTRIBUTES | ACE_WRITE_ACL | ACE_SYNCHRONIZE;
pub const ACE_GENERIC_EXECUTE: u32 = ACE_EXECUTE | ACE_READ_ATTRIBUTES | ACE_SYNCHRONIZE;
pub const ACE_FULL_CONTROL: u32  = 0xFFFFFFFF;

// ── NFSv4 ACE 结构 (磁盘格式) ──

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Nfs4Ace {
    pub ace_type: u8,       // ALLOW/DENY/AUDIT/ALARM
    pub ace_flags: u8,      // 继承/审计标志
    pub ace_flags_hi: u8,   // 高 8 位标志
    pub _reserved: u8,
    pub access_mask: u32,   // 权限掩码
    pub who: u32,           // uid/gid (0=OWNER@, 1=GROUP@, 2=EVERYONE@)
}

impl Nfs4Ace {
    pub const fn empty() -> Self {
        Nfs4Ace {
            ace_type: 0, ace_flags: 0, ace_flags_hi: 0, _reserved: 0,
            access_mask: 0, who: 0,
        }
    }

    /// 判断此 ACE 是否匹配请求的主体
    pub fn matches(&self, uid: u32, _gid: u32, groups: &[u32]) -> bool {
        match self.who {
            0 => true, // OWNER@ - 总是匹配 (由调用者检查 uid)
            1 => true, // GROUP@ - 总是匹配 (由调用者检查 gid)
            2 => true, // EVERYONE@ - 总是匹配
            id => id == uid || groups.contains(&id),
        }
    }

    /// 判断是否为继承专用 ACE
    pub fn is_inherit_only(&self) -> bool {
        self.ace_flags & ACE_FLAG_INHERIT_ONLY != 0
    }

    /// 判断是否应用于文件
    pub fn applies_to_file(&self) -> bool {
        self.ace_flags & ACE_FLAG_FILE_INHERIT != 0
    }

    /// 判断是否应用于目录
    pub fn applies_to_dir(&self) -> bool {
        self.ace_flags & ACE_FLAG_DIR_INHERIT != 0
    }
}

// ── ACL 结构 (最多 64 个 ACE) ──

const ACL_MAX_ACES: usize = 64;

#[repr(C, packed)]
pub struct Nfs4Acl {
    pub ace_count: u16,
    pub default_ace_count: u16,  // 默认 ACL (继承用) 的 ACE 数量
    pub _reserved: u16,
    pub checksum: u32,
    pub aces: [Nfs4Ace; ACL_MAX_ACES],
}

impl Nfs4Acl {
    pub const fn empty() -> Self {
        Nfs4Acl {
            ace_count: 0, default_ace_count: 0, _reserved: 0, checksum: 0,
            aces: [Nfs4Ace::empty(); ACL_MAX_ACES],
        }
    }

    /// 从字节缓冲解析 ACL
    pub fn from_bytes(data: &[u8]) -> FsResult<Self> {
        if data.len() < 8 {
            return Err(FsError::Einval);
        }
        let ace_count = u16::from_le_bytes([data[0], data[1]]) as usize;
        let default_ace_count = u16::from_le_bytes([data[2], data[3]]) as usize;

        let mut acl = Nfs4Acl::empty();
        acl.ace_count = ace_count as u16;
        acl.default_ace_count = default_ace_count as u16;

        let ace_size = core::mem::size_of::<Nfs4Ace>();
        let expected = 8 + ace_count * ace_size;
        if data.len() < expected {
            return Err(FsError::Einval);
        }

        for i in 0..ace_count.min(ACL_MAX_ACES) {
            let off = 8 + i * ace_size;
            acl.aces[i] = unsafe {
                core::ptr::read_unaligned(data.as_ptr().add(off) as *const Nfs4Ace)
            };
        }

        Ok(acl)
    }

    /// 序列化到字节缓冲
    pub fn to_bytes(&self, buf: &mut [u8]) -> FsResult<usize> {
        let ace_size = core::mem::size_of::<Nfs4Ace>();
        let total = 8 + self.ace_count as usize * ace_size;
        if buf.len() < total {
            return Err(FsError::Einval);
        }

        buf[0..2].copy_from_slice(&self.ace_count.to_le_bytes());
        buf[2..4].copy_from_slice(&self.default_ace_count.to_le_bytes());
        buf[4..6].copy_from_slice(&[0; 2]);
        buf[6..8].copy_from_slice(&[0; 2]);

        for i in 0..self.ace_count as usize {
            let off = 8 + i * ace_size;
            unsafe {
                core::ptr::write_unaligned(buf.as_mut_ptr().add(off) as *mut Nfs4Ace, self.aces[i]);
            }
        }

        Ok(total)
    }

    /// 获取有效 ACE (排除 INHERIT_ONLY)
    pub fn effective_aces(&self) -> impl Iterator<Item = &Nfs4Ace> {
        self.aces[..self.ace_count as usize].iter()
            .filter(|ace| !ace.is_inherit_only())
    }
}

// ── ACL 权限检查 ──

/// 检查主体是否拥有请求的访问权限
/// 首次匹配胜出: ALLOW 立即允许, DENY 立即拒绝
pub fn acl_check(
    ino: Ino,
    uid: u32, gid: u32,
    groups: &[u32],
    requested_mask: u32,
) -> FsResult<bool> {
    let mut di = FsDiskInode::empty();
    read_disk_inode(ino, &mut di).map_err(|_| FsError::Eio)?;

    // 获取 inode 的 UID/GID
    let file_uid = di.uid;
    let file_gid = di.gid;

    // 所有权检查 (类似 POSIX)
    let is_owner = uid == file_uid;
    let is_group = gid == file_gid || groups.contains(&file_gid);

    // 先检查 POSIX 模式位 (快速路径)
    let mode = di.mode & 0o777;
    let posix_mask = if is_owner {
        (mode >> 6) & 7
    } else if is_group {
        (mode >> 3) & 7
    } else {
        mode & 7
    };

    let posix_access = map_posix_to_access(posix_mask as u8);
    if posix_access & requested_mask == requested_mask {
        return Ok(true);
    }

    // 尝试加载 ACL
    let mut acl_data = [0u8; 4096];
    if let Ok(len) = crate::fs::fs_fs::xattr::get_xattr(ino, "system.nfs4_acl", &mut acl_data) {
        if len > 0 {
            if let Ok(acl) = Nfs4Acl::from_bytes(&acl_data[..len]) {
                // ACL 存在: ACL 优先于 POSIX 模式位
                for ace in acl.effective_aces() {
                    // 确定此 ACE 的主体是否匹配
                    let matches = match ace.who {
                        0 => is_owner,
                        1 => is_group,
                        2 => true,
                        id => uid == id || groups.contains(&id),
                    };

                    if !matches {
                        continue;
                    }

                    if ace.access_mask & requested_mask == requested_mask {
                        match ace.ace_type {
                            ACE_TYPE_ALLOW => return Ok(true),
                            ACE_TYPE_DENY => return Ok(false),
                            _ => continue,
                        }
                    }
                }
                // ACL 无匹配条目 → 拒绝
                return Ok(false);
            }
        }
    }

    // 回退到 POSIX 检查 (ACL 不存在时)
    Ok(posix_access & requested_mask == requested_mask)
}

/// 检查读权限
pub fn acl_check_read(ino: Ino, uid: u32, gid: u32, groups: &[u32]) -> FsResult<bool> {
    acl_check(ino, uid, gid, groups, ACE_READ_DATA)
}

/// 检查写权限
pub fn acl_check_write(ino: Ino, uid: u32, gid: u32, groups: &[u32]) -> FsResult<bool> {
    acl_check(ino, uid, gid, groups, ACE_WRITE_DATA)
}

/// 检查执行权限
pub fn acl_check_execute(ino: Ino, uid: u32, gid: u32, groups: &[u32]) -> FsResult<bool> {
    acl_check(ino, uid, gid, groups, ACE_EXECUTE)
}

// ── ACL 管理 ──

/// 设置文件 ACL
pub fn set_acl(ino: Ino, acl: &Nfs4Acl) -> FsResult<()> {
    let mut buf = [0u8; 4096];
    let len = acl.to_bytes(&mut buf)?;
    crate::fs::fs_fs::xattr::set_xattr(ino, "system.nfs4_acl", &buf[..len])
}

/// 读取文件 ACL
pub fn get_acl(ino: Ino) -> FsResult<Nfs4Acl> {
    let mut buf = [0u8; 4096];
    let len = crate::fs::fs_fs::xattr::get_xattr(ino, "system.nfs4_acl", &mut buf)?;
    Nfs4Acl::from_bytes(&buf[..len])
}

/// 为子文件生成继承 ACL
pub fn inherit_acl(parent_ino: Ino, is_dir: bool) -> FsResult<Option<Nfs4Acl>> {
    let mut buf = [0u8; 4096];
    if let Ok(len) = crate::fs::fs_fs::xattr::get_xattr(parent_ino, "system.nfs4_acl", &mut buf) {
        if len == 0 { return Ok(None); }
        let parent_acl = Nfs4Acl::from_bytes(&buf[..len])?;

        let mut child_acl = Nfs4Acl::empty();
        let mut child_count = 0u16;

        for i in 0..parent_acl.ace_count as usize {
            let ace = &parent_acl.aces[i];
            let inherits = if is_dir {
                ace.applies_to_dir()
            } else {
                ace.applies_to_file()
            };

            if inherits && child_count < ACL_MAX_ACES as u16 {
                let mut new_ace = *ace;
                // 清除 INHERIT_ONLY 标志
                new_ace.ace_flags &= !ACE_FLAG_INHERIT_ONLY;
                // 如果 NO_PROPAGATE 设置, 清除继承标志
                if ace.ace_flags & ACE_FLAG_NO_PROPAGATE != 0 {
                    new_ace.ace_flags &= !(ACE_FLAG_FILE_INHERIT | ACE_FLAG_DIR_INHERIT);
                }
                child_acl.aces[child_count as usize] = new_ace;
                child_count += 1;
            }
        }

        if child_count > 0 {
            child_acl.ace_count = child_count;
            return Ok(Some(child_acl));
        }
    }
    Ok(None)
}

// ── 默认 ACL 生成 ──

/// 从 POSIX 模式位生成最小 NFSv4 ACL
pub fn mode_to_acl(mode: u16, _uid: u32, _gid: u32) -> Nfs4Acl {
    let mut acl = Nfs4Acl::empty();
    let mut idx = 0u16;

    let owner_perm = ((mode >> 6) & 7) as u8;
    let group_perm = ((mode >> 3) & 7) as u8;
    let other_perm = (mode & 7) as u8;

    // OWNER@ ACE
    acl.aces[idx as usize] = Nfs4Ace {
        ace_type: ACE_TYPE_ALLOW, ace_flags: 0, ace_flags_hi: 0, _reserved: 0,
        access_mask: map_posix_to_access(owner_perm),
        who: 0, // OWNER@
    };
    idx += 1;

    // GROUP@ ACE
    if group_perm != other_perm {
        acl.aces[idx as usize] = Nfs4Ace {
            ace_type: ACE_TYPE_ALLOW, ace_flags: 0, ace_flags_hi: 0, _reserved: 0,
            access_mask: map_posix_to_access(group_perm),
            who: 1, // GROUP@
        };
        idx += 1;
    }

    // EVERYONE@ ACE
    acl.aces[idx as usize] = Nfs4Ace {
        ace_type: ACE_TYPE_ALLOW, ace_flags: 0, ace_flags_hi: 0, _reserved: 0,
        access_mask: map_posix_to_access(other_perm),
        who: 2, // EVERYONE@
    };
    idx += 1;

    acl.ace_count = idx;
    acl
}

// ── 辅助 ──

/// 将 POSIX 权限位映射到 NFSv4 访问掩码
fn map_posix_to_access(perm: u8) -> u32 {
    let mut mask = ACE_READ_ATTRIBUTES | ACE_SYNCHRONIZE;
    if perm & 4 != 0 { mask |= ACE_READ_DATA; }
    if perm & 2 != 0 { mask |= ACE_WRITE_DATA | ACE_APPEND_DATA; }
    if perm & 1 != 0 { mask |= ACE_EXECUTE; }
    mask
}
