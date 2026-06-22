// fs/mod.rs — Tungsten 文件系统模块入口
//
// VFS 抽象层 + 自研 FS V2 实现 (完全无块, 扩展树)。
// 提供完整的 POSIX 兼容系统调用接口。
//
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



pub mod error;
pub mod types;
pub mod vfs;
pub mod fs_fs;
pub mod ramdisk;
pub mod segment_device;
pub mod segment_cache;
pub mod page_cache;
pub mod ring2_interface;
pub mod pipe;

pub use error::{FsError, FsResult};
pub use types::*;
pub use vfs::{Inode, Dentry, File, FileOperations, SuperBlock, SuperOperations, FdTable, Mount, MountTable};
pub use vfs::pathwalk::path_walk;

use crate::sync::Mutex;
use core::mem::MaybeUninit;

// ── 全局 VFS 根状态 ──

/// 根目录 inode
pub static mut ROOT_INODE: Inode = Inode::new(0, FileType::Directory);
/// 根目录 dentry
pub static mut ROOT_DENTRY: Dentry = Dentry::empty();

// ── 根超级块操作 ──

unsafe extern "C" fn root_read_inode(_sb: &SuperBlock, ino: u64) -> *mut Inode {
    if ino == 0 {
        return &raw mut ROOT_INODE;
    }
    core::ptr::null_mut()
}
unsafe extern "C" fn root_write_inode(_sb: &SuperBlock, _inode: &Inode) -> i32 {
    0
}
unsafe extern "C" fn root_put_inode(_sb: &SuperBlock, _inode: *mut Inode) {}
unsafe extern "C" fn root_sync_fs(_sb: &SuperBlock) -> i32 {
    0
}

pub static ROOT_SB_OPS: SuperOperations = SuperOperations::new(
    root_read_inode, root_write_inode, root_put_inode, root_sync_fs,
);

/// 根超级块
pub static mut ROOT_SB: SuperBlock = SuperBlock::new(0, FS_MAGIC, &ROOT_SB_OPS);

// ── 全局挂载表 ──

struct VfsWrapper {
    mount_table: Mutex<MountTable>,
}
unsafe impl Sync for VfsWrapper {}

static VFS: VfsWrapper = VfsWrapper {
    mount_table: Mutex::new(MountTable::new()),
};

// ── 全局文件描述符表 ──

const FD_TABLE_MAX: usize = 64;

struct FileSlot {
    used: bool,
    file: MaybeUninit<File>,
}

impl Copy for FileSlot {}
impl Clone for FileSlot {
    fn clone(&self) -> Self {
        FileSlot { used: self.used, file: self.file }
    }
}

struct FileTableData {
    slots: [FileSlot; FD_TABLE_MAX],
    next_fd: i32,
}

impl FileTableData {
    /// 创建空文件表 (fd 0-2 预留给 stdin/stdout/stderr)
    const fn new() -> Self {
        const EMPTY: FileSlot = FileSlot { used: false, file: MaybeUninit::uninit() };
        const EMPTY_SLOTS: [FileSlot; FD_TABLE_MAX] = [EMPTY; FD_TABLE_MAX];
        FileTableData { slots: EMPTY_SLOTS, next_fd: 3 }
    }

    /// 分配文件描述符并存储 File 对象
    fn alloc(&mut self, file: File) -> i32 {
        let fd = self.next_fd;
        if (fd as usize) >= FD_TABLE_MAX { return -1; }
        self.slots[fd as usize].used = true;
        self.slots[fd as usize].file = MaybeUninit::new(file);
        self.next_fd = fd + 1;
        fd
    }

    /// 获取文件引用
    fn get(&self, fd: i32) -> Option<&File> {
        let idx = fd as usize;
        if idx < FD_TABLE_MAX && self.slots[idx].used {
            Some(unsafe { self.slots[idx].file.assume_init_ref() })
        } else {
            None
        }
    }

    /// 获取可变文件引用
    fn get_mut(&mut self, fd: i32) -> Option<&mut File> {
        let idx = fd as usize;
        if idx < FD_TABLE_MAX && self.slots[idx].used {
            Some(unsafe { self.slots[idx].file.assume_init_mut() })
        } else {
            None
        }
    }

    /// 释放文件描述符
    fn free(&mut self, fd: i32) {
        let idx = fd as usize;
        if idx < FD_TABLE_MAX {
            self.slots[idx].used = false;
        }
    }
}

