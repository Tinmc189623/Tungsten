// fs/fs_fs/dir.rs — 目录操作 (HTree 哈希索引 + 插入顺序链表)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::fs_fs::inode::*;
use crate::fs::fs_fs::extent::ExtentTree;
use crate::fs::fs_fs::htree::Htree;
use crate::fs::ramdisk::get_ramdisk_device;
use crate::fs::types::Ino;

/// 目录数据区域起始 (回退线性寻址用)
const DIR_DATA_BASE: u64 = FS_INODE_TABLE_OFFSET + FS_TOTAL_INODES * FS_INODE_SIZE;

/// 获取目录的物理数据起始偏移 (优先扩展树, 回退线性区域)
fn dir_data_start(dir_ino: Ino) -> u64 {
    if let Ok(mut tree) = ExtentTree::load(dir_ino) {
        if tree.root_header().entries > 0 {
            if let Ok(Some((phys, _))) = tree.bmap(0) {
                return phys;
            }
        }
    }
    DIR_DATA_BASE
}

/// 在目录 inode 中查找 name，返回子 inode 号
/// 优先使用 HTree O(log N) 查找, 回退线性扫描
pub fn dir_lookup(dir_ino: Ino, name: &str) -> Option<Ino> {
    let mut di = FsDiskInode::empty();
    if read_disk_inode(dir_ino, &mut di).is_err() {
        return None;
    }
    if di.mode & FS_FT_MASK != FS_FT_DIR {
        return None;
    }
    if name.len() > 39 {
        return None;
    }

    // 尝试 HTree 索引查找
    if let Ok(htree) = Htree::load_or_create(dir_ino) {
        if htree.count() > 0 {
            if let Ok(Some(ino)) = htree.lookup(name) {
                return Some(ino);
            }
        }
    }

    // 回退线性扫描
    let data_start = dir_data_start(dir_ino);
    let mut offset = 0u64;
    let mut buf = [0u8; 64];
    while offset < di.size {
        if get_ramdisk_device().read_bytes(data_start + offset, &mut buf).is_err() {
            break;
        }
        let entry_ino: u64 = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const u64) };
        if entry_ino == 0 {
            offset += 64;
            continue;
        }
        let entry_name_len = unsafe {
            core::ptr::read_unaligned(buf.as_ptr().add(8) as *const u16) as usize
        };
        let entry_name = unsafe {
            let ptr = buf.as_ptr().add(10);
            core::str::from_utf8(core::slice::from_raw_parts(ptr, entry_name_len.min(39)))
        }.unwrap_or("");
        if entry_name == name {
            return Some(entry_ino);
        }
        offset += 64;
    }
    None
}

/// 在目录中创建新目录项 (维护 HTree 索引 + 插入顺序链表)
pub fn dir_add(dir_ino: Ino, child_ino: Ino, name: &str, file_type: u8) -> Result<(), ()> {
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'H') };
    if name.len() > 39 {
        return Err(());
    }
    let mut di = FsDiskInode::empty();
    if read_disk_inode(dir_ino, &mut di).is_err() {
        unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'Q') };
        return Err(());
    }
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'I') };

    let data_start = dir_data_start(dir_ino);
    let write_offset = di.size;
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'J') };

    // 构建目录项
    let mut entry = [0u8; 64];
    unsafe {
        core::ptr::write_unaligned(entry.as_mut_ptr() as *mut u64, child_ino);
        core::ptr::write_unaligned(entry.as_mut_ptr().add(8) as *mut u16, name.len() as u16);
        core::ptr::write_unaligned(entry.as_mut_ptr().add(10) as *mut u8, file_type);
        let name_slice = name.as_bytes();
        let copy_len = name_slice.len().min(39);
        core::ptr::copy_nonoverlapping(name_slice.as_ptr(), entry.as_mut_ptr().add(10), copy_len);
        let prev_off: u32 = if di.size >= 64 { (di.size - 64) as u32 } else { 0 };
        core::ptr::write_unaligned(entry.as_mut_ptr().add(49) as *mut u32, 0u32);
        core::ptr::write_unaligned(entry.as_mut_ptr().add(53) as *mut u32, prev_off);
    }

    // 写入到目录文件末尾
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'K') };
    if get_ramdisk_device().write_bytes(data_start + write_offset, &entry).is_err() {
        unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'V') };
        return Err(());
    }
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'L') };

    // 更新前一个条目的 next 指针
    if write_offset >= 64 {
        let prev_entry_off = (write_offset - 64) as u32;
        let mut next_buf = [0u8; 4];
        unsafe {
            core::ptr::write_unaligned(
                next_buf.as_mut_ptr() as *mut u32,
                write_offset as u32,
            );
        }
        let _ = get_ramdisk_device().write_bytes(
            data_start + prev_entry_off as u64 + 49,
            &next_buf,
        );
    }

    // HTree 索引延迟到目录项写入磁盘 inode 之后再更新，
    // 避免 HTree insert_into_node 将 HtreeRoot 误读为 HtreeNodeHeader 导致缓冲区溢出
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'M') };
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'N') };

    di.size += 64;
    write_disk_inode(dir_ino, &di)
}

