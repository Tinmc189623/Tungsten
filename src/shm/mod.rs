// shm/mod.rs — POSIX 共享内存 (shmget/shmat/shmdt/shmctl)
// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later

use crate::sync::SpinLock;
pub const SHM_SIZE: usize = 4096; pub const SHM_MAX_SEGMENTS: usize = 128;
#[repr(C)] pub struct ShmSegment { pub id: i32, pub key: i32, pub size: usize, pub addr: u64, pub nattch: u16, pub flags: u16, pub creator_pid: u64, pub ctime: u64 }
pub struct ShmTable { pub segments: [Option<ShmSegment>; SHM_MAX_SEGMENTS], pub count: usize }
unsafe impl Send for ShmTable {}
static SHM_TABLE: SpinLock<ShmTable> = SpinLock::new(ShmTable { segments: [const { None }; SHM_MAX_SEGMENTS], count: 0 });
pub fn sys_shmget(_key: i32, _size: usize, _flags: i32) -> i32 { -1 }
pub fn sys_shmat(_id: i32, _addr: *const (), _flags: i32) -> *mut () { core::ptr::null_mut() }
pub fn sys_shmdt(_addr: *const ()) -> i32 { -1 }
pub fn sys_shmctl(_id: i32, _cmd: i32, _buf: *mut ()) -> i32 { -1 }
pub fn init() { crate::serial::write_str(b"shm: ready\n"); }