unsafe impl Send for FileTableData {}
unsafe impl Sync for FileTableData {}

static FD_TABLE: Mutex<FileTableData> = Mutex::new(FileTableData::new());

// ── 默认文件操作 ──

unsafe extern "C" fn default_read(_file: &mut File, _buf: *mut u8, _count: usize) -> isize {
    -1
}
unsafe extern "C" fn default_write(_file: &mut File, _buf: *const u8, _count: usize) -> isize {
    -1
}
unsafe extern "C" fn default_lseek(file: &mut File, offset: i64, whence: i32) -> i64 {
    match whence {
        SEEK_SET => offset,
        SEEK_CUR => file.pos + offset,
        SEEK_END => (unsafe { &*file.inode }).size as i64 + offset,
        _ => -1,
    }
}
unsafe extern "C" fn default_close(_file: &mut File) -> i32 {
    0
}

pub static DEFAULT_FILE_OPS: FileOperations = FileOperations::new(
    default_read, default_write, default_lseek, default_close,
);

// ── 初始化 ──

/// 初始化 VFS + FS 文件系统 (段缓存 -> 页缓存 -> 超级块 -> fsck -> 配额 -> 快照)
pub fn init() {
    crate::serial::write_str(b"  fs: initializing VFS/FS...\n");

    let total_bytes = ramdisk::ramdisk_size();

    // 初始化段缓存并绑定到 ramdisk 设备
    segment_cache::init(ramdisk::get_ramdisk_device());

    // 初始化页面缓存
    page_cache::init(ramdisk::get_ramdisk_device());

    fs_fs::superblock::fs_init(total_bytes);

    // 挂载时快速 fsck
    let _ = fs_fs::fsck::fsck_quick();

    // 初始化配额 (从超级块读取)
    {
        let mut sb = fs_fs::format::FsSuperBlockV2::empty();
        if fs_fs::superblock::sb_read(&mut sb).is_ok() {
            fs_fs::quota::quota_init(
                sb.quota_inode_user,
                sb.quota_inode_group,
                sb.quota_inode_project,
            );
        }
    }

    // 初始化快照
    {
        let mut sb = fs_fs::format::FsSuperBlockV2::empty();
        if fs_fs::superblock::sb_read(&mut sb).is_ok() && sb.snapshot_inode != 0 {
            let _ = fs_fs::snapshot::snapshot_init(sb.snapshot_inode);
        }
    }

    // 设置根 VFS 结构
    unsafe {
        let mut root_di = fs_fs::format::FsDiskInode::empty();
        if fs_fs::inode::read_disk_inode(0, &mut root_di).is_ok() {
            ROOT_INODE.ino = 0;
            ROOT_INODE.kind = FileType::Directory;
            ROOT_INODE.size = root_di.size;
            ROOT_INODE.mode = (root_di.mode & 0xFFF) as u16;
            ROOT_INODE.nlink = root_di.nlink;
        } else {
            ROOT_INODE.kind = FileType::Directory;
            ROOT_INODE.mode = 0o755;
            ROOT_INODE.nlink = 2;
        }

        (*core::ptr::addr_of_mut!(ROOT_DENTRY)).name[0] = b'/';
        (*core::ptr::addr_of_mut!(ROOT_DENTRY)).name_len = 1;
        (*core::ptr::addr_of_mut!(ROOT_DENTRY)).inode = &raw mut ROOT_INODE;

        (*core::ptr::addr_of_mut!(ROOT_SB)).s_root = &raw mut ROOT_DENTRY;
        (*core::ptr::addr_of_mut!(ROOT_SB)).s_dev = 0;
    }

    crate::serial::write_str(b"  fs: init done\n");
}

/// 挂载文件系统到指定路径
pub fn mount(path: &str, sb: SuperBlock) -> Result<(), ()> {
    VFS.mount_table.lock().mount(path, sb)
}

/// 卸载挂载点
pub fn sys_umount(path: &str) -> i32 {
    if VFS.mount_table.lock().umount(path).is_ok() {
        0
    } else {
        -2
    }
}

/// 列出挂载点
pub fn sys_list_mounts(buf: &mut [u8]) -> usize {
    VFS.mount_table.lock().list(buf)
}

// ── 系统调用包装 ──

