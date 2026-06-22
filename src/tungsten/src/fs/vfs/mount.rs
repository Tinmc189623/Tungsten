// fs/vfs/mount.rs — VFS 挂载表管理
//
// 管理多个文件系统挂载点，支持最多 16 个挂载。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::vfs::superblock::SuperBlock;
use crate::fs::types::MAX_PATH_LEN;

const MOUNT_MAX: usize = 16;

/// 挂载点
pub struct Mount {
    pub path: [u8; MAX_PATH_LEN],
    pub path_len: usize,
    pub sb: SuperBlock,
}

impl Mount {
    /// 创建挂载点
    pub fn new(path: &str, sb: SuperBlock) -> Self {
        let mut m = Mount {
            path: [0u8; MAX_PATH_LEN],
            path_len: path.len().min(MAX_PATH_LEN - 1),
            sb,
        };
        m.path[..m.path_len].copy_from_slice(path.as_bytes());
        m
    }
}

/// 挂载表
pub struct MountTable {
    mounts: [Option<Mount>; MOUNT_MAX],
    count: usize,
}

impl MountTable {
    /// 创建空挂载表
    pub const fn new() -> Self {
        MountTable {
            mounts: [const { None }; MOUNT_MAX],
            count: 0,
        }
    }

    /// 挂载文件系统
    pub fn mount(&mut self, path: &str, sb: SuperBlock) -> Result<(), ()> {
        if self.count >= MOUNT_MAX {
            return Err(());
        }
        self.mounts[self.count] = Some(Mount::new(path, sb));
        self.count += 1;
        Ok(())
    }

    /// 卸载挂载点（禁止卸载根）
    pub fn umount(&mut self, path: &str) -> Result<(), ()> {
        if path == "/" || path.is_empty() {
            return Err(());
        }
        let target = path.as_bytes();
        for i in 0..self.count {
            if let Some(ref mount) = self.mounts[i] {
                if mount.path[..mount.path_len] == *target {
                    for j in i..self.count - 1 {
                        self.mounts[j] = self.mounts[j + 1].take();
                    }
                    self.mounts[self.count - 1] = None;
                    self.count -= 1;
                    return Ok(());
                }
            }
        }
        Err(())
    }

    /// 列出挂载点到缓冲区
    pub fn list(&self, buf: &mut [u8]) -> usize {
        let mut pos = 0usize;
        for m in self.mounts.iter().flatten() {
            let line = &m.path[..m.path_len];
            if pos + line.len() + 12 > buf.len() {
                break;
            }
            buf[pos..pos + line.len()].copy_from_slice(line);
            pos += line.len();
            let suffix = b" -> FS\n";
            buf[pos..pos + suffix.len()].copy_from_slice(suffix);
            pos += suffix.len();
        }
        pos
    }

    /// 查找挂载点
    pub fn find(&self, path: &str) -> Option<&Mount> {
        for m in self.mounts.iter() {
            if let &Some(ref mount) = m {
                if mount.path[..mount.path_len] == *path.as_bytes() {
                    return Some(mount);
                }
            }
        }
        None
    }
}
