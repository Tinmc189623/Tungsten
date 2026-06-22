// fs/fs_fs/format.rs — FS V2 磁盘格式常量与结构体
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::fs::types::*;

// ── 魔数 ──
pub const FS_MAGIC: u32 = 0x4653_5446; // "FSTF"
pub const FS_VERSION: u32 = 2;
pub const FS_EXTENT_MAGIC: u16 = 0xF30A;

// ── 最小分配单元 (扇区对齐) ──
pub const FS_MIN_ALLOC: u64 = 512;  // 设备扇区大小

// ── Inode 常量 ──
pub const FS_INODE_SIZE: u64 = 512;
pub const FS_TOTAL_INODES: u64 = 1024;
pub const FS_INODE_TABLE_OFFSET: u64 = 4096;  // 紧接超级块之后

// ── 超级块大小 ──
pub const FS_SB_SIZE: u64 = 4096;

// ── 特性标志 ──

/// 兼容特性 (老版本 fsck 可安全挂载)
pub const FEATURE_COMPAT_DIR_INDEX: u64   = 1 << 0;
pub const FEATURE_COMPAT_HAS_JOURNAL: u64 = 1 << 1;
pub const FEATURE_COMPAT_EXT_ATTR: u64    = 1 << 2;

/// 不兼容特性 (老版本 fsck 必须拒绝)
pub const FEATURE_INCOMPAT_EXTENTS: u64    = 1 << 0;  // 扩展树
pub const FEATURE_INCOMPAT_64BIT: u64      = 1 << 1;  // 64位偏移
pub const FEATURE_INCOMPAT_FLEX_BG: u64    = 1 << 2;  // 灵活分配组
pub const FEATURE_INCOMPAT_META_CSUM: u64  = 1 << 3;  // 元数据校验和
pub const FEATURE_INCOMPAT_SNAPSHOTS: u64  = 1 << 5;  // COW 快照

/// 只读兼容特性 (老版本 fsck 可只读挂载)
pub const FEATURE_RO_COMPAT_SPARSE_SUPER: u64 = 1 << 0;
pub const FEATURE_RO_COMPAT_LARGE_FILE: u64   = 1 << 1;
pub const FEATURE_RO_COMPAT_QUOTA: u64        = 1 << 2;
pub const FEATURE_RO_COMPAT_ACL: u64          = 1 << 3;
pub const FEATURE_RO_COMPAT_ENCRYPT: u64      = 1 << 4;
pub const FEATURE_RO_COMPAT_COMPRESS: u64     = 1 << 5;

// ── Inode 标志 ──
pub const FS_IMMUTABLE_FL: u32 = 0x00000010;
pub const FS_APPEND_FL: u32    = 0x00000020;
pub const FS_NOATIME_FL: u32   = 0x00000080;
pub const FS_COMPRESSED_FL: u32 = 0x00000004;
pub const FS_ENCRYPTED_FL: u32  = 0x00000800;

// ── 超级块 (V2, 固定 4096 字节) ──

#[repr(C, packed)]
pub struct FsSuperBlockV2 {
    pub magic: u32,                     // FS_MAGIC
    pub version: u32,                   // 2
    pub feature_compat: u64,
    pub feature_incompat: u64,
    pub feature_ro_compat: u64,

    pub uuid: [u8; 16],
    pub volume_name: [u8; 64],
    pub last_mounted: [u8; 64],

    pub min_alloc_size: u64,            // 最小分配单元 (扇区大小)
    pub total_bytes: u64,
    pub free_bytes: u64,

    pub inode_count: u64,
    pub free_inodes: u64,

    pub group_bytes: u64,               // 每分配组字节数
    pub inodes_per_group: u64,

    pub journal_offset: u64,            // 日志起始物理偏移
    pub journal_bytes: u64,             // 日志字节数
    pub quota_inode_user: u64,
    pub quota_inode_group: u64,
    pub quota_inode_project: u64,
    pub snapshot_inode: u64,

    pub root_inode: Ino,
    pub free_space_root: u64,           // 空闲空间树根物理偏移

    pub mount_time: u64,
    pub write_time: u64,
    pub mount_count: u32,
    pub max_mount_count: u32,
    pub last_check: u64,
    pub check_interval: u32,
    pub errors_behavior: u16,
    pub creator_os: [u8; 16],           // "TungstenOS"

    pub sb_copy_count: u8,
    pub sb_sequence: u32,               // 写入序列号 (用于找最新副本)

    pub checksum: u32,                  // CRC32c (此字段计入前清零)
    pub _reserved: [u8; 380],
}

