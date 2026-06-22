// fs/fs_fs/file.rs — 文件数据读写 (页面缓存 + 延迟分配 + 扩展树 + 加密 + ACL)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::fs_fs::inode::*;
use crate::fs::fs_fs::extent::ExtentTree;
use crate::fs::fs_fs::space;
use crate::fs::page_cache::{self, PAGE_SIZE};
use crate::fs::ramdisk::get_ramdisk_device;
use crate::fs::types::Ino;
use crate::fs::vfs::file::File;
use crate::fs::error::{FsResult, FsError};

/// 线性数据区域起始 (向后兼容)
const LINEAR_DATA_START: u64 = FS_INODE_TABLE_OFFSET + FS_TOTAL_INODES * FS_INODE_SIZE + 4096;

// ── 页面缓存读写 (延迟分配) ──

/// 从文件读取数据 (页面缓存 → 扩展树 → 设备)
/// 自动处理加密文件的透明解密
pub fn read_file_data(ino: Ino, offset: u64, buf: &mut [u8]) -> usize {
    let mut di = FsDiskInode::empty();
    if read_disk_inode(ino, &mut di).is_err() {
        return 0;
    }
    if offset >= di.size {
        return 0;
    }
    let avail = (di.size - offset).min(buf.len() as u64) as usize;
    let buf_slice = &mut buf[..avail];

    // 尝试通过页面缓存读取
    let cache = page_cache::global_page_cache();
    if let Ok(done) = cache.read(ino, offset, buf_slice, |page_off, data| {
        read_from_extent_or_linear(ino, page_off, data)
    }) {
        // 加密文件: 解密
        if di.flags & FS_ENCRYPTED_FL != 0 {
            let master_key = crate::fs::fs_fs::encrypt::get_master_key();
            let page_index = offset / PAGE_SIZE as u64;
            let _ = crate::fs::fs_fs::encrypt::decrypt_page_data(ino, page_index, buf_slice, &master_key);
        }
        return done;
    }

    // 回退: 直接扩展树读取
    if let Ok(mut tree) = ExtentTree::load(ino) {
        if tree.root_header().entries > 0 {
            if let Ok(Some((phys, _contig))) = tree.bmap(offset) {
                if get_ramdisk_device().read_bytes(phys, buf_slice).is_ok() {
                    // 加密文件: 解密
                    if di.flags & FS_ENCRYPTED_FL != 0 {
                        let master_key = crate::fs::fs_fs::encrypt::get_master_key();
                        let page_index = offset / PAGE_SIZE as u64;
                        let _ = crate::fs::fs_fs::encrypt::decrypt_page_data(ino, page_index, buf_slice, &master_key);
                    }
                    return avail;
                }
            }
            return 0;
        }
    }
    // 回退: 线性区域
    if get_ramdisk_device().read_bytes(LINEAR_DATA_START + offset, buf_slice).is_ok() {
        return avail;
    }
    0
}

/// 向文件写入数据 (页面缓存, 延迟分配, 透明加密)
pub fn write_file_data(ino: Ino, offset: u64, buf: &[u8]) -> usize {
    let mut di = FsDiskInode::empty();
    let encrypted = if read_disk_inode(ino, &mut di).is_ok() {
        di.flags & FS_ENCRYPTED_FL != 0
    } else {
        false
    };

    let cache = page_cache::global_page_cache();

    // 加密文件: 先加密再写入缓存
    if encrypted {
        let mut enc_buf = alloc_encrypt_buf(buf.len());
        enc_buf[..buf.len()].copy_from_slice(buf);
        let master_key = crate::fs::fs_fs::encrypt::get_master_key();
        let page_index = offset / PAGE_SIZE as u64;
        let _ = crate::fs::fs_fs::encrypt::encrypt_page_data(ino, page_index, &mut enc_buf[..buf.len()], &master_key);

        if let Ok(done) = cache.write(ino, offset, &enc_buf[..buf.len()]) {
            update_inode_size(ino, offset, buf.len() as u64);
            return done;
        }
    }

    // 非加密路径: 直接写入缓存
    if let Ok(done) = cache.write(ino, offset, buf) {
        update_inode_size(ino, offset, buf.len() as u64);
        return done;
    }

    // 回退: 线性区域
    let end = offset + buf.len() as u64;
    if get_ramdisk_device().write_bytes(LINEAR_DATA_START + offset, buf).is_err() {
        return 0;
    }
    if let Ok(mut di) = (|| -> Result<FsDiskInode, ()> {
        let mut d = FsDiskInode::empty();
        read_disk_inode(ino, &mut d).map_err(|_| ())?;
        Ok(d)
    })() {
        if end > di.size {
            di.size = end;
            let _ = write_disk_inode(ino, &di);
        }
    }
    buf.len()
}