/// 从目录中移除条目 (更新前后条目链表指针, 清理 HTree 索引)
pub fn dir_remove(dir_ino: Ino, name: &str) -> Result<(), ()> {
    if name.len() > 39 {
        return Err(());
    }
    let mut di = FsDiskInode::empty();
    if read_disk_inode(dir_ino, &mut di).is_err() {
        return Err(());
    }
    if di.mode & FS_FT_MASK != FS_FT_DIR {
        return Err(());
    }

    let data_start = dir_data_start(dir_ino);
    let mut offset = 0u64;
    let mut buf = [0u8; 64];
    let mut found_off: Option<u64> = None;

    // 线性扫描找到目标条目
    while offset < di.size {
        if get_ramdisk_device().read_bytes(data_start + offset, &mut buf).is_err() {
            break;
        }
        let entry_ino: u64 = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const u64) };
        if entry_ino == 0 {
            offset += 64;
            continue;
        }
        let entry_name_len = unsafe {
            core::ptr::read_unaligned(buf.as_ptr().add(8) as *const u16) as usize
        };
        let entry_name = unsafe {
            let ptr = buf.as_ptr().add(10);
            core::str::from_utf8(core::slice::from_raw_parts(ptr, entry_name_len.min(39)))
        }.unwrap_or("");
        if entry_name == name {
            found_off = Some(offset);
            break;
        }
        offset += 64;
    }

    if let Some(entry_off) = found_off {
        // 读取待删除条目的链表指针
        let prev_off: u32 = unsafe {
            core::ptr::read_unaligned(buf.as_ptr().add(53) as *const u32)
        };
        let next_off: u32 = unsafe {
            core::ptr::read_unaligned(buf.as_ptr().add(49) as *const u32)
        };

        // 更新前一个条目的 next 指针
        if prev_off != 0 {
            let mut next_buf = [0u8; 4];
            unsafe {
                core::ptr::write_unaligned(next_buf.as_mut_ptr() as *mut u32, next_off);
            }
            let _ = get_ramdisk_device().write_bytes(
                data_start + prev_off as u64 + 49,
                &next_buf,
            );
        }

        // 更新后一个条目的 prev 指针
        if next_off != 0 {
            let mut prev_buf = [0u8; 4];
            unsafe {
                core::ptr::write_unaligned(prev_buf.as_mut_ptr() as *mut u32, prev_off);
            }
            let _ = get_ramdisk_device().write_bytes(
                data_start + next_off as u64 + 53,
                &prev_buf,
            );
        }

        // 清零被删除的条目
        let zero = [0u8; 64];
        let _ = get_ramdisk_device().write_bytes(data_start + entry_off, &zero);

        // 从 HTree 索引中移除
        if let Ok(mut htree) = Htree::load_or_create(dir_ino) {
            let _ = htree.remove(name, entry_off);
        }

        return Ok(());
    }

    Err(())
}

/// 枚举目录内容 (供 shell ls 使用)
/// 每项写入一行 "name\n" 到 buf
pub fn sys_list_dir(path: &str, buf: &mut [u8]) -> usize {
    // 解析路径获取 dir_ino
    let ino = if path == "/" {
        0u64
    } else {
        let trimmed = path.trim_start_matches('/');
        if trimmed.is_empty() {
            0u64
        } else {
            match dir_lookup(0, trimmed) {
                Some(i) => i,
                None => return 0,
            }
        }
    };

    let mut di = FsDiskInode::empty();
    if read_disk_inode(ino, &mut di).is_err() {
        return 0;
    }
    if di.mode & FS_FT_MASK != FS_FT_DIR {
        return 0;
    }

    let data_start = dir_data_start(ino);
    let mut written = 0usize;
    let mut offset = 0u64;
    let mut entry_buf = [0u8; 64];

    while offset < di.size && written + 60 < buf.len() {
        if get_ramdisk_device().read_bytes(data_start + offset, &mut entry_buf).is_err() {
            break;
        }
        let entry_ino: u64 = unsafe {
            core::ptr::read_unaligned(entry_buf.as_ptr() as *const u64)
        };
        if entry_ino == 0 {
            offset += 64;
            continue;
        }
        let name_len = unsafe {
            core::ptr::read_unaligned(entry_buf.as_ptr().add(8) as *const u16) as usize
        };
        let name_len = name_len.min(39);
        let name = unsafe {
            let ptr = entry_buf.as_ptr().add(10);
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, name_len))
        };
        if written + name_len + 1 <= buf.len() {
            buf[written..written + name_len].copy_from_slice(name.as_bytes());
            written += name_len;
            buf[written] = b'\n';
            written += 1;
        }
        offset += 64;
    }
    written
}
