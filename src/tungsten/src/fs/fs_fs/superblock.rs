// fs/fs_fs/superblock.rs — 超级块管理 + 格式化
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::fs_fs::format::*;
use crate::fs::fs_fs::journal;
use crate::fs::ramdisk::get_ramdisk_device;

/// 当前使用的超级块类型 (V2)
pub use crate::fs::fs_fs::format::FsSuperBlockV2 as FsSuperBlock;

/// 从设备读取超级块
pub fn sb_read(sb: &mut FsSuperBlock) -> Result<(), ()> {
    let size = core::mem::size_of::<FsSuperBlock>();
    let buf = unsafe { core::slice::from_raw_parts_mut(sb as *mut _ as *mut u8, size) };
    if get_ramdisk_device().read_bytes(0, buf).is_err() {
        return Err(());
    }
    if sb.magic != FS_MAGIC {
        return Err(());
    }
    Ok(())
}

/// 将超级块写入设备
pub fn sb_write(sb: &FsSuperBlock) -> Result<(), ()> {
    let size = core::mem::size_of::<FsSuperBlock>();
    let buf = unsafe { core::slice::from_raw_parts(sb as *const _ as *const u8, size) };
    let _ = get_ramdisk_device().write_bytes(0, buf);
    Ok(())
}

/// 格式化设备为 FS V2 文件系统
pub fn fs_format(total_bytes: u64) {
    let inode_table_bytes = FS_TOTAL_INODES * FS_INODE_SIZE;
    let space_root_phys = FS_INODE_TABLE_OFFSET + inode_table_bytes;  // 空闲树根节点
    let data_start = space_root_phys + 4096;                          // 数据区域起始
    let free_bytes = total_bytes.saturating_sub(data_start);

    // 创建空闲空间树根节点
    let _ = crate::fs::fs_fs::space::create_free_space_root(space_root_phys, data_start, total_bytes);

    // 计算日志区域: 紧接数据区之后, 占总空间 1% (最小 16MB, 最大 128MB)
    let journal_offset = data_start;
    let journal_pct = (total_bytes / 100).max(16 * 1024 * 1024).min(128 * 1024 * 1024);
    let journal_bytes = journal_pct.min(total_bytes.saturating_sub(data_start));
    let _ = journal::init_new_journal(journal_offset, journal_bytes);

    let sb = FsSuperBlock {
        magic: FS_MAGIC,
        version: FS_VERSION,
        feature_compat: FEATURE_COMPAT_DIR_INDEX | FEATURE_COMPAT_HAS_JOURNAL | FEATURE_COMPAT_EXT_ATTR,
        feature_incompat: FEATURE_INCOMPAT_EXTENTS | FEATURE_INCOMPAT_64BIT | FEATURE_INCOMPAT_FLEX_BG | FEATURE_INCOMPAT_META_CSUM,
        feature_ro_compat: FEATURE_RO_COMPAT_SPARSE_SUPER | FEATURE_RO_COMPAT_LARGE_FILE,
        uuid: [0; 16],
        volume_name: [0; 64],
        last_mounted: [0; 64],
        min_alloc_size: FS_MIN_ALLOC,
        total_bytes,
        free_bytes,
        inode_count: FS_TOTAL_INODES,
        free_inodes: FS_TOTAL_INODES - 1, // 保留根 inode
        group_bytes: total_bytes / 8,
        inodes_per_group: FS_TOTAL_INODES / 8,
        journal_offset,
        journal_bytes,
        quota_inode_user: 0,
        quota_inode_group: 0,
        quota_inode_project: 0,
        snapshot_inode: 0,
        root_inode: 0,
        free_space_root: space_root_phys,
        mount_time: 0,
        write_time: 0,
        mount_count: 0,
        max_mount_count: 30,
        last_check: 0,
        check_interval: 3600 * 24 * 180,
        errors_behavior: 2,
        creator_os: *b"TungstenOS\x00\x00\x00\x00\x00\x00",
        sb_copy_count: 3,
        sb_sequence: 1,
        checksum: 0,
        _reserved: [0; 380],
    };
    let _ = sb_write(&sb);

    // 初始化根目录 inode (ino=0)
    let now = 0u64;
    let root_inode = FsDiskInode {
        mode: FS_FT_DIR | 0o755,
        uid: 0, gid: 0, size: 0,
        atime: now, atime_nsec: 0,
        mtime: now, mtime_nsec: 0,
        ctime: now, ctime_nsec: 0,
        btime: now, btime_nsec: 0,
        extent_root: FsExtentHeader::empty(),
        xattr_root: FsExtentHeader::empty(),
        acl_root: FsExtentHeader::empty(),
        encrypt_ctx: [0; 32],
        nlink: 2, flags: 0, generation: 1, project_id: 0,
        checksum: 0, checksum_hi: 0,
        _reserved: [0; 114],
    };
    let _ = crate::fs::fs_fs::inode::write_disk_inode(0, &root_inode);

    crate::serial::write_str(b"  fs: formatted V2 (extent-based, no blocks)\n");
}

/// 初始化 FS (挂载设备或创建 ramdisk)
pub fn fs_init(total_bytes: u64) {
    // 尝试读取超级块
    let mut sb = FsSuperBlock::empty();
    if sb_read(&mut sb).is_ok() && sb.magic == FS_MAGIC {
        crate::serial::write_str(b"  fs: found existing FS V");
        crate::serial::write_str(if sb.version == 2 { b"2" } else { b"? (upgrade required)" });
        crate::serial::write_str(b", mounted\n");

        // 初始化空闲空间树
        if sb.free_space_root != 0 {
            crate::fs::fs_fs::space::init_free_space(sb.free_space_root);
        }

        // 日志重放恢复
        if sb.journal_offset != 0 && sb.journal_bytes > 0 {
            let _ = journal::init_journal(sb.journal_offset, sb.journal_bytes);
            let _ = journal::journal_replay();
        }
    } else {
        crate::serial::write_str(b"  fs: no valid superblock, formatting...\n");
        fs_format(total_bytes);
    }
}
