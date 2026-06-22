// fs/vfs/pathwalk.rs — VFS 路径解析
//
// 从根目录逐分量解析绝对路径，调用 FS 层的 dir_lookup。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::types::Ino;
use crate::fs::fs_fs::dir;

/// 从根目录逐分量解析绝对路径
/// 返回 FS inode 编号, None 表示未找到
pub fn path_walk(path: &str) -> Option<Ino> {
    if path.is_empty() || !path.starts_with('/') {
        return None;
    }
    if path == "/" {
        return Some(0);
    }

    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        return Some(0);
    }

    let mut current_ino = 0u64;
    for component in trimmed.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            continue;
        }
        current_ino = dir::dir_lookup(current_ino, component)?;
    }
    Some(current_ino)
}