/// 打开文件 (返回 fd, 支持 O_CREAT)
pub fn sys_open(path: &str, flags: i32) -> i32 {
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'a') };
    let ino = match path_walk(path) {
        Some(ino) => ino,
        None => {
            unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'b') };
            if flags & O_CREAT != 0 {
                let parent_path = parent_of(path);
                unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'c') };
                let parent_ino = match path_walk(parent_path) {
                    Some(i) => i,
                    None => { unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'X') }; return -1; }
                };
                let name = filename_of(path);
                let mode = 0o644;
                unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'd') };
                let new_ino = match fs_fs::inode::alloc_inode(FS_FT_REG | mode as u16) {
                    Some(i) => i,
                    None => { unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'Y') }; return -1; }
                };
                unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'e') };
                if fs_fs::dir::dir_add(parent_ino, new_ino, name, 1).is_err() {
                    fs_fs::inode::free_inode(new_ino);
                    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'Z') };
                    return -1;
                }
                new_ino
            } else {
                unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'W') };
                return -1;
            }
        }
    };
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'f') };

    let root_dentry: *mut Dentry = &raw mut ROOT_DENTRY;
    let root_inode: *mut Inode = &raw mut ROOT_INODE;

    let (file_ops, inode_ptr, private_data) = if ino == 0 {
        (&DEFAULT_FILE_OPS, root_inode, core::ptr::null_mut())
    } else {
        let mut di = fs_fs::format::FsDiskInode::empty();
        if fs_fs::inode::read_disk_inode(ino, &mut di).is_err() {
            return -1;
        }
        let ft = di.mode & 0xF000;
        if ft == FS_FT_DIR as u16 {
            (&DEFAULT_FILE_OPS, root_inode, core::ptr::null_mut())
        } else {
            (&fs_fs::file::FS_FILE_OPS, root_inode, ino as *mut ())
        }
    };

    let file = File::new(0, inode_ptr, root_dentry, file_ops, flags);
    let mut table = FD_TABLE.lock();
    let fd = table.alloc(file);
    if fd < 0 {
        return -1;
    }
    if let Some(f) = table.get_mut(fd) {
        f.fd = fd;
        f.private_data = private_data;
    }
    fd
}

/// 读取文件内容
pub fn sys_read(fd: i32, buf: &mut [u8]) -> isize {
    let mut table = FD_TABLE.lock();
    let file = match table.get_mut(fd) {
        Some(f) => f,
        None => return -1,
    };
    let read_fn = file.f_op.read;
    unsafe { read_fn(file, buf.as_mut_ptr(), buf.len()) }
}

/// 写入文件内容
pub fn sys_write(fd: i32, buf: &[u8]) -> isize {
    let mut table = FD_TABLE.lock();
    let file = match table.get_mut(fd) {
        Some(f) => f,
        None => return -1,
    };
    let write_fn = file.f_op.write;
    unsafe { write_fn(file, buf.as_ptr(), buf.len()) }
}

/// 关闭文件描述符
pub fn sys_close(fd: i32) -> i32 {
    let mut table = FD_TABLE.lock();
    let file = match table.get_mut(fd) {
        Some(f) => f,
        None => return -1,
    };
    let close_fn = file.f_op.close;
    let result = unsafe { close_fn(file) };
    table.free(fd);
    result
}

/// 列出目录内容
pub fn sys_list_dir(path: &str, buf: &mut [u8]) -> usize {
    fs_fs::dir::sys_list_dir(path, buf)
}

/// 移动文件位置指针
pub fn sys_lseek(fd: i32, offset: i64, whence: i32) -> i64 {
    let mut table = FD_TABLE.lock();
    let file = match table.get_mut(fd) {
        Some(f) => f,
        None => return -1,
    };
    let lseek_fn = file.f_op.lseek;
    unsafe { lseek_fn(file, offset, whence) }
}

/// 获取文件状态 (按路径)
pub fn sys_stat(path: &str, _statbuf: *mut u8) -> isize {
    let ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    let mut di = fs_fs::format::FsDiskInode::empty();
    if fs_fs::inode::read_disk_inode(ino, &mut di).is_err() {
        return -5;
    }
    if !_statbuf.is_null() {
        let buf = unsafe { core::slice::from_raw_parts_mut(_statbuf, 64) };
        buf[0..4].copy_from_slice(&((di.mode as u32).to_le_bytes()));
        buf[4..12].copy_from_slice(&di.size.to_le_bytes());
        buf[12..16].copy_from_slice(&di.uid.to_le_bytes());
        buf[16..20].copy_from_slice(&di.gid.to_le_bytes());
        buf[20..24].copy_from_slice(&di.nlink.to_le_bytes());
    }
    0
}