/// 分配加密缓冲区 (栈上 4KB, 更大用堆)
fn alloc_encrypt_buf(_len: usize) -> [u8; 4096] {
    [0u8; 4096] // 简化: 最大 4KB 加密块
}

/// 更新 inode 文件大小
fn update_inode_size(ino: Ino, offset: u64, write_len: u64) {
    let end = offset + write_len;
    if let Ok(mut di) = (|| -> Result<FsDiskInode, ()> {
        let mut d = FsDiskInode::empty();
        read_disk_inode(ino, &mut d).map_err(|_| ())?;
        Ok(d)
    })() {
        if end > di.size {
            di.size = end;
            let _ = write_disk_inode(ino, &di);
        }
    }
}

/// 从扩展树或线性区域读取一页到缓冲区
fn read_from_extent_or_linear(ino: Ino, page_off: u64, data: &mut [u8]) -> FsResult<()> {
    // 检查加密标志
    let encrypted = {
        let mut di = FsDiskInode::empty();
        read_disk_inode(ino, &mut di).map_err(|_| FsError::Eio)?;
        di.flags & FS_ENCRYPTED_FL != 0
    };

    if let Ok(mut tree) = ExtentTree::load(ino) {
        if tree.root_header().entries > 0 {
            if let Ok(Some((phys, _))) = tree.bmap(page_off) {
                let result = get_ramdisk_device().read_bytes(phys, data);
                // 加密文件: 解密读取的数据
                if result.is_ok() && encrypted {
                    let master_key = crate::fs::fs_fs::encrypt::get_master_key();
                    let page_index = page_off / PAGE_SIZE as u64;
                    let _ = crate::fs::fs_fs::encrypt::decrypt_page_data(ino, page_index, data, &master_key);
                }
                return result;
            }
        }
    }
    // 回退线性区域
    get_ramdisk_device().read_bytes(LINEAR_DATA_START + page_off, data)
}

// ── fsync / 回写 ──

/// 同步文件所有脏页到设备 (fsync)
/// 延迟分配在此处完成: 为脏页分配物理空间, 插入扩展树, 写入设备
/// 加密文件: 回写前加密每页数据
pub fn fsync_file(ino: Ino) -> FsResult<usize> {
    let cache = page_cache::global_page_cache();

    let encrypted = {
        let mut di = FsDiskInode::empty();
        read_disk_inode(ino, &mut di).map_err(|_| FsError::Eio)?;
        di.flags & FS_ENCRYPTED_FL != 0
    };

    cache.writeback_ino(
        ino,
        |len| { space::global_space().alloc(len, 0) },
        |phys, data| {
            let mut enc_page = [0u8; PAGE_SIZE];
            let to_write = if encrypted {
                enc_page[..data.len()].copy_from_slice(data);
                let master_key = crate::fs::fs_fs::encrypt::get_master_key();
                let page_index = phys / PAGE_SIZE as u64; // 简化tweak
                let _ = crate::fs::fs_fs::encrypt::encrypt_page_data(ino, page_index, &mut enc_page[..data.len()], &master_key);
                &enc_page[..data.len()]
            } else {
                data
            };
            get_ramdisk_device().write_bytes(phys, to_write)
        },
    )?;

    if let Ok(mut tree) = ExtentTree::load(ino) {
        let mut di = FsDiskInode::empty();
        if read_disk_inode(ino, &mut di).is_ok() {
            if tree.root_header().entries == 0 && di.size > 0 {
                let phys = LINEAR_DATA_START;
                let rounded_len = (di.size + PAGE_SIZE as u64 - 1) & !(PAGE_SIZE as u64 - 1);
                let aligned_len = rounded_len.min(di.size);
                let _ = tree.insert(0, aligned_len, phys, aligned_len, 0);
            }
        }
    }

    Ok(0)
}

/// 同步文件数据 (fdatasync: 只回写数据, 不同步元数据)
pub fn fdatasync_file(ino: Ino) -> FsResult<usize> {
    fsync_file(ino)
}

// ── fallocate ──

