// fs/pipe.rs — 匿名管道（环形缓冲区）
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::vfs::file::{File, FileOperations};
use crate::fs::vfs::inode::Inode;
use crate::fs::vfs::dentry::Dentry;
use crate::sync::SpinLock;

const PIPE_BUF_SIZE: usize = 4096;
const MAX_PIPES: usize = 32;

/// 管道环形缓冲区
struct PipeInner {
    buf: [u8; PIPE_BUF_SIZE],
    head: usize,
    tail: usize,
    count: usize,
    readers: u32,
    writers: u32,
}

impl PipeInner {
    const fn new() -> Self {
        PipeInner {
            buf: [0; PIPE_BUF_SIZE],
            head: 0,
            tail: 0,
            count: 0,
            readers: 0,
            writers: 0,
        }
    }

    fn write_bytes(&mut self, data: &[u8]) -> usize {
        let mut written = 0usize;
        for &b in data {
            if self.count >= PIPE_BUF_SIZE {
                break;
            }
            self.buf[self.tail] = b;
            self.tail = (self.tail + 1) % PIPE_BUF_SIZE;
            self.count += 1;
            written += 1;
        }
        written
    }

    fn read_bytes(&mut self, out: &mut [u8]) -> usize {
        let mut n = 0usize;
        while n < out.len() && self.count > 0 {
            out[n] = self.buf[self.head];
            self.head = (self.head + 1) % PIPE_BUF_SIZE;
            self.count -= 1;
            n += 1;
        }
        n
    }
}

struct PipeSlot {
    used: bool,
    inner: PipeInner,
}

struct PipeTable {
    slots: [PipeSlot; MAX_PIPES],
}

unsafe impl Send for PipeTable {}

static PIPE_TABLE: SpinLock<PipeTable> = SpinLock::new(PipeTable {
    slots: [const {
        PipeSlot {
            used: false,
            inner: PipeInner::new(),
        }
    }; MAX_PIPES],
});

/// 分配管道槽位索引
fn alloc_pipe() -> Option<usize> {
    let mut tbl = PIPE_TABLE.lock();
    for (i, slot) in tbl.slots.iter_mut().enumerate() {
        if !slot.used {
            slot.used = true;
            slot.inner = PipeInner::new();
            slot.inner.readers = 1;
            slot.inner.writers = 1;
            return Some(i);
        }
    }
    None
}

/// 管道读操作
unsafe extern "C" fn pipe_read(file: &mut File, buf: *mut u8, count: usize) -> isize {
    let idx = file.private_data as usize;
    if idx >= MAX_PIPES {
        return -9;
    }
    let mut tbl = PIPE_TABLE.lock();
    if !tbl.slots[idx].used {
        return -9;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, count) };
    let n = tbl.slots[idx].inner.read_bytes(slice);
    n as isize
}

/// 管道写操作
unsafe extern "C" fn pipe_write(file: &mut File, buf: *const u8, count: usize) -> isize {
    let idx = file.private_data as usize;
    if idx >= MAX_PIPES {
        return -9;
    }
    let mut tbl = PIPE_TABLE.lock();
    if !tbl.slots[idx].used {
        return -9;
    }
    let slice = unsafe { core::slice::from_raw_parts(buf, count) };
    let n = tbl.slots[idx].inner.write_bytes(slice);
    n as isize
}

/// 管道定位（不支持）
unsafe extern "C" fn pipe_lseek(_file: &mut File, _offset: i64, _whence: i32) -> i64 {
    -29
}

/// 管道关闭
unsafe extern "C" fn pipe_close(file: &mut File) -> i32 {
    let idx = file.private_data as usize;
    if idx >= MAX_PIPES {
        return -9;
    }
    let mut tbl = PIPE_TABLE.lock();
    if !tbl.slots[idx].used {
        return -9;
    }
    if file.flags & 1 != 0 {
        tbl.slots[idx].inner.readers = tbl.slots[idx].inner.readers.saturating_sub(1);
    } else {
        tbl.slots[idx].inner.writers = tbl.slots[idx].inner.writers.saturating_sub(1);
    }
    if tbl.slots[idx].inner.readers == 0 && tbl.slots[idx].inner.writers == 0 {
        tbl.slots[idx].used = false;
    }
    0
}

static PIPE_FILE_OPS: FileOperations = FileOperations::new(
    pipe_read,
    pipe_write,
    pipe_lseek,
    pipe_close,
);

/// 创建管道，返回 (读端 fd, 写端 fd)
pub fn create_pipe(alloc_fd: impl Fn(File) -> i32) -> Result<(i32, i32), i32> {
    let idx = alloc_pipe().ok_or(-24)?;
    let read_file = File::new(
        -1,
        core::ptr::null_mut::<Inode>(),
        core::ptr::null_mut::<Dentry>(),
        &PIPE_FILE_OPS,
        1,
    );
    let mut read_file = read_file;
    read_file.private_data = idx as *mut ();
    let write_file = File::new(
        -1,
        core::ptr::null_mut::<Inode>(),
        core::ptr::null_mut::<Dentry>(),
        &PIPE_FILE_OPS,
        0,
    );
    let mut write_file = write_file;
    write_file.private_data = idx as *mut ();
    let rfd = alloc_fd(read_file);
    let wfd = alloc_fd(write_file);
    if rfd < 0 || wfd < 0 {
        let mut tbl = PIPE_TABLE.lock();
        tbl.slots[idx].used = false;
        return Err(-24);
    }
    Ok((rfd, wfd))
}

pub fn init() {}
