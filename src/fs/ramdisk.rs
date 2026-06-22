// fs/ramdisk.rs — 内存段设备后端 (Ramdisk)
// 实现 SegmentDeviceOps, 提供基于字节偏移的同步读写
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later


#![allow(improper_ctypes_definitions)]

use core::cell::UnsafeCell;
use crate::fs::segment_device::*;
use crate::fs::error::{FsResult, FsError};

// ── 存储区 ──

/// Ramdisk 大小 (16 MB)
pub const RAMDISK_BYTES: u64 = 16 * 1024 * 1024;

const RAMDISK_SIZE: usize = RAMDISK_BYTES as usize;
static RAMDISK: RamdiskStorage = RamdiskStorage(UnsafeCell::new([0u8; RAMDISK_SIZE]));

struct RamdiskStorage(UnsafeCell<[u8; RAMDISK_SIZE]>);
unsafe impl Sync for RamdiskStorage {}

// ── Ramdisk 段设备名称 ──

const RAMDISK_NAME: [u8; 32] = *b"ramdisk0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

// ── 底层字节访问 (保留供 fs_fs 早期初始化使用) ──

/// 从 ramdisk 读取字节
pub fn ramdisk_read_bytes(offset: u64, buf: &mut [u8]) -> FsResult<()> {
    if offset + buf.len() as u64 > RAMDISK_BYTES {
        return Err(FsError::Einval);
    }
    let off = offset as usize;
    let len = buf.len();
    unsafe {
        let data = &*RAMDISK.0.get();
        buf.copy_from_slice(&data[off..off + len]);
    }
    Ok(())
}

/// 向 ramdisk 写入字节
pub fn ramdisk_write_bytes(offset: u64, buf: &[u8]) -> FsResult<()> {
    if offset + buf.len() as u64 > RAMDISK_BYTES {
        return Err(FsError::Einval);
    }
    let off = offset as usize;
    let len = buf.len();
    unsafe {
        let data = &mut *RAMDISK.0.get();
        data[off..off + len].copy_from_slice(buf);
    }
    Ok(())
}

/// 清零 ramdisk 区域
pub fn ramdisk_zero_bytes(offset: u64, len: u64) -> FsResult<()> {
    if offset + len > RAMDISK_BYTES {
        return Err(FsError::Einval);
    }
    let off = offset as usize;
    unsafe {
        let data = &mut *RAMDISK.0.get();
        for byte in &mut data[off..off + len as usize] {
            *byte = 0;
        }
    }
    Ok(())
}

/// 获取 ramdisk 总大小
pub fn ramdisk_size() -> u64 {
    RAMDISK_BYTES
}

// ── SegmentDeviceOps 实现 ──

/// 提交 I/O 请求到 ramdisk (同步, 直接内存复制)
unsafe extern "C" fn ramdisk_submit(
    _dev: &SegmentDevice, req: &mut SegmentRequest,
) -> FsResult<()> {
    for i in 0..req.vec_count as usize {
        let vec = &req.vecs[i];
        if vec.len == 0 {
            continue;
        }
        let _off = req.offset + req.total_bytes as u64 - req.vecs.iter()
            .skip(i)
            .map(|v| v.len)
            .sum::<usize>() as u64;
        // 计算此 vec 的绝对设备偏移
        let vec_off = req.offset + req.vecs.iter().take(i).map(|v| v.len as u64).sum::<u64>();

        match req.dir {
            IoDir::Read => {
                ramdisk_read_bytes(vec_off,
                    unsafe { core::slice::from_raw_parts_mut(vec.buf, vec.len) })?;
            }
            IoDir::Write => {
                ramdisk_write_bytes(vec_off,
                    unsafe { core::slice::from_raw_parts(vec.buf, vec.len) })?;
            }
        }
    }

    // 调用完成回调
    if let Some(cb) = req.completion {
        cb(req, 0);
    }

    Ok(())
}

unsafe extern "C" fn ramdisk_capacity(_dev: &SegmentDevice) -> u64 {
    RAMDISK_BYTES
}

unsafe extern "C" fn ramdisk_sector_size(_dev: &SegmentDevice) -> u32 {
    512
}

unsafe extern "C" fn ramdisk_flush(_dev: &SegmentDevice) -> FsResult<()> {
    // 内存盘无需刷新
    Ok(())
}

unsafe extern "C" fn ramdisk_ioctl(
    _dev: &SegmentDevice, _cmd: u32, _arg: usize,
) -> FsResult<usize> {
    Err(FsError::Enosys)
}

/// Ramdisk SegmentDeviceOps vtable
static RAMDISK_OPS: SegmentDeviceOps = SegmentDeviceOps {
    submit: ramdisk_submit,
    capacity: ramdisk_capacity,
    sector_size: ramdisk_sector_size,
    flush: ramdisk_flush,
    ioctl: ramdisk_ioctl,
};

/// 全局 Ramdisk 段设备实例
pub static RAMDISK_DEVICE: SegmentDevice = SegmentDevice::new(
    0,          // dev_id = 0
    &RAMDISK_NAME,
    &RAMDISK_OPS,
    false,      // read_only = false
);

/// 获取全局 ramdisk 设备引用
pub fn get_ramdisk_device() -> &'static SegmentDevice {
    &RAMDISK_DEVICE
}