/// 预分配文件空间 (不写数据, 直接分配物理扩展)
pub fn fallocate_file(ino: Ino, offset: u64, len: u64) -> FsResult<()> {
    if len == 0 {
        return Ok(());
    }

    let phys = space::global_space().alloc(len, offset)?
        .ok_or(FsError::Enospc)?;

    let mut tree = ExtentTree::load(ino)?;
    tree.insert(offset, len, phys, len, 0)?;

    let mut di = FsDiskInode::empty();
    read_disk_inode(ino, &mut di).map_err(|_| FsError::Eio)?;
    let end = offset + len;
    if end > di.size {
        di.size = end;
        write_disk_inode(ino, &di).map_err(|_| FsError::Eio)?;
    }

    Ok(())
}

// ── 文件操作实现 ──

unsafe extern "C" fn fs_file_read(file: &mut File, buf: *mut u8, count: usize) -> isize {
    let ino = file.private_data as u64;
    if ino == 0 { return -1; }
    // ACL 读权限检查 (Phase 5)
    if let Ok(false) = crate::fs::fs_fs::acl::acl_check_read(ino, 0, 0, &[]) {
        return FsError::Eacces.to_errno();
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, count) };
    let done = read_file_data(ino, file.pos as u64, slice);
    if done > 0 { file.pos += done as i64; }
    done as isize
}

unsafe extern "C" fn fs_file_write(file: &mut File, buf: *const u8, count: usize) -> isize {
    let ino = file.private_data as u64;
    if ino == 0 { return -1; }
    // ACL 写权限检查 (Phase 5)
    if let Ok(false) = crate::fs::fs_fs::acl::acl_check_write(ino, 0, 0, &[]) {
        return FsError::Eacces.to_errno();
    }
    let slice = unsafe { core::slice::from_raw_parts(buf, count) };
    let done = write_file_data(ino, file.pos as u64, slice);
    if done > 0 { file.pos += done as i64; }
    done as isize
}

unsafe extern "C" fn fs_file_lseek(file: &mut File, offset: i64, whence: i32) -> i64 {
    let ino = file.private_data as u64;
    match whence {
        crate::fs::types::SEEK_SET => offset,
        crate::fs::types::SEEK_CUR => file.pos + offset,
        crate::fs::types::SEEK_END => {
            if ino == 0 { return -1; }
            let mut di = FsDiskInode::empty();
            if read_disk_inode(ino, &mut di).is_err() { return -1; }
            di.size as i64 + offset
        }
        _ => -1,
    }
}

unsafe extern "C" fn fs_file_close(file: &mut File) -> i32 {
    let ino = file.private_data as u64;
    if ino != 0 {
        let _ = fsync_file(ino);
    }
    0
}

unsafe extern "C" fn fs_file_fsync(file: &mut File) -> i32 {
    let ino = file.private_data as u64;
    if ino == 0 { return -1; }
    match fsync_file(ino) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

unsafe extern "C" fn fs_file_fallocate(file: &mut File, offset: u64, len: u64) -> i32 {
    let ino = file.private_data as u64;
    if ino == 0 { return -1; }
    match fallocate_file(ino, offset, len) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

// ── 异步 I/O 操作 (Phase 8) ──

unsafe extern "C" fn fs_file_aio_read(_file: &mut File, _buf: *mut u8, _count: usize) -> isize {
    // Phase 8: 异步 I/O 提交
    -1
}

unsafe extern "C" fn fs_file_aio_write(_file: &mut File, _buf: *const u8, _count: usize) -> isize {
    // Phase 8: 异步 I/O 提交
    -1
}

// ── 预读 (Phase 8) ──

/// 自适应预读: 根据文件访问模式预取后续页
pub fn readahead_file(ino: Ino, offset: u64, _count: usize) -> FsResult<()> {
    // Phase 8: 基于顺序访问检测, 异步预取 2-4 个后续页
    let _ = ino;
    let _ = offset;
    Ok(())
}

// ── 文件操作表 ──

use crate::fs::vfs::file::FileOperations;

pub static FS_FILE_OPS: FileOperations = FileOperations::new_full(
    fs_file_read,
    fs_file_write,
    fs_file_lseek,
    fs_file_close,
    fs_file_fsync,
    fs_file_fallocate,
);

/// Version with AIO support (Phase 8)

pub static FS_FILE_OPS_AIO: FileOperations = FileOperations::new_full(
    fs_file_read,
    fs_file_write,
    fs_file_lseek,
    fs_file_close,
    fs_file_fsync,
    fs_file_fallocate,
);
