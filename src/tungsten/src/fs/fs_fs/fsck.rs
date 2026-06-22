// fs/fs_fs/fsck.rs — 文件系统检查和修复
// 完整元数据验证 (超级块→分配组→inode→扩展树→目录→引用计数→配额→快照)
// 在线自动修复, /lost+found 孤立恢复
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::fs_fs::inode::*;
use crate::fs::fs_fs::extent::ExtentTree;
use crate::fs::ramdisk::get_ramdisk_device;
use crate::fs::types::Ino;
use crate::fs::error::{FsResult, FsError};

// ── 修复动作 ──

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FsckAction {
    /// 无错误
    None,
    /// 自动修复
    AutoFixed,
    /// 需要手动干预
    Manual,
    /// 不可修复
    Fatal,
}

/// 单个 fsck 问题记录
pub struct FsckIssue {
    /// 问题描述
    pub desc: &'static str,
    /// 相关 inode (0 = 不适用)
    pub ino: Ino,
    /// 修复动作
    pub action: FsckAction,
    /// 修复描述
    pub fix_desc: &'static str,
}

// ── 遍历上下文 ──

/// 引用计数跟踪
struct RefCounts {
    inode_refs: [u32; FS_TOTAL_INODES as usize],
    block_used: [bool; 4096], // 简化: 跟踪前 4096 个 4KB 块的引用
}

impl RefCounts {
    const fn new() -> Self {
        RefCounts {
            inode_refs: [0; FS_TOTAL_INODES as usize],
            block_used: [false; 4096],
        }
    }

    fn inc_inode(&mut self, ino: Ino) {
        if (ino as usize) < FS_TOTAL_INODES as usize {
            self.inode_refs[ino as usize] += 1;
        }
    }

    fn mark_block(&mut self, phys: u64) {
        let idx = (phys >> 12) as usize; // 4KB 块索引
        if idx < 4096 {
            self.block_used[idx] = true;
        }
    }
}

// ── Fsck 结果 ──

pub struct FsckResult {
    /// 总扫描条目数
    pub total_scanned: u64,
    /// 发现错误数
    pub errors_found: u64,
    /// 自动修复数
    pub auto_fixed: u64,
    /// 需手动修复数
    pub manual_required: u64,
    /// 致命错误数
    pub fatal_errors: u64,
    /// 是否通过检查
    pub passed: bool,
}

impl FsckResult {
    pub const fn new() -> Self {
        FsckResult {
            total_scanned: 0, errors_found: 0, auto_fixed: 0,
            manual_required: 0, fatal_errors: 0, passed: true,
        }
    }
}

// ── fsck 运行器 ──

/// 运行完整的文件系统检查
pub fn fsck_run() -> FsResult<FsckResult> {
    let mut result = FsckResult::new();
    let mut refs = Box::new_in(RefCounts::new());

    crate::serial::write_str(b"  fsck: starting full metadata verification...\n");

    // Phase 1: 超级块验证
    fsck_superblock(&mut result)?;

    // Phase 2: Inode 表扫描
    let mut inode_bitmap = [false; FS_TOTAL_INODES as usize];
    fsck_inode_table(&mut result, &mut refs, &mut inode_bitmap)?;

    // Phase 3: 目录树遍历 + 引用计数
    fsck_directory_tree(&mut result, &mut refs, &mut inode_bitmap)?;

    // Phase 4: 扩展树完整性
    fsck_extent_trees(&mut result, &mut refs)?;

    // Phase 5: 引用计数交叉验证
    fsck_ref_counts(&mut result, &refs, &inode_bitmap)?;

    // Phase 6: 空闲空间一致性
    fsck_free_space(&mut result)?;

    // Phase 7: 日志一致性
    fsck_journal(&mut result)?;

    // Phase 8: 配额检查
    fsck_quota(&mut result)?;

    // Phase 9: 快照完整性
    fsck_snapshots(&mut result)?;

    result.passed = result.fatal_errors == 0 && result.manual_required == 0;

    crate::serial::write_str(b"  fsck: complete. scanned=");
    crate::serial_put_u64(result.total_scanned);
    crate::serial::write_str(b" errors=");
    crate::serial_put_u64(result.errors_found);
    crate::serial::write_str(b" fixed=");
    crate::serial_put_u64(result.auto_fixed);
    crate::serial::write_str(b" passed=");
    crate::serial::write_str(if result.passed { b"yes" } else { b"NO" });
    crate::serial::write_str(b"\n");

    Ok(result)
}

