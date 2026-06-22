// syscall.rs — 系统调用派发表 + 处理函数
// Tungsten 专用 API + 完整 POSIX FS 兼容层
// 所有系统调用通过 SYSCALL 指令进入 Ring 0，由此模块分发
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use crate::arch::x86_64::syscall::SyscallFrame;

/* ── 系统调用编号 ── */

/// 内核日志输出
pub const SYS_LOG:            u64 = 0;
/// 帧缓冲写入
pub const SYS_FB_WRITE:       u64 = 1;
/// 内存映射
pub const SYS_MMAP:           u64 = 2;
/// 让出 CPU
pub const SYS_SCHED_YIELD:    u64 = 3;
/// 获取系统时间（毫秒）
pub const SYS_TIMER_MS:       u64 = 4;
/// PCI 读
pub const SYS_PCI_READ:       u64 = 5;
/// PCI 写
pub const SYS_PCI_WRITE:      u64 = 6;
/// AI 推理
pub const SYS_AI_INFERENCE:   u64 = 7;
/// 打开文件
pub const SYS_OPEN:     u64 = 8;
/// 读取文件
pub const SYS_READ:     u64 = 9;
/// 写入文件
pub const SYS_WRITE:    u64 = 10;
/// 关闭文件
pub const SYS_CLOSE:    u64 = 11;
/// 获取进程 ID
pub const SYS_GETPID:   u64 = 12;
/// 创建子进程
pub const SYS_FORK:     u64 = 13;
/// 执行新程序
pub const SYS_EXECVE:   u64 = 14;
/// 退出进程
pub const SYS_EXIT:     u64 = 15;
/// 调整堆大小
pub const SYS_BRK:      u64 = 16;
/// 文件定位
pub const SYS_LSEEK:    u64 = 17;
/// 获取文件状态（路径）
pub const SYS_STAT:     u64 = 18;
/// 获取文件状态（fd）
pub const SYS_FSTAT:    u64 = 19;
/// 同步文件数据到磁盘
pub const SYS_FSYNC:    u64 = 20;
/// 同步文件数据（不含元数据）
pub const SYS_FDATASYNC:u64 = 21;
/// 截断文件
pub const SYS_TRUNCATE: u64 = 22;
/// 截断文件（fd）
pub const SYS_FTRUNCATE:u64 = 23;
/// 读取目录项
pub const SYS_GETDENTS: u64 = 24;
/// 创建目录
pub const SYS_MKDIR:    u64 = 25;
/// 删除目录
pub const SYS_RMDIR:    u64 = 26;
/// 删除文件
pub const SYS_UNLINK:   u64 = 27;
/// 重命名文件
pub const SYS_RENAME:   u64 = 28;
/// 修改文件权限
pub const SYS_CHMOD:    u64 = 29;
/// 修改文件所有者
pub const SYS_CHOWN:    u64 = 30;
/// 创建硬链接
pub const SYS_LINK:     u64 = 31;
/// 创建软链接
pub const SYS_SYMLINK:  u64 = 32;
/// 读取软链接
pub const SYS_READLINK: u64 = 33;
/// 复制文件描述符
pub const SYS_DUP:      u64 = 34;
/// 复制文件描述符（指定新 fd）
pub const SYS_DUP2:     u64 = 35;
/// 创建管道
pub const SYS_PIPE:     u64 = 36;
/// 设备 I/O 控制
pub const SYS_IOCTL:    u64 = 37;
/// 获取扩展属性
pub const SYS_GETXATTR: u64 = 38;
/// 设置扩展属性
pub const SYS_SETXATTR: u64 = 39;
/// 列出扩展属性
pub const SYS_LISTXATTR:u64 = 40;
/// 删除扩展属性
pub const SYS_REMOVEXATTR:u64 = 41;
/// 检查文件访问权限
pub const SYS_ACCESS:   u64 = 42;
/// 文件描述符控制
pub const SYS_FCNTL:    u64 = 43;
/// 预分配文件空间
pub const SYS_FALLOCATE:u64 = 44;
/// 同步所有文件系统
pub const SYS_SYNC:     u64 = 45;
/// 设置文件创建掩码
pub const SYS_UMASK:    u64 = 46;
/// 获取当前时间
pub const SYS_GETTIMEOFDAY: u64 = 47;
/// 获取引导模式
pub const SYS_BOOT_MODE:   u64 = 48;
/// 列出块设备
pub const SYS_BLOCK_LIST:  u64 = 49;
/// 块设备读扇区
pub const SYS_BLOCK_READ:  u64 = 50;
/// 块设备写扇区
pub const SYS_BLOCK_WRITE: u64 = 51;
/// 块设备刷新
pub const SYS_BLOCK_FLUSH: u64 = 52;
/// 真实 UID
pub const SYS_GETUID:     u64 = 53;
/// 真实 GID
pub const SYS_GETGID:     u64 = 54;
/// 帧缓冲信息查询（写入用户态 40 字节：w,h,pitch,bpp,addr）
pub const SYS_FB_INFO:    u64 = 55;
/// 设备注册（Ring 2 驱动框架）
pub const SYS_DEV_REGISTER: u64 = 56;
pub const SYS_DEV_UNREG:    u64 = 57;
pub const SYS_DEV_IOCTL:    u64 = 58;
pub const SYS_AI_LOAD:      u64 = 59;
pub const SYS_AI_UNLOAD:    u64 = 60;
pub const SYS_MOUNT:        u64 = 61;
pub const SYS_UMOUNT:       u64 = 62;
pub const SYS_MSG_SEND:     u64 = 63;

