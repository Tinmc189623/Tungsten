// fs/fs_fs/journal.rs — 元数据日志 (JBD2 风格)
// 循环日志, 原子事务, 挂载时重放恢复
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use crate::fs::ramdisk::get_ramdisk_device;
use crate::fs::error::{FsResult, FsError};

// ── 日志常量 ──

/// 日志块大小
pub const JOURNAL_BLOCK_SIZE: usize = 4096;

/// 魔数
const JSB_MAGIC: u32 = 0x4A53_4246; // "JSBF" — Journal SuperBlock Format
const JDB_MAGIC: u32 = 0x4A44_4246; // "JDBF" — Journal Descriptor Block Format
const JCB_MAGIC: u32 = 0x4A43_4246; // "JCBF" — Journal Commit Block Format

/// 日志标志
const JFS_ESCAPE: u32    = 1 << 0;  // 此 tag 后的数据块是转义块 (非元数据)
const JFS_SAME_UUID: u32 = 1 << 1;  // 使用与前一个 tag 相同的 UUID
const JFS_LAST_TAG: u32  = 1 << 2;  // 描述符块中最后一个有效 tag

/// 每个描述符块最多多少个 tag (4KB - header - checksum) / tag_size
const MAX_TAGS_PER_DESC: u16 = 250;

// ── 日志超级块 ──

#[repr(C, packed)]
struct JournalSuperBlock {
    magic: u32,             // JSB_MAGIC
    block_size: u32,        // 4096
    max_blocks: u64,        // 日志区域总块数
    first_block: u64,       // 日志区域起始物理偏移 (按块对齐)
    sequence: u64,          // 当前事务序列号
    start_block: u64,       // 日志内第一个有效块索引
    head: u64,              // 下一空闲块索引 (写指针)
    tail: u64,              // 最旧有效事务起始块索引
    flags: u32,             // 日志标志
    errno: u32,             // 最后的错误码
    checksum: u32,          // CRC32c
}

impl JournalSuperBlock {
    const fn empty() -> Self {
        JournalSuperBlock {
            magic: 0, block_size: 0, max_blocks: 0, first_block: 0,
            sequence: 0, start_block: 0, head: 0, tail: 0,
            flags: 0, errno: 0, checksum: 0,
        }
    }
}

// ── 描述符块 ──

#[repr(C, packed)]
struct JournalDescriptorBlock {
    magic: u32,             // JDB_MAGIC
    sequence: u64,          // 事务序列号
    tag_count: u16,         // 有效 tag 数量
    flags: u8,              // 标志
    checksum: u32,          // CRC32c
}

// ── 块标签 ──

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct JournalBlockTag {
    /// 此元数据的物理设备偏移
    block_offset: u64,
    /// 标志 (JFS_*)
    flags: u32,
}

// ── 提交块 ──

#[repr(C, packed)]
struct JournalCommitBlock {
    magic: u32,             // JCB_MAGIC
    sequence: u64,          // 事务序列号
    checksum: u32,          // CRC32c
}

// ── 事务上下文 ──

/// 正在构建的事务 (最多缓存 64 个元数据块)
const MAX_TX_BLOCKS: usize = 64;

struct Transaction {
    sequence: u64,
    /// 每个条目: (device_physical_offset, [u8; 4096])
    blocks: [(u64, [u8; JOURNAL_BLOCK_SIZE]); MAX_TX_BLOCKS],
    count: usize,
    committed: bool,
}

impl Transaction {
    const fn new() -> Self {
        const EMPTY: (u64, [u8; JOURNAL_BLOCK_SIZE]) = (0, [0u8; JOURNAL_BLOCK_SIZE]);
        Transaction {
            sequence: 0,
            blocks: [EMPTY; MAX_TX_BLOCKS],
            count: 0,
            committed: false,
        }
    }

    /// 添加一个要日志记录的元数据块
    fn add_block(&mut self, device_offset: u64, data: &[u8]) -> FsResult<()> {
        if self.count >= MAX_TX_BLOCKS {
            return Err(FsError::Enospc);
        }
        let aligned = device_offset & !(JOURNAL_BLOCK_SIZE as u64 - 1);
        let mut buf = [0u8; JOURNAL_BLOCK_SIZE];
        let copy_len = data.len().min(JOURNAL_BLOCK_SIZE);
        buf[..copy_len].copy_from_slice(&data[..copy_len]);
        self.blocks[self.count] = (aligned, buf);
        self.count += 1;
        Ok(())
    }
}