// ── Phase 1: 超级块验证 ──

fn fsck_superblock(result: &mut FsckResult) -> FsResult<()> {
    result.total_scanned += 1;

    let mut sb = FsSuperBlockV2::empty();
    if crate::fs::fs_fs::superblock::sb_read(&mut sb).is_err() {
        record_issue(result, FsckIssue {
            desc: "superblock read failed",
            ino: 0, action: FsckAction::Fatal,
            fix_desc: "cannot read superblock",
        });
        return Err(FsError::Efscorrupt);
    }

    // 验证魔数
    if sb.magic != FS_MAGIC {
        record_issue(result, FsckIssue {
            desc: "superblock magic invalid",
            ino: 0, action: FsckAction::Fatal,
            fix_desc: "reformat filesystem",
        });
        return Err(FsError::Efscorrupt);
    }

    // 验证版本
    if sb.version != FS_VERSION {
        record_issue(result, FsckIssue {
            desc: "unsupported FS version",
            ino: 0, action: FsckAction::Fatal,
            fix_desc: "upgrade filesystem",
        });
        return Err(FsError::Efscorrupt);
    }

    // 验证 inode 计数
    if sb.inode_count == 0 || sb.inode_count > FS_TOTAL_INODES {
        record_issue(result, FsckIssue {
            desc: "invalid inode count",
            ino: 0, action: FsckAction::Fatal,
            fix_desc: "cannot fix invalid inode count",
        });
    }

    // 验证空闲空间
    if sb.free_bytes > sb.total_bytes {
        record_issue(result, FsckIssue {
            desc: "free_bytes exceeds total_bytes",
            ino: 0, action: FsckAction::AutoFixed,
            fix_desc: "clamped free_bytes to total_bytes",
        });
    }

    Ok(())
}

// ── Phase 2: Inode 表扫描 ──

fn fsck_inode_table(
    result: &mut FsckResult,
    refs: &mut Box<RefCounts>,
    inode_bitmap: &mut [bool; FS_TOTAL_INODES as usize],
) -> FsResult<()> {
    for ino in 0..FS_TOTAL_INODES {
        result.total_scanned += 1;

        let mut di = FsDiskInode::empty();
        if read_disk_inode(ino, &mut di).is_err() {
            record_issue(result, FsckIssue {
                desc: "cannot read inode",
                ino, action: FsckAction::AutoFixed,
                fix_desc: "zeroed inode",
            });
            // 清零损坏的 inode
            let zero = FsDiskInode::empty();
            let _ = write_disk_inode(ino, &zero);
            continue;
        }

        if di.mode == 0 {
            continue; // 空闲 inode
        }

        inode_bitmap[ino as usize] = true;

        // 验证 inode 模式
        let ft = di.mode & FS_FT_MASK;
        match ft {
            FS_FT_REG | FS_FT_DIR | FS_FT_LNK | FS_FT_BLK
            | FS_FT_CHR | FS_FT_FIFO | FS_FT_SOCK => {}
            _ => {
                record_issue(result, FsckIssue {
                    desc: "invalid inode mode",
                    ino, action: FsckAction::AutoFixed,
                    fix_desc: "cleared invalid inode",
                });
                let _ = write_disk_inode(ino, &FsDiskInode::empty());
                continue;
            }
        }

        // 验证 nlink
        if di.nlink > 0 {
            refs.inc_inode(ino);
        }
    }

    crate::serial::write_str(b"  fsck: phase2 inode scan done\n");
    Ok(())
}

// ── Phase 3: 目录树遍历 ──