/// Ring 2 完成通知（特殊编号）
pub const SYS_RING2_COMPLETION: u64 = 0x1000;

/// 系统调用表大小
const NUM_SYSCALLS: usize = 64;

/* ── 用户态地址空间边界 ── */

/// 用户态地址空间上限（48-bit 规范地址）
const USER_ADDR_MAX: u64 = 0x0000_7FFF_FFFF_FFFF;

/// 验证指针是否在用户态地址空间内
fn is_user_ptr(ptr: u64) -> bool {
    ptr <= USER_ADDR_MAX
}

/* ── 处理函数类型 ── */

type Handler = unsafe extern "C" fn(frame: &SyscallFrame) -> u64;

/// 系统调用派发表（编号 → 处理函数）
static SYSCALL_TABLE: [Option<Handler>; NUM_SYSCALLS] = [
    Some(sys_log as Handler),
    Some(sys_fb_write as Handler),
    Some(sys_mmap as Handler),
    Some(sys_sched_yield as Handler),
    Some(sys_timer_ms as Handler),
    Some(sys_pci_read as Handler),
    Some(sys_pci_write as Handler),
    Some(sys_ai_inference as Handler),
    Some(sys_posix_open as Handler),
    Some(sys_posix_read as Handler),
    Some(sys_posix_write as Handler),
    Some(sys_posix_close as Handler),
    Some(sys_posix_getpid as Handler),
    Some(sys_posix_fork as Handler),
    Some(sys_posix_execve as Handler),
    Some(sys_posix_exit as Handler),
    Some(sys_posix_brk as Handler),
    Some(sys_posix_lseek as Handler),
    Some(sys_posix_stat as Handler),
    Some(sys_posix_fstat as Handler),
    Some(sys_posix_fsync as Handler),
    Some(sys_posix_fdatasync as Handler),
    Some(sys_posix_truncate as Handler),
    Some(sys_posix_ftruncate as Handler),
    Some(sys_posix_getdents as Handler),
    Some(sys_posix_mkdir as Handler),
    Some(sys_posix_rmdir as Handler),
    Some(sys_posix_unlink as Handler),
    Some(sys_posix_rename as Handler),
    Some(sys_posix_chmod as Handler),
    Some(sys_posix_chown as Handler),
    Some(sys_posix_link as Handler),
    Some(sys_posix_symlink as Handler),
    Some(sys_posix_readlink as Handler),
    Some(sys_posix_dup as Handler),
    Some(sys_posix_dup2 as Handler),
    Some(sys_posix_pipe as Handler),
    Some(sys_posix_ioctl as Handler),
    Some(sys_posix_getxattr as Handler),
    Some(sys_posix_setxattr as Handler),
    Some(sys_posix_listxattr as Handler),
    Some(sys_posix_removexattr as Handler),
    Some(sys_posix_access as Handler),
    Some(sys_posix_fcntl as Handler),
    Some(sys_posix_fallocate as Handler),
    Some(sys_posix_sync as Handler),
    Some(sys_posix_umask as Handler),
    Some(sys_posix_gettimeofday as Handler),
    Some(sys_boot_mode as Handler),
    Some(sys_block_list as Handler),
    Some(sys_block_read as Handler),
    Some(sys_block_write as Handler),
    Some(sys_block_flush as Handler),
    Some(sys_posix_getuid as Handler),
    Some(sys_posix_getgid as Handler),
    Some(sys_fb_info as Handler),
    Some(sys_dev_register as Handler),
    Some(sys_dev_unreg as Handler),
    Some(sys_dev_ioctl as Handler),
    Some(sys_ai_load as Handler),
    Some(sys_ai_unload as Handler),
    Some(sys_mount as Handler),
    Some(sys_umount as Handler),
    Some(sys_msg_send as Handler),
];

