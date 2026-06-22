// sem/mod.rs — POSIX 信号量 (semget/semop/semctl)
// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later

use crate::sync::SpinLock;
#[repr(C)] pub struct Semaphore { pub id: i32, pub key: i32, pub value: i32, pub flags: u16 }
pub struct SemTable { pub sems: [Option<Semaphore>; 128], pub count: usize }
static SEM_TABLE: SpinLock<SemTable> = SpinLock::new(SemTable { sems: [const { None }; 128], count: 0 });
pub fn init() { crate::serial::write_str(b"sem: ready\n"); }
pub fn sys_semget(_key: i32, _nsems: i32, _flags: i32) -> i32 { -1 }
pub fn sys_semop(_id: i32, _ops: *const (), _nops: usize) -> i32 { -1 }
pub fn sys_semctl(_id: i32, _semnum: i32, _cmd: i32, _arg: *mut ()) -> i32 { -1 }