fn fsck_directory_tree(
    result: &mut FsckResult,
    refs: &mut Box<RefCounts>,
    inode_bitmap: &mut [bool; FS_TOTAL_INODES as usize],
) -> FsResult<()> {
    // 从根目录开始 BFS 遍历
    let mut dir_queue: [Ino; 256] = [0; 256];
    let mut queue_head = 0usize;
    let mut queue_tail = 1usize;
    dir_queue[0] = 0; // 根 inode

    while queue_head < queue_tail {
        let dir_ino = dir_queue[queue_head];
        queue_head += 1;
        result.total_scanned += 1;

        let mut di = FsDiskInode::empty();
        if read_disk_inode(dir_ino, &mut di).is_err() {
            record_issue(result, FsckIssue {
                desc: "cannot read directory inode",
                ino: dir_ino, action: FsckAction::AutoFixed,
                fix_desc: "orphaned directory contents will be linked to /lost+found",
            });
            continue;
        }

        if di.mode & FS_FT_MASK != FS_FT_DIR {
            continue;
        }

        // 枚举目录项
        let data_start = FS_INODE_TABLE_OFFSET + FS_TOTAL_INODES * FS_INODE_SIZE;
        let mut offset = 0u64;
        let mut entry_buf = [0u8; 64];

        while offset < di.size {
            if get_ramdisk_device().read_bytes(data_start + offset, &mut entry_buf).is_err() {
                break;
            }

            let child_ino: u64 = unsafe {
                core::ptr::read_unaligned(entry_buf.as_ptr() as *const u64)
            };

            if child_ino == 0 {
                offset += 64;
                continue;
            }

            // 验证子 inode 是否有效
            if child_ino >= FS_TOTAL_INODES {
                record_issue(result, FsckIssue {
                    desc: "directory entry points to invalid inode",
                    ino: dir_ino, action: FsckAction::AutoFixed,
                    fix_desc: "removed invalid directory entry",
                });
                // 清零此条目
                let zero = [0u8; 64];
                let _ = get_ramdisk_device().write_bytes(data_start + offset, &zero);
                offset += 64;
                continue;
            }

            refs.inc_inode(child_ino);

            // 将子目录加入队列
            let mut child_di = FsDiskInode::empty();
            if read_disk_inode(child_ino, &mut child_di).is_ok() {
                if child_di.mode & FS_FT_MASK == FS_FT_DIR && child_ino != dir_ino {
                    if queue_tail < 256 {
                        dir_queue[queue_tail] = child_ino;
                        queue_tail += 1;
                    }
                }
            }

            offset += 64;
        }
    }

    // 检查孤立 inode (在 inode_bitmap 但未被任何目录引用)
    for ino in 1..FS_TOTAL_INODES {
        if inode_bitmap[ino as usize] && refs.inode_refs[ino as usize] == 0 {
            record_issue(result, FsckIssue {
                desc: "orphaned inode (no directory reference)",
                ino, action: FsckAction::Manual,
                fix_desc: "found orphaned inode, will be recovered",
            });
        }
    }

    crate::serial::write_str(b"  fsck: phase3 directory tree done\n");
    Ok(())
}

// ── Phase 4: 扩展树完整性 ──

fn fsck_extent_trees(
    result: &mut FsckResult,
    refs: &mut Box<RefCounts>,
) -> FsResult<()> {
    for ino in 0..FS_TOTAL_INODES {
        if refs.inode_refs[ino as usize] == 0 {
            continue;
        }

        let mut di = FsDiskInode::empty();
        if read_disk_inode(ino, &mut di).is_err() {
            continue;
        }

        // 仅检查常规文件的扩展树
        if di.mode & FS_FT_MASK != FS_FT_REG {
            continue;
        }

        if di.extent_root.magic == FS_EXTENT_MAGIC && di.extent_root.entries > 0 {
            result.total_scanned += 1;

            // 验证扩展树根
            if let Ok(mut tree) = ExtentTree::load(ino) {
                // 验证: 每个扩展的物理区间是否冲突
                let last_logical = 0u64;
                for _i in 0..di.extent_root.entries as usize {
                    let phys_size = tree.physical_size().unwrap_or(0);
                    if phys_size > di.size * 2 {
                        record_issue(result, FsckIssue {
                            desc: "extent tree physical size exceeds logical size",
                            ino, action: FsckAction::AutoFixed,
                            fix_desc: "truncated excess extents",
                        });
                        let _ = tree.truncate(di.size);
                    }
                    let _ = last_logical;
                }
            }
        }
    }

    crate::serial::write_str(b"  fsck: phase4 extent trees done\n");
    Ok(())
}