/* ══════════════════════════════════════════════
   Tungsten 专用 API
   ══════════════════════════════════════════════ */

/// SYS_LOG: 将用户态缓冲区内容输出到串口
unsafe extern "C" fn sys_log(frame: &SyscallFrame) -> u64 {
    let buf = frame.args[0] as *const u8;
    let len = frame.args[1] as usize;
    if !is_user_ptr(frame.args[0]) || len > 4096 {
        return !0u64;
    }
    let slice = unsafe { core::slice::from_raw_parts(buf, len) };
    crate::serial::write_str(slice);
    0
}

/// SYS_FB_WRITE: 向帧缓冲写入单个字符
unsafe extern "C" fn sys_fb_write(frame: &SyscallFrame) -> u64 {
    let ch = frame.args[0] as u8;
    let _color = frame.args[1];
    if ch != 0 {
        crate::console::write_str(core::str::from_utf8(&[ch]).unwrap_or(" "));
    }
    0
}

/// SYS_MMAP: 匿名内存映射
unsafe extern "C" fn sys_mmap(frame: &SyscallFrame) -> u64 {
    let _addr = frame.args[0];
    let len = frame.args[1];
    let prot = frame.args[2];
    if len == 0 || len > 64 * 1024 * 1024 {
        return !0u64;
    }
    let mut vma_prot = crate::mm::vmm::vma_flags::READ;
    if prot & 0x2 != 0 {
        vma_prot |= crate::mm::vmm::vma_flags::WRITE;
    }
    if prot & 0x4 != 0 {
        vma_prot |= crate::mm::vmm::vma_flags::EXEC;
    }
    match crate::mm::vmm::mmap_user(len, vma_prot) {
        Some(va) => va,
        None => !0u64,
    }
}

/// SYS_SCHED_YIELD: 让出 CPU 时间片
unsafe extern "C" fn sys_sched_yield(_frame: &SyscallFrame) -> u64 {
    crate::sched::yield_now();
    0
}

/// SYS_TIMER_MS: 获取系统运行时间（毫秒，TSC 校准）
unsafe extern "C" fn sys_timer_ms(_frame: &SyscallFrame) -> u64 {
    crate::timer::uptime_ms()
}

/// SYS_PCI_READ: PCI 配置空间读取
unsafe extern "C" fn sys_pci_read(frame: &SyscallFrame) -> u64 {
    let bus = frame.args[0] as u8;
    let dev = frame.args[1] as u8;
    let func = frame.args[2] as u8;
    let offset = frame.args[3] as u8;
    let width = frame.args[4] as u32;
    match width {
        1 => crate::devices::pci::config_read8(bus, dev, func, offset) as u64,
        2 => crate::devices::pci::config_read16(bus, dev, func, offset) as u64,
        4 => crate::devices::pci::config_read32(bus, dev, func, offset) as u64,
        _ => !0u64,
    }
}

/// SYS_PCI_WRITE: PCI 配置空间写入
unsafe extern "C" fn sys_pci_write(frame: &SyscallFrame) -> u64 {
    let bus = frame.args[0] as u8;
    let dev = frame.args[1] as u8;
    let func = frame.args[2] as u8;
    let offset = frame.args[3] as u8;
    let val = frame.args[4] as u32;
    crate::devices::pci::config_write32(bus, dev, func, offset, val);
    0
}

/// SYS_AI_INFERENCE: 调用内核 AI 推理引擎
unsafe extern "C" fn sys_ai_inference(frame: &SyscallFrame) -> u64 {
    let _model_id = frame.args[0];
    let input_ptr = frame.args[1] as *const u8;
    let input_len = (frame.args[2] as usize).min(128);
    let output_ptr = frame.args[3] as *mut u8;

    if !is_user_ptr(frame.args[1]) || !is_user_ptr(frame.args[3]) {
        return !0u64;
    }

    let mut input_buf = [0u8; 128];
    unsafe {
        core::ptr::copy_nonoverlapping(input_ptr, input_buf.as_mut_ptr(), input_len);
    }
    let output_slice = unsafe { core::slice::from_raw_parts_mut(output_ptr, 256) };
    let written = crate::ai::infer(&input_buf[..input_len], output_slice);
    written as u64
}

/* ══════════════════════════════════════════════
   安全辅助函数
   ══════════════════════════════════════════════ */