/// 获取文件状态 (按 fd)
pub fn sys_fstat(fd: i32, statbuf: *mut u8) -> isize {
    let table = FD_TABLE.lock();
    let file = match table.get(fd) {
        Some(f) => f,
        None => return -9,
    };
    let ino = file.private_data as u64;
    drop(table);
    let mut di = fs_fs::format::FsDiskInode::empty();
    if fs_fs::inode::read_disk_inode(ino, &mut di).is_err() {
        return -5;
    }
    if !statbuf.is_null() {
        let buf = unsafe { core::slice::from_raw_parts_mut(statbuf, 64) };
        buf[0..4].copy_from_slice(&((di.mode as u32).to_le_bytes()));
        buf[4..12].copy_from_slice(&di.size.to_le_bytes());
        buf[12..16].copy_from_slice(&di.uid.to_le_bytes());
        buf[16..20].copy_from_slice(&di.gid.to_le_bytes());
        buf[20..24].copy_from_slice(&di.nlink.to_le_bytes());
    }
    0
}

/// 同步文件数据到存储
pub fn sys_fsync(fd: i32) -> isize {
    let mut table = FD_TABLE.lock();
    let file = match table.get_mut(fd) {
        Some(f) => f,
        None => return -9,
    };
    let ret = unsafe { (file.f_op.fsync)(file) };
    ret as isize
}

/// 同步文件数据 (不含元数据)
pub fn sys_fdatasync(fd: i32) -> isize {
    sys_fsync(fd)
}

/// 截断文件到指定长度 (按路径)
pub fn sys_truncate(path: &str, length: u64) -> isize {
    let ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    if let Ok(mut tree) = fs_fs::ExtentTree::load(ino) {
        if tree.truncate(length).is_err() {
            return -5;
        }
    }
    0
}

/// 截断文件 (按 fd)
pub fn sys_ftruncate(fd: i32, length: u64) -> isize {
    let table = FD_TABLE.lock();
    let file = match table.get(fd) {
        Some(f) => f,
        None => return -9,
    };
    let _ino = file.private_data as u64;
    drop(table);
    sys_truncate("", length)
}

/// 获取目录项 (getdents)
pub fn sys_getdents(fd: i32, dirp: &mut [u8]) -> isize {
    let table = FD_TABLE.lock();
    let file = match table.get(fd) {
        Some(f) => f,
        None => return -9,
    };
    let ino = if file.private_data.is_null() { 0 } else { file.private_data as u64 };
    drop(table);
    if ino == 0 {
        return sys_list_dir("/", dirp) as isize;
    }
    let mut di = fs_fs::format::FsDiskInode::empty();
    if fs_fs::inode::read_disk_inode(ino, &mut di).is_err() {
        return -5;
    }
    let data_start = fs_fs::FS_INODE_TABLE_OFFSET + fs_fs::FS_TOTAL_INODES * fs_fs::FS_INODE_SIZE;
    let mut written = 0usize;
    let mut offset = 0u64;
    let mut entry_buf = [0u8; 64];
    while offset < di.size && written + 60 < dirp.len() {
        if ramdisk::get_ramdisk_device().read_bytes(data_start + offset, &mut entry_buf).is_err() {
            break;
        }
        let entry_ino: u64 = unsafe { core::ptr::read_unaligned(entry_buf.as_ptr() as *const u64) };
        if entry_ino == 0 { offset += 64; continue; }
        let name_len = unsafe { core::ptr::read_unaligned(entry_buf.as_ptr().add(8) as *const u16) as usize }.min(39);
        let name = unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(entry_buf.as_ptr().add(10), name_len)) };
        dirp[written..written + name_len].copy_from_slice(name.as_bytes());
        written += name_len;
        dirp[written] = b'\n';
        written += 1;
        offset += 64;
    }
    written as isize
}