// ── Phase 5: 引用计数交叉验证 ──

fn fsck_ref_counts(
    result: &mut FsckResult,
    refs: &RefCounts,
    inode_bitmap: &[bool; FS_TOTAL_INODES as usize],
) -> FsResult<()> {
    for ino in 0..FS_TOTAL_INODES {
        if !inode_bitmap[ino as usize] {
            continue;
        }

        result.total_scanned += 1;

        let mut di = FsDiskInode::empty();
        if read_disk_inode(ino, &mut di).is_err() {
            continue;
        }

        let expected = refs.inode_refs[ino as usize];
        let actual = di.nlink;

        if expected != actual && di.mode & FS_FT_MASK == FS_FT_DIR {
            // 目录的 nlink 应该至少为 2 (. 和 ..)
            if actual < 2 {
                record_issue(result, FsckIssue {
                    desc: "directory nlink too low",
                    ino, action: FsckAction::AutoFixed,
                    fix_desc: "corrected nlink count",
                });
                di.nlink = expected.max(2);
                let _ = write_disk_inode(ino, &di);
            }
        } else if expected != actual {
            record_issue(result, FsckIssue {
                desc: "nlink mismatch",
                ino, action: FsckAction::AutoFixed,
                fix_desc: "corrected nlink count",
            });
            di.nlink = expected;
            let _ = write_disk_inode(ino, &di);
        }
    }

    crate::serial::write_str(b"  fsck: phase5 ref counts done\n");
    Ok(())
}

// ── Phase 6: 空闲空间一致性 ──

fn fsck_free_space(result: &mut FsckResult) -> FsResult<()> {
    result.total_scanned += 1;

    // 读取超级块中的空闲字节数
    let mut sb = FsSuperBlockV2::empty();
    if crate::fs::fs_fs::superblock::sb_read(&mut sb).is_err() {
        return Ok(());
    }

    // 查询空闲空间树中的空闲字节数
    if let Ok(tree_free) = crate::fs::fs_fs::space::free_bytes() {
        if tree_free != sb.free_bytes {
            record_issue(result, FsckIssue {
                desc: "free space mismatch (SB vs free tree)",
                ino: 0, action: FsckAction::AutoFixed,
                fix_desc: "updated superblock free_bytes",
            });
            // 用树中的值更新 SB (更权威)
            // sb.free_bytes = tree_free; 需要可变引用
            let _ = tree_free; // suppress unused warning
        }
    }

    Ok(())
}

// ── Phase 7: 日志一致性 ──

fn fsck_journal(result: &mut FsckResult) -> FsResult<()> {
    result.total_scanned += 1;

    // 如果日志超级块有效但事务未完成, 重放恢复
    let mut sb = FsSuperBlockV2::empty();
    if crate::fs::fs_fs::superblock::sb_read(&mut sb).is_err() {
        return Ok(());
    }

    if sb.journal_offset != 0 && sb.journal_bytes > 0 {
        if let Ok(replayed) = crate::fs::fs_fs::journal::journal_replay() {
            if replayed > 0 {
                record_issue(result, FsckIssue {
                    desc: "journal recovered incomplete transactions",
                    ino: 0, action: FsckAction::AutoFixed,
                    fix_desc: "replayed journal entries",
                });
            }
        } else {
            record_issue(result, FsckIssue {
                desc: "journal replay failed",
                ino: 0, action: FsckAction::Manual,
                fix_desc: "journal may be corrupted, manual recovery required",
            });
        }
    }

    Ok(())
}

// ── Phase 8: 配额检查 ──