// ── 日志管理器 ──

pub struct Journal {
    /// 日志区域起始物理偏移
    first_block: u64,
    /// 日志总块数
    max_blocks: u64,
    /// 当前序列号
    sequence: u64,
    /// 写指针 (日志内块索引)
    head: u64,
    /// 最旧有效事务起始
    tail: u64,
    /// 内部块缓冲区 (读写用)
    buf: [u8; JOURNAL_BLOCK_SIZE],
    /// 活跃事务
    active_tx: Transaction,
    /// 是否初始化
    ready: bool,
}

impl Journal {
    pub const fn new() -> Self {
        Journal {
            first_block: 0, max_blocks: 0, sequence: 0, head: 0, tail: 0,
            buf: [0u8; JOURNAL_BLOCK_SIZE],
            active_tx: Transaction::new(),
            ready: false,
        }
    }

    /// 从日志超级块加载已有日志
    pub fn load(&mut self, j_offset: u64, j_bytes: u64) -> FsResult<()> {
        if j_offset == 0 || j_bytes < JOURNAL_BLOCK_SIZE as u64 * 3 {
            return Err(FsError::Einval);
        }

        self.first_block = j_offset;
        self.max_blocks = j_bytes / JOURNAL_BLOCK_SIZE as u64;
        self.head = 1; // 块 0 = 日志超级块
        self.tail = 1;
        self.ready = true;

        // 读取日志超级块
        get_ramdisk_device().read_bytes(j_offset, &mut self.buf)?;
        let jsb: JournalSuperBlock = unsafe {
            core::ptr::read_unaligned(self.buf.as_ptr() as *const JournalSuperBlock)
        };

        if jsb.magic == JSB_MAGIC {
            self.sequence = jsb.sequence;
            self.head = jsb.head;
            self.tail = jsb.tail;
            if self.head == 0 || self.head >= self.max_blocks {
                self.head = 1;
            }
            if self.tail == 0 || self.tail >= self.max_blocks {
                self.tail = 1;
            }
        } else {
            // 初始化日志超级块
            self.sequence = 1;
            self.write_jsb()?;
        }

        crate::serial::write_str(b"  journal: loaded seq=");
        crate::serial_put_u64(self.sequence);
        crate::serial::write_str(b"\n");
        Ok(())
    }

    /// 初始化新日志 (格式化时调用)
    pub fn init_new(&mut self, j_offset: u64, j_bytes: u64) -> FsResult<()> {
        self.first_block = j_offset;
        self.max_blocks = j_bytes / JOURNAL_BLOCK_SIZE as u64;
        self.head = 1;
        self.tail = 1;
        self.sequence = 1;
        self.ready = true;

        // 清零日志区域
        let _total_bytes = self.max_blocks * JOURNAL_BLOCK_SIZE as u64;
        let zero = [0u8; JOURNAL_BLOCK_SIZE];
        for i in 0..self.max_blocks.min(64) {
            let _ = get_ramdisk_device().write_bytes(
                j_offset + i * JOURNAL_BLOCK_SIZE as u64, &zero,
            );
        }

        self.write_jsb()?;
        crate::serial::write_str(b"  journal: init new, blocks=");
        crate::serial_put_u64(self.max_blocks);
        crate::serial::write_str(b"\n");
        Ok(())
    }

    // ── 事务操作 ──

    /// 开始新事务
    pub fn start_transaction(&mut self) -> FsResult<u64> {
        if !self.ready {
            return Err(FsError::Eio);
        }
        self.active_tx = Transaction::new();
        self.active_tx.sequence = self.sequence;
        Ok(self.sequence)
    }