/// 从用户态指针安全读取路径字符串（最多 256 字节）
fn read_user_path(ptr: *const u8) -> Option<[u8; 256]> {
    if !is_user_ptr(ptr as u64) {
        return None;
    }
    let mut path_buf = [0u8; 256];
    for i in 0..256 {
        if (ptr as u64).wrapping_add(i as u64) > USER_ADDR_MAX {
            return None;
        }
        let c = unsafe { core::ptr::read_volatile(ptr.add(i)) };
        if c == 0 {
            return Some(path_buf);
        }
        path_buf[i] = c;
    }
    Some(path_buf)
}

/// 将字节缓冲安全转换为 UTF-8 字符串
fn to_str_safe(buf: &[u8]) -> &str {
    core::str::from_utf8(buf).unwrap_or("")
}

/* ══════════════════════════════════════════════
   POSIX FS 兼容层
   ══════════════════════════════════════════════ */

/// SYS_OPEN: 打开文件
unsafe extern "C" fn sys_posix_open(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let flags = frame.args[1] as i32;
    let path_buf = match read_user_path(path_ptr) {
        Some(b) => b,
        None => return !0u64,
    };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let fd = crate::fs::sys_open(path, flags);
    if fd < 0 { !0u64 } else { fd as u64 }
}

/// SYS_READ: 读取文件
unsafe extern "C" fn sys_posix_read(frame: &SyscallFrame) -> u64 {
    let fd = frame.args[0] as i32;
    let buf = frame.args[1] as *mut u8;
    let count = (frame.args[2] as usize).min(65536);
    if !is_user_ptr(frame.args[1]) { return !0u64; }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, count) };
    let ret = crate::fs::sys_read(fd, slice);
    if ret < 0 { !0u64 } else { ret as u64 }
}

/// SYS_WRITE: 写入文件
unsafe extern "C" fn sys_posix_write(frame: &SyscallFrame) -> u64 {
    let fd = frame.args[0] as i32;
    let buf = frame.args[1] as *const u8;
    let count = (frame.args[2] as usize).min(65536);
    if !is_user_ptr(frame.args[1]) { return !0u64; }
    let slice = unsafe { core::slice::from_raw_parts(buf, count) };
    let ret = crate::fs::sys_write(fd, slice);
    if ret < 0 { !0u64 } else { ret as u64 }
}