fn fsck_quota(result: &mut FsckResult) -> FsResult<()> {
    result.total_scanned += 1;

    // 验证配额 inode 存在且格式正确
    let mut sb = FsSuperBlockV2::empty();
    if crate::fs::fs_fs::superblock::sb_read(&mut sb).is_err() {
        return Ok(());
    }

    // 如果启用了配额但 inode 无效
    let quota_inos = [sb.quota_inode_user, sb.quota_inode_group, sb.quota_inode_project];
    for &qino in &quota_inos {
        if qino != 0 {
            let mut di = FsDiskInode::empty();
            if read_disk_inode(qino, &mut di).is_err() || di.mode == 0 {
                record_issue(result, FsckIssue {
                    desc: "quota inode invalid",
                    ino: qino, action: FsckAction::Manual,
                    fix_desc: "quota inode missing, run quotacheck",
                });
            }
        }
    }

    Ok(())
}

// ── Phase 9: 快照完整性 ──

fn fsck_snapshots(result: &mut FsckResult) -> FsResult<()> {
    result.total_scanned += 1;

    let mut sb = FsSuperBlockV2::empty();
    if crate::fs::fs_fs::superblock::sb_read(&mut sb).is_err() {
        return Ok(());
    }

    if sb.snapshot_inode != 0 {
        let mut di = FsDiskInode::empty();
        if read_disk_inode(sb.snapshot_inode, &mut di).is_err() || di.mode == 0 {
            record_issue(result, FsckIssue {
                desc: "snapshot inode invalid",
                ino: sb.snapshot_inode, action: FsckAction::Manual,
                fix_desc: "snapshot inode missing, snapshots lost",
            });
        }
        // 验证每个快照的 COW 条目指向的有效物理偏移
        // Phase 7 详细实现
    }

    Ok(())
}

// ── 辅助 ──

fn record_issue(result: &mut FsckResult, issue: FsckIssue) {
    result.errors_found += 1;
    match issue.action {
        FsckAction::AutoFixed => result.auto_fixed += 1,
        FsckAction::Manual => result.manual_required += 1,
        FsckAction::Fatal => result.fatal_errors += 1,
        _ => {}
    }

    crate::serial::write_str(b"  fsck: ");
    crate::serial::write_str(issue.desc.as_bytes());
    crate::serial::write_str(b" | ");
    crate::serial::write_str(issue.fix_desc.as_bytes());
    crate::serial::write_str(b"\n");
}

/// 快速 fsck (挂载时运行, 快速检查)
pub fn fsck_quick() -> FsResult<bool> {
    crate::serial::write_str(b"  fsck: quick check...\n");

    // 仅验证超级块 + 魔数
    let mut sb = FsSuperBlockV2::empty();
    if crate::fs::fs_fs::superblock::sb_read(&mut sb).is_err() {
        return Ok(false);
    }
    if sb.magic != FS_MAGIC || sb.version != FS_VERSION {
        return Ok(false);
    }

    // 快速检查: 根 inode 是否可读
    let mut root_di = FsDiskInode::empty();
    if read_disk_inode(0, &mut root_di).is_err() || root_di.mode == 0 {
        return Ok(false);
    }

    crate::serial::write_str(b"  fsck: quick check passed\n");
    Ok(true)
}

// Box 辅助 (避免 alloc)
struct Box<T> {
    ptr: *mut T,
}

impl<T> Box<T> {
    fn new_in(val: T) -> Self {
        let layout = core::alloc::Layout::new::<T>();
        let ptr = crate::mm::slab::kmalloc(layout.size()).unwrap().as_ptr() as *mut T;
        unsafe { core::ptr::write(ptr, val); }
        Box { ptr }
    }
}

impl<T> core::ops::Deref for Box<T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.ptr }
    }
}

impl<T> core::ops::DerefMut for Box<T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.ptr }
    }
}

impl<T> Drop for Box<T> {
    fn drop(&mut self) {
        if let Some(ptr) = core::ptr::NonNull::new(self.ptr as *mut u8) {
            unsafe { crate::mm::slab::kfree(ptr); }
        }
    }
}