    /// 向当前事务添加一个元数据块
    pub fn journal_metadata(&mut self, device_offset: u64, data: &[u8]) -> FsResult<()> {
        if self.active_tx.committed {
            return Err(FsError::Einval);
        }
        // 先写数据到日志区域
        let block_idx = self.head;
        let write_off = self.first_block + block_idx * JOURNAL_BLOCK_SIZE as u64;
        get_ramdisk_device().write_bytes(write_off, data)?;

        self.active_tx.add_block(device_offset, data)?;
        self.head = (self.head + 1) % self.max_blocks;
        if self.head == 0 {
            self.head = 1; // 块 0 保留给超级块
        }

        // 日志几乎满 → 先做检查点
        if self.head == self.tail || self.free_blocks() < 8 {
            self.checkpoint()?;
        }

        Ok(())
    }

    /// 提交当前事务 (原子: 写描述符块 + 数据块 + 提交块)
    pub fn commit_transaction(&mut self) -> FsResult<()> {
        if self.active_tx.count == 0 {
            self.active_tx.committed = true;
            return Ok(());
        }

        // 写描述符块
        let desc_block_idx = self.head;
        let desc_phys = self.first_block + desc_block_idx * JOURNAL_BLOCK_SIZE as u64;
        self.head = (self.head + 1) % self.max_blocks;
        if self.head == 0 { self.head = 1; }

        // 构建描述符块
        self.buf.fill(0);
        let desc = JournalDescriptorBlock {
            magic: JDB_MAGIC,
            sequence: self.sequence,
            tag_count: self.active_tx.count as u16,
            flags: 0,
            checksum: 0,
        };
        unsafe {
            core::ptr::write_unaligned(self.buf.as_mut_ptr() as *mut JournalDescriptorBlock, desc);
        }
        // 写入 tags
        let tag_offset = core::mem::size_of::<JournalDescriptorBlock>();
        for i in 0..self.active_tx.count {
            let mut flags = 0u32;
            if i == self.active_tx.count - 1 {
                flags |= JFS_LAST_TAG;
            }
            let tag = JournalBlockTag {
                block_offset: self.active_tx.blocks[i].0,
                flags,
            };
            let tag_off = tag_offset + i * core::mem::size_of::<JournalBlockTag>();
            unsafe {
                core::ptr::write_unaligned(
                    self.buf.as_mut_ptr().add(tag_off) as *mut JournalBlockTag, tag,
                );
            }
        }
        get_ramdisk_device().write_bytes(desc_phys, &self.buf)?;

        // 写数据块 (在 journal_metadata 中已写入, 此处仅更新指针)
        // 写提交块
        let commit_block_idx = self.head;
        let commit_phys = self.first_block + commit_block_idx * JOURNAL_BLOCK_SIZE as u64;
        self.head = (self.head + 1) % self.max_blocks;
        if self.head == 0 { self.head = 1; }

        self.buf.fill(0);
        let commit = JournalCommitBlock {
            magic: JCB_MAGIC,
            sequence: self.sequence,
            checksum: 0,
        };
        unsafe {
            core::ptr::write_unaligned(self.buf.as_mut_ptr() as *mut JournalCommitBlock, commit);
        }
        get_ramdisk_device().write_bytes(commit_phys, &self.buf)?;

        self.sequence += 1;
        self.active_tx.committed = true;

        // 更新日志超级块
        self.write_jsb()?;

        // 执行检查点: 将日志数据复制到真实位置
        self.checkpoint()?;

        Ok(())
    }

    // ── 检查点 ──

    /// 检查点: 将日志中的数据写回真实位置
    fn checkpoint(&mut self) -> FsResult<()> {
        // 将 active_tx 中的所有块写回它们的原始位置
        for i in 0..self.active_tx.count {
            let (device_off, ref data) = self.active_tx.blocks[i];
            get_ramdisk_device().write_bytes(device_off, data)?;
        }
        // 更新 tail
        self.tail = self.head;
        self.write_jsb()?;
        Ok(())
    }

    // ── 辅助 ──

    fn free_blocks(&self) -> u64 {
        if self.head >= self.tail {
            self.max_blocks - (self.head - self.tail)
        } else {
            self.tail - self.head
        }
    }

