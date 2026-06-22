// fs/fs_fs/mod.rs — 自研 FS 文件系统实现
//
// V2 版本: 扩展树 B+tree 分配 (完全无传统块), HTree 目录索引,
// JBD2 日志, NFSv4 ACL, AES-256-XTS 加密, LZ4/Zstd 压缩,
// COW 快照, 磁盘配额, fsck 校验。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod format;
pub mod extent;
pub mod space;
pub mod inode;
pub mod dir;
pub mod file;
pub mod superblock;
pub mod compress;
pub mod journal;
pub mod htree;
pub mod acl;
pub mod encrypt;
pub mod xattr;
pub mod quota;
pub mod snapshot;
pub mod fsck;

pub use format::*;
pub use superblock::{FsSuperBlock, sb_read, sb_write, fs_init, fs_format};
pub use inode::{read_disk_inode, write_disk_inode, alloc_inode, free_inode};
pub use extent::ExtentTree;
pub use format::{FsExtentHeader, FsExtent, FsExtentIndex};
pub use space::FreeSpaceTree;
pub use dir::{dir_lookup, dir_add, dir_remove, sys_list_dir};
pub use file::{read_file_data, write_file_data, FS_FILE_OPS, fsync_file, fdatasync_file, fallocate_file, readahead_file};
pub use compress::{CompressionAlg, Compressor, get_compressor, compress_page, decompress_page};
pub use journal::{init_journal, init_new_journal, journal_replay};
pub use htree::{Htree, half_md4};
pub use acl::{Nfs4Acl, Nfs4Ace, acl_check, acl_check_read, acl_check_write, set_acl, get_acl, mode_to_acl};
pub use encrypt::{EncryptContext, AesXts, init_encryption, encrypt_page_data, decrypt_page_data, set_master_key, get_file_cipher};
pub use xattr::{get_xattr, set_xattr, list_xattr, remove_xattr, XATTR_NAME_MAX};
pub use quota::{QuotaManager, QuotaInfo, QuotaType, quota_init, user_quota, group_quota, project_quota};
pub use snapshot::{SnapshotManager, snapshot_init, snap_create, snap_delete, snap_rollback, snapshot_manager};
pub use fsck::{fsck_run, fsck_quick, FsckResult};