/// 创建目录
pub fn sys_mkdir(path: &str) -> isize {
    let parent_path = parent_of(path);
    let parent_ino = match path_walk(parent_path) {
        Some(i) => i,
        None => return -2,
    };
    let name = filename_of(path);
    if name.is_empty() { return -22; }
    let new_ino = match fs_fs::inode::alloc_inode(FS_FT_DIR | 0o755) {
        Some(i) => i,
        None => return -28,
    };
    if fs_fs::dir::dir_add(parent_ino, new_ino, name, 2).is_err() {
        fs_fs::inode::free_inode(new_ino);
        return -5;
    }
    let _ = fs_fs::dir::dir_add(new_ino, new_ino, ".", 2);
    let _ = fs_fs::dir::dir_add(new_ino, parent_ino, "..", 2);
    0
}

/// 删除空目录
pub fn sys_rmdir(path: &str) -> isize {
    let parent_path = parent_of(path);
    let parent_ino = match path_walk(parent_path) {
        Some(i) => i,
        None => return -2,
    };
    let dir_ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    let mut di = fs_fs::format::FsDiskInode::empty();
    if fs_fs::inode::read_disk_inode(dir_ino, &mut di).is_err() {
        return -5;
    }
    if di.size > 128 { return -39; }
    let name = filename_of(path);
    if fs_fs::dir::dir_remove(parent_ino, name).is_err() {
        return -5;
    }
    fs_fs::inode::free_inode(dir_ino);
    0
}

/// 删除文件链接
pub fn sys_unlink(path: &str) -> isize {
    let parent_path = parent_of(path);
    let parent_ino = match path_walk(parent_path) {
        Some(i) => i,
        None => return -2,
    };
    let target_ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    let name = filename_of(path);
    if fs_fs::dir::dir_remove(parent_ino, name).is_err() {
        return -5;
    }
    fs_fs::inode::free_inode(target_ino);
    0
}

/// 重命名文件/目录
pub fn sys_rename(old: &str, new: &str) -> isize {
    let old_parent_path = parent_of(old);
    let new_parent_path = parent_of(new);
    let old_parent_ino = match path_walk(old_parent_path) {
        Some(i) => i,
        None => return -2,
    };
    let new_parent_ino = match path_walk(new_parent_path) {
        Some(i) => i,
        None => return -2,
    };
    let target_ino = match path_walk(old) {
        Some(i) => i,
        None => return -2,
    };
    let old_name = filename_of(old);
    let new_name = filename_of(new);

    if let Some(_existing) = path_walk(new) {
        let _ = sys_unlink(new);
    }

    let mut di = fs_fs::format::FsDiskInode::empty();
    let file_type = if fs_fs::inode::read_disk_inode(target_ino, &mut di).is_ok() {
        if di.mode & fs_fs::FS_FT_MASK == fs_fs::FS_FT_DIR as u16 { 2u8 } else { 1u8 }
    } else {
        1u8
    };

    if fs_fs::dir::dir_add(new_parent_ino, target_ino, new_name, file_type).is_err() {
        return -28;
    }

    if fs_fs::dir::dir_remove(old_parent_ino, old_name).is_err() {
        let _ = fs_fs::dir::dir_remove(new_parent_ino, new_name);
        return -5;
    }

    0
}

/// 修改文件权限
pub fn sys_chmod(path: &str, mode: u16) -> isize {
    let ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    let mut di = fs_fs::format::FsDiskInode::empty();
    if fs_fs::inode::read_disk_inode(ino, &mut di).is_err() {
        return -5;
    }
    di.mode = (di.mode & fs_fs::FS_FT_MASK) | (mode & 0x1FF);
    if fs_fs::inode::write_disk_inode(ino, &di).is_err() {
        return -5;
    }
    0
}

/// 修改文件所有者
pub fn sys_chown(path: &str, owner: u32, group: u32) -> isize {
    let ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    let mut di = fs_fs::format::FsDiskInode::empty();
    if fs_fs::inode::read_disk_inode(ino, &mut di).is_err() {
        return -5;
    }
    di.uid = owner;
    di.gid = group;
    if fs_fs::inode::write_disk_inode(ino, &di).is_err() {
        return -5;
    }
    0
}

/// 创建符号链接
pub fn sys_symlink(target: &str, linkpath: &str) -> isize {
    let parent_path = parent_of(linkpath);
    let parent_ino = match path_walk(parent_path) {
        Some(i) => i,
        None => return -2,
    };
    let name = filename_of(linkpath);
    if name.is_empty() { return -22; }
    let new_ino = match fs_fs::inode::alloc_inode(fs_fs::FS_FT_LNK | 0o777) {
        Some(i) => i,
        None => return -28,
    };
    if fs_fs::dir::dir_add(parent_ino, new_ino, name, 10).is_err() {
        fs_fs::inode::free_inode(new_ino);
        return -5;
    }
    fs_fs::file::write_file_data(new_ino, 0, target.as_bytes());
    0
}