    fn write_jsb(&mut self) -> FsResult<()> {
        self.buf.fill(0);
        let jsb = JournalSuperBlock {
            magic: JSB_MAGIC,
            block_size: JOURNAL_BLOCK_SIZE as u32,
            max_blocks: self.max_blocks,
            first_block: self.first_block,
            sequence: self.sequence,
            start_block: self.tail,
            head: self.head,
            tail: self.tail,
            flags: 0,
            errno: 0,
            checksum: 0,
        };
        unsafe {
            core::ptr::write_unaligned(self.buf.as_mut_ptr() as *mut JournalSuperBlock, jsb);
        }
        get_ramdisk_device().write_bytes(self.first_block, &self.buf)
    }

    // ── 恢复 ──

    /// 日志重放: 扫描并重放已提交的事务
    pub fn replay(&mut self) -> FsResult<usize> {
        if !self.ready {
            return Err(FsError::Eio);
        }
        // 重新读取超级块
        get_ramdisk_device().read_bytes(self.first_block, &mut self.buf)?;
        let jsb: JournalSuperBlock = unsafe {
            core::ptr::read_unaligned(self.buf.as_ptr() as *const JournalSuperBlock)
        };

        if jsb.magic != JSB_MAGIC || jsb.tail == jsb.head {
            crate::serial::write_str(b"  journal: clean, no replay needed\n");
            return Ok(0);
        }

        crate::serial::write_str(b"  journal: replaying...\n");
        let mut replayed = 0usize;

        // 从 tail 扫描到 head
        let mut pos = jsb.tail;
        while pos != jsb.head {
            let block_phys = self.first_block + pos * JOURNAL_BLOCK_SIZE as u64;
            if get_ramdisk_device().read_bytes(block_phys, &mut self.buf).is_err() {
                break;
            }

            let desc: JournalDescriptorBlock = unsafe {
                core::ptr::read_unaligned(self.buf.as_ptr() as *const JournalDescriptorBlock)
            };

            if desc.magic == JDB_MAGIC {
                let tag_off = core::mem::size_of::<JournalDescriptorBlock>();
                // 读取 tags 并重放数据块
                for i in 0..desc.tag_count as usize {
                    let t_off = tag_off + i * core::mem::size_of::<JournalBlockTag>();
                    let tag: JournalBlockTag = unsafe {
                        core::ptr::read_unaligned(
                            self.buf.as_ptr().add(t_off) as *const JournalBlockTag,
                        )
                    };
                    // 数据块紧接描述符块之后
                    pos = (pos + 1) % self.max_blocks;
                    if pos == 0 { pos = 1; }
                    let data_phys = self.first_block + pos * JOURNAL_BLOCK_SIZE as u64;

                    let mut data_buf = [0u8; JOURNAL_BLOCK_SIZE];
                    if get_ramdisk_device().read_bytes(data_phys, &mut data_buf).is_ok() {
                        // 写回真实位置
                        let _ = get_ramdisk_device().write_bytes(tag.block_offset, &data_buf);
                        replayed += 1;
                    }

                    if tag.flags & JFS_LAST_TAG != 0 {
                        break;
                    }
                }
            }

            pos = (pos + 1) % self.max_blocks;
            if pos == 0 { pos = 1; }
            // 安全检查
            if replayed > 10000 { break; }
        }

        // 重置日志
        self.sequence = jsb.sequence + 1;
        self.head = 1;
        self.tail = 1;
        self.write_jsb()?;

        crate::serial::write_str(b"  journal: replayed ");
        crate::serial_put_u64(replayed as u64);
        crate::serial::write_str(b" blocks\n");
        Ok(replayed)
    }
}

// ── 全局日志实例 ──

use core::cell::UnsafeCell;

struct JournalWrapper(UnsafeCell<Journal>);
unsafe impl Sync for JournalWrapper {}

static GLOBAL_JOURNAL: JournalWrapper = JournalWrapper(UnsafeCell::new(Journal::new()));

pub fn global_journal() -> &'static mut Journal {
    unsafe { &mut *GLOBAL_JOURNAL.0.get() }
}

/// 初始化全局日志
pub fn init_journal(j_offset: u64, j_bytes: u64) -> FsResult<()> {
    global_journal().load(j_offset, j_bytes)
}

/// 初始化新日志 (格式化时)
pub fn init_new_journal(j_offset: u64, j_bytes: u64) -> FsResult<()> {
    global_journal().init_new(j_offset, j_bytes)
}

/// 日志重放
pub fn journal_replay() -> FsResult<usize> {
    global_journal().replay()
}