impl FsSuperBlockV2 {
    pub const fn empty() -> Self {
        FsSuperBlockV2 {
            magic: 0, version: 0,
            feature_compat: 0, feature_incompat: 0, feature_ro_compat: 0,
            uuid: [0; 16], volume_name: [0; 64], last_mounted: [0; 64],
            min_alloc_size: 0, total_bytes: 0, free_bytes: 0,
            inode_count: 0, free_inodes: 0,
            group_bytes: 0, inodes_per_group: 0,
            journal_offset: 0, journal_bytes: 0,
            quota_inode_user: 0, quota_inode_group: 0, quota_inode_project: 0,
            snapshot_inode: 0, root_inode: 0, free_space_root: 0,
            mount_time: 0, write_time: 0,
            mount_count: 0, max_mount_count: 0,
            last_check: 0, check_interval: 0, errors_behavior: 0,
            creator_os: [0; 16],
            sb_copy_count: 0, sb_sequence: 0,
            checksum: 0, _reserved: [0; 380],
        }
    }
}

// ── 磁盘 Inode (V2, 512 字节) ──

#[repr(C, packed)]
pub struct FsDiskInode {
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub atime: u64,
    pub atime_nsec: u32,
    pub mtime: u64,
    pub mtime_nsec: u32,
    pub ctime: u64,
    pub ctime_nsec: u32,
    pub btime: u64,
    pub btime_nsec: u32,
    pub extent_root: FsExtentHeader,    // 数据扩展树根
    pub xattr_root: FsExtentHeader,     // 扩展属性树根
    pub acl_root: FsExtentHeader,       // ACL 树根
    pub encrypt_ctx: [u8; 32],          // 加密上下文
    pub nlink: u32,
    pub flags: u32,
    pub generation: u32,
    pub project_id: u32,
    pub checksum: u32,
    pub checksum_hi: u16,
    pub _reserved: [u8; 114],
}

impl FsDiskInode {
    pub const fn empty() -> Self {
        FsDiskInode {
            mode: 0, uid: 0, gid: 0, size: 0,
            atime: 0, atime_nsec: 0,
            mtime: 0, mtime_nsec: 0,
            ctime: 0, ctime_nsec: 0,
            btime: 0, btime_nsec: 0,
            extent_root: FsExtentHeader::empty(),
            xattr_root: FsExtentHeader::empty(),
            acl_root: FsExtentHeader::empty(),
            encrypt_ctx: [0; 32],
            nlink: 0, flags: 0, generation: 0, project_id: 0,
            checksum: 0, checksum_hi: 0,
            _reserved: [0; 114],
        }
    }
}

// ── 扩展树头 ──

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct FsExtentHeader {
    pub magic: u16,                     // FS_EXTENT_MAGIC = 0xF30A
    pub entries: u16,
    pub max_entries: u16,
    pub depth: u8,
    pub generation: u32,
    pub checksum: u32,
}

impl FsExtentHeader {
    pub const fn empty() -> Self {
        FsExtentHeader {
            magic: 0, entries: 0, max_entries: 0, depth: 0,
            generation: 0, checksum: 0,
        }
    }
}

// ── 扩展条目 (叶节点) ──

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct FsExtent {
    pub logical_offset: u64,            // 文件内字节偏移
    pub length: u64,                    // 逻辑长度 (字节)
    pub physical_offset: u64,           // 设备字节偏移
    pub physical_length: u64,           // 物理长度 (≤ length if compressed)
    pub compression: u8,                // 0=none, 1=zstd, 2=lz4
    pub flags: u8,
}

impl FsExtent {
    pub const fn empty() -> Self {
        FsExtent {
            logical_offset: 0, length: 0,
            physical_offset: 0, physical_length: 0,
            compression: 0, flags: 0,
        }
    }
}

// ── 扩展索引 (内部节点) ──

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct FsExtentIndex {
    pub logical_offset: u64,            // 子节点覆盖的起始逻辑偏移
    pub child_physical: u64,            // 子节点物理字节偏移
}

// ── 目录项 (64 字节, V2 新增插入顺序链表) ──

#[repr(C, packed)]
pub struct FsDirEntry {
    pub ino: Ino,
    pub name_len: u16,
    pub file_type: u8,
    pub name: [u8; 39],                 // 从 53 缩减以容纳链表指针
    pub next_entry_offset: u32,         // 插入顺序下一个目录项偏移
    pub prev_entry_offset: u32,         // 插入顺序上一个目录项偏移
}

impl FsDirEntry {
    pub const fn empty() -> Self {
        FsDirEntry {
            ino: 0, name_len: 0, file_type: 0,
            name: [0; 39],
            next_entry_offset: 0, prev_entry_offset: 0,
        }
    }
}

// ── 文件类型掩码 ──
pub const FS_FT_MASK: u16  = 0xF000;
pub const FS_FT_REG: u16   = 0x8000;
pub const FS_FT_DIR: u16   = 0x4000;
pub const FS_FT_LNK: u16   = 0xA000;
pub const FS_FT_BLK: u16   = 0x6000;
pub const FS_FT_CHR: u16   = 0x2000;
pub const FS_FT_FIFO: u16  = 0x1000;
pub const FS_FT_SOCK: u16  = 0xC000;