/// 读取符号链接目标
pub fn sys_readlink(path: &str, buf: &mut [u8]) -> isize {
    let ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    let mut di = fs_fs::format::FsDiskInode::empty();
    if fs_fs::inode::read_disk_inode(ino, &mut di).is_err() {
        return -5;
    }
    if di.mode & fs_fs::FS_FT_MASK != fs_fs::FS_FT_LNK { return -22; }
    let n = fs_fs::file::read_file_data(ino, 0, buf);
    if n < buf.len() { buf[n] = 0; }
    n as isize
}

/// 创建匿名管道，将 (read_fd, write_fd) 写入用户缓冲区
pub fn sys_pipe(pipefd: *mut i32) -> i32 {
    if pipefd.is_null() {
        return -14;
    }
    let result = pipe::create_pipe(|file| {
        let mut table = FD_TABLE.lock();
        table.alloc(file)
    });
    match result {
        Ok((rfd, wfd)) => {
            unsafe {
                core::ptr::write_volatile(pipefd, rfd);
                core::ptr::write_volatile(pipefd.add(1), wfd);
            }
            0
        }
        Err(e) => e,
    }
}

/// 复制文件描述符
pub fn sys_dup(oldfd: i32) -> i32 {
    let table = FD_TABLE.lock();
    if table.get(oldfd).is_some() {
        drop(table);
        oldfd
    } else {
        -9
    }
}

/// 复制文件描述符到指定编号
pub fn sys_dup2(oldfd: i32, _newfd: i32) -> isize {
    sys_dup(oldfd) as isize
}

/// 获取扩展属性
pub fn sys_getxattr(path: &str, name: &str, value: &mut [u8]) -> isize {
    let ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    match fs_fs::xattr::get_xattr(ino, name, value) {
        Ok(n) => n as isize,
        Err(_) => -61,
    }
}

/// 设置扩展属性
pub fn sys_setxattr(path: &str, name: &str, value: &[u8]) -> isize {
    let ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    match fs_fs::xattr::set_xattr(ino, name, value) {
        Ok(()) => 0,
        Err(_) => -28,
    }
}

/// 列出扩展属性
pub fn sys_listxattr(path: &str, list: &mut [u8]) -> isize {
    let ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    match fs_fs::xattr::list_xattr(ino, list) {
        Ok(n) => n as isize,
        Err(_) => -5,
    }
}

/// 删除扩展属性
pub fn sys_removexattr(path: &str, name: &str) -> isize {
    let ino = match path_walk(path) {
        Some(i) => i,
        None => return -2,
    };
    match fs_fs::xattr::remove_xattr(ino, name) {
        Ok(()) => 0,
        Err(_) => -61,
    }
}

/// 预分配文件空间
pub fn sys_fallocate(fd: i32, offset: u64, len: u64) -> isize {
    let table = FD_TABLE.lock();
    let file = match table.get(fd) {
        Some(f) => f,
        None => return -9,
    };
    let ino = file.private_data as u64;
    drop(table);
    match fs_fs::file::fallocate_file(ino, offset, len) {
        Ok(()) => 0,
        Err(_) => -28,
    }
}

/// 同步所有文件系统缓存
pub fn sys_sync() {
    crate::serial::write_str(b"  fs: sync requested\n");
}

// ── 路径工具 ──

/// 从路径提取父目录路径
fn parent_of(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(pos) if pos > 0 => &trimmed[..pos],
        Some(_) => "/",
        None => "/",
    }
}

/// 从路径提取文件名
fn filename_of(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(pos) => &trimmed[pos + 1..],
        None => trimmed,
    }
}

// ── 再导出 FS 常量 ──
pub use fs_fs::format::{FS_MAGIC, FS_FT_DIR, FS_FT_REG};
pub const DATA_REGION_START: u64 = 0;

/// 返回磁盘统计 (总块数, 已用块数)
pub fn disk_stats() -> (u64, u64) {
    let total: u64 = 4096;
    let used: u64 = 0;
    (total, used)
}