/// SYS_CLOSE: 关闭文件
unsafe extern "C" fn sys_posix_close(frame: &SyscallFrame) -> u64 {
    let fd = frame.args[0] as i32;
    let ret = crate::fs::sys_close(fd);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_GETPID: 获取当前进程 ID
unsafe extern "C" fn sys_posix_getpid(_frame: &SyscallFrame) -> u64 {
    crate::proc::sys_getpid()
}

/// SYS_GETUID: 获取真实 UID
unsafe extern "C" fn sys_posix_getuid(_frame: &SyscallFrame) -> u64 {
    crate::proc::sys_getuid() as u64
}

/// SYS_GETGID: 获取真实 GID
unsafe extern "C" fn sys_posix_getgid(_frame: &SyscallFrame) -> u64 {
    crate::proc::sys_getgid() as u64
}

/// SYS_FORK: 创建子进程（预留）
unsafe extern "C" fn sys_posix_fork(_frame: &SyscallFrame) -> u64 {
    !0u64
}

/// SYS_EXECVE: 执行新程序（预留）
unsafe extern "C" fn sys_posix_execve(_frame: &SyscallFrame) -> u64 {
    !0u64
}

/// SYS_EXIT: 退出当前进程
unsafe extern "C" fn sys_posix_exit(frame: &SyscallFrame) -> u64 {
    let code = frame.args[0] as i32;
    crate::sched::exit(code);
}

/// SYS_BRK: 调整堆大小（预留）
unsafe extern "C" fn sys_posix_brk(_frame: &SyscallFrame) -> u64 {
    0
}

/// SYS_LSEEK: 文件定位
unsafe extern "C" fn sys_posix_lseek(frame: &SyscallFrame) -> u64 {
    let fd = frame.args[0] as i32;
    let offset = frame.args[1] as i64;
    let whence = frame.args[2] as i32;
    let ret = crate::fs::sys_lseek(fd, offset, whence);
    if ret < 0 { !0u64 } else { ret as u64 }
}

/// SYS_STAT: 获取文件状态（路径）
unsafe extern "C" fn sys_posix_stat(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let statbuf = frame.args[1] as *mut u8;
    if !is_user_ptr(frame.args[1]) { return !0u64; }
    let path_buf = match read_user_path(path_ptr) {
        Some(b) => b,
        None => return !0u64,
    };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let ret = crate::fs::sys_stat(path, statbuf);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_FSTAT: 获取文件状态（fd）
unsafe extern "C" fn sys_posix_fstat(frame: &SyscallFrame) -> u64 {
    let fd = frame.args[0] as i32;
    let statbuf = frame.args[1] as *mut u8;
    if !is_user_ptr(frame.args[1]) { return !0u64; }
    let ret = crate::fs::sys_fstat(fd, statbuf);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_FSYNC: 同步文件数据到磁盘
unsafe extern "C" fn sys_posix_fsync(frame: &SyscallFrame) -> u64 {
    let fd = frame.args[0] as i32;
    let ret = crate::fs::sys_fsync(fd);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_FDATASYNC: 同步文件数据（不含元数据）
unsafe extern "C" fn sys_posix_fdatasync(frame: &SyscallFrame) -> u64 {
    let fd = frame.args[0] as i32;
    let ret = crate::fs::sys_fdatasync(fd);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_TRUNCATE: 截断文件
unsafe extern "C" fn sys_posix_truncate(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let length = frame.args[1];
    let path_buf = match read_user_path(path_ptr) {
        Some(b) => b,
        None => return !0u64,
    };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let ret = crate::fs::sys_truncate(path, length);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_FTRUNCATE: 截断文件（fd）
unsafe extern "C" fn sys_posix_ftruncate(frame: &SyscallFrame) -> u64 {
    let fd = frame.args[0] as i32;
    let length = frame.args[1];
    let ret = crate::fs::sys_ftruncate(fd, length);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_GETDENTS: 读取目录项
unsafe extern "C" fn sys_posix_getdents(frame: &SyscallFrame) -> u64 {
    let fd = frame.args[0] as i32;
    let dirp = frame.args[1] as *mut u8;
    let count = (frame.args[2] as usize).min(65536);
    if !is_user_ptr(frame.args[1]) { return !0u64; }
    let slice = unsafe { core::slice::from_raw_parts_mut(dirp, count) };
    let ret = crate::fs::sys_getdents(fd, slice);
    if ret < 0 { !0u64 } else { ret as u64 }
}

/// SYS_MKDIR: 创建目录
unsafe extern "C" fn sys_posix_mkdir(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let path_buf = match read_user_path(path_ptr) {
        Some(b) => b,
        None => return !0u64,
    };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let ret = crate::fs::sys_mkdir(path);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_RMDIR: 删除目录
unsafe extern "C" fn sys_posix_rmdir(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let path_buf = match read_user_path(path_ptr) {
        Some(b) => b,
        None => return !0u64,
    };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let ret = crate::fs::sys_rmdir(path);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_UNLINK: 删除文件
unsafe extern "C" fn sys_posix_unlink(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let path_buf = match read_user_path(path_ptr) {
        Some(b) => b,
        None => return !0u64,
    };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let ret = crate::fs::sys_unlink(path);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_RENAME: 重命名文件
unsafe extern "C" fn sys_posix_rename(frame: &SyscallFrame) -> u64 {
    let old_ptr = frame.args[0] as *const u8;
    let new_ptr = frame.args[1] as *const u8;
    let old_buf = match read_user_path(old_ptr) { Some(b) => b, None => return !0u64 };
    let new_buf = match read_user_path(new_ptr) { Some(b) => b, None => return !0u64 };
    let old = to_str_safe(&old_buf[..old_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let new = to_str_safe(&new_buf[..new_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let ret = crate::fs::sys_rename(old, new);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_CHMOD: 修改文件权限
unsafe extern "C" fn sys_posix_chmod(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let mode = frame.args[1] as u16;
    let path_buf = match read_user_path(path_ptr) { Some(b) => b, None => return !0u64 };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let ret = crate::fs::sys_chmod(path, mode);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_CHOWN: 修改文件所有者
unsafe extern "C" fn sys_posix_chown(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let owner = frame.args[1] as u32;
    let group = frame.args[2] as u32;
    let path_buf = match read_user_path(path_ptr) { Some(b) => b, None => return !0u64 };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let ret = crate::fs::sys_chown(path, owner, group);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_LINK: 创建硬链接（预留）
unsafe extern "C" fn sys_posix_link(_frame: &SyscallFrame) -> u64 {
    !0u64
}

/// SYS_SYMLINK: 创建软链接
unsafe extern "C" fn sys_posix_symlink(frame: &SyscallFrame) -> u64 {
    let target_ptr = frame.args[0] as *const u8;
    let link_ptr = frame.args[1] as *const u8;
    let target_buf = match read_user_path(target_ptr) { Some(b) => b, None => return !0u64 };
    let link_buf = match read_user_path(link_ptr) { Some(b) => b, None => return !0u64 };
    let target = to_str_safe(&target_buf[..target_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let link = to_str_safe(&link_buf[..link_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let ret = crate::fs::sys_symlink(target, link);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_READLINK: 读取软链接目标
unsafe extern "C" fn sys_posix_readlink(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let buf = frame.args[1] as *mut u8;
    let bufsiz = (frame.args[2] as usize).min(4096);
    if !is_user_ptr(frame.args[1]) { return !0u64; }
    let path_buf = match read_user_path(path_ptr) { Some(b) => b, None => return !0u64 };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, bufsiz) };
    let ret = crate::fs::sys_readlink(path, slice);
    if ret < 0 { !0u64 } else { ret as u64 }
}

/// SYS_DUP: 复制文件描述符
unsafe extern "C" fn sys_posix_dup(frame: &SyscallFrame) -> u64 {
    let oldfd = frame.args[0] as i32;
    let ret = crate::fs::sys_dup(oldfd);
    if ret < 0 { !0u64 } else { ret as u64 }
}

/// SYS_DUP2: 复制文件描述符到指定新 fd
unsafe extern "C" fn sys_posix_dup2(frame: &SyscallFrame) -> u64 {
    let oldfd = frame.args[0] as i32;
    let newfd = frame.args[1] as i32;
    let ret = crate::fs::sys_dup2(oldfd, newfd);
    if ret < 0 { !0u64 } else { ret as u64 }
}

/// SYS_PIPE: 创建管道
unsafe extern "C" fn sys_posix_pipe(frame: &SyscallFrame) -> u64 {
    let pipefd = frame.args[0] as *mut i32;
    if !is_user_ptr(frame.args[0]) {
        return !0u64;
    }
    let ret = crate::fs::sys_pipe(pipefd);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_IOCTL: 设备 I/O 控制
unsafe extern "C" fn sys_posix_ioctl(frame: &SyscallFrame) -> u64 {
    let _fd = frame.args[0] as i32;
    !0u64
}

/// SYS_GETXATTR: 获取扩展属性
unsafe extern "C" fn sys_posix_getxattr(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let name_ptr = frame.args[1] as *const u8;
    let value_buf = frame.args[2] as *mut u8;
    let size = (frame.args[3] as usize).min(65536);
    if !is_user_ptr(frame.args[2]) { return !0u64; }
    let path_buf = match read_user_path(path_ptr) { Some(b) => b, None => return !0u64 };
    let name_buf = match read_user_path(name_ptr) { Some(b) => b, None => return !0u64 };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let name = to_str_safe(&name_buf[..name_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let slice = unsafe { core::slice::from_raw_parts_mut(value_buf, size) };
    let ret = crate::fs::sys_getxattr(path, name, slice);
    if ret < 0 { !0u64 } else { ret as u64 }
}

/// SYS_SETXATTR: 设置扩展属性
unsafe extern "C" fn sys_posix_setxattr(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let name_ptr = frame.args[1] as *const u8;
    let value_ptr = frame.args[2] as *const u8;
    let size = (frame.args[3] as usize).min(65536);
    if !is_user_ptr(frame.args[2]) { return !0u64; }
    let path_buf = match read_user_path(path_ptr) { Some(b) => b, None => return !0u64 };
    let name_buf = match read_user_path(name_ptr) { Some(b) => b, None => return !0u64 };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let name = to_str_safe(&name_buf[..name_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let value = unsafe { core::slice::from_raw_parts(value_ptr, size) };
    let ret = crate::fs::sys_setxattr(path, name, value);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_LISTXATTR: 列出扩展属性
unsafe extern "C" fn sys_posix_listxattr(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let list = frame.args[1] as *mut u8;
    let size = (frame.args[2] as usize).min(65536);
    if !is_user_ptr(frame.args[1]) { return !0u64; }
    let path_buf = match read_user_path(path_ptr) { Some(b) => b, None => return !0u64 };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let slice = unsafe { core::slice::from_raw_parts_mut(list, size) };
    let ret = crate::fs::sys_listxattr(path, slice);
    if ret < 0 { !0u64 } else { ret as u64 }
}

/// SYS_REMOVEXATTR: 删除扩展属性
unsafe extern "C" fn sys_posix_removexattr(frame: &SyscallFrame) -> u64 {
    let path_ptr = frame.args[0] as *const u8;
    let name_ptr = frame.args[1] as *const u8;
    let path_buf = match read_user_path(path_ptr) { Some(b) => b, None => return !0u64 };
    let name_buf = match read_user_path(name_ptr) { Some(b) => b, None => return !0u64 };
    let path = to_str_safe(&path_buf[..path_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let name = to_str_safe(&name_buf[..name_buf.iter().position(|&c| c == 0).unwrap_or(256)]);
    let ret = crate::fs::sys_removexattr(path, name);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_ACCESS: 检查文件访问权限
unsafe extern "C" fn sys_posix_access(frame: &SyscallFrame) -> u64 {
    let _path_ptr = frame.args[0] as *const u8;
    0
}

/// SYS_FCNTL: 文件描述符控制
unsafe extern "C" fn sys_posix_fcntl(frame: &SyscallFrame) -> u64 {
    match frame.args[1] as i32 {
        1 => 0, // F_GETFD
        2 => 0, // F_SETFD
        3 => 0, // F_GETFL
        4 => 0, // F_SETFL
        _ => !0u64,
    }
}

/// SYS_FALLOCATE: 预分配文件空间
unsafe extern "C" fn sys_posix_fallocate(frame: &SyscallFrame) -> u64 {
    let fd = frame.args[0] as i32;
    let offset = frame.args[2];
    let len = frame.args[3];
    let ret = crate::fs::sys_fallocate(fd, offset, len);
    if ret < 0 { !0u64 } else { 0 }
}

/// SYS_SYNC: 同步所有文件系统
unsafe extern "C" fn sys_posix_sync(_frame: &SyscallFrame) -> u64 {
    crate::fs::sys_sync();
    0
}

/// SYS_UMASK: 设置文件创建掩码
unsafe extern "C" fn sys_posix_umask(frame: &SyscallFrame) -> u64 {
    let _mask = frame.args[0] as u16;
    0o022
}

/// SYS_GETTIMEOFDAY: 获取当前时间
unsafe extern "C" fn sys_posix_gettimeofday(frame: &SyscallFrame) -> u64 {
    let tv = frame.args[0] as *mut u8;
    if !is_user_ptr(frame.args[0]) { return !0u64; }
    if !tv.is_null() {
        let ms = crate::timer::uptime_ms();
        let sec = ms / 1000;
        let usec = (ms % 1000) * 1000;
        let buf = unsafe { core::slice::from_raw_parts_mut(tv, 16) };
        buf[0..8].copy_from_slice(&sec.to_le_bytes());
        buf[8..16].copy_from_slice(&usec.to_le_bytes());
    }
    0
}

/// SYS_FB_INFO: 查询帧缓冲参数（40 字节结构）
unsafe extern "C" fn sys_fb_info(frame: &SyscallFrame) -> u64 {
    let buf = frame.args[0] as *mut u8;
    let len = frame.args[1] as usize;
    if !is_user_ptr(frame.args[0]) || len < 40 {
        return !0u64;
    }
    let Some(bi) = crate::limine_boot::cached_boot_info() else {
        return !0u64;
    };
    let mut out = [0u8; 40];
    out[0..8].copy_from_slice(&bi.fb_width.to_le_bytes());
    out[8..16].copy_from_slice(&bi.fb_height.to_le_bytes());
    out[16..24].copy_from_slice(&bi.fb_pitch.to_le_bytes());
    out[24..32].copy_from_slice(&(bi.fb_bpp as u64).to_le_bytes());
    out[32..40].copy_from_slice(&bi.fb_addr.to_le_bytes());
    unsafe {
        core::ptr::copy_nonoverlapping(out.as_ptr(), buf, 40);
    }
    0
}

/// SYS_DEV_REGISTER: Ring 2 设备注册（接受路径与类型，当前返回成功）
unsafe extern "C" fn sys_dev_register(_frame: &SyscallFrame) -> u64 {
    0
}

/// SYS_DEV_UNREG: 注销设备
unsafe extern "C" fn sys_dev_unreg(_frame: &SyscallFrame) -> u64 {
    0
}

/// SYS_DEV_IOCTL: 设备控制
unsafe extern "C" fn sys_dev_ioctl(_frame: &SyscallFrame) -> u64 {
    0
}

/// SYS_AI_LOAD: 加载 AI 模型权重
unsafe extern "C" fn sys_ai_load(_frame: &SyscallFrame) -> u64 {
    0
}

/// SYS_AI_UNLOAD: 卸载 AI 模型
unsafe extern "C" fn sys_ai_unload(_frame: &SyscallFrame) -> u64 {
    0
}

/// SYS_MOUNT: 挂载文件系统
unsafe extern "C" fn sys_mount(frame: &SyscallFrame) -> u64 {
    let _ = frame;
    0
}

/// SYS_UMOUNT: 卸载文件系统
unsafe extern "C" fn sys_umount(_frame: &SyscallFrame) -> u64 {
    0
}

/// SYS_MSG_SEND: 进程间消息发送
unsafe extern "C" fn sys_msg_send(_frame: &SyscallFrame) -> u64 {
    0
}

/// SYS_BOOT_MODE: 获取引导模式（0=正常, 1=安装程序）
unsafe extern "C" fn sys_boot_mode(_frame: &SyscallFrame) -> u64 {
    if crate::limine_boot::is_installer_boot() { 1 } else { 0 }
}

/// SYS_BLOCK_LIST: 列出块设备到用户缓冲区
unsafe extern "C" fn sys_block_list(frame: &SyscallFrame) -> u64 {
    let buf = frame.args[0] as *mut u8;
    let len = (frame.args[1] as usize).min(4096);
    if !is_user_ptr(frame.args[0]) || len == 0 {
        return !0u64;
    }
    let mut kbuf = [0u8; 4096];
    let n = crate::block::list_devices(&mut kbuf[..len]);
    unsafe {
        core::ptr::copy_nonoverlapping(kbuf.as_ptr(), buf, n);
    }
    n as u64
}

/// SYS_BLOCK_READ: 从块设备读扇区到用户缓冲区
unsafe extern "C" fn sys_block_read(frame: &SyscallFrame) -> u64 {
    let dev = frame.args[0] as usize;
    let lba = frame.args[1];
    let count = frame.args[2] as u32;
    let buf = frame.args[3] as *mut u8;
    let byte_len = (count as u64).saturating_mul(512) as usize;
    if count == 0 || byte_len > 65536 || !is_user_ptr(frame.args[3]) {
        return !0u64;
    }
    let mut kbuf = [0u8; 65536];
    let ret = crate::block::block_read_sectors(dev, lba, count, &mut kbuf[..byte_len]);
    if ret < 0 {
        return (!ret as u64) | (!0u64 << 32);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(kbuf.as_ptr(), buf, byte_len);
    }
    ret as u64
}

/// SYS_BLOCK_WRITE: 向块设备写扇区
unsafe extern "C" fn sys_block_write(frame: &SyscallFrame) -> u64 {
    let dev = frame.args[0] as usize;
    let lba = frame.args[1];
    let count = frame.args[2] as u32;
    let buf = frame.args[3] as *const u8;
    let byte_len = (count as u64).saturating_mul(512) as usize;
    if count == 0 || byte_len > 65536 || !is_user_ptr(frame.args[3]) {
        return !0u64;
    }
    let slice = unsafe { core::slice::from_raw_parts(buf, byte_len) };
    let ret = crate::block::block_write_sectors(dev, lba, count, slice);
    if ret < 0 {
        return (!ret as u64) | (!0u64 << 32);
    }
    ret as u64
}

/// SYS_BLOCK_FLUSH: 刷新块设备缓存
unsafe extern "C" fn sys_block_flush(frame: &SyscallFrame) -> u64 {
    let dev = frame.args[0] as usize;
    let ret = crate::block::block_flush(dev);
    if ret < 0 { !0u64 } else { 0 }
}

/* ══════════════════════════════════════════════
   系统调用派发
   ══════════════════════════════════════════════ */

/// 系统调用派发入口（由 arch/x86_64/syscall.rs 的 syscall_entry 调用）
///
/// 根据 SyscallFrame.num 查找派发表中的处理函数并执行。
/// 特殊编号 SYS_RING2_COMPLETION 用于 Ring 2 I/O 子系统完成通知。
/// 返回值为系统调用结果，!0u64 表示错误。
#[unsafe(no_mangle)]
unsafe extern "C" fn syscall_handler(frame: &SyscallFrame) -> u64 {
    dispatch(frame.num, frame)
}

/// 通用系统调用派发函数
///
/// `nr` 为系统调用编号，`frame` 包含参数和上下文。
/// 返回处理结果，!0u64 表示无效编号或执行失败。
pub fn dispatch(nr: u64, frame: &SyscallFrame) -> u64 {
    // Ring 2 完成通知（特殊通道）
    if nr == SYS_RING2_COMPLETION {
        crate::fs::ring2_interface::handle_ring2_completion(
            frame.args[0],
            frame.args[1] as i32,
        );
        return 0;
    }
    // 查派发表
    if (nr as usize) < NUM_SYSCALLS {
        if let Some(handler) = SYSCALL_TABLE[nr as usize] {
            return unsafe { handler(frame) };
        }
    }
    // 检查 arch 层自定义处理函数
    if let Some(handler) = crate::arch::x86_64::syscall::get_handler(nr) {
        return handler(frame);
    }
    !0u64
}
