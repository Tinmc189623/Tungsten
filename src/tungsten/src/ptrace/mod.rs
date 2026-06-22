// ptrace/mod.rs — 进程跟踪/调试接口 (ptrace, 断点, 单步)
// Copyright (C) 2026 Nexsteaduser. All rights reserved. SPDX-License-Identifier: GPL-3.0-or-later

use crate::sync::SpinLock;
pub struct PtraceContext { pub pid: u64, pub attached: bool, pub breakpoints: [u64; 32], pub bp_count: u8 }
pub struct PtraceManager { pub contexts: [Option<PtraceContext>; 64], pub count: usize }
static PTRACE_MGR: SpinLock<PtraceManager> = SpinLock::new(PtraceManager { contexts: [const { None }; 64], count: 0 });
pub fn init() { crate::serial::write_str(b"ptrace: ready\n"); }
pub fn sys_ptrace(_req: i32, _pid: u64, _addr: *mut (), _data: *mut ()) -> i64 { -1 }
