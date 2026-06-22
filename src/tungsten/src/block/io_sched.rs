// block/io_sched.rs — I/O 调度器 (Noop)
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{block_read_sectors, block_write_sectors};

/// 调度读请求（当前为直通 Noop）
pub fn submit_read(dev: usize, lba: u64, count: u32, buf: &mut [u8]) -> i32 {
    block_read_sectors(dev, lba, count, buf)
}

/// 调度写请求
pub fn submit_write(dev: usize, lba: u64, count: u32, buf: &[u8]) -> i32 {
    block_write_sectors(dev, lba, count, buf)
}

pub fn init() {
    crate::serial::write_str(b"  block: io_sched noop ready\n");
}

pub fn probe() {}
